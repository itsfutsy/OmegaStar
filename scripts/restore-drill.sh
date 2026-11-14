#!/usr/bin/env bash
# Proves the newest office backup restores: unzips it into a scratch folder, checks its database, and compares it
# with the office it came from. Prints one line, and exits 1 when the backup would not restore the office.
#
#   scripts/restore-drill.sh [--office <dir>] [--backups <dir>]
#
# --office   the office to compare with (default: the release office, ~/Library/Application Support/OmegaStar).
# --backups  the folder of backup zips (default: the office's own backup folder from its settings).
#
# Nothing outside the scratch folder is written. The office is only read: its database through SQLite's read-only
# mode, its folders by listing them. The scratch folder is deleted at the end, pass or fail.
#
# A gap is fine when the office has moved on since the backup: hires and runs made after the backup was taken, and
# employees or folders removed since. Anything in the office that was there before the backup but is missing from
# it is a failure.
set -euo pipefail

say() { echo "restore-drill: $*"; }
fail() { say "FAILED: $*"; exit 1; }

office="$HOME/Library/Application Support/OmegaStar"
backups=""
while [ $# -gt 0 ]; do
  case "$1" in
    --office) [ $# -ge 2 ] || fail "--office needs a folder"; office="$2"; shift 2 ;;
    --backups) [ $# -ge 2 ] || fail "--backups needs a folder"; backups="$2"; shift 2 ;;
    -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
    *) fail "unknown argument $1" ;;
  esac
done
for tool in sqlite3 unzip; do command -v "$tool" >/dev/null || fail "needs $tool"; done
[ -f "$office/office.db" ] || fail "no office at $office"

# A path inside an SQL string, its quotes doubled.
sq() { printf '%s' "${1//\'/\'\'}"; }
# A database opened read-only, so a running Omega Star is never blocked or changed. SQLite can't open a WAL database
# read-only without its -shm file, which is only missing when nothing has it open: then it is read as it lies.
uri() { if [ -e "$1-wal" ] || [ -e "$1-shm" ]; then echo "file:$1?mode=ro"; else echo "file:$1?mode=ro&immutable=1"; fi; }
ro() { sqlite3 -batch -noheader "$(uri "$1")" "$2"; }

if [ -z "$backups" ]; then
  backups="$(ro "$office/office.db" "SELECT json_extract(value, '$.backup.dir') FROM meta WHERE key = 'settings'")"
  [ -n "$backups" ] || fail "the office has no backup folder in its settings; pass --backups"
fi
[ -d "$backups" ] || fail "no backup folder at $backups"

# Only finished backups, named as Omega Star names them: the stamp sorts by time.
newest="$(find "$backups" -maxdepth 1 -type f -name 'OmegaStar-backup-[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]-[0-9][0-9][0-9][0-9][0-9][0-9].zip' \
  -exec basename {} \; | sort | tail -n 1)"
[ -n "$newest" ] || fail "no backups in $backups"

scratch="$(mktemp -d "${TMPDIR:-/tmp}/omega-restore-XXXXXX")"
trap 'rm -rf "$scratch"' EXIT

unzip -q "$backups/$newest" -d "$scratch" || fail "$newest would not unzip"
[ -f "$scratch/office.db" ] || fail "$newest has no office.db"
[ -f "$scratch/backup.json" ] || fail "$newest has no backup.json"

integrity="$(ro "$scratch/office.db" "PRAGMA integrity_check" 2>&1 | head -n 1)"
[ "$integrity" = "ok" ] || fail "$newest: the database is damaged ($integrity)"

# When the backup was taken, in milliseconds like the office's own times.
made="$(sqlite3 -batch -noheader :memory: "SELECT json_extract(readfile('$(sq "$scratch")/backup.json'), '$.createdAt')")"
cutoff="$(sqlite3 -batch -noheader :memory: "SELECT CAST(round((julianday('$(sq "$made")') - 2440587.5) * 86400000) AS INTEGER)")"
[ -n "$cutoff" ] || fail "$newest: backup.json has no createdAt"
hired_by="CAST(round((julianday(json_extract(data, '$.hiredAt')) - 2440587.5) * 86400000) AS INTEGER) <= $cutoff"

ids() { ro "$1" "SELECT id FROM employees $2 ORDER BY id"; }
# The lines of $1 that aren't in $2, both sorted.
missing() { comm -23 <(printf '%s\n' "$1" | sed '/^$/d') <(printf '%s\n' "$2" | sed '/^$/d'); }
folders() { if [ -d "$1/employees" ]; then find "$1/employees" -mindepth 1 -maxdepth 1 -type d -exec basename {} \; | sort; fi; }
count() { printf '%s\n' "$1" | sed '/^$/d' | wc -l | tr -d ' '; }
inline() { printf '%s' "$1" | tr '\n' ' '; }

backup_ids="$(ids "$scratch/office.db" "")"
lost="$(missing "$(ids "$office/office.db" "WHERE $hired_by")" "$backup_ids")"
[ -z "$lost" ] || fail "$newest: missing employees the office had before it: $(inline "$lost")"

# Runs the office had before the backup that the backup doesn't have.
missing_runs="$(sqlite3 -batch -noheader "$(uri "$office/office.db")" \
  "ATTACH '$(sq "$(uri "$scratch/office.db")")' AS b; SELECT count(*) FROM main.runs WHERE created_at <= $cutoff AND id NOT IN (SELECT id FROM b.runs)")"
[ "$missing_runs" = 0 ] || fail "$newest: missing $missing_runs runs the office had before it"

# Every employee folder in the office belongs in the backup, unless its employee was hired after it.
backup_folders="$(folders "$scratch")"
lost_folders="$(missing "$(missing "$(folders "$office")" "$backup_folders")" "$(ids "$office/office.db" "WHERE NOT ($hired_by)")")"
[ -z "$lost_folders" ] || fail "$newest: missing employee folders: $(inline "$lost_folders")"

employees="$(count "$backup_ids")"
runs="$(ro "$scratch/office.db" "SELECT count(*) FROM runs")"
office_employees="$(ro "$office/office.db" "SELECT count(*) FROM employees")"
office_runs="$(ro "$office/office.db" "SELECT count(*) FROM runs")"
since=""
[ "$office_employees" = "$employees" ] || since="$since; the office has $office_employees employees now"
[ "$office_runs" = "$runs" ] || since="$since; the office has $office_runs runs now"
say "ok: $newest restores: database ok, $employees employees, $runs runs, $(count "$backup_folders") employee folders, nothing from before it missing$since"
