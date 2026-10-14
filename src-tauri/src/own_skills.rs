//! How employees grow: role-skill scaffold, the managed omega-skill-craft skill, skill snapshots and
//! diffs (the raw material of `skill_revisions`), guarded file access, and promotion to the catalog.

use crate::catalog;
use crate::model::*;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const CRAFT_NAME: &str = "omega-skill-craft";
const CRAFT: &str = include_str!("../resources/omega-skill-craft/SKILL.md");
const SNAPSHOT_MAX_BYTES: u64 = 256 * 1024;

/// The employee's own skills. They live outside `.claude/` because Claude Code refuses edits under
/// a project's `.claude/` without an interactive approval, even with allow rules (verified on
/// 2.1.283); `.claude/skills` links here so Claude Code still loads them.
pub fn skills_dir(employee_dir: &Path) -> PathBuf {
    employee_dir.join("skills")
}

pub(crate) fn is_link(p: &Path) -> bool {
    let sym = fs::symlink_metadata(p).map(|m| m.file_type().is_symlink()).unwrap_or(false);
    #[cfg(windows)]
    let sym = sym || junction::exists(p).unwrap_or(false);
    sym
}

fn make_link(real: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    return junction::create(real, link); // junctions need no privileges, unlike symlinks
    #[cfg(unix)]
    return std::os::unix::fs::symlink(real, link);
}

/// Makes `.claude/skills` a link to `skills/`.
pub fn ensure_skill_link(employee_dir: &Path) -> Result<()> {
    let real = skills_dir(employee_dir);
    let link = employee_dir.join(".claude").join("skills");
    fs::create_dir_all(&real)?;
    fs::create_dir_all(employee_dir.join(".claude"))?;
    if is_link(&link) {
        if fs::canonicalize(&link).ok() == fs::canonicalize(&real).ok() {
            return Ok(());
        }
        fs::remove_dir(&link).or_else(|_| fs::remove_file(&link))?;
    } else if link.exists() {
        fs::remove_file(&link)?;
    }
    make_link(&real, &link)?;
    Ok(())
}

/// Lowercase ASCII `[a-z0-9-]` from the name, unique among `taken` (other slugs and catalog names).
pub fn slugify(name: &str, taken: &HashSet<String>) -> String {
    let mut base = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            base.push(c);
        } else if !base.ends_with('-') {
            base.push('-');
        }
    }
    let base = base.trim_matches('-');
    let base = if base.is_empty() { "employee" } else { base };
    let free = |s: &str| !taken.contains(s) && s != CRAFT_NAME;
    if free(base) {
        return base.to_string();
    }
    (2..).map(|n| format!("{base}-{n}")).find(|s| free(s)).expect("unbounded")
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn yaml_quote(s: &str) -> String {
    format!("\"{}\"", one_line(s).replace('\\', "\\\\").replace('"', "\\\""))
}

fn clip(s: &str, n: usize) -> String {
    let s = one_line(s);
    if s.chars().count() <= n {
        s
    } else {
        format!("{}...", s.chars().take(n).collect::<String>().trim_end())
    }
}

