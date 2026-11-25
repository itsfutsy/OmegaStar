#!/usr/bin/env bash
set -euo pipefail

app=$1
exe="$app/Contents/MacOS/$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$app/Contents/Info.plist")"
tmp=$(mktemp -d)
office="$tmp/office"
pid=

stop() {
  if [ -n "$pid" ]; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  rm -rf "$tmp"
}
trap stop EXIT

fail() {
  echo "FAIL: $app $1" >&2
  echo "--- omega.log ---" >&2
  tail -n 50 "$office/omega.log" >&2 2>/dev/null || true
  echo "--- app output ---" >&2
  tail -n 50 "$tmp/output.log" >&2 || true
  exit 1
}

OMEGASTAR_DATA_DIR="$office" "$exe" >"$tmp/output.log" 2>&1 &
pid=$!

for _ in $(seq 60); do
  [ -e "$office/office.db" ] && break
  kill -0 "$pid" 2>/dev/null || fail "exited before opening its office"
  sleep 1
done
[ -e "$office/office.db" ] || fail "did not open its office within 60 s"
sleep 5
kill -0 "$pid" 2>/dev/null || fail "exited after opening its office"
if grep -qs 'could not open the office' "$office/omega.log" "$tmp/output.log"; then
  fail "could not open its office"
fi
echo "OK: $app started and opened its office"
