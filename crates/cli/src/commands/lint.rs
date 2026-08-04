use crate::cli::LintArgs;
use anyhow::{Context, Result};

pub fn run(args: LintArgs) -> Result<()> {
    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("clippy").arg("--workspace").arg("--all-targets");
    if args.fix {
        cmd.arg("--fix");
    }
    cmd.arg("--").arg("-D").arg("warnings");
    let status = cmd.status().context("failed to run cargo clippy")?;
    if !status.success() {
        std::process::exit(1);
    }
    Ok(())
}
