//! The saved pairing, `remote.json` in the office folder. Owner-only where the system has file modes, left out of
//! backups (`backup.rs`), and never in the keychain, which would ask again after every unsigned update.

use crate::log::log;
use crate::model::Ts;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// The saved pairing's file name, at the top of the office folder.
pub const FILE: &str = "remote.json";

/// Where the paired phone's pushes go: its address at Apple or Google.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PushTarget {
    pub platform: String,
    pub token: String,
    pub sandbox: bool,
}

/// A paired phone: the long-term room and key, and what the computer knows about the phone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pairing {
    pub relay: String,
    pub room_id: String,
    pub key: String,
    pub device_name: String,
    pub paired_at: Ts,
    #[serde(default)]
    pub push: Option<PushTarget>,
    /// The Settings switch for phone pushes.
    #[serde(default = "on")]
    pub push_on: bool,
    /// When a phone session last started.
    #[serde(default)]
    pub last_seen_at: Option<Ts>,
}

/// Phone pushes are on until switched off.
fn on() -> bool {
    true
}

/// The saved pairing, if there is one. An unreadable file is logged and counts as no pairing.
pub fn load(dir: &Path) -> Option<Pairing> {
    let text = match fs::read_to_string(dir.join(FILE)) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
        Err(e) => {
            log!("could not read {FILE}: {e}");
            return None;
        }
    };
    serde_json::from_str(&text).inspect_err(|e| log!("could not read {FILE}: {e}")).ok()
}

/// Saves `pairing`, replacing the file whole so a crash never leaves half of it.
pub fn save(dir: &Path, pairing: &Pairing) -> io::Result<()> {
    let tmp = dir.join(format!("{FILE}.partial"));
    let _ = fs::remove_file(&tmp);
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut f = opts.open(&tmp)?;
    f.write_all(&serde_json::to_vec_pretty(pairing)?)?;
    f.sync_all()?;
    fs::rename(&tmp, dir.join(FILE))
}

/// Forgets the saved pairing.
pub fn delete(dir: &Path) -> io::Result<()> {
    match fs::remove_file(dir.join(FILE)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::temp_dir;

    fn pairing() -> Pairing {
        Pairing {
            relay: "relay.example.com".into(),
            room_id: "room".into(),
            key: "key".into(),
            device_name: "Test Phone".into(),
            paired_at: chrono::Utc::now(),
            push: Some(PushTarget { platform: "ios".into(), token: "t1".into(), sandbox: true }),
            push_on: false,
            last_seen_at: None,
        }
    }

    #[test]
    fn a_pairing_saves_loads_and_deletes() {
        let dir = temp_dir("remote-store");
        assert_eq!(load(&dir), None);
        let p = pairing();
        save(&dir, &p).unwrap();
        save(&dir, &p).unwrap();
        assert_eq!(load(&dir), Some(p));
        delete(&dir).unwrap();
        delete(&dir).unwrap();
        assert_eq!(load(&dir), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn only_the_owner_can_read_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("remote-store-mode");
        save(&dir, &pairing()).unwrap();
        assert_eq!(fs::metadata(dir.join(FILE)).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pushes_default_to_on() {
        let dir = temp_dir("remote-store-default");
        let mut v = serde_json::to_value(pairing()).unwrap();
        v.as_object_mut().unwrap().remove("pushOn");
        fs::write(dir.join(FILE), v.to_string()).unwrap();
        assert!(load(&dir).unwrap().push_on);
        let _ = fs::remove_dir_all(&dir);
    }
}
