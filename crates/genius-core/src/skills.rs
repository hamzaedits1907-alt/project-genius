//! Skills: expert playbooks the agent loads for a kind of work, plus project instructions.
//!
//! Built-in skills ship with Genius. A project can add its own as Markdown files in
//! `.genius/skills/*.md` (optional front matter `name:`, `description:`, `keywords:`), and
//! override a built-in by using its id as the file name. The best matches for a request are
//! inlined into the prompt; the rest are listed and loaded on demand with `use_skill`.
//! `GENIUS.md` (or `AGENTS.md` / `CLAUDE.md`) at the project root holds project instructions
//! that every task follows.

use crate::workspace::Workspace;

#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub keywords: Vec<String>,
    pub body: String,
}

struct Builtin {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    keywords: &'static [&'static str],
    body: &'static str,
}

const BUILTIN: &[Builtin] = &[
    Builtin {
        id: "dashboard",
        name: "Dashboards & admin panels",
        description: "data dashboards, admin panels, analytics, KPI and widget layouts",
        keywords: &["dashboard", "admin", "analytics", "kpi", "widget", "metrics", "stats", "panel", "overview", "tracker", "monitor"],
        body: r#"- App shell: collapsible sidebar (icons + labels, active state, collapses to icons < 1024px and to a drawer < 640px) + top bar (page title, search, theme toggle, avatar menu). Content area max-width ~1400px with a 12-column CSS grid (gap 16-24px).
- Above the fold: a row of 3-4 KPI cards (label, big tabular-nums value, delta badge ▲/▼ colored success/danger with an aria-label, sparkline), then the primary chart (full or 2/3 width) beside a secondary panel (recent activity, top items), then tables/lists.
- Charts without dependencies: small typed SVG components (LineChart with area gradient, BarChart, Donut, Sparkline) computing paths from data with a viewBox and preserveAspectRatio="none" for width, axis labels, gridlines at 0.08 opacity, hover tooltip via pointer events, a <title> or an aria-label summary, and a visually-hidden data table for screen readers. Use the accent plus 3-4 harmonious series colors as tokens.
- Tables: sticky header, sortable columns (aria-sort), search/filter input, pagination or "load more", row hover, right-aligned numbers, empty state with an illustration-like icon and a call to action, skeleton rows while "loading".
- Widgets the user manages (add/remove/reorder/resize): keep a typed list in state persisted to localStorage (versioned key, try/catch around JSON.parse with a fallback to defaults), an "Add widget" dialog (role=dialog, focus trap, Esc closes), drag to reorder with pointer events or HTML5 DnD plus keyboard alternatives (move up/down buttons).
- Realistic sample data generated deterministically in src/data/ (labelled "Sample data"), with date ranges (7d/30d/90d segmented control) that actually change the data shown.
- Polish: numbers animate in (respect prefers-reduced-motion), Intl.NumberFormat/DateTimeFormat for formatting, dark mode first-class, cards with subtle borders and soft shadows, 8px spacing rhythm.
- Never leave the root empty: App must render the shell immediately; guard every array access (data?.map) so a missing value can't crash the whole page."#,
    },
    Builtin {
        id: "landing-page",
        name: "Landing pages & marketing sites",
        description: "landing pages, product/SaaS sites, startup and business websites",
        keywords: &["landing", "marketing", "website", "homepage", "saas", "startup", "product page", "business", "company", "agency", "hero", "pricing"],
        body: r#"- Structure: sticky translucent header (logo, nav anchors, CTA; mobile menu button with aria-expanded) → hero (eyebrow, 6-10 word headline with one accented phrase, 1-2 sentence subcopy, primary + secondary CTA, trust line, a product visual built from real UI-like components or a CSS/SVG illustration) → logo strip → features (3-6 cards with icon, title, one sentence) → how it works (3 numbered steps) → showcase/benefit sections alternating text and visual → testimonials (name, role, company, avatar initials) → pricing (3 tiers, highlighted middle tier, monthly/yearly toggle that changes prices) → FAQ (accessible accordion with <details> or buttons + aria-expanded) → final CTA band → footer (columns, newsletter input, social links, legal).
- Copy: specific and benefit-led, believable numbers, consistent product name, no lorem ipsum.
- Visual: one strong accent, generous whitespace (sections padded clamp(4rem, 10vw, 8rem)), large tight headlines with clamp(), subtle gradient or noise backgrounds, rounded cards, soft shadows, scroll-reveal fade/translate via IntersectionObserver (respect prefers-reduced-motion), smooth anchor scrolling with scroll-margin-top for the sticky header.
- Multi-page sites: react-router-dom with a shared Layout (header/footer), ScrollRestoration-like scroll to top on navigation, a 404 page, document.title per page.
- Forms (contact/newsletter): validation, inline errors, a success state; no fake network calls presented as real."#,
    },
    Builtin {
        id: "web-app",
        name: "Interactive web apps",
        description: "CRUD apps, productivity tools, trackers, planners, kanban, notes, todo",
        keywords: &["app", "todo", "task", "notes", "kanban", "planner", "tracker", "crud", "manage", "list", "budget", "habit", "calendar", "booking", "inventory", "crm"],
        body: r#"- Model the domain first in src/types.ts; keep state in a reducer or a small custom hook per domain (useTasks) with pure, testable update functions; persist to localStorage through one helper (versioned key, safe JSON parse with defaults, debounce writes).
- Every collection screen has: create (form or inline input), read (list/grid with sensible sorting), update (inline edit or dialog), delete (with undo toast rather than a confirm prompt), search/filter, and empty / no-results states with a helpful action.
- Interactions: keyboard support (Enter to submit, Esc to cancel, shortcuts shown in tooltips), optimistic UI, focus moves to new items, toasts with role=status.
- Drag and drop (kanban, reorder): pointer-based or HTML5 DnD with a keyboard alternative; persist order.
- Dates: native <input type="date">, Intl.DateTimeFormat, compare with getTime(); timezone-safe "today" helpers.
- Structure: src/components (presentational), src/features/<feature> (stateful), src/hooks, src/lib (pure utils with tests).
- Write tests for the reducer/hook logic and one end-to-end-ish component test of the main flow."#,
    },
    Builtin {
        id: "store",
        name: "Online stores",
        description: "e-commerce: product catalog, product pages, cart, checkout UI",
        keywords: &["shop", "store", "e-commerce", "ecommerce", "cart", "checkout", "product", "catalog", "sell", "merch"],
        body: r#"- Pages: home (hero, featured collections, bestsellers), catalog (filters: category, price range, rating; sort; result count; responsive grid), product detail (gallery with thumbnails, variant selectors as radio groups, quantity stepper, add-to-cart, reviews, related products), cart (line items, quantity edit, remove with undo, subtotal/shipping/tax/total computed in one pure function), checkout (multi-step form: contact → shipping → payment UI with clear "demo — no real payment" note → confirmation with order number).
- Cart in context + localStorage; cart icon badge with count; mini-cart drawer.
- Money: integer cents, Intl.NumberFormat for display; never floats for totals.
- Product data in src/data/products.ts with realistic names, descriptions, prices, ratings, and images as CSS gradients/SVG placeholders or picsum-style URLs clearly marked as sample.
- Accessibility: buttons not divs, labelled controls, focus management in the drawer, live region announcing "Added to cart"."#,
    },
    Builtin {
        id: "game",
        name: "Browser games",
        description: "games: canvas or DOM games, puzzles, arcade, board and card games",
        keywords: &["game", "snake", "tetris", "2048", "puzzle", "arcade", "chess", "quiz", "memory game", "platformer", "play", "score"],
        body: r#"- Separate pure game logic (src/game/*.ts: state type, init, step/reducer, win/lose checks — fully unit-tested) from rendering (React DOM grid or a <canvas> component).
- Real-time games: requestAnimationFrame loop with a fixed timestep accumulator, paused when the tab is hidden; input via keydown (arrows/WASD) plus on-screen/touch controls and swipe for mobile; prevent page scroll on game keys.
- Canvas: scale for devicePixelRatio, resize with the container, crisp pixel art with imageSmoothingEnabled=false if pixel style.
- Screens: start (title, how to play, start button), playing (score, level, best score from localStorage, pause), game over (score, best, play again). Difficulty ramps.
- Juice: small animations, screen shake/particles behind prefers-reduced-motion, optional sound via WebAudio oscillators with a mute toggle (off until the user interacts).
- Accessibility: keyboard playable, pause on Esc/P, announce score changes politely, high-contrast palette."#,
    },
    Builtin {
        id: "portfolio-blog",
        name: "Portfolios, blogs & content sites",
        description: "personal portfolio, resume, blog, magazine, documentation, restaurant or event sites",
        keywords: &["portfolio", "resume", "cv", "blog", "article", "magazine", "docs", "documentation", "restaurant", "menu", "event", "conference", "personal site", "photography"],
        body: r#"- Content lives in typed data modules (src/content/*.ts) or Markdown strings rendered with a tiny safe renderer (headings, paragraphs, lists, links, code) — never dangerouslySetInnerHTML with unsanitised input.
- Portfolio: hero with name, role and a one-line value statement; selected work as large cards (cover, title, tags, role, outcome) with a detail page per project; about; experience timeline; skills; contact (mailto + form with validation).
- Blog/magazine: article list with featured post, tags/categories filter, reading time, author, dates; article page with max-width ~68ch, generous line-height, table of contents for long posts, previous/next links.
- Restaurant/event: hero photo-like section, menu with categories/dietary badges and prices, opening hours, map embed placeholder, reservation/ticket form, schedule/speakers/FAQ for events.
- Typography carries the design: a characterful heading font (e.g. Fraunces, Playfair Display, Space Grotesk) with a clean body font, consistent vertical rhythm.
- Routing with react-router-dom, slugs from titles, 404 page, document.title per page."#,
    },
    Builtin {
        id: "forms",
        name: "Forms & validation",
        description: "forms, sign up/login screens, multi-step wizards, settings pages, validation",
        keywords: &["form", "signup", "sign up", "login", "register", "wizard", "survey", "validation", "checkout form", "settings page", "onboarding"],
        body: r#"- Each field: visible <label>, helpful placeholder only as an example, correct input type/inputmode/autocomplete, required indicator, inline error below the field linked with aria-describedby and aria-invalid, validated on blur and on submit (not on every keystroke before the first blur).
- One validation module of pure functions (tested), errors summarised at the top on submit with focus moved to the first invalid field.
- Submit button shows a pending state and prevents double submit; success and failure states are explicit.
- Multi-step: progress indicator (Step 2 of 4), back/next, state kept across steps, review step before submit.
- Auth screens without a backend: make clear it's a demo, store nothing sensitive, never put passwords in localStorage."#,
    },
    Builtin {
        id: "motion",
        name: "Animation & micro-interactions",
        description: "animations, transitions, scroll effects, delightful micro-interactions",
        keywords: &["animation", "animate", "motion", "transition", "parallax", "scroll effect", "micro-interaction", "smooth", "fancy", "cool", "wow"],
        body: r#"- Prefer CSS transitions/keyframes on transform and opacity (GPU friendly); 150-250ms for UI feedback, 300-600ms for entrances; ease-out for entering, ease-in for leaving; stagger lists by 30-60ms.
- Scroll reveals with one IntersectionObserver hook (useInView) toggling a class; no scroll listeners doing layout reads.
- Hover: slight lift (translateY(-2px)) + shadow, buttons with pressed scale(0.98), focus-visible rings never removed.
- Wrap all non-essential motion in @media (prefers-reduced-motion: no-preference) — reduced-motion users get instant state changes.
- Page transitions in routers: fade/slide the outlet on route change; skeleton shimmer for loading."#,
    },
    Builtin {
        id: "debugging",
        name: "Debugging crashes and blank screens",
        description: "fix blank/black/white screens, crashes, runtime errors, broken builds",
        keywords: &["blank", "black screen", "white screen", "empty", "crash", "error", "broken", "not working", "doesn't work", "does not work", "fix", "bug", "nothing shows", "not showing"],
        body: r#"1. Gather evidence before editing: preview_errors (runtime exceptions with stack, failed module imports, console errors, blank-page reports), run_check typecheck and build, last_run_output for the dev server.
2. A blank/black page in a Vite React app is usually: an exception thrown during the first render (undefined.map, JSON.parse of bad localStorage, a missing context provider), an import of a file/export that doesn't exist, a component returning undefined, CSS making everything invisible (color equals background, height 0, opacity 0), or main.tsx not rendering into #root.
3. Read the exact files and lines from the stack trace; fix the cause, not the symptom. Add defensive defaults where data can be missing (localStorage, optional props), and an ErrorBoundary around the app root that shows a friendly message with a reload button so a crash never shows an empty screen again.
4. Verify: run_check typecheck → build; ask the user to check the preview, or call preview_errors again after the change is applied.
Never "fix" by adding @ts-ignore, any-casts to silence real errors, or deleting the feature."#,
    },
    Builtin {
        id: "testing",
        name: "Testing with Vitest",
        description: "unit and component tests with Vitest and Testing Library",
        keywords: &["test", "tests", "testing", "vitest", "coverage", "unit test", "spec"],
        body: r#"- Tests live next to code (Component.test.tsx) or in src/__tests__; import { describe, it, expect, vi } from 'vitest' and render/screen/userEvent from @testing-library (add @testing-library/user-event with add_dependency if interactions need it, and jsdom/@testing-library/jest-dom when missing).
- Query like a user: getByRole with name, getByLabelText, findBy* for async; avoid test ids unless nothing else works; never assert on class names or implementation details.
- Cover: the main happy path, validation/edge cases, empty states, persistence (mock localStorage per test, clear in beforeEach), and pure logic exhaustively.
- Fake timers for time-based logic (vi.useFakeTimers), restore after.
- Run run_check test; a failing test is either a wrong test or a real bug — say which."#,
    },
];

