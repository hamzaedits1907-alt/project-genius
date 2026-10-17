#!/usr/bin/env python3
"""Prove the in-app updater end to end with REAL releases from GitHub (Linux AppImage).

1. Downloads the AppImage of an older release (default: the release before the latest).
2. Starts it under Xvfb through WebDriver (tauri-driver + WebKitWebDriver).
3. The app finds the newer release on GitHub; the test clicks "Update now".
4. The app downloads the new AppImage, verifies its published SHA-256, replaces its own
   AppImage file and relaunches it.
5. The test checks the file on disk is byte-identical to the new release asset, then starts
   it again and checks the UI reports the new version and greets the user after the update.

Usage: python3 tests/ui/update_e2e.py <screenshot-dir> [old-tag]
Needs: network access to github.com, Xvfb, tauri-driver, WebKitWebDriver, selenium.
"""
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request

from selenium import webdriver
from selenium.webdriver.common.by import By
from selenium.webdriver.common.options import ArgOptions
from selenium.webdriver.support import expected_conditions as EC
from selenium.webdriver.support.ui import WebDriverWait

REPO = "hamzaedits1907-alt/project-genius"
SHOTS = sys.argv[1] if len(sys.argv) > 1 else tempfile.mkdtemp()
os.makedirs(SHOTS, exist_ok=True)


def log(msg):
    print(f"[update] {msg}", flush=True)


def api(path):
    req = urllib.request.Request(f"https://api.github.com/repos/{REPO}{path}", headers={"Accept": "application/vnd.github+json", "User-Agent": "genius-update-e2e"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


def fetch(url, dest):
    req = urllib.request.Request(url, headers={"User-Agent": "genius-update-e2e"})
    with urllib.request.urlopen(req, timeout=600) as r, open(dest, "wb") as f:
        shutil.copyfileobj(r, f)


def appimage(release):
    return next(a for a in release["assets"] if a["name"].endswith(".AppImage"))


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


releases = [r for r in api("/releases") if not r["draft"]]
latest = api("/releases/latest")
old = next(r for r in releases if r["tag_name"] == sys.argv[2]) if len(sys.argv) > 2 else next(r for r in releases if r["tag_name"] != latest["tag_name"])
log(f"old release {old['tag_name']} → latest {latest['tag_name']}")
new_asset = appimage(latest)
new_sha = next((a for a in latest["assets"] if a["name"] == new_asset["name"] + ".sha256"), None)
assert new_sha, "the latest release publishes a .sha256 for its AppImage"

work = tempfile.mkdtemp(prefix="genius-update-")
app = os.path.join(work, "Genius.AppImage")
log(f"downloading {appimage(old)['name']} …")
fetch(appimage(old)["browser_download_url"], app)
os.chmod(app, 0o755)
old_hash = sha256(app)

home = os.path.join(work, "home")
env = dict(os.environ, DISPLAY=":98", HOME=home, XDG_DATA_HOME=os.path.join(home, "data"), XDG_CONFIG_HOME=os.path.join(home, "config"),
           APPIMAGE_EXTRACT_AND_RUN="1", TMPDIR=os.path.join(work, "tmp"))
os.makedirs(env["TMPDIR"], exist_ok=True)
procs = [subprocess.Popen(["Xvfb", ":98", "-screen", "0", "1440x900x24"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)]
time.sleep(1.5)
procs.append(subprocess.Popen(["tauri-driver", "--port", "4445", "--native-port", "4446"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
time.sleep(1.5)


def session():
    opts = ArgOptions()
    opts.set_capability("browserName", "wry")
    opts.set_capability("tauri:options", {"application": app})
    d = webdriver.Remote(command_executor="http://127.0.0.1:4445", options=opts)
    d.set_window_size(1440, 900)
    return d


def shot(d, name):
    path = os.path.join(SHOTS, f"update-{name}.png")
    d.save_screenshot(path)
    log(f"screenshot {path}")


def wait(d, xpath, timeout=60):
    return WebDriverWait(d, timeout).until(EC.presence_of_element_located((By.XPATH, xpath)))


def click(d, xpath, timeout=60):
    el = WebDriverWait(d, timeout).until(EC.element_to_be_clickable((By.XPATH, xpath)))
    el.click()
    return el


def genius_pids():
    out = subprocess.run(["pgrep", "-f", work], capture_output=True, text=True).stdout.split()
    return [int(p) for p in out if int(p) != os.getpid()]


ok = False
try:
    d = session()
    # On start the app checks GitHub and announces the new version by itself.
    wait(d, f"//*[@role='dialog']//h2[contains(., 'Genius {latest['tag_name'].lstrip('v')} is available')]", 90)
    shot(d, "1-announced")
    log("old version found the new release on GitHub")
    click(d, "//*[@role='dialog']//button[contains(., 'Update now')]")
    log("downloading, verifying and installing…")
    # The app replaces its AppImage, relaunches it and exits; the WebDriver session ends.
    deadline = time.time() + 600
    while time.time() < deadline and sha256(app) == old_hash:
        time.sleep(2)
    assert sha256(app) != old_hash, "the AppImage was not replaced"
    try:
        d.quit()
    except Exception:  # noqa: BLE001 - the app already exited
        pass

    expected = urllib.request.urlopen(urllib.request.Request(new_sha["browser_download_url"], headers={"User-Agent": "genius-update-e2e"}), timeout=60).read().decode().split()[0]
    got = sha256(app)
    log(f"AppImage on disk sha256 {got}")
    assert got == expected, f"replaced file does not match the release checksum {expected}"
    assert os.access(app, os.X_OK), "replaced AppImage is executable"
    log("AppImage replaced with the verified new release")

    time.sleep(8)
    relaunched = genius_pids()
    log(f"relaunched processes: {relaunched}")
    assert relaunched, "the updated app was relaunched"
    subprocess.run(["pkill", "-f", work], check=False)
    time.sleep(2)

    # Start the updated app again under WebDriver: it reports the new version and says hello.
    d = session()
    wait(d, f"//*[@role='dialog']//h2[contains(., 'Welcome to Genius {latest['tag_name'].lstrip('v')}')]", 90)
    shot(d, "2-welcome")
    click(d, "//*[@role='dialog']//button[contains(., 'Let') or contains(., 'Later') or @aria-label='Close']")
    click(d, "//button[@aria-label='Settings']")
    click(d, "//nav//button[contains(., 'Updates & about')]")
    wait(d, f"//*[contains(., 'Version {latest['tag_name'].lstrip('v')}')]")
    shot(d, "3-settings-new-version")
    d.quit()
    ok = True
    log(f"UPDATER VERIFIED: {old['tag_name']} → {latest['tag_name']}")
except Exception as e:  # noqa: BLE001
    log(f"FAILED: {type(e).__name__}: {e}")
    try:
        shot(d, "failure")
    except Exception:  # noqa: BLE001
        pass
    raise
finally:
    subprocess.run(["pkill", "-f", work], check=False)
    for p in reversed(procs):
        p.terminate()
    sys.exit(0 if ok else 1)