fn role_skill_text(e: &Employee, granted: &[SkillInfo]) -> String {
    let objective = if e.objective.trim().is_empty() { "Not set yet. Ask the person you work for." } else { e.objective.trim() };
    let about = if e.description.trim().is_empty() { "" } else { e.description.trim() };
    let granted_list = if granted.is_empty() {
        "None granted yet.".to_string()
    } else {
        granted
            .iter()
            .map(|s| match &s.skills {
                Some(list) => format!("- Plugin `{}` ({}): {}", s.name, list.join(", "), clip(&s.description, 200)),
                None => format!("- `{}`: {}", s.name, clip(&s.description, 200)),
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "---\nname: {slug}\ndescription: {desc}\n---\n\n# {name}, {title}\n\n## Objective\n\n{objective}\n\n{about}\n\n\
## Procedures\n\nYour own procedures are sibling skills named `{slug}-<task>`. List each here with one line on when to\n\
use it. None yet: when you learn a repeatable task, write it as a leaf per `omega-skill-craft` and link it here.\n\n\
## Granted skills\n\nCatalog skills you compose by name. Never copy their content.\n\n{granted_list}\n\n\
## Standing preferences\n\nPreferences and corrections you were taught that apply everywhere, one bullet each, ending `(taught YYYY-MM-DD)`. \
Never remove or reword one without the say-so of the person who taught it.\n",
        slug = e.slug,
        desc = yaml_quote(&format!(
            "Role skill for {}, {}. Use at the start of every task to recall your objective, standing preferences, and which procedure or granted skill to reach for.",
            e.name, e.title
        )),
        name = one_line(&e.name),
        title = one_line(&e.title),
    )
}

/// Hire-time scaffold. Never overwrites: after hiring, the role skill belongs to the employee.
pub fn scaffold_role_skill(skills_dir: &Path, e: &Employee, granted: &[SkillInfo]) -> Result<()> {
    let path = skills_dir.join(&e.slug).join("SKILL.md");
    if !path.exists() {
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, role_skill_text(e, granted))?;
    }
    Ok(())
}

pub fn install_craft(skills_dir: &Path) -> Result<()> {
    let dir = skills_dir.join(CRAFT_NAME);
    fs::create_dir_all(&dir)?;
    let path = dir.join("SKILL.md");
    if fs::read_to_string(&path).ok().as_deref() != Some(CRAFT) {
        fs::write(path, CRAFT)?;
    }
    Ok(())
}

/// Text files under the employee's skills, keyed by (skill, relative file), excluding the managed skill.
pub type Snapshot = BTreeMap<(String, String), String>;

fn walk(dir: &Path, rel: &str, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        let p = e.path();
        if p.is_dir() {
            walk(&p, &r, out);
        } else {
            out.push((r, p));
        }
    }
}

pub(crate) fn skill_files(skill_dir: &Path) -> Vec<(String, PathBuf)> {
    let mut v = vec![];
    walk(skill_dir, "", &mut v);
    v.sort();
    v
}

pub(crate) fn skill_dirs(skills_dir: &Path) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = fs::read_dir(skills_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    v.sort();
    v
}

/// Folders a skill's scripts fill with dependencies and caches, and data files they write: neither is
/// what the employee wrote, so snapshots leave them out.
const UNRECORDED_DIRS: [&str; 5] = ["node_modules", "__pycache__", ".cache", ".venv", ".git"];
const UNRECORDED_EXTS: [&str; 3] = ["json", "jsonl", "csv"];

/// The files of a skill worth recording: `skill_files` without UNRECORDED_DIRS and data files.
fn recorded_files(skill_dir: &Path) -> Vec<(String, PathBuf)> {
    fn walk(dir: &Path, rel: &str, out: &mut Vec<(String, PathBuf)>) {
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let p = e.path();
            if p.is_dir() {
                if !UNRECORDED_DIRS.contains(&name.as_str()) {
                    walk(&p, &r, out);
                }
            } else if !Path::new(&name).extension().map(|x| UNRECORDED_EXTS.iter().any(|u| x.eq_ignore_ascii_case(u))).unwrap_or(false) {
                out.push((r, p));
            }
        }
    }
    let mut v = vec![];
    walk(skill_dir, "", &mut v);
    v.sort();
    v
}

pub fn snapshot(skills_dir: &Path) -> Snapshot {
    let mut snap = Snapshot::new();
    for (skill, dir) in skill_dirs(skills_dir).into_iter().filter(|(s, _)| s != CRAFT_NAME) {
        for (file, path) in recorded_files(&dir) {
            let small = fs::metadata(&path).map(|m| m.len() < SNAPSHOT_MAX_BYTES).unwrap_or(false);
            if let (true, Ok(text)) = (small, fs::read_to_string(&path)) {
                snap.insert((skill.clone(), file), text);
            }
        }
    }
    snap
}

