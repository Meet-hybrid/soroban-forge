use clap::{Args, Subcommand};

#[derive(Args, Debug, Clone)]
pub struct BuildArgs {
    #[arg(short, long, default_value = "workspace")]
    pub target: String,
    #[arg(short, long, default_value = "false")]
    pub release: bool,
    #[arg(short, long, default_value = "false")]
    pub all_targets: bool,
}

#[derive(Args, Debug, Clone)]
pub struct LintArgs {
    #[arg(short, long, default_value = "false")]
    pub fix: bool,
}

#[derive(Args, Debug, Clone)]
pub struct TestArgs {
    #[arg(short, long)]
    pub package: Option<String>,
    #[arg(short, long, default_value = "workspace")]
    pub scope: String,
}

#[derive(Args, Debug, Clone)]
pub struct DeployArgs {
    pub wasm: String,
    #[arg(short, long, default_value = "testnet")]
    pub network: String,
    #[arg(short, long)]
    pub source: Option<String>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum CliCommand {
    Build(BuildArgs),
    Lint(LintArgs),
    Test(TestArgs),
    Deploy(DeployArgs),
}
