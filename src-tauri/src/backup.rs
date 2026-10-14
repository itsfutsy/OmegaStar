//! Office backups: one zip of the office folder per backup, with a consistent database snapshot in
//! place of the live `office.db`, pruned to the newest few. Bulk (large files, dependencies and app
//! builds) is left out. File work only; the Office decides when.

use crate::model::*;
use crate::own_skills;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDateTime, Timelike};
use rusqlite::{Connection, OpenFlags};
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{BufWriter, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const PREFIX: &str = "OmegaStar-backup-";
const EXT: &str = ".zip";
const STAMP: &str = "%Y%m%d-%H%M%S";
/// A backup being written; renamed to its final name only once complete.
pub const PARTIAL: &str = ".partial";
/// The live database and its journals. The snapshot goes in as `office.db` instead.
const DB_FILES: [&str; 4] = ["office.db", "office.db-wal", "office.db-shm", "office.db-journal"];
/// Backed up, but they change on their own every session, so the fingerprint ignores them.
const RESTLESS: [&str; 2] = ["omega.log", "omega.running"];
/// Files larger than this are left out, except run transcripts.
pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
const LEFT_OUT_NOTE: &str = "Files over 10 MB outside runs/, node_modules, app bundles and build folders are not in this backup.";
pub const RESTORE_STEPS: &str = "Quit Omega Star from the tray, replace the contents of the office folder with the contents \
of this zip, then start Omega Star.";

impl From<zip::result::ZipError> for AppError {
    fn from(e: zip::result::ZipError) -> Self {
        AppError::internal(format!("zip: {e}"))
    }
}

pub fn file_name(at: DateTime<Local>) -> String {
    format!("{PREFIX}{}{EXT}", at.format(STAMP))
}

fn is_backup_name(name: &str) -> bool {
    name.strip_prefix(PREFIX)
        .and_then(|r| r.strip_suffix(EXT))
        .is_some_and(|stamp| stamp.len() == 15 && NaiveDateTime::parse_from_str(stamp, STAMP).is_ok())
}

/// When the next automatic backup is due: an interval after the last success or the last check that
/// found nothing changed, but no sooner than an hour (or the interval, if shorter) after a failed
/// attempt. `None` when backups are off.
pub fn next_at(freq: BackupFrequency, st: &BackupStatus, now: Ts) -> Option<Ts> {
    let interval = freq.interval()?;
    // A clock set far back would otherwise hold backups off until it caught up.
    let due = st.last_at.max(st.checked_at).map(|t| t + interval).filter(|t| *t <= now + interval);
    let retry = st.last_error.as_ref().and(st.last_attempt_at).map(|t| t + interval.min(Duration::hours(1)));
    Some(match (due, retry) {
        (Some(d), Some(r)) => d.max(r),
        (d, r) => d.or(r).unwrap_or(now),
    })
}

pub fn is_due(freq: BackupFrequency, st: &BackupStatus, now: Ts) -> bool {
    next_at(freq, st, now).is_some_and(|t| t <= now)
}

/// The existing part of `p` resolved (symlinks, case, 8.3 names), with the rest appended as given.
fn resolved(p: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut cur = p;
    loop {
        if let Ok(c) = fs::canonicalize(cur) {
            return rest.iter().rev().fold(c, |acc, part| acc.join(part));
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                cur = parent;
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// Checks a backup folder: a full path, outside the office folder. It need not exist yet.
pub fn validate_dir(dir: &str, data_dir: &Path) -> Result<PathBuf> {
    let d = dir.trim();
    if d.is_empty() {
        return Err(AppError::invalid("Choose a folder for backups."));
    }
    let p = PathBuf::from(d);
    if !p.is_absolute() {
        return Err(AppError::invalid("The backup folder must be a full path, like C:\\Backups."));
    }
    if resolved(&p).starts_with(resolved(data_dir)) {
        return Err(AppError::invalid("Choose a backup folder outside the office folder, so backups survive if it's lost."));
    }
    Ok(p)
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Summary {
    pub files: u64,
    /// Files or folders that couldn't be read and were left out.
    pub skipped: u32,
    /// Large files and build folders left out on purpose.
    pub left_out: u32,
    /// The zip's size.
    pub bytes: u64,
}

fn options(meta: Option<&fs::Metadata>) -> SimpleFileOptions {
    let mut o = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    if let Some(m) = meta {
        o = o.large_file(m.len() >= u32::MAX as u64);
        let t = m.modified().ok().map(DateTime::<Local>::from).and_then(|t| {
            zip::DateTime::from_date_and_time(t.year() as u16, t.month() as u8, t.day() as u8, t.hour() as u8, t.minute() as u8, t.second() as u8).ok()
        });
        if let Some(t) = t {
            o = o.last_modified_time(t);
        }
    }
    o
}

/// Copies `src` into the open entry. Returns false if reading stopped early (the entry keeps what
/// was read); errors are only for writing the zip.
fn copy_into(src: &mut File, out: &mut impl Write) -> Result<bool> {
    let mut buf = vec![0; 64 * 1024];
    loop {
        let n = match src.read(&mut buf) {
            Ok(0) => return Ok(true),
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return Ok(false),
        };
        out.write_all(&buf[..n])?;
    }
}

fn entry_name(root: &Path, p: &Path) -> Option<String> {
    let rel = p.strip_prefix(root).ok()?;
    let parts: Vec<_> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Bulk that backups leave out: dependency and build folders, app bundles, and files over
/// MAX_FILE_BYTES that aren't under `runs/` (transcripts are the office's record and compress well).
fn left_out(path: &Path, rel: &str, is_dir: bool, len: u64) -> bool {
    if !is_dir {
        return len > MAX_FILE_BYTES && !rel.starts_with("runs/");
    }
    let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    name == "node_modules" || name == "__pycache__" || name.ends_with(".app") || path.join("CACHEDIR.TAG").is_file()
}

/// One entry of a folder, as backups see it.
enum Item {
    Dir { path: PathBuf, name: String, meta: Option<fs::Metadata> },
    File { path: PathBuf, name: String },
    LeftOut,
    Unreadable,
}

/// The entries of `dir` in name order, without links (`.claude/skills` points at `skills/` and is
/// rebuilt by Omega Star; following it would store its target twice) and the live database. `None`
/// when the folder can't be read.
fn items(root: &Path, dir: &Path) -> Option<Vec<Item>> {
    let mut entries: Vec<_> = fs::read_dir(dir).ok()?.collect();
    entries.sort_by_key(|e| e.as_ref().map(|e| e.file_name()).ok());
    let mut out = Vec::new();
    for e in entries {
        let Ok(e) = e else {
            out.push(Item::Unreadable);
            continue;
        };
        let path = e.path();
        let Ok(ft) = e.file_type() else {
            out.push(Item::Unreadable);
            continue;
        };
        if ft.is_symlink() || (ft.is_dir() && own_skills::is_link(&path)) {
            continue;
        }
        let Some(name) = entry_name(root, &path) else { continue };
        if dir == root && DB_FILES.contains(&name.as_str()) {
            continue;
        }
        let meta = e.metadata().ok();
        let len = meta.as_ref().map_or(0, |m| m.len());
        if (ft.is_dir() || ft.is_file()) && left_out(&path, &name, ft.is_dir(), len) {
            out.push(Item::LeftOut);
        } else if ft.is_dir() {
            out.push(Item::Dir { path, name, meta });
        } else if ft.is_file() {
            out.push(Item::File { path, name });
        }
    }
    Some(out)
}

fn walk<W: Write + std::io::Seek>(root: &Path, dir: &Path, zip: &mut ZipWriter<W>, s: &mut Summary) -> Result<()> {
    let Some(items) = items(root, dir) else {
        s.skipped += 1;
        return Ok(());
    };
    for item in items {
        match item {
            Item::Unreadable => s.skipped += 1,
            Item::LeftOut => s.left_out += 1,
            Item::Dir { path, name, meta } => {
                zip.add_directory(format!("{name}/"), options(meta.as_ref()))?;
                walk(root, &path, zip, s)?;
            }
            Item::File { path, name } => {
                let Ok(mut f) = File::open(&path) else {
                    s.skipped += 1;
                    continue;
                };
                zip.start_file(name, options(f.metadata().ok().as_ref()))?;
                if copy_into(&mut f, zip)? {
                    s.files += 1;
                } else {
                    s.skipped += 1;
                }
            }
        }
    }
    Ok(())
}

/// Whether anything worth backing up has changed: a hash of the size and modified time of every file
/// a backup would hold (never their content), and of each database table's row count, highest rowid
/// and total data length. The `meta` table, which records backups themselves, and files that change
/// every session are ignored, so a backup or a check never changes it.
pub fn fingerprint(data_dir: &Path) -> String {
    let mut h = DefaultHasher::new();
    hash_files(data_dir, data_dir, &mut h);
    if let Err(e) = hash_db(&data_dir.join("office.db"), &mut h) {
        e.to_string().hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

fn hash_files(root: &Path, dir: &Path, h: &mut DefaultHasher) {
    for item in items(root, dir).unwrap_or_default() {
        match item {
            Item::Dir { path, .. } => hash_files(root, &path, h),
            Item::File { path, name } if !(dir == root && RESTLESS.contains(&name.as_str())) => {
                name.hash(h);
                let meta = fs::metadata(&path).ok();
                meta.as_ref().map(|m| m.len()).hash(h);
                meta.and_then(|m| m.modified().ok()).hash(h);
            }
            _ => {}
        }
    }
}

/// Through its own read-only connection, so the store's lock is never taken (WAL allows it).
fn hash_db(path: &Path, h: &mut DefaultHasher) -> rusqlite::Result<()> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    let names = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT IN ('meta') AND name NOT LIKE 'sqlite_%' ORDER BY name")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for name in names {
        name.hash(h);
        let row = conn.query_row(&format!("SELECT count(*), max(rowid), total(length(data)) FROM \"{name}\""), [], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, f64>(2)?.to_bits()))
        });
        match row {
            Ok(row) => row.hash(h),
            Err(e) => e.to_string().hash(h),
        }
    }
    Ok(())
}

/// Writes a new zip at `dest`: the database snapshot as `office.db`, everything else in `data_dir`
/// except the live database, links and bulk, and `backup.json` (the manifest plus the counts).
pub fn write_archive(data_dir: &Path, db_snapshot: &Path, dest: &Path, mut manifest: serde_json::Value) -> Result<Summary> {
    let mut zip = ZipWriter::new(BufWriter::new(File::create_new(dest)?));
    let mut s = Summary::default();
    let mut db = File::open(db_snapshot)?;
    zip.start_file("office.db", options(db.metadata().ok().as_ref()))?;
    if !copy_into(&mut db, &mut zip)? {
        return Err(AppError::internal("could not read the database snapshot"));
    }
    s.files += 1;
    walk(data_dir, data_dir, &mut zip, &mut s)?;
    manifest["files"] = s.files.into();
    manifest["skipped"] = s.skipped.into();
    manifest["leftOut"] = s.left_out.into();
    manifest["leftOutNote"] = LEFT_OUT_NOTE.into();
    zip.start_file("backup.json", options(None))?;
    zip.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
    let file = zip.finish()?.into_inner().map_err(|e| e.into_error())?;
    file.sync_all()?;
    s.bytes = file.metadata()?.len();
    Ok(s)
}

/// Deletes all but the newest `keep` backups in `dir`, and any half-written ones. Files that aren't
/// Omega Star backups are never touched. Returns how many were deleted.
pub fn prune(dir: &Path, keep: u32) -> Result<usize> {
    let mut done = Vec::new();
    let mut partial = Vec::new();
    for e in fs::read_dir(dir)?.flatten() {
        if !e.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if is_backup_name(&name) {
            done.push(name);
        } else if name.strip_suffix(PARTIAL).is_some_and(is_backup_name) {
            partial.push(name);
        }
    }
    done.sort(); // the stamp sorts by time
    let excess = done.len().saturating_sub(keep.max(1) as usize);
    let doomed: Vec<_> = done.into_iter().take(excess).chain(partial).collect();
    for name in &doomed {
        fs::remove_file(dir.join(name))?;
    }
    Ok(doomed.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn temp_office(what: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("omegastar-backup-{what}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn big(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        File::create(path).unwrap().set_len(11 * 1024 * 1024).unwrap();
    }

    #[test]
    fn bulk_is_left_out() {
        let office = temp_office("bulk");
        let ws = office.join("employees/e1");
        put(&ws.join("notes.md"), "keep me");
        put(&ws.join("node_modules/pkg/index.js"), "x");
        put(&ws.join("Tool.app/Contents/Info.plist"), "x");
        put(&ws.join("target/CACHEDIR.TAG"), "Signature: 8a477f597d28d172789f06886806bc55");
        put(&ws.join("target/debug/out"), "x");
        big(&ws.join("data.bin"));
        big(&office.join("runs/r1/transcript.jsonl"));
        let out = temp_office("bulk-out");
        let snapshot = out.join("snapshot.db");
        put(&snapshot, "db");
        let dest = out.join("backup.zip");

        let s = write_archive(&office, &snapshot, &dest, serde_json::json!({})).unwrap();
        assert_eq!(s.left_out, 4);
        let zip = zip::ZipArchive::new(File::open(&dest).unwrap()).unwrap();
        let names: Vec<&str> = zip.file_names().collect();
        assert!(names.contains(&"employees/e1/notes.md"), "{names:?}");
        assert!(names.contains(&"runs/r1/transcript.jsonl"), "transcripts are kept: {names:?}");
        for gone in ["node_modules", "Tool.app", "target", "data.bin"] {
            assert!(!names.iter().any(|n| n.contains(gone)), "{gone} left out: {names:?}");
        }

        let _ = fs::remove_dir_all(&office);
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn fingerprint_follows_real_changes() {
        let office = temp_office("fingerprint");
        let db = Store::open(&office.join("office.db")).unwrap();
        let notes = office.join("employees/e1/notes.md");
        put(&notes, "first");
        put(&office.join("employees/e1/node_modules/pkg/index.js"), "x");
        put(&office.join("omega.log"), "started\n");

        let fp = fingerprint(&office);
        assert_eq!(fingerprint(&office), fp, "steady");

        put(&notes, "first, then more");
        let fp2 = fingerprint(&office);
        assert_ne!(fp2, fp, "a note was edited");

        let mut run = Run::new("e1", RunKind::Manual, "go".into(), chrono::Utc::now());
        db.insert_run(&run, &run.id).unwrap();
        let fp3 = fingerprint(&office);
        assert_ne!(fp3, fp2, "a run was added");
        run.state = RunState::Succeeded;
        db.save_run(&run).unwrap();
        let fp4 = fingerprint(&office);
        assert_ne!(fp4, fp3, "a run was saved");

        put(&office.join("employees/e1/node_modules/pkg/index.js"), "a much longer file");
        put(&office.join("omega.log"), "started\nstarted again\n");
        db.set_backup_status(&BackupStatus { last_at: Some(chrono::Utc::now()), ..Default::default() }).unwrap();
        assert_eq!(fingerprint(&office), fp4, "dependencies, the log and meta don't count");

        drop(db);
        let _ = fs::remove_dir_all(&office);
    }

    #[test]
    fn a_backup_then_a_check_both_skip() {
        let office = temp_office("check");
        let db = Store::open(&office.join("office.db")).unwrap();
        put(&office.join("employees/e1/notes.md"), "notes");
        let at = chrono::Utc::now();
        let mut st = BackupStatus { last_at: Some(at), fingerprint: Some(fingerprint(&office)), ..Default::default() };
        db.set_backup_status(&st).unwrap();
        assert_eq!(st.fingerprint.as_deref(), Some(fingerprint(&office).as_str()), "recording the backup changes nothing");
        st.checked_at = Some(at + Duration::hours(1));
        db.set_backup_status(&st).unwrap();
        assert_eq!(st.fingerprint.as_deref(), Some(fingerprint(&office).as_str()), "recording the check changes nothing");

        drop(db);
        let _ = fs::remove_dir_all(&office);
    }

    #[test]
    fn next_at_counts_from_a_check() {
        let t = chrono::Utc::now();
        let st = BackupStatus { last_at: Some(t), checked_at: Some(t + Duration::minutes(30)), ..Default::default() };
        let next = next_at(BackupFrequency::Hourly, &st, t + Duration::minutes(50));
        assert_eq!(next, Some(t + Duration::minutes(90)));
    }
}
