#!/usr/bin/env python3
"""Local stand-in for the NVIDIA API, used ONLY by the automated UI test.

It implements the two endpoints Genius uses (GET /v1/models and streaming
POST /v1/chat/completions) and returns canned, deterministic content so the real
desktop app can be driven end to end without network access or credentials.
It is never used by Genius at runtime.

Usage: python3 mock_nvidia.py <port>
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

KEY = "nvapi-ui-test-key-0123456789"

APP = """import { useState } from 'react';
import './App.css';

type Item = { id: number; text: string; done: boolean };

export default function App() {
  const [items, setItems] = useState<Item[]>([]);
  const [text, setText] = useState('');
  const add = () => {
    if (!text.trim()) return;
    setItems([...items, { id: Date.now(), text: text.trim(), done: false }]);
    setText('');
  };
  return (
    <main className="app">
      <h1>Genius Todo</h1>
      <div className="add">
        <input aria-label="New task" value={text} onChange={(e) => setText(e.target.value)} />
        <button onClick={add}>Add task</button>
      </div>
      <ul>
        {items.map((i) => (
          <li key={i.id}>
            <label>
              <input type="checkbox" checked={i.done} onChange={() => setItems(items.map((x) => (x.id === i.id ? { ...x, done: !x.done } : x)))} />
              {i.text}
            </label>
          </li>
        ))}
      </ul>
    </main>
  );
}
"""

CSS = ".app { font-family: system-ui; max-width: 480px; margin: 40px auto; }\n.add { display: flex; gap: 8px; }\n"

APP_FOOTER = APP.replace("      </ul>\n    </main>", "      </ul>\n      <footer>Made with Genius</footer>\n    </main>")


def block(path, content):
    return f"<<<FILE: {path}>>>\n{content}<<<END FILE>>>\n"


APP_DAYJS = "import dayjs from 'dayjs';\n" + APP_FOOTER.replace("      <footer>Made with Genius</footer>", "      <p className=\"year\">Year {dayjs('2024-01-02').format('YYYY')}</p>\n      <footer>Made with Genius</footer>")


def current(text, path):
    """Content of <<<CURRENT: path>>> in a Genius context block."""
    marker = f"<<<CURRENT: {path}>>>\n"
    start = text.find(marker)
    if start < 0:
        return None
    start += len(marker)
    return text[start:text.find("<<<END CURRENT>>>", start)]


def numbered(text):
    """File content from a read_file tool result (strips Genius's line numbers)."""
    out = []
    for line in text.split("\n")[2:]:
        if line.startswith("</tool_result>"):
            break
        stripped = line.lstrip()
        num, _, rest = stripped.partition(" ")
        if num.isdigit():
            out.append(rest)
    return "\n".join(out) + "\n"


def with_dep(pkg_text, name, version):
    pkg = json.loads(pkg_text)
    pkg.setdefault("dependencies", {})[name] = version
    return json.dumps(pkg, indent=2) + "\n"


def request_of(msgs):
    for m in reversed(msgs):
        c = m["content"] if isinstance(m["content"], str) else ""
        if "USER MESSAGE:" in c:
            return c.split("USER MESSAGE:", 1)[1].strip()
    return ""


def answer(system, msgs):
    last = msgs[-1]["content"] if isinstance(msgs[-1]["content"], str) else ""
    if "Reviewer subagent" in system:
        return "<final>No blocking issues found.</final>"
    if "You are Genius, an AI software engineer" in system:
        req = request_of(msgs)
        if "dayjs" in req:
            # The first proposal forgets package.json; Genius blocks it and says why; the model
            # then reads package.json and proposes both files.
            if last.startswith('<tool_result name="read_file"') and "FILE package.json" in last:
                return "<final>Shows the year using the dayjs package.\n" + block("package.json", with_dep(numbered(last), "dayjs", "^1.11.13")) + block("src/App.tsx", APP_DAYJS) + "</final>"
            if "BLOCKED" in last:
                assert "'dayjs', which is not listed in package.json" in last, last
                return 'I need to add the package.\n<tool>{"name": "read_file", "args": {"path": "package.json"}}</tool>'
            return "<final>Shows the year using dayjs.\n" + block("src/App.tsx", APP_DAYJS) + "</final>"
        # Agent tool protocol: inspect the file first, then propose the change.
        if last.startswith('<tool_result name="read_file"'):
            assert "Genius Todo" in last, "the agent received the real file"
            return "<final>Added a footer below the list.\n" + block("src/App.tsx", APP_FOOTER) + "</final>"
        return 'I will read the app first.\n<tool>{"name": "read_file", "args": {"path": "src/App.tsx"}}</tool>'
    if "Build Doctor" in system:
        user = msgs[1]["content"]
        assert "left-pad" in user, "the doctor received the real error"
        pkg = current(user, "package.json")
        assert pkg, "package.json was included as context"
        return "Root cause: src/App.tsx imports left-pad, which is not listed in package.json.\n" + block("package.json", with_dep(pkg, "left-pad", "^1.3.0"))
    if "planning engine" in system:
        return json.dumps({
            "name": "Genius Todo", "purpose": "A small todo list.",
            "features": [{"name": "Add tasks", "description": "Type and add"}, {"name": "Complete tasks", "description": "Check them off"}],
            "pages": [{"name": "Home", "description": "The list", "components": ["App"]}],
            "components": [], "dataModels": [{"name": "Item", "fields": ["id: number", "text: string", "done: boolean"], "relations": []}],
            "architecture": "Single page, React state.", "dependencies": [], "milestones": ["List", "Add", "Complete"],
            "testStrategy": "Component tests", "risks": [], "assumptions": ["No persistence"],
            "files": [{"path": "src/App.tsx", "purpose": "root"}],
        })
    if "implementation engine" in system:
        return "A minimal todo app.\n" + block("src/App.tsx", APP) + block("src/App.css", CSS) + block("README.md", "# Genius Todo\n\nRun `npm install` then `npm run dev`.\n")
    if "code-editing engine" in system:
        return "Added a footer.\n" + block("src/App.tsx", APP_FOOTER)
    if "commit message" in system:
        return "Add todo app"
    return "ready"


class H(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def _json(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/v1/models":
            return self._json(200, {"object": "list", "data": [
                {"id": "mock/coder-1", "object": "model", "owned_by": "mock"},
                {"id": "mock/chat-2", "object": "model", "owned_by": "mock"},
                {"id": "mock/embed-1", "object": "model", "owned_by": "mock"},
            ]})
        self._json(404, {"detail": "not found"})

    def do_POST(self):
        if self.path != "/v1/chat/completions":
            return self._json(404, {"detail": "not found"})
        if self.headers.get("authorization") != f"Bearer {KEY}":
            return self._json(401, {"status": 401, "detail": "Authentication failed"})
        req = json.loads(self.rfile.read(int(self.headers.get("content-length", 0))))
        msgs = req["messages"]
        try:
            text = answer(msgs[0]["content"], msgs)
        except AssertionError as e:  # surface scripted-protocol violations in the test output
            print(f"[mock] ASSERTION: {e}", file=sys.stderr, flush=True)
            text = f"mock assertion failed: {e}"
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.end_headers()
        import time
        for i in range(0, len(text), 40):
            chunk = {"choices": [{"index": 0, "delta": {"content": text[i:i + 40]}, "finish_reason": None}]}
            self.wfile.write(f"data: {json.dumps(chunk)}\n\n".encode())
            self.wfile.flush()
            time.sleep(0.01)
        self.wfile.write(b'data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n')


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
