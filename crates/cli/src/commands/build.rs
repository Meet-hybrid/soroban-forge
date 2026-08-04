use crate::cli::BuildArgs;
use anyhow::{Context, Result};

pub fn run(args: BuildArgs) -> Result<()> {
    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("build");
    match args.package {
        Some(pkg) => {
            cmd.arg("--package").arg(pkg);
        }
        None => {
            cmd.arg("--workspace");
        }
    }
    if args.release {
        cmd.arg("--release");
    }
    if args.all_targets {
        cmd.arg("--all-targets");
    }
    let status = cmd.status().context("failed to run cargo build")?;
    if !status.success() {
        std::process::exit(1);
    }
    Ok(())
}
