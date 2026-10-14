//! The skill catalog of the user's Claude Code install, and grant -> settings.json rendering.

use crate::model::{SkillGrant, SkillInfo, SkillSource, SkillSources};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// `$CLAUDE_CONFIG_DIR`, else `~/.claude`.
pub fn config_dir() -> PathBuf {
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => dirs::home_dir().unwrap_or_default().join(".claude"),
    }
}

/// Claude Code's own settings file, where it keeps the MCP servers of each project folder:
/// `$CLAUDE_CONFIG_DIR/.claude.json`, else `~/.claude.json`.
pub fn claude_json_path() -> PathBuf {
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(d) if !d.is_empty() => PathBuf::from(d).join(".claude.json"),
        _ => dirs::home_dir().unwrap_or_default().join(".claude.json"),
    }
}

/// What a scan could not read. A missing optional file or folder is not a failure; the skills
/// folder itself missing is, because then the catalog is empty for a reason the user can act on.
#[derive(Debug, Default, PartialEq)]
pub struct ScanReport {
    /// Skill folders (or their SKILL.md) that exist but could not be read.
    pub unreadable: usize,
    /// The first I/O error, with its path.
    pub error: Option<String>,
}

impl ScanReport {
    fn note(&mut self, path: &Path, e: &io::Error) {
        self.error.get_or_insert_with(|| format!("{}: {e}", path.display()));
    }

    fn unreadable(&mut self, path: &Path, e: &io::Error) {
        self.unreadable += 1;
        self.note(path, e);
    }

    /// Reads an optional file: `None` when it is absent, noted when it exists but cannot be read.
    fn read_optional(&mut self, path: &Path) -> Option<String> {
        match fs::read_to_string(path) {
            Ok(t) => Some(t),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => {
                self.note(path, &e);
                None
            }
        }
    }
}

/// Catalog in display order: user, synced, plugins; each sorted by name.
pub fn scan(config: &Path) -> Vec<SkillInfo> {
    scan_with_report(config, &claude_json_path()).0
}

fn scan_with_report(config: &Path, claude_json: &Path) -> (Vec<SkillInfo>, ScanReport) {
    let mut r = ScanReport::default();
    let root = config.join("skills");
    let mut out = user_skills(&root, &mut r);
    let user_names: HashSet<String> = out.iter().map(|s| s.name.clone()).collect();
    if let Some(bucket) = active_bucket(claude_json) {
        out.extend(synced_skills(&root.join("synced").join(bucket), &mut r).into_iter().filter(|s| !user_names.contains(&s.name)));
    }
    out.extend(plugins(&config.join("plugins").join("installed_plugins.json"), &mut r));
    (out, r)
}

/// The synced-skills folder of the signed-in organisation and account, named
/// `<organization uuid>_<account uuid or "unbound">` in lower case. Other folders belong to
/// organisations the account is not using now, whose skills would not load. `None` when signed out.
fn active_bucket(claude_json: &Path) -> Option<String> {
    let v: Value = serde_json::from_str(&fs::read_to_string(claude_json).ok()?).ok()?;
    let account = v.get("oauthAccount")?;
    let uuid = |k: &str| account.get(k).and_then(|u| u.as_str()).map(|u| u.trim().to_lowercase()).filter(|u| !u.is_empty());
    Some(format!("{}_{}", uuid("organizationUuid")?, uuid("accountUuid").unwrap_or_else(|| "unbound".into())))
}

/// Where the catalog comes from and how the last scan went.
pub fn sources(config: &Path) -> SkillSources {
    let (catalog, r) = scan_with_report(config, &claude_json_path());
    SkillSources {
        config_dir: config.to_string_lossy().into_owned(),
        skills_dir: config.join("skills").to_string_lossy().into_owned(),
        found: catalog.len(),
        unreadable: r.unreadable,
        error: r.error,
    }
}

/// Subfolders of `dir`, sorted. `required` makes a missing `dir` a noted failure.
fn subdirs(dir: &Path, required: bool, r: &mut ScanReport) -> Vec<PathBuf> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            if required || e.kind() != io::ErrorKind::NotFound {
                r.note(dir, &e);
            }
            return vec![];
        }
    };
    let mut v = vec![];
    for entry in entries {
        let p = match entry {
            Ok(e) => e.path(),
            Err(e) => {
                r.note(dir, &e);
                continue;
            }
        };
        match fs::metadata(&p) {
            // Follows symlinks and junctions.
            Ok(m) if m.is_dir() => v.push(p),
            Ok(_) => {}
            // A dangling link is not a skill.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => r.unreadable(&p, &e),
        }
    }
    v.sort();
    v
}

fn dir_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Name and description of a skill folder, if it has a SKILL.md.
pub fn read_skill(dir: &Path) -> Option<(String, String)> {
    read_skill_reported(dir, &mut ScanReport::default())
}

