use crate::cli::TestArgs;
use anyhow::{Context, Result};

pub fn run(args: TestArgs) -> Result<()> {
    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("test");
    match args.package {
        Some(pkg) => {
            cmd.arg("--package").arg(pkg);
        }
        None => {
            cmd.arg("--workspace");
        }
    }
    let status = cmd.status().context("failed to run cargo test")?;
    if !status.success() {
        std::process::exit(1);
    }
    Ok(())
}
