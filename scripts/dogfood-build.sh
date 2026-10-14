#!/usr/bin/env bash
# Builds a test copy of Omega Star: the next patch version with a -dogfood prerelease tag, stamped
# with the dogfood channel. Signing and CARGO_TARGET_DIR come from the caller's environment.
set -euo pipefail

cd "$(dirname "$0")/.."
current=$(node -p 'require("./src-tauri/tauri.conf.json").version')
IFS=. read -r major minor patch <<< "${current%%-*}"
version="$major.$minor.$((patch + 1))-dogfood.$(date +%Y%m%d%H%M)"

OMEGA_CHANNEL=dogfood npx tauri build --bundles app --config "{\"version\":\"$version\"}"

echo "BUILT $version"
echo "Run it on a copy of the office (set OMEGASTAR_DATA_DIR) until this branch ships: an older build that saves a run drops the newer fields."
