#![allow(clippy::collapsible_if)]
//! Model identity, request metadata and image routing of Genius Chat.
//!
//! The NVIDIA API is replaced by a scripted wiremock server that records every request body, so
//! these tests prove what Genius really *sends* (model id, image payload, system prompt) and what
//! it *stores* (message metadata), against a stand-in. They do NOT prove that a live NVIDIA model
//! accepts the payload or answers well: see `live_nvidia.rs` for the opt-in live test.

use base64::Engine;
use genius_core::app::Genius;
use genius_core::chat_mode::ChatRequest;
use genius_core::db::MessageRow;
use genius_core::events::{EventKind, EventStatus, NullEmitter};
use genius_core::secrets::MemoryStore;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const KEY: &str = "nvapi-identity-test-key-0123456789";
const VISION: &str = "meta/llama-3.2-90b-vision-instruct";
const TEXT: &str = "nvidia/nemotron-3-super-120b-a12b";

/// A real PNG signature followed by filler: enough for the magic-number check, not a decodable picture.
fn png() -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    b.extend((0..64u8).map(|i| i.wrapping_mul(7)));
    b
}

type Reply = dyn Fn(&Value) -> String + Send + Sync;

struct Scripted {
    reply: Box<Reply>,
    /// The model id the stand-in puts in its response (what a provider "reports").
    reported: Option<String>,
    log: Arc<Mutex<Vec<Value>>>,
}

impl Respond for Scripted {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        self.log.lock().unwrap().push(body.clone());
        let text = (self.reply)(&body);
        let mut s = String::new();
        for chunk in text.chars().collect::<Vec<_>>().chunks(40) {
            let c: String = chunk.iter().collect();
            let mut v = json!({"choices":[{"index":0,"delta":{"content":c},"finish_reason":null}]});
            if let Some(m) = &self.reported {
                v["model"] = json!(m);
            }
            s.push_str(&format!("data: {v}\n\n"));
        }
        s.push_str("data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":7,\"total_tokens\":18}}\n\ndata: [DONE]\n\n");
        ResponseTemplate::new(200).set_body_raw(s, "text/event-stream")
    }
}

struct Env {
    _tmp: tempfile::TempDir,
    g: Arc<Genius>,
    log: Arc<Mutex<Vec<Value>>>,
    _server: MockServer,
}