/// Keeps a snapshot on disk (a JSON list of [skill, file, text]), so a run's edits can still be
/// recorded after a crash.
pub fn save_snapshot(path: &Path, s: &Snapshot) -> std::io::Result<()> {
    let rows: Vec<[&str; 3]> = s.iter().map(|((skill, file), text)| [skill.as_str(), file.as_str(), text.as_str()]).collect();
    fs::write(path, serde_json::to_string(&rows)?)
}

/// A snapshot `save_snapshot` wrote; `None` when it is missing or unreadable.
pub fn load_snapshot(path: &Path) -> Option<Snapshot> {
    let rows: Vec<[String; 3]> = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    Some(rows.into_iter().map(|[skill, file, text]| ((skill, file), text)).collect())
}

/// The name a tidy's changes to an employee's notes (top-level markdown in `workspace/`) are recorded
/// under, so they can be reviewed and reverted like skill edits. Not a folder, so never a skill name.
pub const NOTES: &str = "@workspace";

/// Top-level markdown files in the employee's `workspace/`, keyed like skill files. Only a tidy takes
/// this snapshot: ordinary runs rewrite their state files every time, which would drown the history.
pub fn notes_snapshot(workspace: &Path) -> Snapshot {
    let mut snap = Snapshot::new();
    for e in fs::read_dir(workspace).into_iter().flatten().flatten() {
        let (p, name) = (e.path(), e.file_name().to_string_lossy().into_owned());
        let small = fs::metadata(&p).map(|m| m.len() < SNAPSHOT_MAX_BYTES).unwrap_or(false);
        if p.is_file() && small && name.to_lowercase().ends_with(".md") {
            if let Ok(text) = fs::read_to_string(&p) {
                snap.insert((NOTES.to_string(), name), text);
            }
        }
    }
    snap
}

/// Markdown notes anywhere under `workspace/` changed since `since`: (path inside it, when), newest
/// first. Hidden files and folders and `node_modules` are left out.
pub fn changed_notes(workspace: &Path, since: Ts) -> Vec<(String, Ts)> {
    fn walk(dir: &Path, rel: &str, since: Ts, out: &mut Vec<(String, Ts)>) {
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                walk(&e.path(), &r, since, out);
            } else if name.to_lowercase().ends_with(".md") {
                if let Some(at) = meta.modified().ok().map(Ts::from).filter(|at| *at >= since) {
                    out.push((r, at));
                }
            }
        }
    }
    let mut v = vec![];
    walk(workspace, "", since, &mut v);
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v
}

/// The company page and what an employee owns are recorded under these names. Only a conversation
/// with the person they all work for changes either, so Undo never overwrites a newer change to them.
pub const COMPANY: &str = "@company";
pub const COMPANY_FILE: &str = "CLAUDE.md";
pub const OWN: &str = "@own";
pub const OWN_FILE: &str = "what-you-own.md";

pub fn guarded(skill: &str) -> bool {
    skill == COMPANY || skill == OWN
}

/// The company page: a CLAUDE.md above every employee's folder, so Claude Code loads it in every run.
pub fn company_page_path(employees_dir: &Path) -> PathBuf {
    employees_dir.join(COMPANY_FILE)
}

const COMPANY_PAGE: &str = "# Company page

Every employee reads this page at the start of every run, so keep it short: one line per fact, and point to where details live instead of copying them. Never write a password, key or account number here. It changes only in a conversation with the person you all work for.

## What we sell

## Who to ask

## How things work here

## Public copy rules

## Right now

Dated lines, newest first.

## Decisions

Dated lines, newest first: what was decided, and why, in one line.
";

/// Writes the empty company page once. An existing page is never touched.
pub fn seed_company_page(employees_dir: &Path) -> Result<()> {
    let path = company_page_path(employees_dir);
    if !path.exists() {
        fs::write(path, COMPANY_PAGE)?;
    }
    Ok(())
}

