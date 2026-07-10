use crate::LintArgs;
use anyhow::{Context, Result};

pub fn run(_args: LintArgs) -> Result<()> {
    let status = std::process::Command::new("cargo")
        .arg("clippy")
        .arg("--workspace")
        .arg("--all-targets")
        .arg("--")
        .arg("-D")
        .arg("warnings")
        .status()
        .context("failed to run cargo clippy")?;
    if !status.success() {
        std::process::exit(1);
    }
    Ok(())
}
