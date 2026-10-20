#!/usr/bin/env bash
# Installs the APK on the running emulator/device, starts the app and checks that it really came up:
# the process runs, there is no crash in the log, and the WebView shows Genius's sign-in screen (read through
# the WebView debugging port, because Android does not expose a WebView's text to uiautomator).
# Usage: scripts/android-smoke.sh path/to/app.apk
set -euo pipefail
APK="${1:?usage: android-smoke.sh app.apk}"
PKG=app.projectgenius.mobile
adb wait-for-device
adb shell 'while [ "$(getprop sys.boot_completed)" != "1" ]; do sleep 2; done'
adb logcat -c
adb install -r "$APK"
adb shell pm list packages | grep -q "package:$PKG"
adb shell am start -W -n "$PKG/.MainActivity"
sleep 15
pid=$(adb shell pidof "$PKG" | tr -d '\r' || true)
[ -n "$pid" ] || { echo "::error::the app is not running after launch"; adb logcat -d | tail -80; exit 1; }
if adb logcat -d | grep -E "FATAL EXCEPTION|ANR in $PKG"; then echo "::error::the app crashed"; exit 1; fi
adb exec-out screencap -p > launch.png || true
adb forward tcp:9222 "localabstract:webview_devtools_remote_${pid}"
node scripts/android-webview-check.mjs 9222 || { adb logcat -d | grep -i -E "chromium|capacitor|console" | tail -40; exit 1; }
echo "OK: $PKG installed, running (pid $pid)"
