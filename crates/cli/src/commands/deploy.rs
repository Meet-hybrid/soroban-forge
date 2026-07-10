use crate::DeployArgs;
use anyhow::{Context, Result};

pub fn run(args: DeployArgs) -> Result<()> {
    let mut cmd = std::process::Command::new("stellar");
    cmd.arg("contract").arg("deploy");
    cmd.arg("--wasm").arg(args.wasm);
    if let Some(source) = args.source {
        cmd.arg("--source-account").arg(source);
    }
    cmd.arg("--network").arg(args.network);
    let status = cmd
        .status()
        .context("failed to run stellar contract deploy")?;
    if !status.success() {
        std::process::exit(1);
    }
    Ok(())
}
