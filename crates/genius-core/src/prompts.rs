//! System prompts and message builders. Kept in one place so the contract between Genius
//! and the model (JSON plan, file-block protocol) is explicit and testable.

use crate::nvidia::ChatMessage;
use crate::spec::ProjectSpec;

pub const FILE_PROTOCOL: &str = r#"OUTPUT FORMAT (mandatory):
Write each file you create or change as a block, with the COMPLETE file content:
<<<FILE: relative/path/to/file.ext>>>
...entire file content...
<<<END FILE>>>
To delete a file write a single line: <<<DELETE: relative/path>>>
Rules:
- Paths are relative to the project root (e.g. src/components/Header.tsx). Never use absolute paths or "..".
- Every block replaces the whole file, so never abbreviate with "..." or "rest unchanged".
- Do not wrap blocks in markdown code fences.
- Never write .env files, credentials, API keys or tokens.
- Text outside blocks is shown to the user as your explanation; keep it short."#;

const STACK: &str = r#"TARGET STACK: Vite 7 + React 19 + TypeScript 5 (strict) single-page web app, built with `tsc --noEmit && vite build`, tested with Vitest + @testing-library/react (jsdom).
The project template already contains: package.json (scripts dev/build/typecheck/test/preview), index.html, tsconfig.json, vite.config.ts, src/main.tsx (renders the default export of ./App and imports ./index.css), src/index.css, .gitignore.
CODE RULES:
- src/App.tsx must default-export the root component `App`.
- Plain CSS files (imported from components) for styling; modern, responsive, accessible UI (semantic HTML, labels, focus states, sufficient contrast).
- No backend server: persist data with localStorage when persistence is needed.
- Only import packages listed in package.json dependencies. If a new package is truly needed, output the full updated package.json keeping every existing script EXACTLY unchanged, and only add well-known, actively maintained npm packages.
- Code must compile under TypeScript strict mode: type every prop, avoid implicit any, handle possibly-undefined values.
- Implement real working behaviour. No placeholder text like "TODO" or "coming soon", no fake data presented as live data (clearly labelled sample data is fine)."#;

pub fn plan_messages(description: &str, constraints: &str) -> Vec<ChatMessage> {
    let system = format!(
        r#"You are the planning engine of Project Genius, an AI software studio. Turn the user's idea into a precise, buildable specification.
{STACK}
Make ordinary product decisions yourself and record the important ones under "assumptions". Keep scope realistic for a first version that can be fully implemented in one pass (typically 4-12 source files).
Respond with ONLY one JSON object (no markdown fences, no commentary) with exactly these keys:
{{
  "name": "short product name",
  "purpose": "one or two sentences",
  "features": [{{"name": "...", "description": "..."}}],
  "pages": [{{"name": "...", "description": "...", "components": ["ComponentName"]}}],
  "components": [{{"name": "...", "description": "..."}}],
  "dataModels": [{{"name": "...", "fields": ["field: type"], "relations": ["..."]}}],
  "architecture": "state management, persistence, routing and folder structure",
  "dependencies": [{{"name": "npm-package", "reason": "..."}}],
  "milestones": ["..."],
  "testStrategy": "...",
  "risks": ["..."],
  "assumptions": ["..."],
  "files": [{{"path": "src/...", "purpose": "..."}}]
}}
"dependencies" lists only packages beyond react/react-dom (prefer none)."#
    );
    let mut user = format!("APP IDEA:\n{}", description.trim());
    if !constraints.trim().is_empty() {
        user.push_str(&format!("\n\nPREFERENCES AND CONSTRAINTS:\n{}", constraints.trim()));
    }
    vec![ChatMessage::system(system), ChatMessage::user(user)]
}

pub fn plan_repair_message(error: &str) -> ChatMessage {
    ChatMessage::user(format!(
        "Your previous answer could not be used: {error}\nReply again with ONLY the JSON object described in the instructions."
    ))
}

pub fn generate_messages(spec: &ProjectSpec, template_context: &str, memory: &str) -> Vec<ChatMessage> {
    let system = format!(
        "You are the implementation engine of Project Genius. You write complete, production-quality source code.\n{STACK}\n\n{FILE_PROTOCOL}\n\nStart with one short paragraph describing what you are building, then output all files."
    );
    let spec_json = serde_json::to_string_pretty(spec).unwrap_or_default();
    let mut user = format!("Implement this approved specification completely.\n\nSPECIFICATION:\n{spec_json}\n\n{template_context}");
    if !memory.trim().is_empty() {
        user.push_str(&format!("\n\nPROJECT MEMORY (decisions and conventions to respect):\n{memory}"));
    }
    user.push_str("\n\nWrite src/App.tsx and every component, hook, utility, stylesheet and test needed. Include a README.md describing the app and how to run it.");
    vec![ChatMessage::system(system), ChatMessage::user(user)]
}

pub fn continue_message(done: &[String]) -> ChatMessage {
    ChatMessage::user(format!(
        "Your output was cut off by the length limit. Continue with the remaining files only. Files already completed: {}. If a file was cut off part-way, output it again in full. Use the same block format.",
        if done.is_empty() { "none".into() } else { done.join(", ") }
    ))
}

