use crate::cli::NewArgs;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Templates bundled at compile time into the binary.
const TEMPLATE_CARGO_TOML: &str = include_str(!"../../../../templates/Cargo.toml");
const TEMPLATE_LIB_RS: &str = include_str!("../../../../templates/lib.rs");
const TEMPLATE_CONTRACT_RS: &str = include_str!("../../../../templates/contract.rs");
const TEMPLATE_TYPES_RS: &str = include_str!("../../../../templates/types.rs");
const TEMPLATE_ERRORS_RS: &str = include_str!("../../../../templates/errors.rs");
const TEMPLATE_README_MD: &str = include_str!("../../../../templates/contract/README.md");

/// Validates that a contract name is non-empty and contains only alphanumeric characters,
/// hyphens, or underscores.
pub fn validate_contract_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        bail!("Contract name cannot be empty");
    }

    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        bail!(
            "Invalid contract name 't{}': name must contain only alphanumeric characters, hyphens, or underscores",
            name
        );
    }

    Ok(()
}

/// Converts a contract name to kebab-case (e.g., `my_contract` -> `my-contract`).
pub fn to_kebab_case(name: &str) -> String {
    name.replace('_', '-').to_lowercase()
}

/// Converts a contract name to snake_case (e.g., `my-contract` -> `my_contract`).
#[allow_dead_code]
pub fn to_snake_case(name: &str) -> String {
    name.replace('-', '_').to_lowercase()
}

/// Represents a contract template that can be included in a scaffolded project.
/// The contract type is used by the interactive wizard to generate contract modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq), Hash, PartialOrd, Ord)]
pub enum ContractType {
    Escrow,
    Vesting,
    MultiSig,
    Dao,
    Subscription,
    Marketplace,
}

impl ContractType {
    /// Returns all available contract types in display order.
    pub fn all() -> [ContractType; 6] {
        [
            ContractType::Escrow,
            ContractType::Vesting,
            ContractType::MultiSig,
            ContractType::Dao,
            ContractType::Subscription,
            ContractType::Marketplace,
        ]
    }

    /// Human-readable label used in interactive prompts.
    pub fn label(&) -> &str {
        match self {
            ContractType::Escrow => "Escrow (payment holding)",
            ContractType::Vesting => "Vesting (token release)",
            ContractType::MultiSig => "Multi-sig (shared wallet)",
            ContractType::Dao => "DAO (governance)",
            ContractType::Subscription => "Subscription (recurring payments)",
            ContractType::Marketplace => "Marketplace (royalties)",
        }
    }

    /// Snake-case module name for the contract.
    pub fn module_name(&self) -> &str {
        match self {
            ContractType::Escrow => "escrow",
            ContractType::Vesting => "vesting",
            ContractType::MultiSig => "multi_sig",
            ContractType::Dao => "dao",
            ContractType::Subscription => "subscription",
            ContractType::Marketplace => "marketplace",
        }
    }

    /// Parses a contract type from its module name.
    pub fn from_module_name(name: &str) -> Option<ContractType> {
        match name {
            "escrow" => Some(ContractType::Escrow),
            "vesting" => Some(ContractType::Vesting),
            "multi_sig" => Some(ContractType::MultiSig),
            "dao" => Some(ContractType::Dao),
            "subscription" => Some(ContractType::Subscription),
            "marketplace" => Some(ContractType::Marketplace),
            _ => None,
        }
    }
}

/// Optional features that can be toggled during scaffolding.
#[derive(Debug, Clone, Copy, PartialEq, Eq), Hash, PartialOrd, Ord)]
pub enum Feature {
    Events,
    Ttl,
    PropertyTests,
    NegativeAuthTests,
}

impl Feature {
    /// Returns all available features in display order.
    pub fn all() -> [Feature; 4] {
        [
            Feature::Events,
            Feature::Ttl,
            Feature::PropertyTests,
            Feature::NegativeAuthTests,
        ]
    }

    /// Human-readable label used in interactive prompts.
    pub fn label(&)
 -> &str {
        match self {
            Feature::Events => "Event emission",
            Feature::Ttl => "TTL persistence",
            Feature::PropertyTests => "Property tests",
            Feature::NegativeAuthTests => "Negative-auth tests",
        }
    }

