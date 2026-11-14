use std::{os::unix::process::CommandExt, process::Command};

use anyhow::{Context, Result, bail};
use serde_json::Value;

const REPO: &str = "tcmarkfeld/terminal-git";
const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// Reinstalls tg from the latest GitHub release when it is newer than this build.
pub fn run() -> Result<()> {
    let out = Command::new("curl")
        .args(["-fsSL", &format!("https://api.github.com/repos/{REPO}/releases/latest")])
        .output()
        .context("curl is needed to update tg")?;
    if !out.status.success() {
        bail!("couldn't check GitHub for updates: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let release: Value = serde_json::from_slice(&out.stdout)?;
    let latest = release["tag_name"].as_str().context("no release found on GitHub")?.trim_start_matches('v');
    if !is_newer(latest, CURRENT) {
        println!("tg {CURRENT} is up to date");
        return Ok(());
    }
    println!("Updating tg {CURRENT} → {latest}");
    // exec replaces this process, so the installer can overwrite the running binary.
    let installer = format!("curl -LsSf https://github.com/{REPO}/releases/latest/download/tg-installer.sh | sh");
    Err(Command::new("sh").args(["-c", &installer]).exec()).context("couldn't run the installer")
}

fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| v.split('.').map(|part| part.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>();
    parse(candidate) > parse(current)
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.2", "0.1.2"));
        assert!(!is_newer("0.1.1", "0.1.2"));
    }
}
