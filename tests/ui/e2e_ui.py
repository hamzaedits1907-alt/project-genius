#!/usr/bin/env python3
"""Drive the real Genius desktop app through WebDriver (tauri-driver + WebKitWebDriver).

Covers the main workflow through the actual UI: first-run quick setup on Home (key → model
picked automatically) → one prompt in simple mode → plan → code → install approval → the
preview starts by itself (interacting with the running app inside the iframe) → Genius asks
a question with clickable answers → element picker → change request → inline Apply in chat
→ verified build → hot reload → Advanced mode (terminal, history, Git init/commit,
readiness) → update check against GitHub Releases → restart and reopen.

Only the NVIDIA API is replaced by tests/ui/mock_nvidia.py. Requires: Xvfb, tauri-driver,
WebKitWebDriver, selenium, Node/npm, and a debug build (npx tauri build --debug --no-bundle).

Usage: python3 tests/ui/e2e_ui.py <screenshot-dir>
"""
import os
import subprocess
import sys
import tempfile
import time

from selenium import webdriver
from selenium.webdriver.common.by import By
from selenium.webdriver.common.options import ArgOptions
from selenium.webdriver.support import expected_conditions as EC
from selenium.webdriver.support.ui import WebDriverWait

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
APP = os.path.join(ROOT, "target", "debug", "genius")
SHOTS = sys.argv[1] if len(sys.argv) > 1 else tempfile.mkdtemp()
PORT = 18787
os.makedirs(SHOTS, exist_ok=True)