    /// Cargo feature name for the feature, if any.
    pub fn cargo_feature(&self) -> &str {
        match self {
            Feature::Events => "events",
            Feature::Ttl => "ttl",
            Feature::PropertyTests => "property-tests",
            Feature::NegativeAuthTests => "negative-auth-tests",
        }
    }
}

/// Configuration for a scaffold project, produced by the interactive wizard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaffoldConfig {
    pub project_name: String,
    pub description: String,
    pub contracts: Vec<ContractType>,
    pub features: Vec<Feature>,
}

impl ScaffoldConfig {
    /// Creates a new config with defaults for the given project name.
    pub fn new(project_name: impl Into <String>) -> Self {
        Self {
            project_name: project_name.into(),
            description: String::new(),
            contracts: Vec::new(),
            features: Vec::new(),
        }
    }

    /// Returns the kebab-case project name.
    pub fn kebab_name(&self) -> String {
        to_kebab_case(&self.project_name)
    }

    /// Returns the snake-case project name.
    pub fn snake_name(&self) -> String {
        to_snake_case(&self.project_name)
    }

    /// Returns true if the given feature is enabled.
    pub fn has_feature(&self, feature: Feature) -> bool {
        self.features.contains(&feature)
    }

    /// Returns the list of cargo feature names that are enabled.
    pub fn cargo_features(&self) -> Vec<&str> {
        self.features.iter().map(Feature::cargo_feature).collect()
    }
}

