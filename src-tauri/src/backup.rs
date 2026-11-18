//! Office backups: one zip of the office folder per backup, with a consistent database snapshot in
//! place of the live `office.db`, pruned to the newest few. Bulk (large files, dependencies and app
//! builds) is left out. File work only; the Office decides when.

use crate::model::*;
use crate::office::RUNNING_MARKER;
use crate::remote::store as remote;
use crate::own_skills;
use crate::store::{BACKUP_STATUS, DB_FILE};
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
/// Backed up, but it changes on its own every session, so the fingerprint ignores it.
const LOG: &str = "omega.log";
/// Files larger than this are left out, except run transcripts.
const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
/// Dependency, build and cache folders, left out wherever they are in the office folder.
const BULK_DIRS: [&str; 12] = ["node_modules", "target", ".gradle", "DerivedData", "Pods", "__pycache__", ".venv", ".cache", ".next", ".turbo", "build", "dist"];
const LEFT_OUT_NOTE: &str = "Files over 10 MB outside runs/, app bundles, and dependency, build and cache folders (such as node_modules, target, build and dist) are not in this backup.";
const GB: u64 = 1024 * 1024 * 1024;
pub const RESTORE_STEPS: &str = "Quit OmegaStar from the tray, replace the contents of the office folder with the contents \
of this zip, then start OmegaStar.";

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
pub(crate) fn resolved(p: &Path) -> PathBuf {
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

/// Whether the last backup already holds the office as `fingerprint` says it is, and is still in the
/// folder backups now go to: a new folder, or a deleted zip, needs a fresh backup.
pub fn up_to_date(st: &BackupStatus, fingerprint: &str, dir: &str, data_dir: &Path) -> bool {
    let in_place = |f: &Path| f.is_file() && validate_dir(dir, data_dir).is_ok_and(|d| f.parent() == Some(d.as_path()));
    st.last_error.is_none() && st.fingerprint.as_deref() == Some(fingerprint) && st.last_file.as_deref().map(Path::new).is_some_and(in_place)
}

#[derive(Debug, Default)]
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

/// Bulk that backups leave out: dependency, build and cache folders (by name, or marked with a
/// CACHEDIR.TAG), app bundles, and files over MAX_FILE_BYTES that aren't under `runs/` (transcripts
/// are the office's record and compress well).
fn left_out(path: &Path, rel: &str, is_dir: bool, len: u64) -> bool {
    if !is_dir {
        return len > MAX_FILE_BYTES && !rel.starts_with("runs/");
    }
    let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    BULK_DIRS.contains(&name.as_ref()) || name.ends_with(".app") || path.join("CACHEDIR.TAG").is_file()
}

/// One entry of a folder, as backups see it.
enum Item {
    Dir { path: PathBuf, name: String, meta: Option<fs::Metadata> },
    File { path: PathBuf, name: String },
    LeftOut,
    Unreadable,
}

/// The entries of `dir` in name order, without links (`.claude/skills` points at `skills/` and is
/// rebuilt by OmegaStar; following it would store its target twice) and the live database. `None`
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
        // The clean-exit marker is always there while OmegaStar runs: restored, it would read as a crash. A restored
        // phone pairing, or its copy mid-save, would bring a second computer into the same relay room.
        if dir == root && (DB_FILES.contains(&name.as_str()) || name == RUNNING_MARKER || name.starts_with(remote::FILE)) {
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
/// a backup would hold (never their content), and of every database row. The backup status in `meta`
/// and the log are ignored, so a backup or a check never changes it.
pub fn fingerprint(data_dir: &Path) -> String {
    let mut h = DefaultHasher::new();
    hash_files(data_dir, data_dir, &mut h);
    if let Err(e) = hash_db(&data_dir.join(DB_FILE), &mut h) {
        e.to_string().hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

fn hash_files(root: &Path, dir: &Path, h: &mut DefaultHasher) {
    for item in items(root, dir).unwrap_or_default() {
        match item {
            Item::Dir { path, .. } => hash_files(root, &path, h),
            Item::File { path, name } if !(dir == root && name == LOG) => {
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
    let tables = names.into_iter().map(|name| {
        let sql = format!("SELECT CAST(rowid AS TEXT), data FROM \"{name}\" ORDER BY rowid");
        (name, sql)
    });
    let meta = ("meta".to_string(), format!("SELECT key, value FROM meta WHERE key <> '{BACKUP_STATUS}' ORDER BY key"));
    for (name, sql) in tables.chain([meta]) {
        name.hash(h);
        if let Err(e) = hash_rows(&conn, &sql, h) {
            e.to_string().hash(h);
        }
    }
    Ok(())
}

fn hash_rows(conn: &Connection, sql: &str, h: &mut DefaultHasher) -> rusqlite::Result<()> {
    let mut st = conn.prepare(sql)?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        r.get::<_, String>(0)?.hash(h);
        r.get::<_, String>(1)?.hash(h);
    }
    Ok(())
}

/// Writes a new zip at `dest`: the database snapshot as `office.db`, everything else in `data_dir`
/// except the live database, links and bulk, and `backup.json` (the manifest plus the counts).
pub fn write_archive(data_dir: &Path, db_snapshot: &Path, dest: &Path, mut manifest: serde_json::Value) -> Result<Summary> {
    let mut zip = ZipWriter::new(BufWriter::new(File::create_new(dest)?));
    let mut s = Summary::default();
    let mut db = File::open(db_snapshot)?;
    zip.start_file(DB_FILE, options(db.metadata().ok().as_ref()))?;
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
/// OmegaStar backups are never touched.
pub fn prune(dir: &Path, keep: u32) -> Result<()> {
    for path in prunable(dir, keep)? {
        fs::remove_file(path)?;
    }
    Ok(())
}

/// What `prune` deletes: the backups older than the newest `keep`, and any half-written ones.
fn prunable(dir: &Path, keep: u32) -> Result<Vec<PathBuf>> {
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
    Ok(done.into_iter().take(excess).chain(partial).map(|name| dir.join(name)).collect())
}

/// Makes room in `dir` for a backup: it wants twice the size of the last one free (`last_bytes`), or a
/// gigabyte before the first. When `free` reports less, the oldest backups go first, down to one fewer
/// than `keep` (never the last one, see `prune`), but only when that frees enough on this disk (`on_disk`, the
/// space each takes here): a backup a cloud service keeps only online frees nothing, so it is never deleted for
/// nothing. When there is still not enough, no backup is written. A volume whose free space can't be read is let
/// through.
pub fn make_room(dir: &Path, keep: u32, last_bytes: Option<u64>, free: impl Fn(&Path) -> Option<u64>, on_disk: impl Fn(&Path) -> u64) -> Result<()> {
    let need = last_bytes.map_or(GB, |b| b.saturating_mul(2));
    let short = || free(dir).is_some_and(|f| f < need);
    if short() {
        let doomed = prunable(dir, keep.saturating_sub(1))?;
        let freed: u64 = doomed.iter().map(|p| on_disk(p)).sum();
        if free(dir).is_some_and(|f| f.saturating_add(freed) >= need) {
            for path in &doomed {
                fs::remove_file(path)?;
            }
        }
    }
    if short() {
        return Err(AppError::invalid(format!("Not enough free disk space for a backup: it needs about {:.1} GB.", need as f64 / GB as f64)));
    }
    Ok(())
}

/// The space a file takes on its disk: its allocated blocks, not its length, so a file a cloud service keeps only
/// online takes next to none. Nothing when it can't be read.
#[cfg(unix)]
pub fn on_disk(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).map_or(0, |m| m.blocks().saturating_mul(512))
}

/// The space a file takes on its disk: its length, or nothing for a file a cloud service keeps only online (offline,
/// or fetched when it is opened or read). Nothing when it can't be read.
#[cfg(windows)]
pub fn on_disk(path: &Path) -> u64 {
    use std::os::windows::fs::MetadataExt;
    // FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_OPEN and FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS.
    const ONLINE_ONLY: u32 = 0x1000 | 0x4_0000 | 0x40_0000;
    fs::metadata(path).map_or(0, |m| if m.file_attributes() & ONLINE_ONLY != 0 { 0 } else { m.len() })
}

#[cfg(not(any(unix, windows)))]
pub fn on_disk(path: &Path) -> u64 {
    fs::metadata(path).map_or(0, |m| m.len())
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
        put(&ws.join("scratch/CACHEDIR.TAG"), "Signature: 8a477f597d28d172789f06886806bc55");
        put(&ws.join("scratch/debug/out"), "x");
        big(&ws.join("data.bin"));
        big(&office.join("runs/r1/transcript.jsonl"));
        put(&office.join(RUNNING_MARKER), "");
        put(&office.join(remote::FILE), "{}");
        put(&office.join(format!("{}.partial", remote::FILE)), "{}");
        let out = temp_office("bulk-out");
        let snapshot = out.join("snapshot.db");
        put(&snapshot, "db");
        let dest = out.join("backup.zip");

        let s = write_archive(&office, &snapshot, &dest, serde_json::json!({})).unwrap();
        assert_eq!(s.left_out, 4);
        let zip = zip::ZipArchive::new(File::open(&dest).unwrap()).unwrap();
        let owned: Vec<_> = zip.file_names().collect::<zip::result::ZipResult<_>>().unwrap();
        let names: Vec<&str> = owned.iter().map(|n| n.as_ref()).collect();
        assert!(names.contains(&"employees/e1/notes.md"), "{names:?}");
        assert!(names.contains(&"runs/r1/transcript.jsonl"), "transcripts are kept: {names:?}");
        for gone in ["node_modules", "Tool.app", "scratch", "data.bin", RUNNING_MARKER, remote::FILE] {
            assert!(!names.iter().any(|n| n.contains(gone)), "{gone} left out: {names:?}");
        }

        let _ = fs::remove_dir_all(&office);
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn build_and_cache_folders_are_left_out_by_name_anywhere() {
        let office = temp_office("names");
        let ws = office.join("employees/e1/workspace");
        for (i, name) in BULK_DIRS.iter().enumerate() {
            let depth = if i % 2 == 0 { format!("app-{i}") } else { format!("apps/app-{i}/src") };
            put(&ws.join(depth).join(name).join("out.txt"), "x");
        }
        put(&ws.join("build"), "a file, not a folder");
        put(&ws.join("plans/build-plan.md"), "keep me");
        put(&ws.join("builds/new-login/notes.md"), "keep me");
        put(&office.join("employees/e1/skills/e1-notes/SKILL.md"), "keep me");
        let out = temp_office("names-out");
        let snapshot = out.join("snapshot.db");
        put(&snapshot, "db");
        let dest = out.join("backup.zip");

        let s = write_archive(&office, &snapshot, &dest, serde_json::json!({})).unwrap();
        assert_eq!(s.left_out, BULK_DIRS.len() as u32);
        let zip = zip::ZipArchive::new(File::open(&dest).unwrap()).unwrap();
        let owned: Vec<_> = zip.file_names().collect::<zip::result::ZipResult<_>>().unwrap();
        let names: Vec<&str> = owned.iter().map(|n| n.as_ref()).collect();
        for kept in ["employees/e1/workspace/build", "employees/e1/workspace/plans/build-plan.md", "employees/e1/workspace/builds/new-login/notes.md", "employees/e1/skills/e1-notes/SKILL.md"] {
            assert!(names.contains(&kept), "{kept} kept: {names:?}");
        }
        assert!(!names.iter().any(|n| n.ends_with("out.txt")), "{names:?}");

        let _ = fs::remove_dir_all(&office);
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn a_short_disk_drops_the_oldest_backups_before_giving_up() {
        let dir = temp_office("room");
        let backups = ["OmegaStar-backup-20260101-090000.zip", "OmegaStar-backup-20260102-090000.zip", "OmegaStar-backup-20260103-090000.zip"];
        for name in backups {
            put(&dir.join(name), "zip");
        }
        put(&dir.join(format!("OmegaStar-backup-20260104-090000.zip{PARTIAL}")), "half");
        put(&dir.join("notes.txt"), "not a backup");
        let left = || {
            let mut v: Vec<String> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
            v.sort();
            v
        };
        // A 6 GB disk where each file in the folder takes a gigabyte.
        let free = |d: &Path| Some((6 - fs::read_dir(d).unwrap().count() as u64) * GB);
        let local = |_: &Path| GB;

        make_room(&dir, 3, Some(GB / 4), free, local).unwrap();
        assert_eq!(left().len(), 5, "1 GB free is twice the last backup and more");
        make_room(&dir, 3, None, |_| None, local).unwrap();
        assert_eq!(left().len(), 5, "free space that can't be read lets the backup go ahead");

        // Kept only online, the backups take no space here, so deleting them would make no room.
        let err = make_room(&dir, 3, Some(GB), free, |_| 0).unwrap_err();
        assert_eq!(err.message, "Not enough free disk space for a backup: it needs about 2.0 GB.");
        assert_eq!(left().len(), 5, "nothing is deleted for nothing");

        make_room(&dir, 3, Some(GB), free, local).unwrap();
        assert_eq!(left(), vec![backups[1], backups[2], "notes.txt"], "the oldest backup and the half-written one made room");

        let err = make_room(&dir, 2, Some(5 * GB / 2), free, local).unwrap_err();
        assert_eq!(err.message, "Not enough free disk space for a backup: it needs about 5.0 GB.");
        assert_eq!(left(), vec![backups[1], backups[2], "notes.txt"], "down to one fewer than keep would still be short, so nothing goes");
        make_room(&dir, 2, Some(2 * GB), free, local).unwrap();
        assert_eq!(left(), vec![backups[2], "notes.txt"], "down to one fewer than keep");
        let err = make_room(&dir, 1, None, |_| Some(GB / 2), local).unwrap_err();
        assert_eq!(err.message, "Not enough free disk space for a backup: it needs about 1.0 GB.");
        assert_eq!(left(), vec![backups[2], "notes.txt"], "the last backup always stays");

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_file_counts_the_space_it_takes_here_not_its_length() {
        let dir = temp_office("on-disk");
        let sparse = dir.join("OmegaStar-backup-20260101-090000.zip");
        File::create(&sparse).unwrap().set_len(GB).unwrap();
        put(&dir.join("notes.txt"), &"x".repeat(10_000));
        assert!(on_disk(&sparse) < 1024 * 1024, "a gigabyte long, but next to nothing here, as a backup kept only online is");
        assert!(on_disk(&dir.join("notes.txt")) >= 10_000);
        assert_eq!(on_disk(&dir.join("gone.zip")), 0);
        let _ = fs::remove_dir_all(&dir);
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
        run.prompt = "do".into();
        db.save_run(&run).unwrap();
        let fp5 = fingerprint(&office);
        assert_ne!(fp5, fp4, "an edit of the same length");
        let company = Company { name: "Sample Co".into(), ceo_name: "Sam".into(), title: "CEO".into(), email: None, reports_to: None, image: None };
        db.set_company(&company).unwrap();
        let fp6 = fingerprint(&office);
        assert_ne!(fp6, fp5, "the company profile");
        db.set_settings(&AppSettings { paused: true, ..Default::default() }).unwrap();
        let fp7 = fingerprint(&office);
        assert_ne!(fp7, fp6, "a setting");

        put(&office.join("employees/e1/node_modules/pkg/index.js"), "a much longer file");
        put(&office.join("omega.log"), "started\nstarted again\n");
        db.set_backup_status(&BackupStatus { last_at: Some(chrono::Utc::now()), ..Default::default() }).unwrap();
        assert_eq!(fingerprint(&office), fp7, "dependencies, the log and the backup status don't count");

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
    fn up_to_date_needs_the_last_zip_in_the_current_folder() {
        let office = temp_office("current");
        let dest = temp_office("current-dest");
        let zip = dest.join("OmegaStar-backup-20260101-090000.zip");
        put(&zip, "zip");
        let dir = dest.to_string_lossy().into_owned();
        let st = BackupStatus { fingerprint: Some("fp".into()), last_file: Some(zip.to_string_lossy().into_owned()), ..Default::default() };
        assert!(up_to_date(&st, "fp", &dir, &office));
        assert!(!up_to_date(&st, "other", &dir, &office), "the office changed");
        let moved = office.parent().unwrap().join(format!("elsewhere-{}", uuid::Uuid::new_v4()));
        assert!(!up_to_date(&st, "fp", &moved.to_string_lossy(), &office), "a new folder");
        fs::remove_file(&zip).unwrap();
        assert!(!up_to_date(&st, "fp", &dir, &office), "the zip is gone");

        let _ = fs::remove_dir_all(&office);
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn next_at_counts_from_a_check() {
        let t = chrono::Utc::now();
        let st = BackupStatus { last_at: Some(t), checked_at: Some(t + Duration::minutes(30)), ..Default::default() };
        let next = next_at(BackupFrequency::Hourly, &st, t + Duration::minutes(50));
        assert_eq!(next, Some(t + Duration::minutes(90)));
    }

    #[test]
    fn storage_names_agree_with_the_store() {
        assert_eq!(DB_FILES[0], DB_FILE);
        assert!(DB_FILES.iter().all(|f| f.starts_with(DB_FILE)));
        assert!(LEFT_OUT_NOTE.starts_with(&format!("Files over {} MB ", MAX_FILE_BYTES / (1024 * 1024))));
    }
}
