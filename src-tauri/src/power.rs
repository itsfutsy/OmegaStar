//! The Mac's power state: whether it is in a full wake, and keeping it awake while a run works.

/// False in a Power Nap dark wake, when the display and graphics stay off and background work would
/// freeze when the Mac naps again. Any failure counts as a full wake. Blocks: runs `ioreg`.
pub fn full_wake() -> bool {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("/usr/sbin/ioreg").args(["-n", "IOPMrootDomain", "-r", "-d1"]).output();
        match out {
            Ok(o) if o.status.success() => graphics_on(&String::from_utf8_lossy(&o.stdout)).unwrap_or(true),
            _ => true,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Whether the root power domain's "System Capabilities" include graphics (bit 0x2), which a dark
/// wake leaves out. `None` when the line is missing.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn graphics_on(ioreg_out: &str) -> Option<bool> {
    let line = ioreg_out.lines().find(|l| l.contains("\"System Capabilities\""))?;
    let caps: u64 = line.split('=').nth(1)?.trim().parse().ok()?;
    Some(caps & 0x2 != 0)
}

/// Asks macOS not to idle-sleep until the process `pid` exits, so the Mac does not nap mid-run.
pub fn stay_awake_while(pid: u32) {
    #[cfg(target_os = "macos")]
    {
        let child = tokio::process::Command::new("/usr/bin/caffeinate")
            .args(["-i", "-w", &pid.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Ok(mut child) = child {
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = pid;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphics_on_reads_the_capability_bit() {
        let out = |caps: &str| format!("+-o IOPMrootDomain  <class IOPMrootDomain>\n    {{\n      \"Supported Features\" = {{}}\n      \"System Capabilities\" = {caps}\n    }}\n");
        assert_eq!(graphics_on(&out("15")), Some(true));
        assert_eq!(graphics_on(&out("9")), Some(false));
        assert_eq!(graphics_on("+-o IOPMrootDomain\n    {\n    }\n"), None);
    }
}
