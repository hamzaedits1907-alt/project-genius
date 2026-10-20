// Reads the page the Android app's WebView is showing (through the WebView's debugging port that adb forwarded
// to localhost:9222) and checks that Genius's own screen is on it. Usage: node android-webview-check.mjs [port]
import { writeFileSync } from 'node:fs';

const port = process.argv[2] ?? '9222';
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
let page = null;
for (let i = 0; i < 30 && !page; i++) {
  try {
    const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
    page = list.find((p) => p.type === 'page');
  } catch {
    /* the app is still starting */
  }
  if (!page) await wait(2000);
}
if (!page) {
  console.error('::error::no WebView page found on the debugging port');
  process.exit(1);
}
console.log(`WebView page: "${page.title}" at ${page.url}`);

const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  ws.onopen = resolve;
  ws.onerror = () => reject(new Error('could not open the debugging socket'));
});
let id = 0;
const call = (method, params = {}) =>
  new Promise((resolve) => {
    const mine = ++id;
    const on = (m) => {
      const d = JSON.parse(m.data);
      if (d.id === mine) {
        ws.removeEventListener('message', on);
        resolve(d.result);
      }
    };
    ws.addEventListener('message', on);
    ws.send(JSON.stringify({ id: mine, method, params }));
  });

let text = '';
for (let i = 0; i < 20; i++) {
  const r = await call('Runtime.evaluate', { expression: 'document.body ? document.body.innerText : ""', returnByValue: true });
  text = r?.result?.value ?? '';
  if (/Welcome back|Sign in|Genius/.test(text)) break;
  await wait(1500);
}
const errors = await call('Runtime.evaluate', { expression: 'document.querySelector("[role=alert]") ? document.querySelector("[role=alert]").innerText : ""', returnByValue: true });
writeFileSync('webview-text.txt', text);
ws.close();
console.log('--- what the app shows ---\n' + text.slice(0, 600));
if (!/Welcome back/.test(text) || !/Email/.test(text)) {
  console.error('::error::the sign-in screen is not on the WebView page');
  process.exit(1);
}
if (errors?.result?.value) {
  console.error('::error::the app shows an error: ' + errors.result.value);
  process.exit(1);
}
console.log('OK: the app is running and shows the Genius sign-in screen');
process.exit(0);
