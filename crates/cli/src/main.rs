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
//! soroban-forge storage-diff --source old.wasm --target new.wasm
//! ```

mod cli;
mod commands;
#[path = "commands/decode_failure.rs"]
mod decode_failure;

use clap::{Parser, Subcommand};
use cli::{
    BuildArgs, CompareArgs, DeployArgs, DoctorArgs, EventsArgs, InspectArgs, InvokeArgs, LintArgs,
    NewArgs, ScaffoldValidateArgs, StorageDiffArgs, TestArgs, ValidatePlanArgs, VerifyArgs,
};
use decode_failure::DecodeFailureArgs;

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
    DecodeFailure(DecodeFailureArgs),
    Events(EventsArgs),
    Inspect(InspectArgs),
    Doctor(DoctorArgs),
    #[command(
        about = "Validate a transaction plan and emit a structured report",
        visible_alias = "plan"
    )]
    ValidatePlan(ValidatePlanArgs),
    Compare(CompareArgs),
    StorageDiff(StorageDiffArgs),
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
        Commands::DecodeFailure(args) => decode_failure::run(args)?,
        Commands::Events(args) => commands::events::run(args)?,
        Commands::Inspect(args) => commands::inspect::run(args)?,
        Commands::Doctor(args) => commands::doctor::run(args)?,
        Commands::ValidatePlan(args) => commands::plan::run(args)?,
        Commands::Compare(args) => commands::compare::run(args)?,
        Commands::StorageDiff(args) => commands::storage_diff::run(args)?,
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
    fn events_follow_alias_topic_and_format_flags_parse() {
        let cli = Cli::try_parse_from([
            "soroban-forge",
            "events",
            "--contract",
            "C123",
            "--follow",
            "--topic",
            "Transfer",
            "--format",
            "jsonl",
        ])
        .unwrap();

        let Commands::Events(args) = cli.command else {
            panic!("expected events command");
        };
        assert!(args.stream);
        assert_eq!(args.topic.as_deref(), Some("Transfer"));
        assert_eq!(args.format, crate::cli::EventsFormat::Jsonl);
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
    fn decode_failure_accepts_receipt_xdr_and_optional_diagnostics() {
        let cli = Cli::try_parse_from([
            "soroban-forge",
            "decode-failure",
            "--result-xdr",
            "AAAA",
            "--diagnostic-events-xdr",
            "BBBB",
        ])
        .unwrap();

        let Commands::DecodeFailure(args) = cli.command else {
            panic!("expected decode-failure command");
        };
        assert_eq!(args.result_xdr.as_deref(), Some("AAAA"));
        assert_eq!(args.diagnostic_events_xdr.as_deref(), Some("BBBB"));
        assert!(args.error.is_none());
    }

    #[test]
    fn decode_failure_requires_exactly_one_input() {
        assert!(Cli::try_parse_from(["soroban-forge", "decode-failure"]).is_err());
        assert!(Cli::try_parse_from([
            "soroban-forge",
            "decode-failure",
            "--error",
            "failed",
            "--result-xdr",
            "AAAA",
        ])
        .is_err());
    }

    #[test]
    fn verify_json_is_a_manifest_output_mode() {
        let cli = Cli::try_parse_from([
            "soroban-forge",
            "verify",
            "--manifest",
            "local.json",
            "--json",
        ])
        .unwrap();
        let Commands::Verify(args) = cli.command else {
            panic!("expected verify command");
        };
        assert!(args.json);
        assert_eq!(args.manifest, "local.json");
        for option in ["--wasm", "--expected"] {
            assert!(
                Cli::try_parse_from(["soroban-forge", "verify", "--json", option, "value"])
                    .is_err()
            );
        }
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
        assert!(help.contains("--json"));
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

    #[test]
    fn validate_plan_flags_and_alias_parse() {
        let cli = Cli::try_parse_from([
            "soroban-forge",
            "validate-plan",
            "--plan",
            "plan.json",
            "--expected-network",
            "testnet",
            "--json",
        ])
        .unwrap();
        let Commands::ValidatePlan(args) = cli.command else {
            panic!("expected validate-plan command");
        };
        assert_eq!(
            args.plan.as_deref(),
            Some(std::path::Path::new("plan.json"))
        );
        assert_eq!(args.expected_network.as_deref(), Some("testnet"));
        assert!(args.json);

        let aliased = Cli::try_parse_from(["soroban-forge", "plan", "plan.json"]).unwrap();
        assert!(matches!(aliased.command, Commands::ValidatePlan(_)));
    }

    #[test]
    fn inspect_dry_run_flags_parse() {
        let cli = Cli::try_parse_from([
            "soroban-forge",
            "inspect",
            "--dry-run",
            "--wasm",
            "target/wasm32v1-none/release/soroban_forge_escrow.wasm",
            "--expected",
            &"a".repeat(64),
        ])
        .unwrap();

        let Commands::Inspect(args) = cli.command else {
            panic!("expected inspect command");
        };
        assert!(args.dry_run);
        assert_eq!(
            args.wasm.as_deref(),
            Some(std::path::Path::new(
                "target/wasm32v1-none/release/soroban_forge_escrow.wasm"
            ))
        );
        assert_eq!(args.expected.as_deref(), Some("a".repeat(64).as_str()));
    }

    #[test]
    fn storage_diff_flags_parse() {
        let cli = Cli::try_parse_from([
            "soroban-forge",
            "storage-diff",
            "--source",
            "old.wasm",
            "--target",
            "new.wasm",
            "--json",
        ])
        .unwrap();

        let Commands::StorageDiff(args) = cli.command else {
            panic!("expected storage-diff command");
        };
        assert_eq!(args.source, std::path::Path::new("old.wasm"));
        assert_eq!(args.target, std::path::Path::new("new.wasm"));
        assert!(args.json);
    }

    #[test]
    fn storage_diff_requires_source_and_target() {
        assert!(Cli::try_parse_from(["soroban-forge", "storage-diff"]).is_err());
        assert!(
            Cli::try_parse_from(["soroban-forge", "storage-diff", "--source", "old.wasm",])
                .is_err()
        );
        assert!(
            Cli::try_parse_from(["soroban-forge", "storage-diff", "--target", "new.wasm",])
                .is_err()
        );
    }

    #[test]
    fn inspect_contract_json_flags_parse() {
        let cli = Cli::try_parse_from([
            "soroban-forge",
            "inspect",
            "CA3D5KRYM6CB7OWQ6TWYRR3Z4T7GNZLKERYNZGGA5SOAOPIFY6YQGAXE",
            "--json",
            "--network",
            "testnet",
        ])
        .unwrap();

        let Commands::Inspect(args) = cli.command else {
            panic!("expected inspect command");
        };
        assert_eq!(
            args.contract.as_deref(),
            Some("CA3D5KRYM6CB7OWQ6TWYRR3Z4T7GNZLKERYNZGGA5SOAOPIFY6YQGAXE")
        );
        assert!(args.json);
        assert_eq!(args.network, "testnet");
    }
}