impl Builtin {
    fn to_skill(&self) -> Skill {
        Skill { id: self.id.into(), name: self.name.into(), description: self.description.into(), keywords: self.keywords.iter().map(|k| k.to_string()).collect(), body: self.body.into() }
    }
}

/// Built-in skills plus the project's own `.genius/skills/*.md` (which override by id).
pub fn all(ws: &Workspace) -> Vec<Skill> {
    let mut skills: Vec<Skill> = BUILTIN.iter().map(Builtin::to_skill).collect();
    let dir = ws.root().join(".genius").join("skills");
    let Ok(entries) = std::fs::read_dir(&dir) else { return skills };
    let mut files: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "md") && p.is_file()).collect();
    files.sort();
    for path in files.into_iter().take(40) {
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if meta.file_type().is_symlink() || meta.len() > 40_000 {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let id = path.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        if let Some(s) = parse_skill(&id, &text) {
            skills.retain(|b| b.id != s.id);
            skills.push(s);
        }
    }
    skills
}

/// Parse a user skill file: optional `---` front matter with name/description/keywords.
pub fn parse_skill(id: &str, text: &str) -> Option<Skill> {
    let id: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    if id.is_empty() {
        return None;
    }
    let mut s = Skill { id: id.clone(), name: id.clone(), description: String::new(), keywords: vec![], body: text.trim().to_string() };
    if let Some(rest) = text.trim_start().strip_prefix("---") {
        if let Some((front, body)) = rest.split_once("\n---") {
            for line in front.lines() {
                if let Some((k, v)) = line.split_once(':') {
                    let v = v.trim().trim_matches('"').to_string();
                    match k.trim() {
                        "name" => s.name = v,
                        "description" => s.description = v,
                        "keywords" => s.keywords = v.split(',').map(|k| k.trim().to_lowercase()).filter(|k| !k.is_empty()).collect(),
                        _ => {}
                    }
                }
            }
            s.body = body.trim().to_string();
        }
    }
    if s.description.is_empty() {
        s.description = s.body.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim_start_matches('#').trim().chars().take(120).collect();
    }
    (!s.body.is_empty()).then_some(s)
}

