//! Deterministic starter template for the MVP project type (Vite + React + TypeScript).
//!
//! This is a real, buildable project skeleton written by Genius itself (clearly labelled
//! as a template in history), not AI output. The model then writes the application code
//! on top of it. Keeping toolchain configuration deterministic removes the most common
//! source of broken generated projects.

use serde_json::json;

pub const KIND_VITE_REACT: &str = "vite-react-ts";

/// npm scripts Genius ships and therefore trusts to run without extra approval.
/// If a script is changed (e.g. by the AI), running it requires explicit approval and the
/// exact script text is shown to the user.
pub fn trusted_scripts() -> Vec<(&'static str, &'static str)> {
    vec![
        ("dev", "vite"),
        ("build", "tsc --noEmit && vite build"),
        ("typecheck", "tsc --noEmit"),
        ("test", "vitest run --passWithNoTests"),
        ("preview", "vite preview"),
    ]
}

pub fn slugify(name: &str) -> String {
    let mut s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-').chars().take(48).collect::<String>();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "genius-app".into()
    } else {
        s
    }
}

pub fn files(name: &str, description: &str) -> Vec<(String, String)> {
    let slug = slugify(name);
    let scripts: serde_json::Map<String, serde_json::Value> =
        trusted_scripts().into_iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
    let pkg = json!({
        "name": slug,
        "private": true,
        "version": "0.1.0",
        "type": "module",
        "scripts": scripts,
        "dependencies": {
            "react": "^19.1.0",
            "react-dom": "^19.1.0"
        },
        "devDependencies": {
            "@testing-library/react": "^16.3.0",
            "@types/react": "^19.1.0",
            "@types/react-dom": "^19.1.0",
            "@vitejs/plugin-react": "^5.0.0",
            "jsdom": "^26.1.0",
            "typescript": "~5.9.3",
            "vite": "^7.1.0",
            "vitest": "^3.2.0"
        }
    });
    let title = html_escape(name);
    vec![
        ("package.json".into(), format!("{}\n", serde_json::to_string_pretty(&pkg).unwrap_or_default())),
        (
            "index.html".into(),
            format!(
                r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <meta name="description" content="{desc}" />
    <title>{title}</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
"#,
                desc = html_escape(&description.chars().take(150).collect::<String>())
            ),
        ),
        (
            "tsconfig.json".into(),
            r#"{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "moduleResolution": "bundler",
    "jsx": "react-jsx",
    "strict": true,
    "noEmit": true,
    "isolatedModules": true,
    "allowImportingTsExtensions": true,
    "skipLibCheck": true,
    "noUnusedLocals": false,
    "noUnusedParameters": false,
    "noFallthroughCasesInSwitch": true,
    "types": ["vite/client"]
  },
  "include": ["src", "vite.config.ts"]
}
"#
            .into(),
        ),
        (
            "vite.config.ts".into(),
            r#"/// <reference types="vitest/config" />
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'jsdom',
  },
});
"#
            .into(),
        ),
        (
            "src/main.tsx".into(),
            r#"import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import App from './App';
import './index.css';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
"#
            .into(),
        ),
        (
            "src/App.tsx".into(),
            format!(
                r#"// Genius template. The AI replaces this file when it generates the application.
export default function App() {{
  return (
    <main style={{{{ fontFamily: 'system-ui', padding: 32 }}}}>
      <h1>{title}</h1>
      <p>Project template created. Application code has not been generated yet.</p>
    </main>
  );
}}
"#
            ),
        ),
        (
            "src/index.css".into(),
            ":root {\n  font-family: system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif;\n  line-height: 1.5;\n}\n\n* {\n  box-sizing: border-box;\n}\n\nbody {\n  margin: 0;\n}\n".into(),
        ),
        (
            ".gitignore".into(),
            "node_modules\ndist\ncoverage\n*.log\n.DS_Store\n.env\n.env.*\n!.env.example\n.vite\n".into(),
        ),
    ]
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slugify("My Task App!"), "my-task-app");
        assert_eq!(slugify("  "), "genius-app");
        assert_eq!(slugify("Über ☃ Tool"), "ber-tool");
    }

    #[test]
    fn template_is_consistent() {
        let f = files("Demo <App>", "desc");
        let pkg: serde_json::Value = serde_json::from_str(&f[0].1).unwrap();
        assert_eq!(pkg["name"], "demo-app");
        assert_eq!(pkg["scripts"]["build"], "tsc --noEmit && vite build");
        assert!(f.iter().any(|(p, _)| p == "src/main.tsx"));
        let html = &f.iter().find(|(p, _)| p == "index.html").unwrap().1;
        assert!(html.contains("Demo &lt;App&gt;"));
        assert!(f.iter().find(|(p, _)| p == ".gitignore").unwrap().1.contains(".env"));
    }
}