pub fn change_messages(request: &str, context: &str, memory: &str, spec_summary: &str) -> Vec<ChatMessage> {
    let system = format!(
        "You are the code-editing engine of Project Genius. You make focused, correct changes to an existing project.\n{STACK}\n\n{FILE_PROTOCOL}\n\nOnly output files you actually change (full new content). Preserve existing behaviour and code style unless the request says otherwise. First write 1-3 sentences explaining the change."
    );
    let mut user = String::new();
    if !spec_summary.trim().is_empty() {
        user.push_str(&format!("PROJECT SUMMARY:\n{spec_summary}\n\n"));
    }
    if !memory.trim().is_empty() {
        user.push_str(&format!("PROJECT MEMORY (decisions and conventions to respect):\n{memory}\n\n"));
    }
    user.push_str(context);
    user.push_str(&format!("\n\nCHANGE REQUEST:\n{}", request.trim()));
    vec![ChatMessage::system(system), ChatMessage::user(user)]
}

pub fn repair_messages(command: &str, exit_code: Option<i32>, diagnostics: &str, log: &str, context: &str, previous_attempts: &[String], memory: &str) -> Vec<ChatMessage> {
    let system = format!(
        "You are the Build Doctor of Project Genius. You diagnose real build and test failures and propose minimal, targeted fixes.\n{STACK}\n\n{FILE_PROTOCOL}\n\nBegin with a line starting 'Root cause:' (1-3 sentences, based only on the evidence provided), then output only the files that must change.\nNever weaken checks to hide errors: do not disable TypeScript strictness, add @ts-ignore/@ts-nocheck, delete or skip tests, or change package.json scripts."
    );
    let mut user = format!(
        "FAILED COMMAND: {command}\nEXIT CODE: {}\n\nPARSED DIAGNOSTICS:\n{diagnostics}\n\nOUTPUT (redacted, possibly shortened):\n{log}\n\n{context}",
        exit_code.map(|c| c.to_string()).unwrap_or_else(|| "none".into())
    );
    if !previous_attempts.is_empty() {
        user.push_str(&format!(
            "\n\nPREVIOUS REPAIR ATTEMPTS THAT DID NOT RESOLVE THE FAILURE (try a different approach):\n- {}",
            previous_attempts.join("\n- ")
        ));
    }
    if !memory.trim().is_empty() {
        user.push_str(&format!("\n\nPROJECT MEMORY:\n{memory}"));
    }
    vec![ChatMessage::system(system), ChatMessage::user(user)]
}

pub fn commit_message_messages(diff: &str, files: &[String]) -> Vec<ChatMessage> {
    vec![
        ChatMessage::system(
            "Write a git commit message for the staged changes. Format: an imperative subject line of at most 72 characters, then optionally a blank line and a short body of bullet points. Output only the commit message, nothing else.",
        ),
        ChatMessage::user(format!("FILES:\n{}\n\nSTAGED DIFF (may be truncated):\n{diff}", files.join("\n"))),
    ]
}

pub fn conflict_messages(path: &str, content: &str) -> Vec<ChatMessage> {
    vec![
        ChatMessage::system(format!(
            "You resolve Git merge conflicts. Combine both sides so that the intent of each is preserved and the result compiles. Remove every conflict marker.\n\n{FILE_PROTOCOL}\n\nOutput exactly one file block for the resolved file, preceded by one sentence explaining the resolution."
        )),
        ChatMessage::user(format!("FILE WITH CONFLICTS: {path}\n<<<CURRENT: {path}>>>\n{content}\n<<<END CURRENT>>>")),
    ]
}

pub fn clean_commit_message(text: &str) -> String {
    let t = text.trim().trim_start_matches("```").trim_end_matches("```").trim();
    let t = t.strip_prefix("text").unwrap_or(t).trim();
    let mut lines: Vec<&str> = t.lines().collect();
    if let Some(first) = lines.first_mut() {
        *first = first.trim().trim_matches('"');
    }
    let mut msg = lines.join("\n").trim().to_string();
    if let Some(first) = msg.lines().next() {
        if first.chars().count() > 72 {
            let short: String = first.chars().take(72).collect();
            msg = msg.replacen(first, &short, 1);
        }
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_prompt_contains_contract() {
        let m = plan_messages("A todo app", "dark mode");
        assert_eq!(m.len(), 2);
        assert!(m[0].content.contains("\"dataModels\""));
        assert!(m[1].content.contains("dark mode"));
    }

    #[test]
    fn repair_prompt_forbids_cheating() {
        let m = repair_messages("npm run build", Some(2), "src/App.tsx:1 TS2304", "log", "ctx", &["tried X".into()], "");
        assert!(m[0].content.contains("@ts-ignore"));
        assert!(m[1].content.contains("EXIT CODE: 2"));
        assert!(m[1].content.contains("tried X"));
    }

    #[test]
    fn commit_message_cleanup() {
        assert_eq!(clean_commit_message("```\n\"Add search to task board\"\n```"), "Add search to task board");
        let long = "x".repeat(100);
        assert_eq!(clean_commit_message(&long).chars().count(), 72);
    }
}
