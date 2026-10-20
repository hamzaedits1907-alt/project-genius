#!/usr/bin/env bash
# Installs the APK on the running emulator/device, starts the app and checks that it really came up:
# the process runs, there is no crash in the log, and the sign-in screen's text is on screen.
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
sleep 20
pid=$(adb shell pidof "$PKG" | tr -d '\r' || true)
[ -n "$pid" ] || { echo "::error::the app is not running after launch"; adb logcat -d | tail -80; exit 1; }
if adb logcat -d | grep -E "FATAL EXCEPTION|Process: $PKG.*died|ANR in $PKG" ; then echo "::error::the app crashed"; exit 1; fi
adb shell uiautomator dump /sdcard/ui.xml >/dev/null
adb pull /sdcard/ui.xml ui.xml >/dev/null
adb exec-out screencap -p > launch.png
# The WebView exposes its page text to accessibility: the sign-in screen must be visible
if grep -q -E "Welcome back|Sign in|Email" ui.xml; then
  echo "OK: $PKG installed, running (pid $pid) and showing the sign-in screen"
else
  echo "::error::the app is running but the sign-in screen text was not found on screen"; head -c 3000 ui.xml; exit 1
fi
