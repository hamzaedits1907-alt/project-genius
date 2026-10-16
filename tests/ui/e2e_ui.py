#!/usr/bin/env python3
"""Drive the real Genius desktop app through WebDriver (tauri-driver + WebKitWebDriver).

Covers the MVP workflow through the actual UI: settings → key → model picker → test
connection → describe app → plan → generate (with approval dialog) → live preview
(interacting with the running app inside the iframe) → change request → diff review →
apply → history → Git init/commit → restart and reopen.

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


def shot(d, name):
    global step_no
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

    # ---- Create project → plan
    click(d, "//button[@aria-label='New project']")
    set_input(wait(d, "//input[@id='pname']"), "Genius Todo")
    set_input(wait(d, "//textarea[@id='pdesc']"), "A small todo list where I can add tasks and check them off.")
    click(d, btn("Create plan"))
    wait(d, "//h2[.='Genius Todo']", 60)
    text_present(d, "Add tasks")
    shot(d, "plan-review")
    log("plan OK")

    # ---- Generate: approval for npm install, then real build and preview
    click(d, "//button[contains(., 'Generate application')]")
    wait(d, "//*[@role='dialog'][@aria-label='Approval required']", 90)
    text_present(d, "npm install --no-audit --no-fund")
    shot(d, "approval-install")
    click(d, "//*[@role='dialog']//button[.='Approve']")
    log("install approved; waiting for real install + build + dev server…")
    frame = wait(d, "//iframe[@title='Application preview']", 300)
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
    shot(d, "preview-interacted")
    log("generated app runs and responds inside the preview")

    # ---- Element picker: select the heading inside the running app
    click(d, "//button[@aria-label='Select an element to change']")
    frame = wait(d, "//iframe[@title='Application preview']")
    d.switch_to.frame(frame)
    wait(d, "//h1[.='Genius Todo']")
    d.execute_script("document.querySelector('h1').click()")
    d.switch_to.default_content()
    text_present(d, "Selected in preview:", 15)
    shot(d, "element-picked")
    log("element picker returned the clicked element")

    # ---- Change request → review diff → apply → verified build
    click(d, "//button[@role='tab'][contains(., 'Assistant')]")
    set_input(wait(d, "//textarea[@aria-label='Change request']"), "Add a footer that says Made with Genius")
    time.sleep(0.8)  # let the context preview compute
    click(d, btn("Send"))
    wait(d, "//button[starts-with(normalize-space(.), 'Apply 1 file')]", 90)
    time.sleep(0.5)
    shot(d, "review-proposed")
    click(d, "//button[starts-with(normalize-space(.), 'Apply 1 file')]")
    wait(d, "//*[contains(@class,'ev')]//*[contains(., 'Production build')]", 30)
    WebDriverWait(d, 240).until(lambda drv: drv.find_elements(By.XPATH, "//header//span[contains(@class,'badge')][contains(., 'build succeeded')]") and not drv.find_elements(By.XPATH, "//header//span[contains(@class,'badge run')]"))
    click(d, "//button[@role='tab'][contains(., 'Preview')]")
    frame = wait(d, "//iframe[@title='Application preview']")
    d.switch_to.frame(frame)
    wait(d, "//footer[contains(., 'Made with Genius')]", 60)
    d.switch_to.default_content()
    shot(d, "preview-after-change")
    log("change applied, build verified, preview hot-reloaded")

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
    wait(d, "//h3[.='Genius Todo']", 30)
    text_present(d, "build succeeded")
    shot(d, "home-after-restart")
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