/// Where a recorded file lives: a skill file under `skills/`, a notes file under `workspace/`, the
/// company page above the employee's folder, or what they own in it.
pub fn recorded_path(employee_dir: &Path, skill: &str, file: &str) -> Result<PathBuf> {
    if guarded(skill) {
        let (want, dir) = if skill == COMPANY { (COMPANY_FILE, employee_dir.parent()) } else { (OWN_FILE, Some(employee_dir)) };
        return match dir {
            Some(dir) if file == want => Ok(dir.join(file)),
            _ => Err(AppError::invalid(format!("invalid file '{skill}/{file}'"))),
        };
    }
    if skill == NOTES {
        if file.contains(['/', '\\']) || !safe_segment(file) {
            return Err(AppError::invalid(format!("invalid notes file '{file}'")));
        }
        return Ok(employee_dir.join("workspace").join(file));
    }
    resolve(&skills_dir(employee_dir), skill, file)
}

/// Writes (`Some`) or deletes (`None`) a recorded file. Deleting a notes or guarded file never removes
/// the folder it sits in.
pub fn write_recorded(path: &Path, skill: &str, text: Option<&str>) -> Result<()> {
    if skill != NOTES && !guarded(skill) {
        return write_file(path, text);
    }
    match text {
        Some(t) => fs::write(path, t)?,
        None => {
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
    }
    Ok(())
}

/// The company page and what the employee owns, keyed like skill files, each only while it exists. No size
/// cap: a page too large to read would look missing, and putting that back would delete it. Kept apart from
/// the skills snapshot: a run that should not change them has its change put back instead of recorded.
pub fn guarded_snapshot(employee_dir: &Path) -> Snapshot {
    let mut snap = Snapshot::new();
    let files = [(COMPANY, COMPANY_FILE, employee_dir.parent()), (OWN, OWN_FILE, Some(employee_dir))];
    for (skill, file, dir) in files {
        let Some(path) = dir.map(|d| d.join(file)) else { continue };
        let is_file = fs::metadata(&path).map(|m| m.is_file()).unwrap_or(false);
        if let (true, Ok(text)) = (is_file, fs::read_to_string(&path)) {
            snap.insert((skill.to_string(), file.to_string()), text);
        }
    }
    snap
}

pub struct FileChange {
    pub change: SkillChange,
    pub before: Option<String>,
    pub after: Option<String>,
}

pub fn diff(before: &Snapshot, after: &Snapshot) -> Vec<FileChange> {
    let keys: std::collections::BTreeSet<&(String, String)> = before.keys().chain(after.keys()).collect();
    keys.into_iter()
        .filter_map(|k| {
            let (b, a) = (before.get(k), after.get(k));
            let change = match (b, a) {
                (None, Some(_)) => ChangeKind::Added,
                (Some(_), None) => ChangeKind::Removed,
                (Some(x), Some(y)) if x != y => ChangeKind::Modified,
                _ => return None,
            };
            Some(FileChange {
                change: SkillChange { skill: k.0.clone(), file: k.1.clone(), change },
                before: b.cloned(),
                after: a.cloned(),
            })
        })
        .collect()
}

pub fn revision(fc: FileChange, run_id: Option<String>, at: Ts) -> SkillRevision {
    SkillRevision {
        change: fc.change,
        id: uuid::Uuid::new_v4().to_string(),
        run_id,
        at,
        before: fc.before,
        after: fc.after,
    }
}

fn safe_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s.contains(':')
        && matches!(Path::new(s).components().next(), Some(Component::Normal(_)))
        && Path::new(s).components().count() == 1
}

/// Resolves `<skills_dir>/<skill>/<file>`, refusing anything that could leave the skill folder.
pub fn resolve(skills_dir: &Path, skill: &str, file: &str) -> Result<PathBuf> {
    let bad = || AppError::invalid(format!("invalid skill path '{skill}/{file}'"));
    if !safe_segment(skill) || skill.contains(['/', '\\']) {
        return Err(bad());
    }
    let parts: Vec<&str> = file.split(['/', '\\']).collect();
    if parts.iter().any(|p| !safe_segment(p)) {
        return Err(bad());
    }
    let mut p = skills_dir.join(skill);
    for part in parts {
        p.push(part);
    }
    Ok(p)
}

