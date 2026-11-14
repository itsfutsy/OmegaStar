//! The computer's power state: whether a Mac is in a full wake, and keeping the computer awake while a run works.

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

/// Asks the computer not to idle-sleep until the process `pid` exits, so it does not nap mid-run: `caffeinate` on a
/// Mac, a hidden PowerShell holding `SetThreadExecutionState` on Windows (`windows_stay_awake`).
pub fn stay_awake_while(pid: u32) {
    #[cfg(target_os = "macos")]
    let child = tokio::process::Command::new("/usr/bin/caffeinate")
        .args(["-i", "-w", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    #[cfg(windows)]
    let child = {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        tokio::process::Command::new("powershell.exe")
            .args(windows_stay_awake(pid))
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    };
    #[cfg(any(target_os = "macos", windows))]
    if let Ok(mut child) = child {
        tokio::spawn(async move {
            let _ = child.wait().await;
        });
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = pid;
}

/// PowerShell's arguments for keeping Windows awake until process `pid` exits, as `caffeinate -i -w` does on a Mac:
/// it asks for ES_CONTINUOUS | ES_SYSTEM_REQUIRED, which holds until PowerShell exits, then waits for `pid`. The
/// script goes in encoded (UTF-16, base64), so no quote in it passes through Windows' command-line quoting. Pure.
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_stay_awake(pid: u32) -> [String; 6] {
    use base64::Engine;
    let script = format!(
        "$k = Add-Type -Name Power -Namespace OmegaStar -PassThru -MemberDefinition '[DllImport(\"kernel32.dll\")] public static extern uint SetThreadExecutionState(uint flags);'; \
         [void]$k::SetThreadExecutionState(0x80000001); Wait-Process -Id {pid} -ErrorAction SilentlyContinue"
    );
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    ["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-EncodedCommand", &encoded].map(String::from)
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

    #[test]
    fn windows_stays_awake_until_the_run_exits_with_no_quotes_on_the_command_line() {
        use base64::Engine;
        let args = windows_stay_awake(4242);
        assert_eq!(args[..5], ["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-EncodedCommand"]);
        assert!(!args.iter().any(|a| a.contains('"') || a.contains(' ')), "nothing for cmd-line quoting to get wrong: {args:?}");
        let bytes = base64::engine::general_purpose::STANDARD.decode(&args[5]).unwrap();
        let units: Vec<u16> = bytes.chunks(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
        let script = String::from_utf16(&units).unwrap();
        assert!(script.contains("SetThreadExecutionState(0x80000001)"), "ES_CONTINUOUS | ES_SYSTEM_REQUIRED: {script}");
        assert!(script.ends_with("Wait-Process -Id 4242 -ErrorAction SilentlyContinue"), "{script}");
    }
}
