//! Slash commands for the chat composer.
//!
//! `prompt` commands are sent to the agent as typed (`/review src/App.tsx`) and expanded here
//! into a full instruction, optionally forcing plan mode (read-only work). `action` commands
//! are handled by the UI (clear, undo, effort …) and listed here so both sides agree.

use crate::agent::AgentMode;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommand {
    pub name: &'static str,
    pub description: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<&'static str>,
    pub kind: &'static str,
}

struct PromptCommand {
    name: &'static str,
    description: &'static str,
    args: Option<&'static str>,
    read_only: bool,
    /// `{args}` is replaced by the arguments (or `default`).
    template: &'static str,
    default: &'static str,
}

const PROMPTS: &[PromptCommand] = &[
    PromptCommand { name: "init", description: "Write GENIUS.md: project instructions every future task follows", args: None, read_only: false, default: "", template: "Create (or update, keeping what the user wrote) GENIUS.md at the project root: concise instructions for future AI work on this project. Inspect the project first. Cover: what the app is and who it's for; tech stack and key libraries; folder structure and where things live; how to run, build and test; coding conventions you observe (naming, state management, styling approach); the design system (tokens, fonts, components to reuse); and gotchas. Under ~120 lines, Markdown, facts only." },
    PromptCommand { name: "fix", description: "Find and fix what's broken (build errors, crashes, blank page)", args: Some("[what's wrong]"), read_only: false, default: "the app is broken or doesn't work as expected", template: "Something is broken: {args}.\nFind the root cause from real evidence — run_check typecheck (and build), last_run_output, preview_errors for runtime crashes or a blank page — then fix it properly (no @ts-ignore, no disabling checks, no deleting features) and verify with run_check. Explain the cause in one or two sentences." },
    PromptCommand { name: "review", description: "Code review of recent changes or a file (no edits)", args: Some("[file or area]"), read_only: true, default: "the uncommitted and recent changes in this project", template: "Do a thorough code review of {args}. Use git_status/git_diff/recent_changes and read the code. Report concrete findings ordered by severity — bugs, security, accessibility, performance, maintainability, design inconsistencies — each with file:line, why it matters and the suggested fix. Don't change files; end by asking whether I should fix them." },
    PromptCommand { name: "test", description: "Write tests for the app (or a part) and make them pass", args: Some("[component or feature]"), read_only: false, default: "the most important user-facing behaviour that isn't tested yet", template: "Write meaningful Vitest + @testing-library/react tests for {args}: real user interactions, edge cases and accessibility queries (getByRole/getByLabelText). Run run_check test, and fix failures — in the tests if they're wrong, in the code if you found a real bug (tell me which). Never weaken assertions to make them pass." },
    PromptCommand { name: "feature", description: "Build a new feature end to end", args: Some("<description>"), read_only: false, default: "", template: "Implement this feature end to end: {args}\nPlan it with todo, follow the existing architecture and design system, handle empty/loading/error states, make it responsive and accessible, add tests for the core behaviour, and verify with run_check." },
    PromptCommand { name: "design", description: "Make it look stunning: redesign the UI", args: Some("[direction]"), read_only: false, default: "a polished, modern, cohesive look", template: "Upgrade the visual design of this app towards {args}. Load the relevant skill, ask the designer subagent for a concrete direction, then implement it: design tokens first (src/index.css), then every component and page — typography, spacing, color, depth, hover/focus states, empty states, motion and dark mode. Keep all behaviour working." },
    PromptCommand { name: "explain", description: "Explain how the project (or a part) works", args: Some("[file or topic]"), read_only: true, default: "how this project works: its structure, main components, data flow and where to change things", template: "Explain {args} clearly for me, with file references. Start with a short overview, then the details that matter. Don't change files." },
    PromptCommand { name: "refactor", description: "Clean up code without changing behaviour", args: Some("[file or area]"), read_only: false, default: "the messiest part of the codebase", template: "Refactor {args} for clarity and maintainability without changing behaviour: remove duplication, split oversized components, improve names and types. Verify with run_check typecheck (and test if tests exist)." },
    PromptCommand { name: "optimize", description: "Speed up the app (bundle, rendering, loading)", args: Some("[area]"), read_only: false, default: "the whole app", template: "Improve the performance of {args}: unnecessary re-renders, expensive computations (memoize where it matters), large dependencies, lazy-loading routes and heavy components, image sizing, layout shift. Make measurable, safe changes and verify with run_check build." },
    PromptCommand { name: "a11y", description: "Accessibility audit and fixes (WCAG AA)", args: None, read_only: false, default: "", template: "Audit and fix accessibility to WCAG 2.2 AA: semantic landmarks and headings, labels for every control, keyboard navigation and visible focus, focus management in dialogs/menus, color contrast, alt text, aria only where needed, reduced motion. Summarise what you fixed." },
    PromptCommand { name: "responsive", description: "Make every screen work great on phones and tablets", args: None, read_only: false, default: "", template: "Make the app fully responsive: check every page and component at 360px, 768px and 1280px widths (by reading the CSS/layout code), fix overflow, tap targets (>= 44px), navigation on small screens, tables and grids, font sizes. Mobile-first CSS." },
    PromptCommand { name: "seo", description: "SEO and social sharing: titles, meta, Open Graph", args: None, read_only: false, default: "", template: "Improve SEO and link previews: descriptive <title> and meta description per page (update document.title on route changes), Open Graph and Twitter card tags in index.html, semantic headings (one h1 per page), alt text, a favicon, robots.txt and a sitemap.xml in public/ when there are routes." },
    PromptCommand { name: "darkmode", description: "Add a polished light/dark theme with a toggle", args: None, read_only: false, default: "", template: "Add a polished dark mode: dark values for every design token via prefers-color-scheme and a [data-theme] override, a theme toggle in the header that remembers the choice (localStorage) and respects the system setting by default, no flash of the wrong theme on load, and contrast checked in both themes." },
    PromptCommand { name: "docs", description: "Write a great README for the project", args: None, read_only: false, default: "", template: "Write or update README.md: what the app does (with a short feature list), how to run, build and test it, the project structure, the design system, and how to deploy it as a static site. Accurate to the code — inspect it first." },
    PromptCommand { name: "security", description: "Security review (no edits)", args: None, read_only: true, default: "", template: "Do a security review of this project: XSS (dangerouslySetInnerHTML, unescaped URLs), secrets in code, unsafe storage of sensitive data, dependency risks, input validation, external links (rel=noopener). Report findings by severity with file:line and the fix. Don't change files." },
];

