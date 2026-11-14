#!/usr/bin/env bash
# Builds a test copy of Omega Star: the next patch version with a -dogfood prerelease tag, stamped
# with the dogfood channel. Signing and CARGO_TARGET_DIR come from the caller's environment.
set -euo pipefail

cd "$(dirname "$0")/.."
current=$(node -p 'require("./src-tauri/tauri.conf.json").version')
IFS=. read -r major minor patch <<< "${current%%-*}"
version="$major.$minor.$((patch + 1))-dogfood.$(date +%Y%m%d%H%M)"

# Its own app id and name, so it opens beside the release (its own single-instance check, login item and saved
# window state) instead of handing over to it, and its own link scheme, so omegastar:// links keep opening the release.
config="{\"version\":\"$version\",\"identifier\":\"com.abramindustries.omegastar.dogfood\",\"productName\":\"Omega Star Dogfood\",\"plugins\":{\"deep-link\":{\"desktop\":{\"schemes\":[\"omegastar-dogfood\"]}}}}"
OMEGA_CHANNEL=dogfood npx tauri build --bundles app --config "$config"

echo "BUILT $version"
echo "It opens its own office in ~/Library/Application Support/OmegaStar-dogfood (or OMEGASTAR_DATA_DIR), never the real one: an older build that saves a run drops the newer fields."
