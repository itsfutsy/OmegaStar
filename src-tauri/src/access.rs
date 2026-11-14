//! What an employee who was blocked may ask you for, and what you may give them in one click on the blocked card:
//! a permission rule (`Bash(npm:*)`) and a folder outside their own to work in. The rules are kept narrow and the
//! folders real, because an approval is one click.
//!
//! `full_folder` is a small copy of the folder tidying on the workspace folders branch (`folders::normalize`), kept
//! here so this change stands on its own. When both are in, this one should call that one.

use crate::model::Denial;
use serde_json::Value;
use std::path::{Component, Path, PathBuf};

/// The tool name of an ask for a folder alone, with no rule.
pub const FOLDER_TOOL: &str = "folder";

/// The longest reason an employee may give when they ask.
const WHY_CHARS: usize = 300;

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

/// `raw` as one full path: quotes and spaces trimmed, `~` and `~/x` expanded, `.` and `..` worked out. Text only:
/// it does not look on disk. A path that is not full, or `~name`, is refused with the reason.
pub fn full_folder(raw: &str, home: &Path) -> Result<PathBuf, String> {
    let t = raw.trim().trim_matches(|c| c == '"' || c == '\'').trim();
    if t.is_empty() {
        return Err("is blank".into());
    }
    let p = match t.strip_prefix('~') {
        Some("") => home.to_path_buf(),
        Some(rest) if rest.starts_with('/') || rest.starts_with('\\') => home.join(rest.trim_start_matches(['/', '\\'])),
        Some(_) => return Err("starts with ~ and a name; write the full path instead".into()),
        None => PathBuf::from(t),
    };
    if !p.is_absolute() {
        return Err("is not a full path, such as /Users/you/projects/site".into());
    }
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    Ok(out)
}

/// A folder you may give an employee from a blocked card: one that is there, inside your home folder and not the
/// whole of it. Returns it as a full path, as typed (not following links), or why it can't be given.
pub fn check_folder(raw: &str, home: &Path) -> Result<String, String> {
    let p = full_folder(raw, home)?;
    if !p.is_dir() {
        return Err("is not there".into());
    }
    // Compared with links followed, so a link inside your home to somewhere outside it is still outside.
    let (real, real_home) = (p.canonicalize().map_err(|_| "can't be read".to_string())?, home.canonicalize().unwrap_or_else(|_| home.to_path_buf()));
    if real == real_home {
        return Err("is your whole home folder; pick the folder the work is in".into());
    }
    if !real.starts_with(&real_home) {
        return Err("is outside your home folder".into());
    }
    Ok(p.to_string_lossy().into_owned())
}

/// A rule so wide it would let the employee do nearly anything, which no one-click approval should grant.
pub fn too_broad(rule: &str) -> bool {
    let r: String = rule.chars().filter(|c| !c.is_whitespace()).collect();
    matches!(r.as_str(), "*" | "Bash" | "Bash(*)" | "Bash(:*)" | "Bash(**)" | "mcp__*" | "mcp__*__*")
}

/// The folder a blocked step reached outside the employee's own: the folder of the file it read or wrote, the
/// folder it searched, or the folder a command first changed into (`cd /x && ...`). `None` when it named none or
/// gave a path that isn't full (that one is inside their own folder).
pub fn folder_of(tool: &str, input: &Value, home: &Path) -> Option<String> {
    let s = |k: &str| input.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty());
    let raw = match tool {
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
            let file = full_folder(s("file_path").or(s("notebook_path"))?, home).ok()?;
            return file.parent().map(|p| p.to_string_lossy().into_owned());
        }
        "Glob" | "Grep" | "LS" => s("path")?.to_string(),
        "Bash" => cd_target(s("command")?)?.0.to_string(),
        _ => return None,
    };
    full_folder(&raw, home).ok().map(|p| p.to_string_lossy().into_owned())
}

/// For a command that starts `cd <folder> && <rest>`: the folder and the rest.
pub fn cd_target(command: &str) -> Option<(&str, &str)> {
    let rest = command.trim().strip_prefix("cd ")?;
    let (dir, then) = rest.split_once("&&")?;
    let dir = dir.trim().trim_matches(|c| c == '"' || c == '\'');
    (!dir.is_empty() && !dir.contains(char::is_whitespace)).then_some((dir, then.trim()))
}

/// What an employee asked for with `request_access`, as blocked steps for its card: one per rule, then one per
/// folder no rule already carries. Each is marked as asked, with their reason as its line.
pub fn asked(rules: &[String], folders: &[String], why: &str) -> Vec<Denial> {
    let why: String = crate::claude::one_line(why).chars().take(WHY_CHARS).collect();
    let mut out: Vec<Denial> = rules
        .iter()
        .map(|r| Denial { tool: tool_of(r), brief: why.clone(), suggested_rule: r.clone(), folder: None, asked: true })
        .collect();
    // The first folder rides on the first rule, so the card shows one item when they asked for a command in a folder.
    let mut folders = folders.iter();
    if let (Some(first), Some(f)) = (out.first_mut(), folders.clone().next()) {
        first.folder = Some(f.clone());
        folders.next();
    }
    out.extend(folders.map(|f| Denial { tool: FOLDER_TOOL.into(), brief: why.clone(), suggested_rule: String::new(), folder: Some(f.clone()), asked: true }));
    out
}

