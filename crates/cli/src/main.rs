//! Soroban Forge developer CLI.
//!
//! ```text
//! soroban-forge build
//! soroban-forge test --package soroban-forge-escrow
//! soroban-forge lint --fix
//! soroban-forge deploy path/to/escrow.wasm --network testnet
//! ```

mod cli;
mod commands;

use clap::{Parser, Subcommand};
use cli::{BuildArgs, DeployArgs, LintArgs, TestArgs};

#[derive(Parser, Debug)]
#[command(
    name = "soroban-forge",
    about = "Developer CLI for Soroban Forge",
    version,
    author
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Build(BuildArgs),
    Lint(LintArgs),
    Test(TestArgs),
    Deploy(DeployArgs),
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::Build(args) => commands::build::run(args)?,
        Commands::Lint(args) => commands::lint::run(args)?,
        Commands::Test(args) => commands::test::run(args)?,
        Commands::Deploy(args) => commands::deploy::run(args)?,
    }

    Ok(())
}
