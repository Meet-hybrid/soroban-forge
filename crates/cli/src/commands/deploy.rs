use crate::cli::DeployArgs;
use anyhow::{Context, Result};

pub fn run(args: DeployArgs) -> Result<()> {
    let network = crate::network_profiles::resolve_network(
        args.network.as_deref(),
        args.network_profile.as_deref(),
        args.rpc_url.as_deref(),
        args.network_passphrase.as_deref(),
    )?;
    let mut cmd = std::process::Command::new("stellar");
    cmd.arg("contract").arg("deploy");
    cmd.arg("--wasm").arg(args.wasm);
    if let Some(source) = args.source {
        cmd.arg("--source-account").arg(source);
    }
    cmd.arg("--network").arg(network.name);
    if let Some(rpc_url) = network.rpc_url {
        cmd.arg("--rpc-url").arg(rpc_url);
    }
    if let Some(passphrase) = network.network_passphrase {
        cmd.arg("--network-passphrase").arg(passphrase);
    }
    let status = cmd
        .status()
        .context("failed to run stellar contract deploy")?;
    if !status.success() {
        std::process::exit(1);
    }
    Ok(())
}