home = tempfile.mkdtemp(prefix="genius-ui-")
env = dict(os.environ, DISPLAY=":99", HOME=home, XDG_DATA_HOME=os.path.join(home, "data"), XDG_CONFIG_HOME=os.path.join(home, "config"))
procs = [
    subprocess.Popen(["Xvfb", ":99", "-screen", "0", "1600x1000x24"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL),
    subprocess.Popen([sys.executable, os.path.join(ROOT, "tests", "ui", "mock_nvidia.py"), str(PORT)]),
]
time.sleep(1.5)
procs.append(subprocess.Popen(["tauri-driver"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
time.sleep(1.5)

step_no = 0


def log(msg):
    print(f"[ui] {msg}", flush=True)


def session():
    opts = ArgOptions()
    opts.set_capability("browserName", "wry")
    opts.set_capability("tauri:options", {"application": APP})
    d = webdriver.Remote(command_executor="http://127.0.0.1:4444", options=opts)
    d.set_window_size(1600, 1000)
    return d


CRASH_HOOK = """
if (!window.__crash) {
  window.__crash = [];
  const push = (m) => window.__crash.push(String(m).slice(0, 2000));
  window.addEventListener('error', (e) => push('error: ' + ((e.error && e.error.stack) || e.message)));
  window.addEventListener('unhandledrejection', (e) => push('rejection: ' + String((e.reason && (e.reason.stack || e.reason.message)) || e.reason)));
  const ce = console.error.bind(console);
  console.error = (...a) => { push('console: ' + a.map((x) => (x && x.stack) || String(x)).join(' ')); ce(...a); };
}
"""
crashes = []


def check_ui(d, name):
    """Fail on any error-boundary screen or React effect-cleanup error (regression guard)."""
    d.execute_script(CRASH_HOOK)
    errs = d.execute_script("return window.__crash.splice(0)")
    boundary = d.execute_script("const h=[...document.querySelectorAll('.empty h3')].find(x=>x.textContent.includes('Something went wrong'));return h?h.parentElement.textContent:null")
    for e in errs:
        log(f"  [{name}] {e[:600]}")
    bad = [e for e in errs if 'is not a function' in e or 'must not return anything' in e]
    if boundary or bad:
        crashes.append((name, boundary, bad))
        raise AssertionError(f"UI crash at {name}: {boundary or bad[0]}")


def shot(d, name):
    global step_no
    d.switch_to.default_content()
    check_ui(d, name)
    step_no += 1
    path = os.path.join(SHOTS, f"{step_no:02d}-{name}.png")
    d.save_screenshot(path)
    log(f"screenshot {path}")


def wait(d, xpath, timeout=30):
    return WebDriverWait(d, timeout).until(EC.presence_of_element_located((By.XPATH, xpath)))


def click(d, xpath, timeout=30):
    el = WebDriverWait(d, timeout).until(EC.element_to_be_clickable((By.XPATH, xpath)))
    d.execute_script("arguments[0].scrollIntoView({block:'center'})", el)
    el.click()
    return el


def text_present(d, text, timeout=30):
    return wait(d, f"//*[contains(normalize-space(.), {xq(text)})]", timeout)


def xq(s):
    return "'" + s + "'" if "'" not in s else '"' + s + '"'


def btn(text):
    return f"//button[normalize-space(.)={xq(text)} or .//text()[normalize-space()={xq(text)}]]"


def set_input(el, value):
    el.clear()
    el.send_keys(value)


ok = False
try:
    d = session()
    wait(d, "//h1[contains(., 'What should we build today?')]")
    text_present(d, "Connect Genius to its AI brain")
    shot(d, "home-first-run")

    # ---- Point Genius at the mock NVIDIA endpoint (test-only), then quick setup on Home
    click(d, "//button[@aria-label='Settings']")
    wait(d, "//h2[.='NVIDIA API']")
    base = wait(d, "//label[.='API base URL']/following-sibling::input")
    set_input(base, f"http://127.0.0.1:{PORT}/v1")
    click(d, btn("Save generation settings"))
    text_present(d, "Settings saved")
    click(d, "//button[@aria-label='Home']")
    set_input(wait(d, "//input[@aria-label='NVIDIA API key']"), "nvapi-ui-test-key-0123456789")
    click(d, btn("Connect"))
    text_present(d, "All set! Genius will use mock/", 60)
    wait(d, "//button[@aria-label='Build it']")
    assert not d.find_elements(By.XPATH, "//*[contains(., 'Connect Genius to its AI brain')]/ancestor::div[contains(@class,'quick-setup')]"), "setup card gone"
    shot(d, "home-ready")
    log("quick setup OK (key saved, model picked automatically)")

    # ---- Simple mode: one prompt → plan → code → install approval → running app
    set_input(wait(d, "//textarea[@aria-label='Describe your app']"), "A small todo list where I can add tasks and check them off.")
    click(d, "//button[@aria-label='Build it']")
    wait(d, "//*[contains(@class,'chat-name')]", 30)
    assert not d.find_elements(By.XPATH, "//button[@aria-label='Source control']"), "simple mode hides the IDE"
    wait(d, "//*[@role='dialog'][@aria-label='Approval required']", 180)
    text_present(d, "npm install --no-audit --no-fund")
    shot(d, "simple-approval-install")
    click(d, "//*[@role='dialog']//button[.='Approve']")
    log("install approved; waiting for generation, real install and the auto-started preview…")
    frame = wait(d, "//iframe[@title='Application preview']", 300)
    time.sleep(2)
    shot(d, "simple-preview-running")
    d.switch_to.frame(frame)
    wait(d, "//h1[.='Genius Todo']", 60)
    # WebKitWebDriver cannot focus elements inside a cross-origin iframe, so interact via
    # DOM events dispatched inside the frame (exercises the real React handlers).
    wait(d, "//input[@aria-label='New task']")
    d.execute_script("""
      const input = document.querySelector('input[aria-label="New task"]');
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, 'Write the report');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    """)
    time.sleep(0.2)
    d.execute_script("[...document.querySelectorAll('button')].find(b => b.textContent === 'Add task').click()")
    wait(d, "//li[contains(., 'Write the report')]")
    d.switch_to.default_content()
    shot(d, "simple-preview-interacted")
    log("one prompt produced a running app that responds inside the preview")

    # ---- Genius asks a question with one-click answers
    set_input(wait(d, "//textarea[@aria-label='Change request']"), "Surprise me")
    click(d, "//button[@aria-label='Send']")
    text_present(d, "Which style do you prefer?", 60)
    shot(d, "ask-question")
    click(d, btn("Bold & colorful"))
    text_present(d, "bold and colorful", 60)
    log("question with clickable answers OK")

    # ---- Element picker → change request → inline Apply in chat → verified build → hot reload
    click(d, "//button[@aria-label='Select an element to change']")
    frame = wait(d, "//iframe[@title='Application preview']")
    d.switch_to.frame(frame)
    wait(d, "//h1[.='Genius Todo']")
    d.execute_script("document.querySelector('h1').click()")
    d.switch_to.default_content()
    text_present(d, "Selected in preview:", 15)
    set_input(wait(d, "//textarea[@aria-label='Change request']"), "Add a footer that says Made with Genius")
    click(d, "//button[@aria-label='Send']")
    apply = "//div[contains(@class,'proposal')]//button[normalize-space(.)='Apply']"
    wait(d, apply, 90)
    text_present(d, "Proposed changes")
    text_present(d, "What Genius did")
    shot(d, "simple-proposal")
    click(d, apply)
    # The automatic verification build of the applied change succeeds (shown on the proposal).
    wait(d, "//div[contains(@class,'proposal')]//*[contains(@class,'badge')][contains(., 'build succeeded')]", 240)
    WebDriverWait(d, 60).until(lambda drv: drv.find_elements(By.XPATH, "//header//span[contains(@class,'pill ok')][contains(., 'Ready')]"))
    frame = wait(d, "//iframe[@title='Application preview']")
    d.switch_to.frame(frame)
    wait(d, "//footer[contains(., 'Made with Genius')]", 60)
    d.switch_to.default_content()
    shot(d, "simple-after-change")
    log("change applied from the chat, build verified, preview hot-reloaded")

    # ---- Code view in simple mode
    click(d, "//button[@role='tab'][contains(., 'Code')]")
    wait(d, "//*[contains(@class,'tree-row')][contains(., 'App.tsx')]")
    shot(d, "simple-code")

    # ---- Switch to Advanced: the full IDE
    click(d, "//*[@role='radio'][normalize-space(.)='Advanced']")
    wait(d, "//button[@aria-label='Source control']")
    click(d, "//button[@role='tab'][contains(., 'Terminal')]")
    text_present(d, "npm run build")
    shot(d, "advanced-terminal")
    click(d, "//button[@aria-label='History & recovery']")
    text_present(d, "AI generation")
    shot(d, "advanced-history")

    # ---- Git: init, identity, stage, commit
    click(d, "//button[@aria-label='Source control']")
    click(d, btn("Initialize repository"))
    set_input(wait(d, "//input[@placeholder='Your name']"), "UI Test")
    set_input(wait(d, "//input[@placeholder='you@example.com']"), "ui@example.com")
    click(d, btn("Save for this repository"))
    click(d, btn("Stage all"))
    set_input(wait(d, "//textarea[@aria-label='Commit message']"), "Initial version of Genius Todo")
    click(d, "//button[starts-with(normalize-space(.), 'Commit')]")
    text_present(d, "Initial version of Genius Todo", 30)
    text_present(d, "Working tree clean")
    shot(d, "advanced-git-committed")
    log("git commit OK")

    click(d, "//button[@role='tab'][contains(., 'Readiness')]")
    text_present(d, "Changes committed")
    shot(d, "advanced-readiness")

    # ---- Settings: updates (real GitHub check) and interface mode
    click(d, "//button[@aria-label='Settings']")
    click(d, "//nav//button[contains(., 'Updates & about')]")
    text_present(d, "Version 0.")
    click(d, btn("Check for updates"))
    WebDriverWait(d, 40).until(lambda drv: drv.find_elements(By.XPATH, "//*[contains(@class,'badge')][contains(., 'Up to date') or contains(., 'available')]") or drv.find_elements(By.XPATH, "//*[contains(@class,'toast')]"))
    time.sleep(1)
    shot(d, "settings-updates")
    log("update check answered: " + d.execute_script("return [...document.querySelectorAll('.toast, .badge')].map(x=>x.textContent).join(' | ')")[:300])

    d.quit()
    time.sleep(2)

    # ---- Restart: project, conversation and the chosen mode persist
    d = session()
    wait(d, "//h3[.='Todo List Tasks' or contains(.,'Todo')]", 30)
    text_present(d, "build succeeded")
    shot(d, "home-after-restart")
    click(d, "//*[contains(@class,'project-card')]")
    text_present(d, "Add a footer that says Made with Genius", 30)
    text_present(d, "Added a footer below the list")
    wait(d, "//button[@aria-label='Source control']")
    shot(d, "conversation-after-restart")
    log("conversation history and advanced mode persisted across restart")
    d.quit()
    ok = True
    log("ALL UI STEPS PASSED")
except Exception as e:  # noqa: BLE001
    log(f"FAILED: {type(e).__name__}: {e}")
    try:
        shot(d, "failure")
    except Exception:  # noqa: BLE001
        pass
    raise
finally:
    for p in reversed(procs):
        p.terminate()
    subprocess.run(["pkill", "-f", "target/debug/genius"], check=False)
    sys.exit(0 if ok else 1)
