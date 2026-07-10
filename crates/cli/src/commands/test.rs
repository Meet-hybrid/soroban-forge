use crate::TestArgs;
use anyhow::{Context, Result};

pub fn run(args: TestArgs) -> Result<()> {
    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("test").arg(args.scope.clone());
    if let Some(pkg) = args.package {
        cmd.arg("--package").arg(pkg);
    }
    let status = cmd.status().context("failed to run cargo test")?;
    if !status.success() {
        std::process::exit(1);
    }
    Ok(())
}
