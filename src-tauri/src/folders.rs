//! Workspace folders: the folders outside their own that an employee's runs may read and edit.
//!
//! One normaliser turns what was typed (`~/dev/site`, `C:\work\site`, `/Users/jack/dev/site/`) into one full path,
//! and every place that uses a workspace goes through it: saving an employee or a proposal, `--add-dir`, and the
//! check that a card's file is in the employee's folders.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// This computer's home folder, which `~` stands for.
pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

/// A workspace folder as a full path, or why it cannot be one. `~` and `~/…` become the home folder; `.`, `..`,
/// doubled and trailing separators are tidied away. A Windows drive (`C:\work`, `c:/work`) or share
/// (`\\server\share`) counts as full on any computer, so a draft written on one is read the same on another.
/// It only looks at the text: whether the folder is there is `check`'s job.
pub fn normalize(raw: &str, home: &Path) -> Result<String, String> {
    let raw = raw.trim();
    let raw = ["\"", "'"].iter().find_map(|q| raw.strip_prefix(q).and_then(|r| r.strip_suffix(q))).unwrap_or(raw).trim();
    if raw.is_empty() {
        return Err("is empty".into());
    }
    let expanded = match raw.strip_prefix('~') {
        Some("") => home.to_string_lossy().into_owned(),
        Some(rest) if rest.starts_with('/') || rest.starts_with('\\') => format!("{}{rest}", home.to_string_lossy()),
        Some(_) => return Err("starts with ~ and a name; only ~ on its own means your home folder".into()),
        None => raw.to_string(),
    };
    let (root, rest, sep) = split_root(&expanded).ok_or_else(|| "is not a full path. Start it with / or ~/ (or a drive such as C:\\ on Windows)".to_string())?;
    let mut parts: Vec<&str> = vec![];
    // Windows takes either separator; elsewhere a backslash can be part of a folder's name.
    let windows = sep == '\\' || cfg!(windows);
    for part in rest.split(|c| c == '/' || (windows && c == '\\')) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    Ok(format!("{root}{}", parts.join(&sep.to_string())))
}

/// A full path's root (`/`, `C:\` or `\\server\share\`), the rest of it, and the separator it uses. None when the
/// path is not full, including a drive with no separator after it (`C:work`, relative to that drive).
fn split_root(path: &str) -> Option<(String, &str, char)> {
    let b = path.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        let drive = (b[0] as char).to_ascii_uppercase();
        return match b.get(2) {
            None => Some((format!("{drive}:\\"), "", '\\')),
            Some(b'/' | b'\\') => Some((format!("{drive}:\\"), &path[3..], '\\')),
            Some(_) => None,
        };
    }
    for lead in ["\\\\", "//"] {
        if let Some(unc) = path.strip_prefix(lead) {
            let mut it = unc.splitn(3, ['/', '\\']);
            let (server, share) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
            if server.is_empty() || share.is_empty() {
                return None;
            }
            return Some((format!("\\\\{server}\\{share}\\"), it.next().unwrap_or(""), '\\'));
        }
    }
    path.strip_prefix('/').map(|rest| ("/".to_string(), rest, '/'))
}

/// Where a saved workspace is on this computer. A folder saved before workspaces were checked may still be
/// relative: it stays relative to the employee's folder, where their runs start, as it always was.
pub fn resolve(raw: &str, employee_dir: &Path, home: &Path) -> PathBuf {
    match normalize(raw, home) {
        Ok(full) => PathBuf::from(full),
        Err(_) => employee_dir.join(raw.trim()),
    }
}

/// Every saved workspace that is not a folder on this computer, as the path runs would use, so a run can say
/// which before it starts instead of failing quietly.
pub fn missing(workspaces: &[String], employee_dir: &Path, home: &Path) -> Vec<PathBuf> {
    workspaces.iter().filter(|w| !w.trim().is_empty()).map(|w| resolve(w, employee_dir, home)).filter(|p| !p.is_dir()).collect()
}

