#![no_std]
//! Soroban Forge developer CLI.
//!
//! ```text
//! soroban-forge build --all
//! soroban-forge test --package escrow
//! soroban-forge deploy --wasm target/.../escrow.wasm --network testnet
//! soroban-forge lint --all
//! ```
//!
//! This crate is compiled as a standard binary, not a WASM contract.
//! The `#![no_std]` annotation is not used here.

#[macro_use]
extern crate tracing;

mod cli;
mod commands;
mod error;
mod utils;

use clap::{Parser, Subcommand};
use commands::{BuildArgs, CliCommand, DeployArgs, LintArgs, TestArgs};

type Result<T> = std::result::Result<T, error::CliError>;

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

fn main() -> Result<()> {
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