/// Writes (`Some`) or deletes (`None`) a skill file, removing the skill folder once it is empty.
fn write_file(path: &Path, text: Option<&str>) -> Result<()> {
    match text {
        Some(t) => {
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(path, t)?;
        }
        None => {
            if path.exists() {
                fs::remove_file(path)?;
            }
            let mut dir = path.parent();
            while let Some(d) = dir {
                if d.file_name().map(|n| n == "skills").unwrap_or(true) || fs::remove_dir(d).is_err() {
                    break;
                }
                dir = d.parent();
            }
        }
    }
    Ok(())
}

/// The employee's skills: role first, then leaves by name, the managed skill last.
pub fn own_skills(skills_dir: &Path, slug: &str) -> Vec<OwnSkill> {
    let mut v: Vec<OwnSkill> = skill_dirs(skills_dir)
        .into_iter()
        .map(|(name, dir)| {
            let files = skill_files(&dir);
            let updated = files
                .iter()
                .filter_map(|(_, p)| fs::metadata(p).and_then(|m| m.modified()).ok())
                .max()
                .map(Ts::from)
                .unwrap_or_default();
            OwnSkill {
                description: catalog::read_skill(&dir).map(|s| s.1).unwrap_or_default(),
                role: name == slug,
                managed: name == CRAFT_NAME,
                files: files.into_iter().map(|f| f.0).collect(),
                updated_at: updated,
                name,
            }
        })
        .collect();
    v.sort_by_key(|s| (!s.role, s.managed, s.name.clone()));
    v
}