/// A typed folder, checked for the profile and hire sheet: its full path, whether it is there, or why it cannot
/// be used.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderCheck {
    pub raw: String,
    pub path: Option<String>,
    pub found: bool,
    pub problem: Option<String>,
}

pub fn check(raw: &str, home: &Path) -> FolderCheck {
    match normalize(raw, home) {
        Ok(path) => FolderCheck { raw: raw.to_string(), found: Path::new(&path).is_dir(), path: Some(path), problem: None },
        Err(problem) => FolderCheck { raw: raw.to_string(), path: None, found: false, problem: Some(problem) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> &'static Path {
        Path::new("/Users/jack")
    }

    fn norm(raw: &str) -> Result<String, String> {
        normalize(raw, home())
    }

    #[test]
    fn a_tilde_becomes_the_home_folder() {
        assert_eq!(norm("~/dev/site").unwrap(), "/Users/jack/dev/site");
        assert_eq!(norm("~").unwrap(), "/Users/jack");
        assert_eq!(norm("  ~/dev/site/  ").unwrap(), "/Users/jack/dev/site");
    }

    #[test]
    fn a_full_path_is_tidied_but_kept() {
        assert_eq!(norm("/Users/jack/dev//site/./src/../").unwrap(), "/Users/jack/dev/site");
        assert_eq!(norm("/").unwrap(), "/");
        assert_eq!(norm("/../..").unwrap(), "/");
        assert_eq!(norm("'/Users/jack/My Site'").unwrap(), "/Users/jack/My Site");
    }

    #[test]
    fn a_relative_or_empty_folder_is_refused_with_the_reason() {
        assert!(norm("dev/site").unwrap_err().contains("not a full path"));
        assert!(norm("./site").unwrap_err().contains("not a full path"));
        assert_eq!(norm("   ").unwrap_err(), "is empty");
        assert!(norm("~bob/site").unwrap_err().contains("only ~ on its own"));
    }

    #[test]
    fn windows_drives_and_shares_are_full_paths_on_any_computer() {
        assert_eq!(norm(r"C:\work\site\").unwrap(), r"C:\work\site");
        assert_eq!(norm("c:/work/site").unwrap(), r"C:\work\site");
        assert_eq!(norm(r"D:\work\..\other\.\x").unwrap(), r"D:\other\x");
        assert_eq!(norm("E:").unwrap(), r"E:\");
        assert_eq!(norm(r"\\nas\share\projects").unwrap(), r"\\nas\share\projects");
        assert_eq!(norm("//nas/share").unwrap(), r"\\nas\share\");
        assert!(norm("C:work").unwrap_err().contains("not a full path"));
        assert!(norm(r"\\nas").unwrap_err().contains("not a full path"));
    }

    #[test]
    fn a_tilde_follows_a_windows_home() {
        assert_eq!(normalize(r"~\work\site", Path::new(r"C:\Users\jack")).unwrap(), r"C:\Users\jack\work\site");
        assert_eq!(normalize("~/work/site", Path::new(r"C:\Users\jack")).unwrap(), r"C:\Users\jack\work\site");
    }

    #[test]
    fn a_tilde_folder_saved_earlier_resolves_to_home_not_inside_the_employee_folder() {
        let employee = Path::new("/office/employees/e1");
        assert_eq!(resolve("~/x", employee, home()), PathBuf::from("/Users/jack/x"));
        assert_eq!(resolve("workspace/shared", employee, home()), PathBuf::from("/office/employees/e1/workspace/shared"));
    }

    #[test]
    fn missing_lists_only_folders_that_are_not_there() {
        let dir = std::env::temp_dir().join(format!("omegastar-folders-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("here")).unwrap();
        let there = dir.join("here").to_string_lossy().into_owned();
        let gone = dir.join("gone").to_string_lossy().into_owned();
        assert_eq!(missing(&[there.clone(), gone.clone(), " ".into()], &dir, home()), vec![dir.join("gone")]);
        assert!(check(&there, home()).found);
        assert_eq!(check(&gone, home()), FolderCheck { raw: gone.clone(), path: Some(gone), found: false, problem: None });
        assert!(check("dev/site", home()).problem.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