/// `manual`: `Some(model)` selects that model in manual mode; `None` is Auto with `listed` as the key's models.
async fn setup(manual: Option<&str>, listed: &[&str], reported: Option<&str>, reply: impl Fn(&Value) -> String + Send + Sync + 'static) -> Env {
    let server = MockServer::start().await;
    let log = Arc::new(Mutex::new(Vec::new()));
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(Scripted { reply: Box::new(reply), reported: reported.map(String::from), log: log.clone() }).mount(&server).await;
    let data: Vec<Value> = listed.iter().map(|id| json!({ "id": id, "object": "model" })).collect();
    Mock::given(method("GET")).and(path("/v1/models")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": data }))).mount(&server).await;
    let tmp = tempfile::tempdir().unwrap();
    let g = Genius::new(&tmp.path().join("data"), Arc::new(NullEmitter), Arc::new(MemoryStore::default())).unwrap();
    let mut s = g.settings().unwrap();
    s.base_url = format!("http://127.0.0.1:{}/v1", server.address().port());
    s.workspace_dir = Some(tmp.path().join("projects").to_string_lossy().to_string());
    match manual {
        Some(m) => {
            s.model_mode = "manual".into();
            s.model = Some(m.into());
        }
        None => {
            s.model_mode = "auto".into();
            s.model = Some("z-ai/glm-5.3".into()); // a stale preference: Auto must not treat it as the served model
        }
    }
    g.save_settings(&s).unwrap();
    g.save_api_key(KEY).unwrap();
    Env { _tmp: tmp, g, log, _server: server }
}

async fn wait_idle(g: &Arc<Genius>, pid: &str) {
    for _ in 0..800 {
        if g.active_tasks(Some(pid)).is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("task did not finish");
}

async fn send(g: &Arc<Genius>, conv: &str, message: &str, attachments: &[&str]) -> (EventStatus, Vec<MessageRow>) {
    let pid = g.chat_conversation(conv).unwrap().row.project_id;
    let t = g.start_chat(conv, ChatRequest { message: message.into(), attachment_ids: attachments.iter().map(|s| s.to_string()).collect(), effort: Some("balanced".into()) }).unwrap();
    wait_idle(g, &pid).await;
    let ev = g.bus.list(Some(&pid), 3000).unwrap().into_iter().find(|e| e.task_id.as_deref() == Some(t.id.as_str()) && e.kind == EventKind::Task && e.detail.get("taskKind").is_some()).unwrap();
    (ev.status, g.db.list_conversation_messages(conv, 200).unwrap())
}

fn last_assistant(msgs: &[MessageRow]) -> &MessageRow {
    msgs.iter().rev().find(|m| m.role == "assistant").unwrap()
}

fn system_of(body: &Value) -> String {
    body["messages"][0]["content"].as_str().unwrap_or("").to_string()
}

fn last_text(body: &Value) -> String {
    let c = &body["messages"].as_array().unwrap().last().unwrap()["content"];
    match c {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

fn image_urls(body: &Value) -> Vec<String> {
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["content"].as_array())
        .flatten()
        .filter(|p| p["type"] == "image_url")
        .map(|p| p["image_url"]["url"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_prompt_never_names_a_model_and_an_introduction_is_plain() {
    // Settings say glm-5.3; the provider labels its answers nemotron — the exact mismatch seen in the field.
    let env = setup(Some("z-ai/glm-5.3"), &[], Some("nvidia/nemotron-3-ultra-550b-a55b"), |_| "<final>I'm Genius, the AI assistant built into Project Genius.</final>".into()).await;
    let g = &env.g;
    let conv = g.create_chat_conversation("chat", None).unwrap();
    let (status, msgs) = send(g, &conv.row.id, "Hi! Who are you?", &[]).await;
    assert_eq!(status, EventStatus::Completed);

    let bodies = env.log.lock().unwrap().clone();
    assert_eq!(bodies.len(), 1, "a greeting is answered directly, without tools");
    let system = system_of(&bodies[0]);
    for needle in ["glm", "nemotron", "NVIDIA-hosted", "z-ai", "550b"] {
        assert!(!system.to_lowercase().contains(&needle.to_lowercase()), "the system prompt must not contain {needle}: {system}");
    }
    assert!(system.contains("I'm Genius, the AI assistant built into Project Genius"), "the standard introduction is in the prompt");
    assert!(system.contains("runtime_info"), "model questions are routed to measured facts");
    assert_eq!(last_assistant(&msgs).content, "I'm Genius, the AI assistant built into Project Genius.");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metadata_keeps_the_requested_and_the_reported_model_apart() {
    let env = setup(Some("z-ai/glm-5.3"), &[], Some("nvidia/nemotron-3-ultra-550b-a55b"), |_| "<final>Hello.</final>".into()).await;
    let g = &env.g;
    let conv = g.create_chat_conversation("chat", None).unwrap();
    let (_, msgs) = send(g, &conv.row.id, "Hello", &[]).await;
    let bodies = env.log.lock().unwrap().clone();
    assert_eq!(bodies[0]["model"], "z-ai/glm-5.3", "the request names the model chosen in Settings");
    let report = &last_assistant(&msgs).meta["report"];
    assert_eq!(report["model"], "z-ai/glm-5.3", "the shown model is the one requested");
    assert_eq!(report["lifecycle"], "answered", "a conversation answer changes nothing, so there is nothing to verify");
    assert_eq!(report["reportedModel"], "nvidia/nemotron-3-ultra-550b-a55b", "what the provider said is kept separately, never merged or hidden");
    let calls = report["modelCalls"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["promptTokens"], 11, "tokens belong to this request");
    assert_eq!(calls[0]["completionTokens"], 7);
    assert_eq!(report["usage"]["totalTokens"], 18);
    assert!(calls[0]["totalMs"].is_u64() && report["timing"]["totalMs"].is_u64(), "timings are measured, not estimated");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn changing_the_model_in_settings_changes_the_next_request() {
    let env = setup(Some("meta/llama-3.3-70b-instruct"), &[], None, |_| "<final>ok</final>".into()).await;
    let g = &env.g;
    let conv = g.create_chat_conversation("chat", None).unwrap();
    let (_, first) = send(g, &conv.row.id, "first", &[]).await;
    let mut s = g.settings().unwrap();
    s.model = Some("qwen/qwen3-next-80b-a3b-instruct".into());
    g.save_settings(&s).unwrap();
    let (_, second) = send(g, &conv.row.id, "second", &[]).await;

    let bodies = env.log.lock().unwrap().clone();
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0]["model"], "meta/llama-3.3-70b-instruct");
    assert_eq!(bodies[1]["model"], "qwen/qwen3-next-80b-a3b-instruct", "the saved model applies to the very next request");
    assert_eq!(last_assistant(&first).meta["report"]["model"], "meta/llama-3.3-70b-instruct");
    assert_eq!(last_assistant(&second).meta["report"]["model"], "qwen/qwen3-next-80b-a3b-instruct");
    assert_eq!(system_of(&bodies[0]), system_of(&bodies[1]).replace("second", "first"), "the identity text does not depend on the model");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_explicit_model_question_is_answered_from_measured_facts() {
    let env = setup(Some("z-ai/glm-5.3"), &[], Some("nvidia/nemotron-3-ultra-550b-a55b"), |body| {
        let last = last_text(body);
        if last.contains("USER MESSAGE:\nWhich model are you running on?") {
            return "I'll check.\n<tool>{\"name\":\"runtime_info\",\"args\":{}}</tool>".into();
        }
        assert!(last.starts_with("<tool_result name=\"runtime_info\" ok=\"true\">"), "{last}");
        assert!(last.contains("z-ai/glm-5.3"), "the served model id is in the facts: {last}");
        assert!(last.contains("labelled it \"nvidia/nemotron-3-ultra-550b-a55b\""), "a differing provider label is reported as a label, not as the model: {last}");
        assert!(last.contains("manual"), "{last}");
        "<final>The request was served by z-ai/glm-5.3 (the provider labelled the response differently); I can't verify more than that.</final>".into()
    })
    .await;
    let g = &env.g;
    let conv = g.create_chat_conversation("chat", None).unwrap();
    let (status, msgs) = send(g, &conv.row.id, "Which model are you running on?", &[]).await;
    assert_eq!(status, EventStatus::Completed);
    assert!(last_assistant(&msgs).content.contains("z-ai/glm-5.3"));
    assert_eq!(env.log.lock().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn auto_mode_does_not_pass_the_stale_preference_off_as_the_served_model() {
    // Auto: the key lists a text model; the saved preference (glm-5.3) is not even available.
    let env = setup(None, &[TEXT], None, |_| "<final>Hello.</final>".into()).await;
    let g = &env.g;
    let conv = g.create_chat_conversation("chat", None).unwrap();
    let (_, msgs) = send(g, &conv.row.id, "Hello", &[]).await;
    let bodies = env.log.lock().unwrap().clone();
    assert_eq!(bodies[0]["model"], TEXT, "routing picked an available model");
    let report = &last_assistant(&msgs).meta["report"];
    assert_eq!(report["model"], TEXT);
    assert!(report["reportedModel"].is_null(), "the provider stated nothing, so nothing is claimed");
    assert!(!system_of(&bodies[0]).contains("glm"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_screenshot_reaches_a_vision_model_as_a_real_image_payload() {
    let env = setup(None, &[TEXT, VISION], Some(VISION), |_| "<final>This looks like a Roblox game screenshot.</final>".into()).await;
    let g = &env.g;
    let ws = g.general_workspace().unwrap();
    let conv = g.create_chat_conversation("chat", None).unwrap();
    let bytes = png();
    let att = g.import_attachment_bytes(&ws.id, "roblox.png", &bytes, "message", Some(&conv.row.id)).unwrap();
    assert_eq!(att.status, "ready");
    assert_eq!(att.mime, "image/png");

    let (status, msgs) = send(g, &conv.row.id, "what is this?", &[&att.id]).await;
    assert_eq!(status, EventStatus::Completed);
    let bodies = env.log.lock().unwrap().clone();
    assert_eq!(bodies.len(), 1, "one request: the vision model answers directly (no describe-then-answer round trip)");
    assert_eq!(bodies[0]["model"], VISION, "a text-only model is not asked about pixels");
    let urls = image_urls(&bodies[0]);
    assert_eq!(urls.len(), 1, "the image is sent exactly once");
    let expected = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes));
    assert_eq!(urls[0], expected, "the bytes the user attached are the bytes on the wire");
    assert_eq!(last_text(&bodies[0]).matches("what is this?").count(), 1, "the question is sent once");

    let report = &last_assistant(&msgs).meta["report"];
    assert_eq!(report["vision"]["route"], "native");
    assert_eq!(report["vision"]["answeredBy"], VISION);
    assert_eq!(report["model"], VISION);

    // A follow-up without a new attachment still has the picture.
    let (_, _) = send(g, &conv.row.id, "what colour is the sky in it?", &[]).await;
    let bodies = env.log.lock().unwrap().clone();
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[1]["model"], VISION);
    assert_eq!(image_urls(&bodies[1]), vec![expected], "follow-up questions keep the image");

    // An unrelated message does not re-upload the picture or wake the (slower) vision model.
    let (_, _) = send(g, &conv.row.id, "Thanks, who are you?", &[]).await;
    let bodies = env.log.lock().unwrap().clone();
    assert_eq!(bodies.len(), 3);
    assert!(image_urls(&bodies[2]).is_empty(), "no image in an unrelated turn");
    assert_eq!(bodies[2]["model"], TEXT, "the regular chat model answers");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_any_vision_model_the_limit_is_explained_and_nothing_is_faked() {
    let env = setup(Some("meta/llama-3.3-70b-instruct"), &["meta/llama-3.3-70b-instruct"], None, |_| "<final>I can see a cat.</final>".into()).await;
    let g = &env.g;
    let ws = g.general_workspace().unwrap();
    let conv = g.create_chat_conversation("chat", None).unwrap();
    let att = g.import_attachment_bytes(&ws.id, "cat.png", &png(), "message", Some(&conv.row.id)).unwrap();
    let (status, msgs) = send(g, &conv.row.id, "what is this?", &[&att.id]).await;
    assert_eq!(status, EventStatus::Failed);
    assert!(env.log.lock().unwrap().is_empty(), "no model was asked to 'describe' an image it cannot see");
    let a = last_assistant(&msgs);
    assert_eq!(a.kind, "error");
    assert!(a.content.to_lowercase().contains("image"), "{}", a.content);
    assert!(!a.content.contains("I can see a cat"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_that_is_not_an_image_is_rejected_with_a_clear_reason() {
    let env = setup(Some(VISION), &[], None, |_| "<final>ok</final>".into()).await;
    let g = &env.g;
    let ws = g.general_workspace().unwrap();
    let conv = g.create_chat_conversation("chat", None).unwrap();
    let fake = g.import_attachment_bytes(&ws.id, "photo.png", b"this is plain text, not a PNG", "message", Some(&conv.row.id)).unwrap();
    assert_eq!(fake.status, "failed");
    assert!(fake.error.as_deref().unwrap_or("").contains("not a PNG, JPEG, GIF or WebP image"), "{:?}", fake.error);
    // a JPEG saved with a .png name is accepted, and its type is corrected from the bytes
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
    jpeg.extend([0u8; 32]);
    let ok = g.import_attachment_bytes(&ws.id, "really-a-jpeg.png", &jpeg, "message", Some(&conv.row.id)).unwrap();
    assert_eq!(ok.status, "ready");
    assert_eq!(ok.mime, "image/jpeg");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn arithmetic_and_table_statistics_come_from_tools_not_from_the_model() {
    let id_cell: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let cell = id_cell.clone();
    let env = setup(Some("meta/llama-3.3-70b-instruct"), &[], None, move |body| {
        let last = last_text(body);
        if last.contains("USER MESSAGE:
Total and average sales, and 19% of the total?") {
            let id = cell.lock().unwrap().clone();
            return format!("I'll compute that.
<tool>{{\"name\":\"data_summary\",\"args\":{{\"id\":\"{id}\",\"column\":\"sales\"}}}}</tool>
<tool>{{\"name\":\"calculate\",\"args\":{{\"expression\":\"1030 * 0.19\"}}}}</tool>");
        }
        if last.contains("<tool_result name=\"data_summary\" ok=\"false\">") {
            return "<final>I can't read that file from here.</final>".into();
        }
        assert!(last.contains("<tool_result name=\"data_summary\" ok=\"true\">"), "{last}");
        assert!(last.contains("sum 1030") && last.contains("mean 343.333333333"), "real statistics, computed from the whole file: {last}");
        assert!(last.contains("<tool_result name=\"calculate\" ok=\"true\">") && last.contains("1030 * 0.19 = 195.7"), "{last}");
        "<final>Total 1030, average 343.33, and 19% of the total is 195.7.</final>".into()
    })
    .await;
    let g = &env.g;
    let ws = g.general_workspace().unwrap();
    let conv = g.create_chat_conversation("chat", None).unwrap();
    let csv = "city,sales
Berlin,10
Berlin,20
Paris,\"1,000\"
";
    let att = g.import_attachment_bytes(&ws.id, "sales.csv", csv.as_bytes(), "message", Some(&conv.row.id)).unwrap();
    assert_eq!(att.status, "ready");
    *id_cell.lock().unwrap() = att.id.clone();
    let (status, msgs) = send(g, &conv.row.id, "Total and average sales, and 19% of the total?", &[&att.id]).await;
    assert_eq!(status, EventStatus::Completed);
    assert!(last_assistant(&msgs).content.contains("195.7"));

    // a file attached to another conversation is not readable from this one
    let other = g.create_chat_conversation("chat", None).unwrap();
    let (_, msgs) = send(g, &other.row.id, "Total and average sales, and 19% of the total?", &[]).await;
    assert!(last_assistant(&msgs).content.contains("can't read that file"), "{}", last_assistant(&msgs).content);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_product_creator_is_configured_and_never_confused_with_the_user() {
    let env = setup(Some("meta/llama-3.3-70b-instruct"), &[], None, |_| "<final>Project Genius was created by Hamza.</final>".into()).await;
    let g = &env.g;
    let conv = g.create_chat_conversation("chat", None).unwrap();
    let (status, _) = send(g, &conv.row.id, "Hi, I'm Sara. Who created you?", &[]).await;
    assert_eq!(status, EventStatus::Completed);
    let system = system_of(&env.log.lock().unwrap()[0]);
    assert!(system.contains("Project Genius was created by Hamza"), "{system}");
    assert!(system.contains("never assume the user is Hamza"), "the creator is not the user: {system}");
    assert!(system.contains("do not say \"a team\""), "{system}");

    // The creator is a setting, not a hard-coded string.
    let mut s = g.settings().unwrap();
    s.product_creator = "Ada".into();
    g.save_settings(&s).unwrap();
    send(g, &conv.row.id, "And who made Genius?", &[]).await;
    let second = system_of(&env.log.lock().unwrap()[1]);
    assert!(second.contains("created by Ada") && !second.contains("created by Hamza"), "{second}");

    // The earlier turns are carried, so what was established (the user's name) is still in context.
    let bodies = env.log.lock().unwrap().clone();
    let all: String = bodies[1]["messages"].as_array().unwrap().iter().map(|m| m["content"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>().join("\n");
    assert!(all.contains("I'm Sara"), "follow-ups keep the conversation facts");
    s.product_creator = String::new();
    g.save_settings(&s).unwrap();
    send(g, &conv.row.id, "Who built you?", &[]).await;
    let third = system_of(&env.log.lock().unwrap()[2]);
    assert!(third.contains("do not invent a team") && !third.contains("created by"), "{third}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bare_clarifying_question_is_nudged_once_into_a_real_answer() {
    // The field report: "What was the last route of a340 before retire" got "Which A340 do you mean?" three times.
    let env = setup(Some(TEXT), &[], None, |body| {
        if last_text(body).contains("Do not ask the user to clarify") {
            "<final>Reading \"a340\" as the Airbus A340 family: it was retired from scheduled service in stages; I could not verify one single last route from what I read.</final>".into()
        } else {
            "<final>I'm not sure which A340 you're referring to. Could you clarify which aircraft's last route you want to know about?</final>".into()
        }
    })
    .await;
    let g = &env.g;
    let conv = g.create_chat_conversation("chat", None).unwrap();
    let (status, msgs) = send(g, &conv.row.id, "What was the last route of a340 before retire", &[]).await;
    assert_eq!(status, EventStatus::Completed);
    let answer = &last_assistant(&msgs).content;
    assert!(answer.contains("Airbus A340") && !answer.trim_end().ends_with('?'), "{answer}");
    let bodies = env.log.lock().unwrap().clone();
    assert_eq!(bodies.len(), 2, "one nudge, then the answer");
    assert!(system_of(&bodies[0]).contains("Never answer a factual question with only a question back"));
    assert!(system_of(&bodies[0]).contains("Never invent facts, dates, routes, numbers, names or links"));
    assert!(last_text(&bodies[1]).contains("web_search"), "with internet on, the nudge sends the model to the web");

    // a model that keeps asking is not looped: one nudge only
    let env2 = setup(Some(TEXT), &[], None, |_| "<final>Which aircraft do you mean?</final>".into()).await;
    let conv2 = env2.g.create_chat_conversation("chat", None).unwrap();
    let (_, msgs2) = send(&env2.g, &conv2.row.id, "tell me about it", &[]).await;
    assert_eq!(last_assistant(&msgs2).content.trim(), "Which aircraft do you mean?");
    assert_eq!(env2.log.lock().unwrap().len(), 2);
}
