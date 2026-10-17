//! Prompt enhancer: turns a rough app idea ("a recipe site") into a detailed, buildable
//! brief before Genius plans it. One short, non-streaming model call; nothing is stored and
//! no project is touched.

use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::app::Genius;
use crate::error::{GeniusError, Result};
use crate::events::{EventKind, EventStatus};
use crate::nvidia::{ChatMessage, ChatOptions, ProviderError};

/// Shortest idea worth improving (in characters, after trimming).
pub const MIN_CHARS: usize = 3;
/// Longest idea accepted (in characters, after trimming).
pub const MAX_CHARS: usize = 4000;

const TEMPERATURE: f32 = 0.4;
const MAX_TOKENS: u32 = 700;

const SYSTEM: &str = "You are the prompt editor of Genius, an AI software engineer that builds complete client-side web apps from one description.
Rewrite the user's rough idea into a clear, specific brief that Genius can build in one go.

Rules:
- Reply with the brief only. No preamble (\"Here is…\"), no closing remarks, no code, no code fences, no markdown headings.
- Write in the same language as the user's idea, including the labels below.
- Keep every concrete detail the user gave (names, colours, features, audience) and never contradict them. Fill the gaps with sensible, specific choices.
- Stay under 180 words.
- Format: one opening sentence with the goal and the audience, then short labelled lines:
  Pages/sections: …
  Key features: …
  Data: …
  Visual style: …
  Must-haves: …
- Unless the user asked for a backend, data is saved in the browser (localStorage). Use realistic sample content that is clearly labelled as sample data.
- Do not add sign-ups, payments or integrations that need API keys unless the user asked for them; mock them in the UI instead.
- Must-haves always include a responsive layout and accessible, keyboard-friendly controls.";

/// Messages for one enhancement call. The idea is fenced so instructions inside it are
/// treated as content to rewrite, not as instructions to the editor.
pub fn enhance_messages(idea: &str) -> Vec<ChatMessage> {
    vec![
        ChatMessage::system(SYSTEM),
        ChatMessage::user(format!(
            "Rough idea (between the markers):\n<<<IDEA\n{}\nIDEA>>>\n\nWrite the improved brief now.",
            idea.trim()
        )),
    ]
}

/// Trimmed idea, or a validation error explaining the accepted length.
pub fn validate_idea(text: &str) -> Result<&str> {
    let t = text.trim();
    let n = t.chars().count();
    if n < MIN_CHARS {
        return Err(GeniusError::Validation("Type a few words about your idea first.".into()));
    }
    if n > MAX_CHARS {
        return Err(GeniusError::Validation(format!("That description is too long to improve ({n} characters, the limit is {MAX_CHARS}).")));
    }
    Ok(t)
}

/// Removes everything around the brief that models like to add: reasoning sections, code
/// fences, wrapping quotes, "Here is your prompt:" lead-ins, heading and bold markers.
pub fn clean_reply(raw: &str) -> String {
    let mut s = strip_think(raw);

    // A reply wrapped entirely in a code fence: keep the inside.
    let trimmed = s.trim();
    if trimmed.starts_with("```") {
        let inner = trimmed.trim_start_matches('`');
        let inner = match inner.find('\n') {
            Some(i) => &inner[i + 1..], // drop the info string ("text", "markdown")
            None => inner,
        };
        s = inner.trim_end().trim_end_matches('`').to_string();
    }

    let mut lines: Vec<String> = s
        .lines()
        .filter(|l| !l.trim_start().starts_with("```"))
        .map(|l| {
            let l = l.trim_end();
            let t = l.trim_start();
            let t = if t.starts_with('#') { t.trim_start_matches('#').trim_start() } else { l };
            t.replace("**", "").replace("__", "")
        })
        .collect();

    // Lead-in line ("Here's an improved version:", "Improved prompt:").
    while let Some(first) = lines.iter().position(|l| !l.trim().is_empty()) {
        if is_lead_in(&lines[first]) {
            lines.drain(..=first);
        } else {
            break;
        }
    }

    // Collapse runs of blank lines.
    let mut out = String::new();
    let mut blank = 0;
    for l in lines {
        if l.trim().is_empty() {
            blank += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank > 0 { "\n\n" } else { "\n" });
        }
        blank = 0;
        out.push_str(&l);
    }

    unquote(out.trim()).trim().to_string()
}

