#!/usr/bin/env python3
"""Smoke-walk every view of the real Genius app (no AI calls) and fail on any UI crash.

Captures console errors and the error-boundary text after each step. Used to reproduce and
guard the "destroy is not a function" regression in the real webview (not jsdom).
Usage: python3 tests/ui/walk_views.py <screenshot-dir>
"""
import json, os, subprocess, sys, tempfile, time
from selenium import webdriver
from selenium.webdriver.common.by import By
from selenium.webdriver.common.options import ArgOptions

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
APP = os.path.join(ROOT, "target", "debug", "genius")
SHOTS = sys.argv[1] if len(sys.argv) > 1 else tempfile.mkdtemp()
os.makedirs(SHOTS, exist_ok=True)
home = tempfile.mkdtemp(prefix="genius-walk-")
env = dict(os.environ, DISPLAY=":98", HOME=home, XDG_DATA_HOME=os.path.join(home, "data"), XDG_CONFIG_HOME=os.path.join(home, "config"))
procs = [subprocess.Popen(["Xvfb", ":98", "-screen", "0", "1600x1000x24"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)]
time.sleep(1)
procs.append(subprocess.Popen(["tauri-driver", "--port", "4455", "--native-port", "4456"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
time.sleep(1.5)

HOOK = """
if (!window.__walk) {
  window.__walk = [];
  const push = (m) => window.__walk.push(String(m));
  window.addEventListener('error', (e) => push('error: ' + (e.error && e.error.stack || e.message)));
  window.addEventListener('unhandledrejection', (e) => push('rejection: ' + (e.reason && (e.reason.stack || e.reason.message) || JSON.stringify(e.reason))));
  const ce = console.error.bind(console);
  console.error = (...a) => { push('console: ' + a.map((x) => x && x.stack || (typeof x === 'object' ? JSON.stringify(x) : String(x))).join(' ')); ce(...a); };
}
"""
failures = []
n = 0

def check(d, step):
    global n
    n += 1
    time.sleep(0.8)
    d.execute_script(HOOK)
    errs = d.execute_script("const e = window.__walk.splice(0); return e;")
    boundary = d.execute_script("const h=[...document.querySelectorAll('.empty h3')].find(x=>x.textContent.includes('Something went wrong'));return h?h.parentElement.textContent:null")
    d.save_screenshot(os.path.join(SHOTS, f"{n:02d}-{step}.png"))
    print(f"[walk] {step}: boundary={boundary!r} errors={len(errs)}", flush=True)
    for e in errs:
        print("    ", e[:1500], flush=True)
    if boundary or any("is not a function" in e for e in errs):
        failures.append((step, boundary, errs))

def btn(d, label):
    return d.find_element(By.XPATH, f"//button[@aria-label={json.dumps(label)} or normalize-space(.)={json.dumps(label)}]")

try:
    opts = ArgOptions()
    opts.set_capability("browserName", "wry")
    opts.set_capability("tauri:options", {"application": APP})
    d = webdriver.Remote(command_executor="http://127.0.0.1:4455", options=opts)
    d.set_window_size(1600, 1000)
    time.sleep(3)
    d.execute_script(HOOK)
    check(d, "home")
    # A plain project via the real backend (no AI involved).
    pid = d.execute_async_script("""const done = arguments[arguments.length-1];
      window.__TAURI_INTERNALS__.invoke('create_project', {name: 'Walk', description: 'walk test'}).then(p => done(p.id), e => done('ERR ' + JSON.stringify(e)));""")
    print("[walk] project", pid)
    for label in ["New project", "Settings", "Home"]:
        btn(d, label).click(); check(d, label)
    btn(d, "Settings").click()
    for tab in ["GitHub", "Approvals", "Workspace & tools", "Appearance", "AI provider"]:
        try:
            d.find_element(By.XPATH, f"//button[contains(normalize-space(.), {json.dumps(tab)})]").click()
        except Exception as e:
            print("   (no tab)", tab, e.__class__.__name__)
        check(d, f"settings-{tab}")
    btn(d, "Home").click(); check(d, "home2")
    d.find_element(By.CSS_SELECTOR, ".project-card").click()
    time.sleep(2); check(d, "workspace")
    for label in ["Search", "Source control", "History & recovery", "Project memory", "Files"]:
        btn(d, label).click(); check(d, f"rail-{label}")
    for tab in ["Terminal", "Readiness", "Activity", "Preview", "Assistant"]:
        d.find_element(By.XPATH, f"//*[@role='tab' and contains(normalize-space(.), {json.dumps(tab)})]").click(); check(d, f"tab-{tab}")
    btn(d, "Home").click(); check(d, "leave-workspace")
    btn(d, "Open workspace").click(); time.sleep(1); check(d, "reenter-workspace")
    btn(d, "Settings").click(); check(d, "settings-from-workspace")
    d.quit()
finally:
    for p in procs:
        p.terminate()

print(f"[walk] {'FAIL' if failures else 'OK'}: {len(failures)} failing step(s)")
sys.exit(1 if failures else 0)
