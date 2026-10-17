#!/usr/bin/env python3
"""Drive the real Genius desktop app through WebDriver (tauri-driver + WebKitWebDriver).

Covers the beginner workflow through the actual UI: settings → key → model picker → test
connection → describe the app on Home → one-step creation (plan, code, real npm install,
build, dev server) → live preview (interacting with the running app inside the iframe) →
element picker → follow-up change applied from the conversation → a blocked proposal
corrected + new-package approval + preview restart → a broken edit repaired with
"Fix automatically" → terminal, history, Git commit, readiness → restart and reopen.

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
from selenium.webdriver.common.keys import Keys
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
    wait(d, "//h1[contains(., 'What do you want to build?')]")
    shot(d, "home-first-run")

    # ---- Settings: endpoint, key, models, test connection
    click(d, "//button[@aria-label='Settings']")
    wait(d, "//h2[.='NVIDIA API']")
    base = wait(d, "//label[.='API base URL']/following-sibling::input")
    set_input(base, f"http://127.0.0.1:{PORT}/v1")
    click(d, btn("Save generation settings"))
    text_present(d, "Settings saved")
    set_input(wait(d, "//input[@aria-label='NVIDIA API key']"), "nvapi-ui-test-key-0123456789")
    click(d, btn("Save key"))
    text_present(d, "Saved nvapi-")
    click(d, btn("Refresh models"))
    wait(d, "//*[@role='option'][.//span[.='mock/coder-1']]")
    assert not d.find_elements(By.XPATH, "//*[@role='option'][.//span[.='mock/embed-1']]"), "embedding model hidden by default"
    click(d, "//*[@role='option'][.//span[.='mock/coder-1']]")
    text_present(d, "Selected: mock/coder-1")
    click(d, btn("Test connection"))
    text_present(d, "Connected. 3 models available.", 30)
    shot(d, "settings-connected")
    log("settings OK")

    # ---- Describe the app on Home → one step: plan → code → install → build → preview
    click(d, "//button[@aria-label='Home']")
    set_input(wait(d, "//textarea[@aria-label='Describe your app']"), "A small todo list where I can add tasks and check them off.")
    shot(d, "home-describe")
    click(d, btn("Create"))
    text_present(d, "Ask Genius", 30)
    shot(d, "workspace-creating")
    log("creating: waiting for real plan → files → npm install → build → dev server…")
    frame = wait(d, "//iframe[@title='Application preview']", 360)
    time.sleep(2)
    shot(d, "preview-running")
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
    text_present(d, "Your app is ready", 120)
    text_present(d, "Preview is running at http://127.0.0.1:")
    text_present(d, "Production build succeeded")
    wait(d, "//header//span[contains(@class,'badge')][contains(., 'Preview running')]")
    shot(d, "preview-interacted")
    log("one-step creation: the generated app runs and responds inside the preview")

    # ---- Element picker: select the heading inside the running app
    click(d, "//button[@aria-label='Select an element to change']")
    frame = wait(d, "//iframe[@title='Application preview']")
    d.switch_to.frame(frame)
    wait(d, "//h1[.='Genius Todo']")
    d.execute_script("document.querySelector('h1').click()")
    d.switch_to.default_content()
    text_present(d, "Selected in preview:", 15)
    shot(d, "element-picked")
    click(d, "//button[@aria-label='Clear selected element']")
    log("element picker returned the clicked element")

    # ---- Follow-up in the same conversation → Apply in the chat → preview updates
    set_input(wait(d, "//textarea[@aria-label='Change request']"), "Add a footer that says Made with Genius")
    click(d, btn("Send"))
    apply_xp = "//div[contains(@class,'task-card')]//button[normalize-space(.)='Apply']"
    wait(d, apply_xp, 90)
    text_present(d, "Proposed changes")
    text_present(d, "What Genius did")
    text_present(d, "Files read: src/App.tsx")
    shot(d, "change-proposed")
    click(d, apply_xp)
    frame = wait(d, "//iframe[@title='Application preview']", 60)
    d.switch_to.frame(frame)
    wait(d, "//footer[contains(., 'Made with Genius')]", 120)
    d.switch_to.default_content()
    shot(d, "preview-after-change")
    log("change applied in the same project; preview hot-reloaded")

    # ---- A proposal that imports an undeclared package is blocked and corrected; the new
    #      package needs approval; dependencies are recalculated and the preview restarts.
    WebDriverWait(d, 240).until(lambda drv: not drv.find_elements(By.XPATH, "//header//span[contains(@class,'badge run')]"))
    set_input(wait(d, "//textarea[@aria-label='Change request']"), "Show the current year using dayjs")
    click(d, btn("Send"))
    wait(d, apply_xp, 120)
    shot(d, "dayjs-proposed")
    click(d, apply_xp)
    wait(d, "//*[@role='dialog'][@aria-label='Approval required']", 120)
    text_present(d, "Install 1 new package(s): dayjs")
    shot(d, "approval-new-package")
    click(d, "//*[@role='dialog']//button[.='Approve']")
    frame = wait(d, "//iframe[@title='Application preview']", 300)
    d.switch_to.frame(frame)
    wait(d, "//p[contains(@class,'year')][contains(., 'Year 2024')]", 300)
    d.switch_to.default_content()
    click(d, "//button[@role='tab'][contains(., 'Activity')]")
    text_present(d, "Blocked an invalid proposal", 15)
    shot(d, "preview-with-new-package")
    log("blocked proposal corrected; new package approved, installed; preview restarted")

    # ---- Advanced tools: break the app in the editor, then Fix automatically
    WebDriverWait(d, 240).until(lambda drv: not drv.find_elements(By.XPATH, "//header//span[contains(@class,'badge run')]"))
    click(d, btn("Show code & tools"))
    click(d, "//div[contains(@class,'tabs')]//div[@role='tab'][@title='src/App.tsx']")
    editor = wait(d, "//div[contains(@class,'cm-content')]", 30)
    editor.click()
    editor.send_keys(Keys.CONTROL, Keys.HOME)
    # a side-effect import is never elided (an unused `import x from` would be dropped by
    # esbuild, and the dev preview would rightly keep working)
    editor.send_keys("import 'left-pad';")
    editor.send_keys(Keys.ENTER)
    editor.send_keys(Keys.CONTROL, "s")
    click(d, "//button[@role='tab'][contains(., 'Preview')]")
    text_present(d, "Your website has an error", 60)
    text_present(d, "left-pad")
    shot(d, "preview-compile-error")
    click(d, "//div[contains(@class,'pv-problem')]//button[normalize-space(.)='Fix automatically']")
    wait(d, "//*[@role='dialog'][@aria-label='Approval required']", 180)
    text_present(d, "left-pad")
    click(d, "//*[@role='dialog']//button[.='Approve']")
    click(d, "//button[@role='tab'][contains(., 'Assistant')]")
    text_present(d, "Fixed. I applied 1 change(s) and verified the result", 360)
    shot(d, "fixed-automatically")
    click(d, "//button[@role='tab'][contains(., 'Preview')]")
    wait(d, "//iframe[@title='Application preview']", 60)
    log("Fix automatically repaired the missing dependency and verified it")

    # ---- Terminal + history
    click(d, "//button[@role='tab'][contains(., 'Terminal')]")
    text_present(d, "npm run build")
    shot(d, "terminal")
    click(d, "//button[@aria-label='History & recovery']")
    text_present(d, "AI generation")
    shot(d, "history")

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
    shot(d, "git-committed")
    log("git commit OK")

    # ---- Readiness report
    click(d, "//button[@role='tab'][contains(., 'Readiness')]")
    text_present(d, "Changes committed")
    shot(d, "readiness")

    d.quit()
    time.sleep(2)

    # ---- Restart: project persisted
    d = session()
    wait(d, "//h3[.='Small Todo List']", 30)  # name derived from the description
    text_present(d, "build succeeded")
    shot(d, "home-after-restart")
    click(d, "//*[contains(@class,'project-card')]")
    text_present(d, "Add a footer that says Made with Genius", 30)
    text_present(d, "Added a footer below the list")
    text_present(d, "Show the current year using dayjs")
    shot(d, "conversation-after-restart")
    log("conversation history persisted across restart")
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