fn strip_think(raw: &str) -> String {
    let mut s = raw.to_string();
    while let Some(start) = s.find("<think>") {
        match s[start..].find("</think>") {
            Some(end) => s.replace_range(start..start + end + "</think>".len(), ""),
            None => s.truncate(start), // unterminated: everything after is reasoning
        }
    }
    // A closing tag without an opening one: the reasoning came before it.
    if let Some(end) = s.find("</think>") {
        s.replace_range(..end + "</think>".len(), "");
    }
    s
}

fn is_lead_in(line: &str) -> bool {
    let l = line.trim().to_lowercase();
    if l.chars().count() > 90 || !l.ends_with(':') {
        return false;
    }
    ["here", "sure", "certainly", "okay", "ok,", "improved", "enhanced", "rewritten", "revised", "refined", "your improved", "your brief", "brief:", "prompt:"]
        .iter()
        .any(|p| l.starts_with(p))
}

fn unquote(s: &str) -> &str {
    for (a, b) in [('"', '"'), ('“', '”'), ('\'', '\''), ('«', '»'), ('„', '“')] {
        if s.chars().count() >= 2 && s.starts_with(a) && s.ends_with(b) {
            let inner = &s[a.len_utf8()..s.len() - b.len_utf8()];
            // Only when the quotes wrap the whole reply, not a quoted phrase at both ends.
            if !inner.contains(a) && !inner.contains(b) {
                return inner;
            }
        }
    }
    s
}

impl Genius {
    /// The model used to improve prompts. Kept in one place so it can move to a faster
    /// model without touching the call itself.
    fn enhance_model(&self) -> Result<String> {
        let s = self.settings()?;
        s.model.filter(|m| !m.trim().is_empty()).ok_or_else(|| ProviderError::NoModel.into())
    }

