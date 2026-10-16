use clap::{Args, ValueEnum};
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
    pub name: String,
    /// Destination directory path for the new contract.
    #[arg(short, long)]
    pub path: Option<std::path::PathBuf>,
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

/// Output encoding format for `soroban-forge invoke` simulation and execution results.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InvokeOutputFormat {
    /// Human-readable decoded summary (default).
    #[default]
    Text,
    /// Structured JSON containing decoded ScVal JSON, type metadata, and base64 XDR.
    Json,
    /// Raw base64-encoded XDR of the returned ScVal.
    Xdr,
}

#[derive(Args, Debug, Clone)]
#[command(
    about = "Invoke or simulate a Soroban contract function with typed ScVal arguments",
    after_help = "TYPED ARGUMENT FORMS (--arg):\n  \
        Named typed:       <name>:<type>=<value>  or  <name>=<type>:<value>\n  \
        Positional typed:  <type>:<value>\n  \
        Simple (legacy):   <name>=<value>         or  <value>\n\n\
        SUPPORTED <type> TAGS:\n  \
        address (addr)     Stellar account (G...) or contract (C...) Strkey address\n  \
        i128, u128         Signed / unsigned 128-bit integer (e.g. amount:i128=1000)\n  \
        i64, u64           Signed / unsigned 64-bit integer (e.g. deadline:u64=1720003600)\n  \
        i32, u32           Signed / unsigned 32-bit integer (e.g. bps:u32=500)\n  \
        bool               Boolean literal: true or false\n  \
        symbol (sym)       Soroban ScSymbol ([a-zA-Z0-9_]{1,32})\n  \
        string (str)       UTF-8 ScString\n  \
        bytes (hex)        Hex-encoded byte slice, with or without 0x prefix\n  \
        void (unit)        ScVal::Void (e.g. unit:void=)\n  \
        vec                JSON array of ScVal elements (e.g. 'items:vec=[{\"u64\":1},{\"u64\":2}]')\n  \
        map                JSON object or array of {\"key\":...,\"val\":...} entries\n  \
        json (scval)       Canonical JSON ScVal object (e.g. 'arg:json={\"address\":\"G...\"}')\n  \
        xdr                Base64-encoded XDR ScVal\n\n\
        QUOTING & VALIDATION:\n  \
        - Wrap JSON arguments (vec, map, json) in single quotes on bash/zsh or escaped quotes in PowerShell.\n  \
        - All typed arguments, Strkey addresses, and optional --wasm/--spec contract specs are validated\n    \
          offline before any network request is made."
)]
pub struct InvokeArgs {
    /// Contract address (C... Strkey) or alias
    #[arg(long)]
    pub contract: String,
    /// Function name to invoke
    #[arg(long)]
    pub function: String,
    /// Function arguments: typed (<name>:<type>=<value>, <name>=<type>:<value>, <type>:<value>) or simple (<name>=<value>, <value>)
    #[arg(long = "arg", action = clap::ArgAction::Append)]
    pub args: Vec<String>,
    /// Optional compiled WASM or raw XDR spec file for offline contract-spec validation and error decoding
    #[arg(long, visible_alias = "spec", value_name = "PATH")]
    pub wasm: Option<PathBuf>,
    /// Only simulate the invocation (--send=no) without submitting a transaction
    #[arg(long, visible_alias = "simulate", default_value_t = false)]
    pub simulate_only: bool,
    /// Output format for decoded simulation/invocation result (text, json, xdr)
    #[arg(long, visible_alias = "format", value_enum, default_value_t = InvokeOutputFormat::Text)]
    pub output: InvokeOutputFormat,
    /// Shorthand for `--output json`
    #[arg(long, default_value_t = false)]
    pub json: bool,
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
    /// Contract address or alias.
    pub contract: String,
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
}

#[derive(Args, Debug, Clone)]
pub struct DoctorArgs {
    /// Optional WASM artifact to validate as a regular, non-empty file.
    #[arg(long, value_name = "PATH")]
    pub wasm: Option<PathBuf>,
}
