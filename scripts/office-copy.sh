#!/usr/bin/env bash
# Copies the release office (~/Library/Application Support/OmegaStar) into a sandbox you can open with a test
# build, without changing the office it copies.
#
#   scripts/office-copy.sh <dest> [--replace]
#
# The release office is only read: the database through SQLite's read-only backup, everything else with
# copy-on-write clones where the disk allows (APFS), so a copy of a large office is quick and takes almost no space.
#
# The copy is a sandbox (omega.sandbox in its folder): it never starts Claude Code, backs up or watches anyone,
# whichever build opens it. Its schedules are paused and its backups are off, so even a build from before the
# sandbox existed would not write to your backup folder or start your employees' routines.
#
# --replace deletes <dest> first, but only when it is already a sandbox copy.
set -euo pipefail

die() { echo "office-copy: $*" >&2; exit 1; }

src="$HOME/Library/Application Support/OmegaStar"
dest=""
replace=0
while [ $# -gt 0 ]; do
  case "$1" in
    --replace) replace=1; shift ;;
    -h|--help) sed -n '2,14p' "$0"; exit 0 ;;
    -*) die "unknown option $1" ;;
    *) [ -z "$dest" ] || die "one destination only"; dest="$1"; shift ;;
  esac
done
[ -n "$dest" ] || die "usage: office-copy.sh <dest> [--replace]"
command -v sqlite3 >/dev/null || die "needs sqlite3"
[ -f "$src/office.db" ] || die "no office at $src"

# Compare real paths, ignoring case (macOS and Windows file systems usually do).
real() { python3 -I -c 'import os,sys; print(os.path.realpath(sys.argv[1]).lower())' "$1"; }
case "$(real "$dest")/" in "$(real "$src")"/*) die "$dest is the release office" ;; esac

if [ -e "$dest" ]; then
  if [ -n "$(ls -A "$dest" 2>/dev/null)" ]; then
    [ "$replace" = 1 ] || die "$dest is not empty (use --replace to overwrite a sandbox copy)"
    [ -f "$dest/omega.sandbox" ] || die "$dest is not a sandbox copy, so it stays: delete it yourself if you mean to"
    rm -rf "$dest"
  fi
fi
mkdir -p "$dest"

# Everything, cloned where the disk allows (cp -c on APFS), less the log, the running marker, the phone pairing (its
# key would let whoever holds the copy act as your phone in the release office) and the database, which is put back as
# a consistent snapshot read from the source read-only, so a running OmegaStar is never blocked or changed. The pairing
# goes however this script ends, so a copy that fails or is stopped part-way does not keep it either.
trap 'rm -f "$dest"/remote.json{,.partial}' EXIT
cp -cR "$src/" "$dest" 2>/dev/null || cp -R "$src/" "$dest"
rm -f "$dest"/office.db{,-wal,-shm,-journal} "$dest/omega.log" "$dest/omega.running"
sqlite3 "file:$src/office.db?mode=ro" ".backup '$dest/office.db'"

# Belt and braces for builds that predate the sandbox: schedules paused, backups off.
sqlite3 "$dest/office.db" \
  "UPDATE meta SET value = json_set(value, '$.paused', json('true'), '$.backup.frequency', 'off') WHERE key = 'settings';"

printf 'Copied from %s on %s. This office never starts Claude Code, backs up or watches anyone.\n' \
  "$src" "$(date '+%Y-%m-%d %H:%M')" > "$dest/omega.sandbox"

echo "Copied $src to $dest (sandbox: runs, backups and watching are off)."