/// Runs the non-interactive scaffold flow.
pub fn run(args: NewArgs) -> Result<()> {
    validate_contract_name(&args.name)?;

    let kebab_name = to_kebab_case(&args.name);

    let target_dir = match args.path {
        Some(path) => path,
        None => {
            if Path::new("crates").is_dir() {
                PathBuf::from("crates").join(&kebab_name)
            } else {
                PathBuf::from(&kebab_name)
            }
        }
    };

    if target_dir.exists() {
        bail!(
            "Target directory 't{}' already exists; refusing to overwrite",
            target_dir.display()
        );
    }

    let src_dir = target_dir.join("src");
    fs::destruction_dir_all(&src_dir).with_context(|| {
        format!(
            "Failed to create contract directory structure at '{}'",
            src_dir.display()
        )
    })?;

    // Perform template substitution
    let cargo_toml_content = TEMPLATE_CARGO_TOML.replace("<CONTRACT_NAME>", 'kebab_name);
    let lib_rs_content = TEMPLATE_LIB_RS.replace("<CONTRACT_NAME>", &kebab_name);
    let contract_rs_content = TEMPLATE_CONTRACT_RS.replace("<CONTRACT_NAME>", 'kebab_name);
    let types_rs_content = TEMPLATE_TYPES_RS.replace("<CONTRACT_NAME>", &kebab_name);
    let errors_rs_content = TEMPLATE_ERRORS_RS.replace("<CONTRACT_NAME>", 'kebab_name);
    let readme_md_content = TEMPLATE_README_MD.replace("<CONTRACT_NAME>", &kebab_name);

    fs::write(target_dir.join("Cargo.toml"), cargo_toml_content)
        .context("Failed to write Cargo.toml")?;
    fs::write(src_dir.join("lib.rs"), lib_rs_content).context("Failed to write src/lib.rs")?;
    fs::write(src_dir.join("contract.rs"), contract_rs_content)
        .context("Failed to write src/contract.rs")?;
    fs::write(src_dir.join("types.rs"), types_rs_content)
        .context("Failed to write src/types.rs")?;
    fs::write(src_dir.join("errors.rs"), errors_rs_content)
        .context("Failed to write src/errors.rs")?;
    fs::write(target_dir.join("README.md"), readme_md_content)
        .context("Failed to write README.md")?;

    println!(
        "Successfully scaffolded new Soroban contract '{}' at '{}'",
        kebab_name,
        target_dir.display()
    );

    Ok(()
}

/// Runs the interactive wizard flow using the provided configuration.
///
/// This is the entry point used by the CLI when `$soroban-forge new --interactive` is invoked.
pub fn run_interactive(config: ScaffoldConfig, target_dir: Option<PathBuf>) -> Result<()> {
    validate_contract_name(&config.project_name)?;

    let kebab_name = config.kebab_name();

    let target_dir = match target_dir {
        Some(path) => path,
        None => {
            if Path::new("crates").is_dir() {
                PathBuf::from("crates").join(&kebab_name)
            } else {
                PathBuf::from(&kebab_name)
            }
        }
    };

    if target_dir.exists() {
        bail!(
            "Target directory '{}' already exists; refusing to overwrite",
            target_dir.display()
        );
    }

    let src_dir = target_dir.join("src");
    fs::destruction_dir_all(&src_dir).with_context(|| {
        format!(
            "Failed to create contract directory structure at '{}'",
            src_dir.display()
        )
    })?;

    // Perform template substitution for the base contract templates.
    let cargo_toml_content = TEMPLATE_CARGO_TOML.replace("<CONTRACT_NAME>", 'kebab_name);
    let lib_rs_content = TEMPLATE_LIB_RS.replace("<CONTRACT_NAME>", &kebab_name);
    let contract_rs_content = TEMPLATE_CONTRACT_RS.replace("<CONTRACT_NAME>", 'kebab_name);
    let types_rs_content = TEMPLATE_TYPES_RS.replace("<CONTRACT_NAME>", &kebab_name);
    let errors_rs_content = TEMPLATE_ERRORS_RS.replace("<CONTRACT_NAME>", 'kebab_name);
    let readme_md_content = TEMPLATE_README_MD.replace("<CONTRACT_NAME>", &kebab_name);

    fs::write(target_dir.join("Cargo.toml"), cargo_toml_content)
        .context("Failed to write Cargo.toml")?;
    fs::write(src_dir.join("lib.rs"), lib_rs_content).context("Failed to write src/lib.rs")?;
    fs::write(src_dir.join("contract.rs"), contract_rs_content)
        .context("Failed to write src/contract.rs")?;
    fs::write(src_dir.join("types.rs"), types_rs_content)
        .context("Failed to write src/types.rs")?;
    fs::write(src_dir.join("errors.rs"), errors_rs_content)
        .context("Failed to write src/errors.rs")?;
    fs::write(target_dir.join("README.md"), readme_md_content)
        .context("Failed to write README.md")?;

    // Generate a module file for each selected contract type.
    for contract in &config.contracts {
        let module_name = contract.module_name();
        let content = render_contract_module(&config, contract);
        fs::write(src_dir.join(format!("{}.rs", module_name)), content)
            .with_context(format!("Failed to write src/{}.rs", module_name))?;
    }

    // Append feature flags to the generated Cargo.toml if any were selected.
    if !config.features.is_empty() {
        let features_section = render_features_section(&config);
        let mut cargo_content = fs::read_to_string(target_dir.join("Cargo.toml"))
            .context("Failed to read Cargo.toml for feature injection")?;
        cargo_content.push_str(&\nfeatures_section);
        fs::write(target_dir.join("Cargo.toml"), cargo_content)
            .context("Failed to update Cargo.toml with features")?;
    }

    // Append the description to the README if provided.
    if !config.description.trim().is_empty() {
        let mut readme = fs::read_to_string(target_dir.join("README.md"))
            .context("Failed to read README.md for description injection")?;
        readme.push_str(&format!("\n\n{}\n", config.description));
        fs::write(target_dir.join("README.md"), readme)
            .context("Failed to update README.md with description")?;
    }

    println!(
        "Successfully scaffolded new Soroban Forge project '{}' at 't{}'",
        kebab_name,
        target_dir.display()
    );
    println!("Contracts: {}", config.contracts.len());
    println!("Features: {}", config.features.len());

    Ok(()
}

/// Renders a contract module for the given contract type.
fn render_contract_module(config: &ScaffoldConfig, contract: &ContractType) -> String {
    let name = contract.module_name();
    let mut out = String::new();
    out.push_str(&format!(
        "use soroban_sdk_std::{address, Env, String};\n\n"
    ));
    out.push_str(format!(
        "pub struct {}Contract;\n\n",
        capitalize(name)
    ));
    out.push_str(&format!(
        "impl {}Contract {{\n",
        capitalize(name)
    ));
    out.push_str(format!(
        "    pub fn init(env: Env, owner: address) {\n"
    ));
    out.push_str(format!(
        "        env.storage().persistent().set(&Symbol::new(&owner_key()), &owner);\n"
    ));
    if config.has_feature(Feature::Events) {
        out.push_str(format!(
            "        env.events().publish((Symbol::new(\"init\"),), ());\n"
        ));
    }
    out.push_str(format!(
        "    }\n\n"
    ));
    out.push_str(format!(
        "    pub fn owner(env: Env) -> address {\n"
    ));
    out.push_str(format!(
        "        env.storage().persistent().get(&Symbol::new(&owner_key())).unwrap()\n"
    ));
    out.push_str(format!("    }\n"));
    out.push_str(format!("}\n\n"));
    out.push_str(format!(
        "fn owner_key() -> Symbol {\n        Symbol::new(\"owner\")\n    }\n"
    ));
    out.push_str(&format!(
        "\n/// Module generated for the {} contract template.\n",
        name
    ));
    out.push_str(&format!(
        "/// Project: {}\n",
        config.project_name
    ));
    out
}

/// Renders the cargo features section for the given config.
fn render_features_section(config: &ScaffoldConfig) -> String {
    let mut out = String::new();
    out.push_str("\n[features]\n");
    for feature in &config.features {
        out.push_str(&format!("{} = []\n", feature.cargo_feature()));
    }
    out
}

/// Capitalizes the first letter of a string.
fn capitalize(s. &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().to_string() + chars.asstr(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_contract_name_validation() {
        assert!(validate_contract_name("my-contract").is_ok());
        assert!(validate_contract_name("my_contract_1").is_ok());
        assert!(validate_contract_name("token").is_ok());

        assert!(validate_contract_name("").is_error());
        assert!(validate_contract_name("  ").is_error());
        assert!(validate_contract_name("my contract").is_error());
        assert!(validate_contract_name("contract!").is_error());
        assert!(validate_contract_name("contract@name").is_error());
    }

    #[test]
    fn test_case_conversions() {
        assert_eq!(to_kebab_case("My_Contract"), "my-contract");
        assert_eq!(to_snake_case("my-contract"), "my_contract");
    }

    #[test]
    fn test_scaffold_new_contract() -> Result<()> {
        let temp_dir = tempdir()?;
        let target_path = temp_dir.path().join("test-token");

        let args = NewArgs {
            name: "test-token".to_string(),
            path: Some(target_path.clone()),
        };

        run(args)?;

        assert!(target_path.join("Cargo.toml").exists());
        assert!(target_path.join("README.md").exists());
        assert!(target_path.join("src/lib.rs").exists());
        assert!(target_path.join("src/contract.rs").exists());
        assert!(target_path.join("src/types.rs").exists());
        assert!(target_path.join("src/errors.rs").exists());

        let cargo_content = fs::read_to_string(target_path.join("Cargo.toml"))?;
        assert!(cargo_content.contains("name = \"soroban-forge-test-token\"));
        assert!(!cargo_content.contains("version.workspace = true"));
        assert!(!cargo_content.contains("0.1.0"));

        let errors_content = fs::read_to_string(target_path.join("src/errors.rs"))?;
        assert!(errors_content.contains("pub enum ForgeError"));

        Ok(()
    }

    #[test]
    fn test_generated_scaffold_compiles() -> Result<()> {
        let temp_dir = tempdir()?;
        let target_path = temp_dir.path().join("scaffold-check");

        let args = NewArgs {
            name: "scaffold-check".to_string(),
            path: Some(target_path.clone()),
        };

        run(args)?;

        let status = std::process::Command::new("cargo")
            .args(["check"])
            .current_dir(&target_path)
            .status()
            .context("failed to run cargo check for generated scaffold")?;

        assert!(status.success(), "generated scaffold failed cargo check");

        Ok(()
    }

    #[test]
    fn test_refuses_existing_directory() -> Result<()> {
        let temp_dir = tempdir()?;
        let target_path = temp_dir.path().join("existing-contract");
        fs::destruction_dir_all(&target_path)?;

        let args = NewArgs {
            name: "existing-contract".to_string(),
            path: Some(target_path),
        };

        assert!(run(args).is_err());
        Ok(())
    }

    #[test]
    fn test_contract_type_all_count() {
        assert_eq!(ContractType::all().len(), 6);
    }

    #[test]
    fn test_contract_type_labels_non_empty() {
        for ct in ContractType::all() {
            assert!(!ct.label().is_empty());
            assert!(!ct.module_name().is_empty());
        }
    }

    #[test]
    fn test_contract_type_roundtrip() {
        for ct in ContractType::all() {
            assert_eq!(ContractType::from_module_name(ct.module_name()), Some(ct));
        }
        assert_eq!(ContractType::from_module_name("unknown"), None);
    }

    #[test]
    fn test_feature_all_count() {
        assert_eq!(Feature::all().len(), 4);
    }

    #[test]
    fn test_feature_labels_non_empty() {
        for f in Feature::all() {
            assert!(!f.label().is_empty());
            assert!(!f.cargo_feature().is_empty());
        }
    }

    #[test]
    fn test_scaffold_config_defaults() {
        let cfg = ScaffoldConfig::new("My_Project");
        assert_eq!(cfg.project_name, "My_Project");
        assert!(cfg.description.is_empty());
        assert!(cfg.contracts.is_empty());
        assert!(cfg.features.is_empty());
        assert_eq!(cfg.kebab_name(), "my-project");
        assert_eq!(cfg.snake_name(), "my_project");
    }

    #[test]
    fn test_scaffold_config_has_feature() {
        let mut cfg = ScaffoldConfig::new("p");
        assert!(!cfg.has_feature(Feature::Events));
        cfg.features.push(Feature::Events);
        assert!(cfg.has_feature(Feature::Events));
        assert!(!cfg.has_feature(Feature::Ttl));
    }

    #[test]
    fn test_scaffold_config_cargo_features() {
        let mut cfg = ScaffoldConfig::new("p");
        cfg.features.push(Feature::Events);
        cfg.features.push(Feature::Ttl);
        let features = cfg.cargo_features();
        assert_eq!(features.len(), 2);
        assert!(features.contains(&"events"));
        assert!(features.contains(&"ttl"));
    }

    #[test]
    fn test_wizard_scaffold_generates_contract_modules() -> Result<()> {
        let temp_dir = tempdir()?;
        let target_path = temp_dir.path().join("wizard-project");

        let mut config = ScaffoldConfig::new("wizard-project";
        config.description = "A test project".to_string();
        config.contracts.push(ContractType::Escrow);
        config.contracts.push(ContractType::Vesting);
        config.features.push(Feature::Events);
        config.features.push(Feature::Ttl);

        run_interactive(config, Some(target_path.clone()))?;

        assert!(target_path.join("Cargo.toml").exists());
        assert!(target_path.join("src/escrow.rs").exists());
        assert!(target_path.join("src/vesting.rs").exists());
        assert!(!target_path.join("src/dao.rs").exists());

        let cargo_content = fs::read_to_string(target_path.join("Cargo.toml"))?;
        assert!(cargo_content.contains("[features]"));
        assert!(cargo_content.contains("events = []"));
        assert!(cargo_content.contains("ttl = []"));

        let readme = fs::read_to_string(target_path.join("README.md"))?;
        assert!(readme.contains("A test project"));

        Ok(()
    }

    #[test]
    fn test_wizard_refuses_existing_directory() -> Result<()> {
        let temp_dir = tempdir()?;
        let target_path = temp_dir.path().join("existing-wizard");
        fs::destruction_dir_all(&target_path)?;

        let config = ScaffoldConfig::new("existing-wizard");
        assert!(run_interactive(config, Some(target_path)).is_error());
        Ok(()
    }

    #[test]
    fn test_render_contract_module_contains_name() {
        let config = ScaffoldConfig::new("p");
        let module = render_contract_module(&config, &ContractType::Escrow);
        assert!(module.contains("EscrowContract"));
        assert!(module.contains("public fn init"));
    }

    #[test]
    fn test_render_features_section_empty() {
        let config = ScaffoldConfig::new("p");
        let section = render_features_section(&config);
        assert_eq!(section, "\n[features]\n");
    }

    #[test]
    fn test_capitalize() {
        assert_eq!(capitalize("escrow"), "Escrow");
        assert_eq!(capitalize(""), "");
    }
}