/// A folder without SKILL.md is not a skill; one whose SKILL.md cannot be read is counted.
fn read_skill_reported(dir: &Path, r: &mut ScanReport) -> Option<(String, String)> {
    let path = dir.join("SKILL.md");
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            if e.kind() != io::ErrorKind::NotFound {
                r.unreadable(&path, &e);
            }
            return None;
        }
    };
    let fm = frontmatter(&text);
    let name = fm.get("name").filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| dir_name(dir));
    Some((name, fm.get("description").cloned().unwrap_or_default()))
}

fn user_skills(root: &Path, r: &mut ScanReport) -> Vec<SkillInfo> {
    subdirs(root, true, r)
        .into_iter()
        .filter(|d| dir_name(d) != "synced")
        // Claude Code names a user skill by its folder, whatever its frontmatter says (verified).
        .filter_map(|d| read_skill_reported(&d, r).map(|(_, desc)| (dir_name(&d), desc)))
        .map(|(name, description)| SkillInfo {
            id: format!("user:{name}"),
            name,
            description,
            source: SkillSource::User,
            skills: None,
        })
        .collect()
}

/// The skills in one synced bucket folder.
fn synced_skills(bucket: &Path, r: &mut ScanReport) -> Vec<SkillInfo> {
    let manifest: BTreeMap<String, String> = r
        .read_optional(&bucket.join("manifest.json"))
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("skills").and_then(|s| s.as_array()).cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|s| Some((s.get("name")?.as_str()?.to_string(), s.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string())))
        .collect();
    let mut by_name: BTreeMap<String, String> = BTreeMap::new();
    for d in subdirs(bucket, false, r) {
        if let Some((_, desc)) = read_skill_reported(&d, r) {
            let name = dir_name(&d);
            let desc = manifest.get(&name).filter(|m| !m.is_empty()).cloned().unwrap_or(desc);
            by_name.insert(name, desc);
        }
    }
    by_name
        .into_iter()
        .map(|(name, description)| SkillInfo {
            id: format!("synced:{name}"),
            name,
            description,
            source: SkillSource::Synced,
            skills: None,
        })
        .collect()
}

/// `installed_plugins.json` has had several shapes; accept `{plugins: {key: [{installPath}] | {installPath}}}`
/// (v1/v2) and a bare `{key: ...}` map.
fn plugins(manifest: &Path, r: &mut ScanReport) -> Vec<SkillInfo> {
    let Some(v) = r.read_optional(manifest).and_then(|t| serde_json::from_str::<Value>(&t).ok()) else {
        return vec![];
    };
    let map = match v.get("plugins").and_then(|p| p.as_object()) {
        Some(m) => m.clone(),
        None => v.as_object().cloned().unwrap_or_default(),
    };
    let mut out = vec![];
    for (key, entry) in map {
        if !key.contains('@') {
            continue;
        }
        let install = match &entry {
            Value::Array(a) => a.iter().find_map(|i| i.get("installPath").and_then(|p| p.as_str())),
            Value::Object(_) => entry.get("installPath").and_then(|p| p.as_str()),
            _ => None,
        };
        let Some(install) = install.map(PathBuf::from) else { continue };
        let skill_list: Vec<(String, String)> = subdirs(&install.join("skills"), false, r).iter().filter_map(|d| read_skill_reported(d, r)).collect();
        let description = r
            .read_optional(&install.join(".claude-plugin").join("plugin.json"))
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|p| p.get("description").and_then(|d| d.as_str()).map(String::from))
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| skill_list.first().map(|s| s.1.clone()).unwrap_or_default());
        out.push(SkillInfo {
            id: format!("plugin:{key}"),
            name: key.split('@').next().unwrap_or(&key).to_string(),
            description,
            source: SkillSource::Plugin,
            skills: Some(skill_list.into_iter().map(|s| s.0).collect()),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The folder a user or synced catalog skill is read from. Plugin skills belong to their plugin.
pub fn source_dir(config: &Path, s: &SkillInfo) -> Option<PathBuf> {
    source_dir_with(config, &claude_json_path(), s)
}

fn source_dir_with(config: &Path, claude_json: &Path, s: &SkillInfo) -> Option<PathBuf> {
    let root = config.join("skills");
    let dir = match s.source {
        SkillSource::User => root.join(&s.name),
        SkillSource::Synced => root.join("synced").join(active_bucket(claude_json)?).join(&s.name),
        SkillSource::Plugin => return None,
    };
    Some(dir).filter(|d| d.join("SKILL.md").is_file())
}

/// Minimal YAML frontmatter reader: top-level `key: value` pairs, with quoted, plain multi-line
/// and block (`|`, `>`) scalars. Enough for `name` and `description`.
pub fn frontmatter(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return out;
    }
    let body: Vec<&str> = lines.take_while(|l| l.trim_end() != "---").collect();
    let mut i = 0;
    while i < body.len() {
        let line = body[i];
        i += 1;
        if line.starts_with(' ') || line.starts_with('\t') || line.trim_start().starts_with('#') {
            continue;
        }
        let Some((key, rest)) = line.split_once(':') else { continue };
        let rest = rest.trim();
        let mut cont = vec![];
        while i < body.len() && (body[i].starts_with(' ') || body[i].starts_with('\t') || body[i].trim().is_empty()) {
            cont.push(body[i].trim());
            i += 1;
        }
        while cont.last() == Some(&"") {
            cont.pop();
        }
        let value = if rest.starts_with('|') {
            cont.join("\n")
        } else if rest.starts_with('>') {
            cont.join(" ")
        } else if let Some(q) = rest.strip_prefix('"') {
            let joined = std::iter::once(q).chain(cont.iter().copied()).collect::<Vec<_>>().join(" ");
            unquote_double(&joined)
        } else if let Some(q) = rest.strip_prefix('\'') {
            let joined = std::iter::once(q).chain(cont.iter().copied()).collect::<Vec<_>>().join(" ");
            joined.rsplit_once('\'').map(|(s, _)| s).unwrap_or(&joined).replace("''", "'")
        } else {
            std::iter::once(rest).chain(cont.iter().copied()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
        };
        out.insert(key.trim().to_string(), value);
    }
    out
}

fn unquote_double(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(o) => out.push(o),
                None => {}
            },
            c => out.push(c),
        }
    }
    out
}

