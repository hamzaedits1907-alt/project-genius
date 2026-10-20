// Signs in inside the running Android app (through the WebView debugging port) with a throw-away account and
// checks that the app really gets past the spinner to the chat screen. Prints what failed if it does not.
// Usage: node android-login-check.mjs [port]
const port = process.argv[2] ?? '9222';
const URL_ = process.env.GENIUS_URL ?? 'https://mkfcqdpuujnkikiwibgo.supabase.co';
const ANON = process.env.GENIUS_ANON ?? 'sb_publishable_UYlXKaRzyfFZuFSFocr7gg_popNqXXa';
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

// a fresh account for this run (the project has e-mail confirmation off)
const stamp = `${Date.now()}${Math.floor(Math.random() * 1e6)}`;
const email = `genius.ci.${stamp}@example.com`;
const password = `Ci-${stamp}-Pw!x`;
const su = await fetch(`${URL_}/auth/v1/signup`, { method: 'POST', headers: { apikey: ANON, 'content-type': 'application/json' }, body: JSON.stringify({ email, password }) });
if (!su.ok) {
  console.error(`::error::could not create the test account (${su.status}): ${(await su.text()).slice(0, 200)}`);
  process.exit(1);
}

const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = list.find((p) => p.type === 'page');
if (!page) {
  console.error('::error::no WebView page');
  process.exit(1);
}
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((res, rej) => {
  ws.onopen = res;
  ws.onerror = () => rej(new Error('debug socket'));
});
let id = 0;
const pending = new Map();
const events = [];
ws.onmessage = (m) => {
  const d = JSON.parse(m.data);
  if (d.id && pending.has(d.id)) {
    pending.get(d.id)(d.result);
    pending.delete(d.id);
  } else if (d.method) {
    if (d.method === 'Runtime.consoleAPICalled') events.push(`console.${d.params.type}: ${d.params.args.map((a) => a.value ?? a.description ?? '').join(' ').slice(0, 300)}`);
    else if (d.method === 'Runtime.exceptionThrown') events.push(`exception: ${(d.params.exceptionDetails.exception?.description ?? d.params.exceptionDetails.text).slice(0, 400)}`);
    else if (d.method === 'Network.loadingFailed') events.push(`request failed: ${d.params.errorText}`);
    else if (d.method === 'Network.responseReceived' && d.params.response.status >= 400) events.push(`http ${d.params.response.status}: ${d.params.response.url.slice(0, 120)}`);
  }
};
const call = (method, params = {}) =>
  new Promise((resolve) => {
    const mine = ++id;
    pending.set(mine, resolve);
    ws.send(JSON.stringify({ id: mine, method, params }));
  });
const evalJs = async (expression) => (await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true }))?.result?.value;
await call('Runtime.enable');
await call('Network.enable');

const typeInto = async (selector, text) => {
  await evalJs(`(() => { const e = document.querySelector(${JSON.stringify(selector)}); e.focus(); e.select && e.select(); })()`);
  await call('Input.insertText', { text });
};
await typeInto('input[type=email]', email);
await typeInto('input[type=password]', password);
await evalJs(`[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Sign in').click()`);

const t0 = Date.now();
let text = '';
let ok = false;
while (Date.now() - t0 < 60_000) {
  await wait(1500);
  text = (await evalJs('document.body ? document.body.innerText : ""')) ?? '';
  if (/How can I help|New chat/.test(text) && !/Welcome back/.test(text)) {
    ok = true;
    break;
  }
}
console.log(`after sign-in (${Math.round((Date.now() - t0) / 1000)} s) the app shows:\n${text.slice(0, 500)}`);
if (!ok) {
  console.error('::error::signed in, but the app never reached the chat screen');
  console.error(events.slice(-40).join('\n'));
  process.exit(1);
}
console.log('OK: sign-in works inside the app and the chat screen is reached');
process.exit(0);