/// The tool a rule is for: `Bash` for `Bash(npm:*)`, the tool itself for `mcp__x__y`.
fn tool_of(rule: &str) -> String {
    rule.split_once('(').map(|(t, _)| t).unwrap_or(rule).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_folder_is_written_in_full_from_what_was_typed() {
        let home = Path::new("/Users/maren");
        assert_eq!(full_folder("~/dev/site/", home).unwrap(), PathBuf::from("/Users/maren/dev/site"));
        assert_eq!(full_folder(" \"/Users/maren/dev/./site/../app\" ", home).unwrap(), PathBuf::from("/Users/maren/dev/app"));
        assert_eq!(full_folder("~", home).unwrap(), PathBuf::from("/Users/maren"));
        assert!(full_folder("dev/site", home).unwrap_err().contains("not a full path"));
        assert!(full_folder("~bob/x", home).unwrap_err().contains("full path"));
        assert_eq!(full_folder("  ", home).unwrap_err(), "is blank");
    }

    #[test]
    fn only_a_real_folder_inside_home_and_not_home_itself_can_be_given() {
        let home = std::env::temp_dir().join(format!("access-home-{}", std::process::id()));
        let site = home.join("dev").join("site");
        std::fs::create_dir_all(&site).unwrap();
        assert_eq!(check_folder("~/dev/site", &home).unwrap(), site.to_string_lossy());
        assert_eq!(check_folder("~/dev/missing", &home).unwrap_err(), "is not there");
        assert!(check_folder("~", &home).unwrap_err().contains("whole home folder"));
        assert_eq!(check_folder(&std::env::temp_dir().to_string_lossy(), &home).unwrap_err(), "is outside your home folder");
        assert_eq!(check_folder("~/dev/site/../../..", &home).unwrap_err(), "is outside your home folder");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_rule_that_would_allow_almost_anything_is_too_broad() {
        for r in ["*", "Bash", "Bash(*)", "Bash(:*)", " Bash( * ) ", "mcp__*"] {
            assert!(too_broad(r), "{r}");
        }
        for r in ["Bash(npm:*)", "Bash(git push:*)", "mcp__linear__*", "WebFetch"] {
            assert!(!too_broad(r), "{r}");
        }
    }

    #[test]
    fn the_folder_a_blocked_step_reached_is_found_from_its_path_or_its_cd() {
        let home = Path::new("/Users/maren");
        assert_eq!(folder_of("Read", &json!({ "file_path": "/Users/maren/dev/site/README.md" }), home).as_deref(), Some("/Users/maren/dev/site"));
        assert_eq!(folder_of("Write", &json!({ "file_path": "~/notes/a.md" }), home).as_deref(), Some("/Users/maren/notes"));
        assert_eq!(folder_of("Grep", &json!({ "path": "/Users/maren/dev/site", "pattern": "x" }), home).as_deref(), Some("/Users/maren/dev/site"));
        assert_eq!(folder_of("Bash", &json!({ "command": "cd ~/dev/site && npm test" }), home).as_deref(), Some("/Users/maren/dev/site"));
        assert_eq!(folder_of("Bash", &json!({ "command": "npm test" }), home), None);
        assert_eq!(folder_of("Read", &json!({ "file_path": "workspace/notes.md" }), home), None, "a relative path is inside their own folder");
        assert_eq!(folder_of("mcp__linear__create_issue", &json!({ "title": "x" }), home), None);
    }

    #[test]
    fn an_ask_puts_the_first_folder_with_the_first_rule_and_the_rest_on_their_own() {
        let a = asked(&["Bash(npm:*)".into()], &["/Users/maren/dev/site".into(), "/Users/maren/dev/api".into()], "To run the site's tests.");
        assert_eq!(a.len(), 2);
        assert_eq!((a[0].tool.as_str(), a[0].suggested_rule.as_str(), a[0].folder.as_deref()), ("Bash", "Bash(npm:*)", Some("/Users/maren/dev/site")));
        assert_eq!((a[1].tool.as_str(), a[1].suggested_rule.as_str(), a[1].folder.as_deref()), (FOLDER_TOOL, "", Some("/Users/maren/dev/api")));
        assert!(a.iter().all(|d| d.asked && d.brief == "To run the site's tests."));
        let only_folder = asked(&[], &["/Users/maren/dev/site".into()], "Read the brief.");
        assert_eq!(only_folder[0].tool, FOLDER_TOOL);
    }
}