/// The context size at which Claude Code compacts a long session on its own.
const AUTO_COMPACT_WINDOW: u64 = 200_000;

/// The per-run `settings.json`: exactly the granted catalog skills stay visible. `own` are the
/// employee's project skills, which must never be switched off even if a catalog skill shares a name.
pub fn render_settings(grant: &SkillGrant, catalog: &[SkillInfo], own: &HashSet<String>, allow_rules: &[String]) -> Value {
    let granted: HashSet<&str> = grant.keys.iter().map(String::as_str).collect();
    let mut overrides = Map::new();
    let mut plugins = Map::new();
    for s in catalog {
        let on = granted.contains(s.id.as_str());
        match s.source {
            SkillSource::User | SkillSource::Synced => {
                if !on && !own.contains(&s.name) {
                    overrides.insert(s.name.clone(), json!("off"));
                }
            }
            SkillSource::Plugin => {
                let key = s.id.trim_start_matches("plugin:");
                plugins.insert(key.to_string(), json!(on));
            }
        }
    }
    let mut allow: Vec<Value> = allow_rules.iter().map(|r| json!(r)).collect();
    allow.push(json!("mcp__omega__*"));
    json!({
        "autoCompactWindow": AUTO_COMPACT_WINDOW,
        "disableBundledSkills": !grant.builtins,
        "skillOverrides": overrides,
        "enabledPlugins": plugins,
        "permissions": { "allow": allow },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(dir: &Path, name: &str) {
        fs::create_dir_all(dir.join(name)).unwrap();
        fs::write(dir.join(name).join("SKILL.md"), format!("---\nname: {name}\ndescription: Does {name}.\n---\n")).unwrap();
    }

    #[test]
    fn synced_skills_come_from_the_signed_in_bucket_only() {
        let dir = std::env::temp_dir().join(format!("omegastar-catalog-{}", uuid::Uuid::new_v4()));
        let config = dir.join("config");
        let synced = config.join("skills").join("synced");
        skill(&synced.join("a_x"), "old-org-skill");
        skill(&synced.join("b_y"), "current-skill");
        let claude_json = dir.join(".claude.json");
        let names = |claude_json: &Path| scan_with_report(&config, claude_json).0.into_iter().map(|s| s.name).collect::<Vec<_>>();

        fs::write(&claude_json, r#"{"oauthAccount":{"organizationUuid":"B","accountUuid":"Y"}}"#).unwrap();
        assert_eq!(names(&claude_json), ["current-skill"]);
        let found = SkillInfo { id: "synced:current-skill".into(), name: "current-skill".into(), description: String::new(), source: SkillSource::Synced, skills: None };
        assert_eq!(source_dir_with(&config, &claude_json, &found), Some(synced.join("b_y").join("current-skill")));
        let other = SkillInfo { name: "old-org-skill".into(), ..found };
        assert_eq!(source_dir_with(&config, &claude_json, &other), None);

        fs::write(&claude_json, r#"{"numStartups":3}"#).unwrap();
        assert!(names(&claude_json).is_empty(), "signed out: no synced skills");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_set_the_auto_compact_window() {
        let s = render_settings(&SkillGrant::default(), &[], &HashSet::new(), &[]);
        assert_eq!(s["autoCompactWindow"], 200_000);
    }
}