const ACTIONS: &[(&str, &str, Option<&str>)] = &[
    ("clear", "Start a new conversation", None),
    ("plan", "Plan only: Genius explains its plan without changing files", None),
    ("build", "Build mode: Genius changes files", None),
    ("effort", "How hard Genius works: fast, balanced or max", Some("<fast|balanced|max>")),
    ("autopilot", "Apply and verify changes automatically", Some("<on|off>")),
    ("undo", "Rewind the last request and undo its file changes", None),
    ("compact", "Summarise the conversation to free up context", None),
    ("model", "Choose the AI model", None),
    ("help", "Show all commands", None),
];

pub fn list() -> Vec<SlashCommand> {
    PROMPTS
        .iter()
        .map(|c| SlashCommand { name: c.name, description: c.description, args: c.args, kind: "prompt" })
        .chain(ACTIONS.iter().map(|(name, description, args)| SlashCommand { name, description, args: *args, kind: "action" }))
        .collect()
}

/// Expand `/name args` into the instruction for the agent, and the mode it forces (if any).
/// Returns None for messages that are not a known prompt command.
pub fn expand(message: &str) -> Option<(String, Option<AgentMode>)> {
    let t = message.trim();
    let rest = t.strip_prefix('/')?;
    let (name, args) = rest.split_once(char::is_whitespace).map(|(n, a)| (n, a.trim())).unwrap_or((rest, ""));
    let c = PROMPTS.iter().find(|c| c.name.eq_ignore_ascii_case(name))?;
    if c.args.is_some_and(|a| a.starts_with('<')) && args.is_empty() {
        return Some((format!("The user typed /{} without saying what. Ask them briefly what they want ({}).", c.name, c.description.to_lowercase()), Some(AgentMode::Plan)));
    }
    let text = c.template.replace("{args}", if args.is_empty() { c.default } else { args });
    let text = if c.args.is_none() && !args.is_empty() { format!("{text}\nAdditional instructions from the user: {args}") } else { text };
    Some((text, c.read_only.then_some(AgentMode::Plan)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_prompt_commands() {
        let (t, m) = expand("/review src/App.tsx").unwrap();
        assert!(t.contains("code review of src/App.tsx"));
        assert_eq!(m, Some(AgentMode::Plan));
        let (t, m) = expand("/fix").unwrap();
        assert!(t.contains("the app is broken") && t.contains("preview_errors"));
        assert_eq!(m, None);
        let (t, _) = expand("/a11y focus on forms").unwrap();
        assert!(t.contains("Additional instructions from the user: focus on forms"));
        let (t, m) = expand("/feature").unwrap();
        assert!(t.contains("Ask them") && m == Some(AgentMode::Plan), "required args missing → ask");
        assert!(expand("/FIX login").unwrap().0.contains("login"));
    }

    #[test]
    fn non_commands_pass_through() {
        assert!(expand("fix the header").is_none());
        assert!(expand("/clear").is_none(), "actions are handled by the UI");
        assert!(expand("/unknown thing").is_none());
        assert!(expand("/src/App.tsx is broken").is_none());
    }

    #[test]
    fn list_has_unique_names() {
        let l = list();
        let mut names: Vec<_> = l.iter().map(|c| c.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), l.len());
        assert!(l.iter().any(|c| c.name == "undo" && c.kind == "action"));
    }
}
