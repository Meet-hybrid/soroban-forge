//! Soroban Forge developer CLI.
//!
//! ```text
//! soroban-forge build
//! soroban-forge build --wasm --check-size
//! soroban-forge test --package soroban-forge-escrow
//! soroban-forge lint --fix
//! soroban-forge deploy path/to/escrow.wasm --network testnet
//! soroban-forge verify --wasm path/to/contract.wasm
//! soroban-forge verify --expected <sha256>
//! soroban-forge verify --manifest provenance-manifest.json
//! soroban-forge doctor --wasm target/wasm32v1-none/release/contract.wasm
//! soroban-forge ttl-audit --contract <CONTRACT_ID> --network testnet
//! soroban-forge ttl-audit --contract <ID> --threshold 50000 --format json
//! soroban-forge ttl-audit --contract <ID> --keys keys.json --plan extend.json
//! soroban-forge plan my-plan.json --dry-run
//! ```

mod cli;
mod commands;

use std::process;

use clap::{Parser, Subcommand};
use cli::{
    BuildArgs, DeployArgs, DoctorArgs, EventsArgs, InspectArgs, InvokeArgs, LintArgs, NewArgs,
    PlanArgs, ScaffoldValidateArgs, TestArgs, TtlAuditArgs, VerifyArgs,
};

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
    New(NewArgs),
    ScaffoldValidate(ScaffoldValidateArgs),
    Verify(VerifyArgs),
    Invoke(InvokeArgs),
    Events(EventsArgs),
    Inspect(InspectArgs),
    Doctor(DoctorArgs),
    /// Audit TTL (time-to-live) for contract storage entries.
    TtlAudit(TtlAuditArgs),
    /// Execute a transaction plan file.
    Plan(PlanArgs),
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::Build(args) => commands::build::run(args)?,
        Commands::Lint(args) => commands::lint::run(args)?,
        Commands::Test(args) => commands::test::run(args)?,
        Commands::Deploy(args) => commands::deploy::run(args)?,
        Commands::New(args) => commands::new::run(args)?,
        Commands::ScaffoldValidate(args) => commands::scaffold_validate::run(args)?,
        Commands::Verify(args) => commands::verify::run(args)?,
        Commands::Invoke(args) => commands::invoke::run(args)?,
        Commands::Events(args) => commands::events::run(args)?,
        Commands::Inspect(args) => commands::inspect::run(args)?,
        Commands::Doctor(args) => commands::doctor::run(args)?,
        Commands::Plan(args) => commands::plan::run(args)?,
        Commands::TtlAudit(args) => match commands::ttl_audit::run(args) {
            Ok(exit_code) => {
                let code = exit_code as i32;
                if code != 0 {
                    process::exit(code);
                }
            }
            Err(e) => {
                eprintln!("error: {e:#}");
                process::exit(2);
            }
        },
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Cli, Commands};
    use clap::{CommandFactory, Parser};

    #[test]
    fn build_wasm_and_size_flags_parse() {
        let cli = Cli::try_parse_from([
            "soroban-forge",
            "build",
            "--wasm",
            "--check-size",
            "--package",
            "soroban-forge-escrow",
        ])
        .unwrap();

        let Commands::Build(args) = cli.command else {
            panic!("expected build command");
        };
        assert!(args.wasm);
        assert!(args.check_size);
        assert_eq!(args.package.as_deref(), Some("soroban-forge-escrow"));
    }

    #[test]
    fn build_help_documents_wasm_and_size_flags() {
        let mut command = Cli::command();
        let help = command
            .find_subcommand_mut("build")
            .expect("build subcommand must exist")
            .render_long_help()
            .to_string();
        assert!(help.contains("--wasm"));
        assert!(help.contains("--check-size"));
    }

    #[test]
    fn verify_flags_parse() {
        let cli = Cli::try_parse_from([
            "soroban-forge",
            "verify",
            "--wasm",
            "target/wasm32v1-none/release/soroban_forge_escrow.wasm",
            "--expected",
            &"a".repeat(64),
        ])
        .unwrap();

        let Commands::Verify(args) = cli.command else {
            panic!("expected verify command");
        };
        assert_eq!(
            args.wasm.as_deref(),
            Some("target/wasm32v1-none/release/soroban_forge_escrow.wasm")
        );
        assert_eq!(args.expected.as_deref(), Some("a".repeat(64).as_str()));
        assert_eq!(args.manifest, "provenance-manifest.json");
    }

    #[test]
    fn verify_manifest_defaults_to_provenance_manifest() {
        let cli = Cli::try_parse_from(["soroban-forge", "verify", "--wasm", "x.wasm"]).unwrap();
        let Commands::Verify(args) = cli.command else {
            panic!("expected verify command");
        };
        assert_eq!(args.manifest, "provenance-manifest.json");
    }

    #[test]
    fn verify_help_documents_flags() {
        let mut command = Cli::command();
        let help = command
            .find_subcommand_mut("verify")
            .expect("verify subcommand must exist")
            .render_long_help()
            .to_string();
        assert!(help.contains("--wasm"));
        assert!(help.contains("--expected"));
        assert!(help.contains("--manifest"));
    }

    #[test]
    fn scaffold_validate_accepts_a_contract_path() {
        let cli = Cli::try_parse_from([
            "soroban-forge",
            "scaffold-validate",
            "crates/example-contract",
        ])
        .unwrap();

        let Commands::ScaffoldValidate(args) = cli.command else {
            panic!("expected scaffold-validate command");
        };
        assert_eq!(
            args.path,
            std::path::PathBuf::from("crates/example-contract")
        );
    }

    // ── TTL audit CLI tests ──────────────────────────────────────────────────

    #[test]
    fn ttl_audit_required_contract_flag() {
        let cli = Cli::try_parse_from([
            "soroban-forge",
            "ttl-audit",
            "--contract",
            "CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB",
        ])
        .unwrap();

        let Commands::TtlAudit(args) = cli.command else {
            panic!("expected ttl-audit command");
        };
        assert_eq!(
            args.contract,
            "CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB"
        );
        assert_eq!(args.network, "testnet");
        assert!(args.keys.is_none());
        assert!(args.threshold.is_none());
        assert!(args.format.is_none());
        assert!(args.plan.is_none());
    }

    #[test]
    fn ttl_audit_all_flags_parse() {
        let cli = Cli::try_parse_from([
            "soroban-forge",
            "ttl-audit",
            "--contract",
            "CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB",
            "--network",
            "mainnet",
            "--keys",
            "keys.json",
            "--threshold",
            "50000",
            "--format",
            "json",
            "--plan",
            "extend.json",
            "--source",
            "alice",
        ])
        .unwrap();

        let Commands::TtlAudit(args) = cli.command else {
            panic!("expected ttl-audit command");
        };
        assert_eq!(args.network, "mainnet");
        assert_eq!(
            args.keys.as_deref(),
            Some(std::path::Path::new("keys.json"))
        );
        assert_eq!(args.threshold, Some(50_000));
        assert_eq!(args.format.as_deref(), Some("json"));
        assert_eq!(
            args.plan.as_deref(),
            Some(std::path::Path::new("extend.json"))
        );
        assert_eq!(args.source.as_deref(), Some("alice"));
    }

    #[test]
    fn ttl_audit_invalid_format_rejected() {
        let result = Cli::try_parse_from([
            "soroban-forge",
            "ttl-audit",
            "--contract",
            "CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB",
            "--format",
            "xml",
        ]);
        assert!(result.is_err(), "xml format should be rejected by clap");
    }

    #[test]
    fn ttl_audit_missing_contract_rejected() {
        let result = Cli::try_parse_from(["soroban-forge", "ttl-audit"]);
        assert!(result.is_err(), "missing --contract should be rejected");
    }

    #[test]
    fn ttl_audit_help_documents_flags() {
        let mut command = Cli::command();
        let help = command
            .find_subcommand_mut("ttl-audit")
            .expect("ttl-audit subcommand must exist")
            .render_long_help()
            .to_string();
        assert!(help.contains("--contract"));
        assert!(help.contains("--network"));
        assert!(help.contains("--keys"));
        assert!(help.contains("--threshold"));
        assert!(help.contains("--format"));
        assert!(help.contains("--plan"));
        assert!(help.contains("--source"));
    }

    // ── Plan CLI tests ───────────────────────────────────────────────────────

    #[test]
    fn plan_required_positional_arg() {
        let cli = Cli::try_parse_from(["soroban-forge", "plan", "deploy.json"]).unwrap();
        let Commands::Plan(args) = cli.command else {
            panic!("expected plan command");
        };
        assert_eq!(args.plan, std::path::PathBuf::from("deploy.json"));
        assert!(!args.dry_run);
        assert!(!args.force);
        assert!(args.receipt.is_none());
    }

    #[test]
    fn plan_all_flags_parse() {
        let cli = Cli::try_parse_from([
            "soroban-forge",
            "plan",
            "deploy.json",
            "--dry-run",
            "--force",
            "--receipt",
            "receipt.json",
        ])
        .unwrap();
        let Commands::Plan(args) = cli.command else {
            panic!("expected plan command");
        };
        assert!(args.dry_run);
        assert!(args.force);
        assert_eq!(
            args.receipt.as_deref(),
            Some(std::path::Path::new("receipt.json"))
        );
    }

    #[test]
    fn plan_missing_positional_rejected() {
        let result = Cli::try_parse_from(["soroban-forge", "plan"]);
        assert!(result.is_err(), "missing PLAN_FILE should be rejected");
    }
}
