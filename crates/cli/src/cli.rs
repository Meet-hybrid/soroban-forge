use clap::Args;

#[derive(Args, Debug, Clone)]
pub struct BuildArgs {
    /// Crate to build; builds the whole workspace when omitted.
    #[arg(short, long)]
    pub package: Option<String>,
    #[arg(short, long, default_value_t = false)]
    pub release: bool,
    #[arg(short, long, default_value_t = false)]
    pub all_targets: bool,
    /// Build release WASM for Soroban contracts using wasm32v1-none.
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
    ///
    /// Optional when --interactive is used; the wizard will prompt for it.
    #[arg(required_unless = "interactive")]
    pub name: Option<String>,
    /// Destination directory path for the new contract.
    #[arg(short, long)]
    pub path: Option<std::path::PathBuf>,
    /// Run the interactive scaffold wizard.
    ///
    /// When enabled, prompts for the project name, description, contract
    /// templates to include, and optional features (events, TTL, testing).
    /// When omitted, the command behaves as a non-interactive scaffolder.
    #[arg(short, long, default_value_t = false)]
    pub interactive: bool,
    /// Bref description of the project (used in generated metadata).
    #[arg(long)]
    pub description: Option<String>,
    /// Contract templates to include (comma-separated); repeatable.
    ///
    /// Valid values: escrow, vesting, multisig, dao, subscription, marketplace.
    #[arg(long = "contract", value_delimiter = ',', action = clap::ArgAction::Append)]
    pub contracts: Vec<String>,
    /// Optional features to enable (comma-separated); repeatable.
    ///
    /// Valid values: events, ttl, property-tests, negative-auth-tests.
    #[arg(long = "feature", value_delimiter = ',', action = clap::ArgAction::Append)]
    pub features: Vec<String>,
    /// Overwrite existing files without prompting.
    #[arg(long, default_value_t = false)]
    pub force: bool,
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
    /// Network to use
    #[arg(long, default_value = "testnet")]
    pub network: String,
}

#[derive(Args, Debug, Clone)]
pub struct InspectArgs {
    /// Contract address or alias
    pub contract: String,
    /// Specific DataKey entries to inspect (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pub keys: Vec<String>,
    /// Output as JSON for programmatic use
    #[arg(long, default_value_t = false)]
    pub json: bool,
    /// Network to use
    #[arg(long, default_value = "testnet")]
    pub network: String,
    /// Source account
    #[arg(long)]
    pub source: Option<String>,
}
