use clap::Args;
use clap::ValueEnum;
use std::path::PathBuf;

#[derive(Args, Debug, Clone)]
pub struct BuildArgs {
    /// Crate to build; builds the whole workspace when omitted.
    #[arg(short, long)]
    pub package: Option<String>,
    #[arg(short, long, default_value_t = false)]
    pub release: bool,
    #[arg(short, long, default_value_t = false)]
    pub all_targets: bool,
    /// Build release WASM for the contract crates with `stellar contract build` (stellar-cli v25.2.0+), as CI does.
    #[arg(long, default_value_t = false)]
    pub wasm: bool,
    /// Check built WASM artifacts against the 150,000-byte contract budget (implies --wasm).
    #[arg(long, default_value_t = false)]
    pub check_size: bool,
}

#[derive(Args, Debug, Clone)]
pub struct LintArgs {
    #[arg(short, long, default_value_t = false)]
    pub fix: bool,
}

#[derive(Args, Debug, Clone)]
pub struct TestArgs {
    /// Crate to test; tests the whole workspace when omitted.
    #[arg(short, long)]
    pub package: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct DeployArgs {
    pub wasm: String,
    #[arg(short, long, default_value = "testnet")]
    pub network: String,
    #[arg(short, long)]
    pub source: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct NewArgs {
    /// Contract name (alphanumeric, hyphens, or underscores).
    pub name: String,
    /// Destination directory path for the new contract.
    #[arg(short, long)]
    pub path: Option<std::path::PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub struct ScaffoldValidateArgs {
    /// Path to a scaffolded contract directory; defaults to the current directory.
    #[arg(value_name = "PATH", default_value = ".")]
    pub path: PathBuf,
}

#[derive(Args, Debug, Clone)]
pub struct VerifyArgs {
    /// Path to a local WASM artifact to verify against a deterministic rebuild.
    #[arg(long)]
    pub wasm: Option<String>,
    /// Expected SHA-256 of the rebuilt artifact (hex, lowercase).
    #[arg(long)]
    pub expected: Option<String>,
    /// Provenance manifest to validate; defaults to `provenance-manifest.json`.
    #[arg(long, default_value = "provenance-manifest.json")]
    pub manifest: String,
    /// Print a JSON summary of manifest verification to stdout.
    #[arg(long, conflicts_with_all = ["wasm", "expected"])]
    pub json: bool,
    /// Crate to rebuild; inferred from the WASM file name when omitted.
    #[arg(short, long)]
    pub package: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct InvokeArgs {
    /// Contract address or alias
    #[arg(long)]
    pub contract: String,
    /// Function name to invoke
    #[arg(long)]
    pub function: String,
    /// Function arguments (key=value format)
    #[arg(long = "arg", action = clap::ArgAction::Append)]
    pub args: Vec<String>,
    /// Network to use
    #[arg(long, default_value = "testnet")]
    pub network: String,
    /// Source account
    #[arg(long)]
    pub source: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct EventsArgs {
    /// Contract address or alias
    #[arg(long)]
    pub contract: String,
    /// Starting ledger number
    #[arg(long)]
    pub since: Option<u64>,
    /// Event type filter
    #[arg(long = "type")]
    pub event_type: Option<String>,
    /// Event topic name filter
    #[arg(long)]
    pub topic: Option<String>,
    /// Network to use
    #[arg(long, default_value = "testnet")]
    pub network: String,
    /// Follow new events continuously (alias: --follow)
    #[arg(long, alias = "follow")]
    pub stream: bool,
    /// Output format
    #[arg(long, value_enum, default_value = "pretty")]
    pub format: EventsFormat,
    /// Emit one JSON object per event (deprecated; use --format jsonl)
    #[arg(long)]
    pub jsonl: bool,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EventsFormat {
    #[default]
    Pretty,
    Jsonl,
}

#[derive(Args, Debug, Clone)]
pub struct InspectArgs {
    /// Contract address or alias to inspect on-chain. Optional when using the local dry-run check.
    pub contract: Option<String>,
    /// Specific DataKey entries to inspect (comma-separated).
    #[arg(long, value_delimiter = ',')]
    pub keys: Vec<String>,
    /// Output as JSON for programmatic use.
    #[arg(long, default_value_t = false)]
    pub json: bool,
    /// Network to use.
    #[arg(long, default_value = "testnet")]
    pub network: String,
    /// Source account.
    #[arg(long)]
    pub source: Option<String>,
    /// Local WASM artifact to inspect without network access or signing.
    #[arg(long, value_name = "PATH")]
    pub wasm: Option<std::path::PathBuf>,
    /// Expected SHA-256 for a dry-run artifact comparison.
    #[arg(long)]
    pub expected: Option<String>,
    /// Perform a credential-free local dry-run artifact inspection.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DoctorArgs {
    /// Optional WASM artifact to validate as a regular, non-empty file.
    #[arg(long, value_name = "PATH")]
    pub wasm: Option<PathBuf>,
    /// Emit machine-readable JSON diagnostics.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct CompareArgs {
    /// Baseline/local WASM artifact.
    pub old: PathBuf,
    /// Candidate/new WASM artifact.
    pub new: PathBuf,
    /// Emit a machine-readable JSON report.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct StorageDiffArgs {
    /// Source WASM artifact (the currently deployed contract).
    #[arg(long, value_name = "PATH")]
    pub source: PathBuf,
    /// Target WASM artifact (the proposed replacement contract).
    #[arg(long, value_name = "PATH")]
    pub target: PathBuf,
    /// Emit a machine-readable JSON report; diagnostics go to stderr.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct ValidatePlanArgs {
    #[arg(
        value_name = "PLAN_PATH",
        help = "Path to a transaction-plan JSON or TOML file"
    )]
    pub file: Option<PathBuf>,
    #[arg(
        short = 'f',
        long,
        value_name = "PATH",
        help = "Path to a transaction-plan JSON or TOML file"
    )]
    pub plan: Option<PathBuf>,
    #[arg(long, help = "Expected target network used to detect mismatches")]
    pub expected_network: Option<String>,
    #[arg(long, help = "Network override for the plan being validated")]
    pub network: Option<String>,
    #[arg(long, help = "Contract strkey or workspace alias override")]
    pub contract: Option<String>,
    #[arg(
        long,
        visible_alias = "function",
        help = "Contract method name override"
    )]
    pub method: Option<String>,
    #[arg(long, help = "Source account override")]
    pub source: Option<String>,
    #[arg(
        long = "arg",
        action = clap::ArgAction::Append,
        help = "Typed argument as name=type:value"
    )]
    pub args: Vec<String>,
    #[arg(long, help = "Emit a machine-readable JSON validation report")]
    pub json: bool,
}