pub fn find(ws: &Workspace, name: &str) -> Option<Skill> {
    let n = name.trim().to_lowercase();
    all(ws).into_iter().find(|s| s.id == n || s.name.to_lowercase() == n)
}

/// The best-matching skills for a request (keyword hits, longer keywords weigh more).
pub fn suggest<'a>(skills: &'a [Skill], request: &str, max: usize) -> Vec<&'a Skill> {
    let r = format!(" {} ", request.to_lowercase());
    let mut scored: Vec<(usize, &Skill)> = skills
        .iter()
        .map(|s| {
            let score: usize = s.keywords.iter().filter(|k| r.contains(&format!(" {k}")) || (k.len() > 5 && r.contains(k.as_str()))).map(|k| 1 + k.len() / 6).sum();
            (score, s)
        })
        .filter(|(score, _)| *score > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().take(max).map(|(_, s)| s).collect()
}

/// Project instructions file: GENIUS.md, else AGENTS.md, else CLAUDE.md.
pub fn project_instructions(ws: &Workspace) -> Option<(&'static str, String)> {
    ["GENIUS.md", "AGENTS.md", "CLAUDE.md"].into_iter().find_map(|f| ws.read_text_opt(f).ok().flatten().filter(|t| !t.trim().is_empty()).map(|t| (f, t)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> (tempfile::TempDir, Workspace) {
        let d = tempfile::tempdir().unwrap();
        let w = Workspace::new(d.path()).unwrap();
        (d, w)
    }

    #[test]
    fn suggests_relevant_builtins() {
        let (_d, w) = ws();
        let skills = all(&w);
        let ids = |q: &str| suggest(&skills, q, 2).iter().map(|s| s.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids("I want a cool Dashboard for my stuff")[0], "dashboard");
        assert_eq!(ids("Preview just shows a black screen, please fix it")[0], "debugging");
        assert!(ids("build a snake game").contains(&"game".to_string()));
        assert!(ids("hello there").is_empty());
    }

    #[test]
    fn project_skills_and_instructions() {
        let (d, w) = ws();
        std::fs::create_dir_all(d.path().join(".genius/skills")).unwrap();
        std::fs::write(d.path().join(".genius/skills/brand.md"), "---\nname: Our brand\ndescription: house style\nkeywords: brand, style\n---\nUse teal.\n").unwrap();
        std::fs::write(d.path().join(".genius/skills/game.md"), "Always use canvas.").unwrap();
        let s = all(&w);
        let brand = s.iter().find(|s| s.id == "brand").unwrap();
        assert_eq!((brand.name.as_str(), brand.body.as_str(), brand.keywords.len()), ("Our brand", "Use teal.", 2));
        assert_eq!(find(&w, "game").unwrap().body, "Always use canvas.", "project skill overrides the built-in");
        assert!(project_instructions(&w).is_none());
        std::fs::write(d.path().join("AGENTS.md"), "Use pnpm").unwrap();
        assert_eq!(project_instructions(&w).unwrap().0, "AGENTS.md");
        std::fs::write(d.path().join("GENIUS.md"), "Use npm").unwrap();
        assert_eq!(project_instructions(&w).unwrap(), ("GENIUS.md", "Use npm".to_string()));
    }

    #[test]
    fn parse_skill_without_front_matter() {
        let s = parse_skill("My Skill!", "# Charts\nUse SVG.").unwrap();
        assert_eq!((s.id.as_str(), s.description.as_str()), ("MySkill", "Charts"));
        assert!(parse_skill("x", "  ").is_none());
    }
}
