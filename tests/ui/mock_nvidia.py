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


def answer(system, user):
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
        text = answer(msgs[0]["content"], msgs[1]["content"] if len(msgs) > 1 else "")
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