/// Folder and frontmatter names of the employee's skills.
pub fn own_skill_names(skills_dir: &Path) -> HashSet<String> {
    let mut names = HashSet::new();
    for (name, dir) in skill_dirs(skills_dir) {
        if let Some((n, _)) = catalog::read_skill(&dir) {
            names.insert(n);
        }
        names.insert(name);
    }
    names
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for (rel, path) in skill_files(from) {
        let dest = to.join(&rel);
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::copy(path, dest)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- importing and organizing

/// Where imported skills wait to be organized. Outside `skills/`, so Claude Code never loads a
/// copy that still shares its name with your original.
pub fn imports_dir(employee_dir: &Path) -> PathBuf {
    employee_dir.join("imports")
}

/// Copies a catalog skill folder, scripts and all, into `imports/<name>`.
pub fn stage_import(imports: &Path, from: &Path, name: &str) -> Result<()> {
    if !safe_segment(name) || name.contains(['/', '\\']) {
        return Err(AppError::invalid(format!("invalid skill name '{name}'")));
    }
    let dest = imports.join(name);
    if dest.exists() {
        return Err(AppError::conflict(format!("'{name}' is already waiting to be organized")));
    }
    copy_dir(from, &dest)
}

pub fn staged_imports(imports: &Path) -> Vec<StagedImport> {
    skill_dirs(imports)
        .into_iter()
        .map(|(name, dir)| StagedImport {
            description: catalog::read_skill(&dir).map(|s| s.1).unwrap_or_default(),
            files: skill_files(&dir).into_iter().map(|f| f.0).collect(),
            name,
        })
        .collect()
}

/// Copies every staged import under `dest`, so reverting an Organize run can put them back.
pub fn backup_imports(imports: &Path, dest: &Path) -> Result<()> {
    for (name, dir) in skill_dirs(imports) {
        if !dest.join(&name).exists() {
            copy_dir(&dir, &dest.join(&name))?;
        }
    }
    Ok(())
}

/// Restores backed-up imports that are neither staged nor in the family any more.
pub fn restore_imports(backup: &Path, imports: &Path, names: &[String], skills_dir: &Path) -> Result<Vec<String>> {
    let mut restored = vec![];
    for name in names {
        let (src, dest) = (backup.join(name), imports.join(name));
        if src.is_dir() && !dest.exists() && !skills_dir.join(name).exists() {
            copy_dir(&src, &dest)?;
            restored.push(name.clone());
        }
    }
    Ok(restored)
}

/// `<slug>-<words>`: lowercase ASCII words joined by single hyphens.
fn family_name_ok(slug: &str, name: &str) -> bool {
    let Some(rest) = name.strip_prefix(slug).and_then(|r| r.strip_prefix('-')) else { return false };
    name.len() <= 64 && !rest.is_empty() && rest.split('-').all(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
}

/// Sets the frontmatter `name`, adding frontmatter when the file has none.
fn with_name(text: &str, name: &str) -> String {
    let body = text.trim_start_matches('\u{feff}');
    let Some(rest) = body.strip_prefix("---\n").or_else(|| body.strip_prefix("---\r\n")) else {
        return format!("---\nname: {name}\n---\n\n{body}");
    };
    let mut lines: Vec<String> = vec![];
    let mut done = false;
    let mut in_fm = true;
    for line in rest.split_inclusive('\n') {
        if in_fm && line.trim_end() == "---" {
            if !done {
                lines.push(format!("name: {name}\n"));
                done = true;
            }
            in_fm = false;
        } else if in_fm && !done && line.starts_with("name:") {
            lines.push(format!("name: {name}\n"));
            done = true;
            continue;
        }
        lines.push(line.to_string());
    }
    format!("---\n{}", lines.concat())
}

/// Finds `name` among the staged imports first, then the family. The role skill and the managed
/// skill never move.
fn movable(skills_dir: &Path, imports: &Path, slug: &str, name: &str) -> Result<PathBuf> {
    if !safe_segment(name) || name.contains(['/', '\\']) {
        return Err(AppError::invalid(format!("invalid skill name '{name}'")));
    }
    if name == CRAFT_NAME || name == slug {
        return Err(AppError::invalid(format!("'{name}' cannot be moved or retired")));
    }
    [imports.join(name), skills_dir.join(name)]
        .into_iter()
        .find(|d| d.is_dir())
        .ok_or_else(|| AppError::not_found(format!("no skill '{name}' in imports/ or in your skills")))
}

/// Moves a staged import or one of the employee's skills to `skills/<to>`, whole folder, and sets
/// its frontmatter name.
pub fn move_skill(skills_dir: &Path, imports: &Path, slug: &str, from: &str, to: &str) -> Result<PathBuf> {
    let src = movable(skills_dir, imports, slug, from)?;
    if !family_name_ok(slug, to) {
        return Err(AppError::invalid(format!("'{to}' is not a family name: use {slug}-<task>, lowercase words joined by hyphens")));
    }
    let dest = skills_dir.join(to);
    if dest.exists() {
        return Err(AppError::conflict(format!("you already have a skill named '{to}'")));
    }
    fs::rename(&src, &dest)?;
    let md = dest.join("SKILL.md");
    if let Ok(text) = fs::read_to_string(&md) {
        fs::write(&md, with_name(&text, to))?;
    }
    Ok(dest)
}

/// Moves a staged import or one of the employee's skills out of the way, into `archive/<name>`.
pub fn retire_skill(skills_dir: &Path, imports: &Path, slug: &str, name: &str, archive: &Path) -> Result<PathBuf> {
    let src = movable(skills_dir, imports, slug, name)?;
    let dest = (1..).map(|n| if n == 1 { archive.join(name) } else { archive.join(format!("{name}-{n}")) }).find(|d| !d.exists()).expect("unbounded");
    fs::create_dir_all(archive)?;
    if fs::rename(&src, &dest).is_err() {
        copy_dir(&src, &dest)?;
        fs::remove_dir_all(&src)?;
    }
    Ok(dest)
}

/// Copies an employee skill into the user's Claude Code skills, where it joins the catalog.
pub fn promote(skills_dir: &Path, skill: &str, config_dir: &Path, catalog: &[SkillInfo]) -> Result<SkillInfo> {
    let src = resolve(skills_dir, skill, "SKILL.md")?;
    if skill == CRAFT_NAME {
        return Err(AppError::invalid("omega-skill-craft is managed by Omega Star"));
    }
    let Some((name, _)) = catalog::read_skill(src.parent().unwrap()) else {
        return Err(AppError::not_found(format!("skill '{skill}' has no SKILL.md")));
    };
    let dest = config_dir.join("skills").join(skill);
    if dest.exists() || catalog.iter().any(|s| s.name == name || s.name == skill) {
        return Err(AppError::conflict(format!("a skill named '{skill}' already exists in Claude Code")));
    }
    copy_dir(src.parent().unwrap(), &dest)?;
    let (name, description) = catalog::read_skill(&dest).unwrap_or((skill.to_string(), String::new()));
    Ok(SkillInfo { id: format!("user:{name}"), name, description, source: SkillSource::User, skills: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("omegastar-own-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn company_page_is_seeded_once() {
        let dir = temp_dir();
        seed_company_page(&dir).unwrap();
        assert_eq!(fs::read_to_string(company_page_path(&dir)).unwrap(), COMPANY_PAGE);
        let mine = "# Company page\n\n## What we sell\n\nGarden tools.\n";
        fs::write(company_page_path(&dir), mine).unwrap();
        seed_company_page(&dir).unwrap();
        assert_eq!(fs::read(company_page_path(&dir)).unwrap(), mine.as_bytes(), "an existing page is left byte for byte");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn changed_notes_are_markdown_outside_hidden_and_dependency_folders() {
        let dir = temp_dir();
        for f in ["campaign.md", "gads/log.md", "data.csv", "node_modules/pkg/README.md", ".cache/old.md", ".hidden.md"] {
            let p = dir.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, "x").unwrap();
        }
        let since = Ts::from(std::time::SystemTime::now() - std::time::Duration::from_secs(60));
        let mut names: Vec<String> = changed_notes(&dir, since).into_iter().map(|(f, _)| f).collect();
        names.sort();
        assert_eq!(names, ["campaign.md", "gads/log.md"]);
        assert!(changed_notes(&dir, Ts::from(std::time::SystemTime::now() + std::time::Duration::from_secs(60))).is_empty(), "older than since");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn guarded_files_have_one_place_each() {
        let employee = Path::new("/office/employees/e1");
        assert_eq!(recorded_path(employee, COMPANY, COMPANY_FILE).unwrap(), Path::new("/office/employees/CLAUDE.md"));
        assert_eq!(recorded_path(employee, OWN, OWN_FILE).unwrap(), Path::new("/office/employees/e1/what-you-own.md"));
        for (skill, file) in [(COMPANY, "x.md"), (COMPANY, "../CLAUDE.md"), (OWN, "../what-you-own.md"), (OWN, COMPANY_FILE), (NOTES, "../state.md")] {
            assert!(recorded_path(employee, skill, file).is_err(), "{skill}/{file}");
        }
        assert!(guarded(COMPANY) && guarded(OWN) && !guarded(NOTES) && !guarded("ada"));
    }

    #[test]
    fn guarded_snapshot_reads_the_page_and_what_they_own() {
        let root = temp_dir();
        let employee = root.join("e1");
        fs::create_dir_all(&employee).unwrap();
        assert!(guarded_snapshot(&employee).is_empty(), "neither file exists yet");
        fs::write(company_page_path(&root), "# Company page\n").unwrap();
        fs::write(employee.join(OWN_FILE), "The weekly signups.\n").unwrap();
        fs::write(employee.join(COMPANY_FILE), "# Handbook\n").unwrap();
        let snap = guarded_snapshot(&employee);
        assert_eq!(snap.len(), 2, "the employee's own handbook is not guarded");
        assert_eq!(snap[&(COMPANY.to_string(), COMPANY_FILE.to_string())], "# Company page\n");
        assert_eq!(snap[&(OWN.to_string(), OWN_FILE.to_string())], "The weekly signups.\n");
        let large = "- A line about the company.\n".repeat(SNAPSHOT_MAX_BYTES as usize / 10);
        fs::write(company_page_path(&root), &large).unwrap();
        assert_eq!(guarded_snapshot(&employee)[&(COMPANY.to_string(), COMPANY_FILE.to_string())], large, "a large page is not taken for a missing one");
        let _ = fs::remove_dir_all(&root);
    }
}
