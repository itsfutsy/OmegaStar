use std::process::Command;

/// `git` with these arguments, trimmed; `None` when git is missing or fails.
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}

/// Stamps the build with its commit and channel (OMEGA_COMMIT, OMEGA_CHANNEL), read with `env!`.
fn stamp() {
    println!("cargo:rerun-if-env-changed=OMEGA_COMMIT");
    println!("cargo:rerun-if-env-changed=OMEGA_CHANNEL");
    // HEAD and the branch it points at, so a new commit restamps. `--git-path` also works in a worktree.
    for path in [git(&["rev-parse", "--git-path", "HEAD"]), git(&["rev-parse", "--symbolic-full-name", "HEAD"]).and_then(|r| git(&["rev-parse", "--git-path", &r]))].into_iter().flatten() {
        println!("cargo:rerun-if-changed={path}");
    }
    let commit = std::env::var("OMEGA_COMMIT").ok().filter(|c| !c.is_empty()).or_else(|| git(&["rev-parse", "--short=10", "HEAD"])).unwrap_or_else(|| "unknown".into());
    let channel = match std::env::var("OMEGA_CHANNEL").as_deref() {
        Ok(c @ ("release" | "dogfood")) => c.to_string(),
        _ => "dev".into(),
    };
    println!("cargo:rustc-env=OMEGA_COMMIT={commit}");
    println!("cargo:rustc-env=OMEGA_CHANNEL={channel}");
}

fn main() {
    stamp();
    tauri_build::build()
}