    /// Rewrites a rough idea into a detailed, buildable brief in the idea's language.
    pub async fn enhance_prompt(&self, text: &str, cancel: &CancellationToken) -> Result<String> {
        let idea = validate_idea(text)?;
        let (client, _s) = self.client()?;
        let model = self.enhance_model()?;
        let opts = ChatOptions { temperature: TEMPERATURE, top_p: 0.9, max_tokens: MAX_TOKENS, stream: false };
        let mut ev = self.bus.start(None, None, EventKind::Ai, EventStatus::Running, format!("Improving your prompt ({model})"), json!({ "model": model, "purpose": "enhance prompt", "promptChars": idea.len() }));
        match client.chat(&model, &enhance_messages(idea), &opts, cancel, |_| {}).await {
            Ok(r) => {
                let brief = clean_reply(&r.content);
                if brief.chars().count() < MIN_CHARS {
                    let e = GeniusError::from(ProviderError::Malformed("the model returned an empty brief".into()));
                    ev.update(EventStatus::Failed, json!({ "error": { "kind": e.kind(), "message": e.to_string() } }));
                    return Err(e);
                }
                ev.update(EventStatus::Completed, json!({ "outputChars": brief.len(), "finishReason": r.finish_reason }));
                Ok(brief)
            }
            Err(e) => {
                let e = GeniusError::from(e);
                ev.update(if e.kind() == "cancelled" { EventStatus::Cancelled } else { EventStatus::Failed }, json!({ "error": { "kind": e.kind(), "message": e.to_string() } }));
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn messages_fence_the_idea_and_state_the_rules() {
        let m = enhance_messages("  a recipe site  ");
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].role, "system");
        for rule in ["same language", "under 180 words", "localStorage", "sample", "no markdown headings", "Pages/sections:", "Must-haves:"] {
            assert!(m[0].content.to_lowercase().contains(&rule.to_lowercase()), "system prompt mentions {rule}");
        }
        assert_eq!(m[1].role, "user");
        assert!(m[1].content.contains("<<<IDEA\na recipe site\nIDEA>>>"));
    }

    #[test]
    fn validates_length_in_characters() {
        assert!(matches!(validate_idea("  hi  "), Err(GeniusError::Validation(_))));
        assert_eq!(validate_idea("  app ").unwrap(), "app");
        assert_eq!(validate_idea("日本語").unwrap(), "日本語", "three characters, nine bytes");
        assert!(validate_idea(&"x".repeat(MAX_CHARS)).is_ok());
        assert!(matches!(validate_idea(&"x".repeat(MAX_CHARS + 1)), Err(GeniusError::Validation(_))));
    }

    #[test]
    fn cleanup_strips_reasoning_fences_quotes_and_lead_ins() {
        assert_eq!(clean_reply("<think>plan it</think>\nA recipe site."), "A recipe site.");
        assert_eq!(clean_reply("reasoning…</think>A recipe site."), "A recipe site.");
        assert_eq!(clean_reply("A recipe site.<think>unfinished"), "A recipe site.");
        assert_eq!(clean_reply("```text\nA recipe site.\nKey features: tags\n```"), "A recipe site.\nKey features: tags");
        assert_eq!(clean_reply("\"A recipe site for home cooks.\""), "A recipe site for home cooks.");
        assert_eq!(clean_reply("“Un site de recettes.”"), "Un site de recettes.");
        assert_eq!(clean_reply("Here is an improved prompt:\n\nA recipe site."), "A recipe site.");
        assert_eq!(clean_reply("Sure! Here's your brief:\nA recipe site."), "A recipe site.");
        assert_eq!(clean_reply("## Goal\nA recipe site.\n\n\n\n**Key features:** tags, search"), "Goal\nA recipe site.\n\nKey features: tags, search");
    }

    #[test]
    fn cleanup_keeps_legitimate_content() {
        let brief = "A habit tracker for students.\nPages/sections: today, history\nData: saved in localStorage";
        assert_eq!(clean_reply(brief), brief);
        // A label line is not a lead-in.
        assert_eq!(clean_reply("Key features:\n- streaks"), "Key features:\n- streaks");
        // Quotes inside the text stay.
        assert_eq!(clean_reply("\"Fast\" checkout and \"easy\" returns"), "\"Fast\" checkout and \"easy\" returns");
    }

    fn genius(dir: &std::path::Path) -> Arc<Genius> {
        Genius::new(dir, Arc::new(crate::events::NullEmitter), Arc::new(crate::secrets::MemoryStore::default())).unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn enhances_through_the_provider_and_cleans_the_reply() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(json!({ "model": "mock/coder", "max_tokens": 700, "stream": false })))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                json!({ "choices": [{ "message": { "role": "assistant", "content": "<think>ok</think>Here is the improved prompt:\n```\nA recipe site for home cooks.\nKey features: tags, search\n```" }, "finish_reason": "stop" }] }).to_string(),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let g = genius(dir.path());
        let mut s = g.settings().unwrap();
        s.base_url = format!("http://127.0.0.1:{}/v1", server.address().port());
        s.model = Some("mock/coder".into());
        g.save_settings(&s).unwrap();
        g.save_api_key("nvapi-enhance-test-key-0123456789").unwrap();

        let out = g.enhance_prompt("  recipe site ", &CancellationToken::new()).await.unwrap();
        assert_eq!(out, "A recipe site for home cooks.\nKey features: tags, search");
        let req = &server.received_requests().await.unwrap()[0];
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        assert!((body["temperature"].as_f64().unwrap() - 0.4).abs() < 1e-6);
        assert!(body["messages"][1]["content"].as_str().unwrap().contains("<<<IDEA\nrecipe site\nIDEA>>>"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rejects_bad_input_and_missing_model_before_calling() {
        let dir = tempfile::tempdir().unwrap();
        let g = genius(dir.path());
        let err = g.enhance_prompt("x", &CancellationToken::new()).await.unwrap_err();
        assert_eq!(err.kind(), "validation");
        let err = g.enhance_prompt("a recipe site", &CancellationToken::new()).await.unwrap_err();
        assert_eq!(err.kind(), "provider_no_model");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn empty_reply_is_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"choices":[{"message":{"role":"assistant","content":"<think>hmm</think>  "},"finish_reason":"stop"}]}"#))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let g = genius(dir.path());
        let mut s = g.settings().unwrap();
        s.base_url = format!("http://127.0.0.1:{}/v1", server.address().port());
        s.model = Some("mock/coder".into());
        g.save_settings(&s).unwrap();
        g.save_api_key("nvapi-enhance-test-key-0123456789").unwrap();
        let err = g.enhance_prompt("a recipe site", &CancellationToken::new()).await.unwrap_err();
        assert_eq!(err.kind(), "provider_malformed");
    }
}
