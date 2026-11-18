use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

use crate::cli::InspectArgs;
use crate::commands::build::CONTRACT_PACKAGES;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InspectEntry {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct ArtifactInspection {
    artifact: String,
    package: Option<String>,
    size_bytes: u64,
    sha256: String,
    expected_sha256: Option<String>,
    status: String,
    dry_run: bool,
}

pub fn run(args: InspectArgs) -> Result<()> {
    if args.dry_run || args.wasm.is_some() || args.expected.is_some() {
        return run_dry_run(&args);
    }

    let contract = args
        .contract
        .as_deref()
        .context("a contract id or local artifact is required; use --wasm with --dry-run for the credential-free check")?;
    let output = fetch_state(&args, contract)?;
    let entries = parse_entries(&output)?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        print_table(&entries);
    }

    Ok(())
}

fn run_dry_run(args: &InspectArgs) -> Result<()> {
    let wasm_path = args
        .wasm
        .as_ref()
        .context("dry-run inspection requires --wasm <PATH>")?;
    if !wasm_path.exists() {
        bail!("WASM artifact not found at {}", wasm_path.display());
    }
    if !wasm_path.is_file() {
        bail!("WASM artifact path is not a file: {}", wasm_path.display());
    }

    let expected = match args.expected.as_deref() {
        Some(value) => Some(expect_valid_sha256(value)?),
        None => None,
    };
    let actual_hash = sha256_hex_file(wasm_path)?;
    let file_size = fs::metadata(wasm_path)
        .with_context(|| format!("failed to read metadata for {}", wasm_path.display()))?
        .len();
    let package = infer_package_from_wasm_name(wasm_path)
        .ok()
        .map(|pkg| pkg.to_string());
    let status = match expected.as_deref() {
        Some(expected_hash) if actual_hash.eq_ignore_ascii_case(expected_hash) => "MATCH",
        Some(_) => "MISMATCH",
        None => "OK",
    };

    let result = ArtifactInspection {
        artifact: wasm_path.display().to_string(),
        package,
        size_bytes: file_size,
        sha256: actual_hash.clone(),
        expected_sha256: expected.clone(),
        status: status.to_owned(),
        dry_run: true,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("Dry-run artifact inspection");
        println!("  artifact: {}", result.artifact);
        println!(
            "  package: {}",
            result.package.as_deref().unwrap_or("unknown")
        );
        println!("  size_bytes: {}", result.size_bytes);
        println!("  sha256: {}", result.sha256);
        if let Some(expected_hash) = &result.expected_sha256 {
            println!("  expected_sha256: {expected_hash}");
            println!("  status: {status}");
        } else {
            println!("  status: {status}");
        }
    }

    if matches!(expected.as_deref(), Some(expected_hash) if !actual_hash.eq_ignore_ascii_case(expected_hash))
    {
        bail!("dry-run inspection failed: local artifact hash does not match expected SHA-256");
    }

    Ok(())
}

fn fetch_state(args: &InspectArgs, contract: &str) -> Result<String> {
    let mut command = std::process::Command::new("stellar");
    command
        .arg("contract")
        .arg("invoke")
        .arg("--id")
        .arg(contract)
        .arg("--network")
        .arg(&args.network)
        .arg("--")
        .arg("inspect")
        .arg("--json");

    if let Some(source) = &args.source {
        command.arg("--source").arg(source);
    }
    if !args.keys.is_empty() {
        command.arg("--keys").arg(args.keys.join(","));
    }

    let output = command
        .output()
        .context("failed to invoke the stellar CLI")?;
    if !output.status.success() {
        anyhow::bail!(
            "stellar contract inspect failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_entries(raw: &str) -> Result<Vec<InspectEntry>> {
    let value: Value = serde_json::from_str(raw).context("failed to parse inspect output")?;
    let object = value
        .as_object()
        .context("inspect output must be a JSON object")?;

    Ok(object
        .iter()
        .map(|(key, value)| InspectEntry {
            key: key.clone(),
            value: value.to_string(),
        })
        .collect())
}

fn print_table(entries: &[InspectEntry]) {
    if entries.is_empty() {
        println!("No contract state found.");
        return;
    }

    let key_width = entries
        .iter()
        .map(|entry| entry.key.len())
        .max()
        .unwrap_or(3);
    println!("{:<key_width$} | VALUE", "KEY");
    println!("{}-+-{}", "-".repeat(key_width), "-".repeat(20));
    for entry in entries {
        println!("{:<key_width$} | {}", entry.key, entry.value);
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256_hex_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(sha256_hex(&bytes))
}

fn expect_valid_sha256(input: &str) -> Result<String> {
    let input = input.trim();
    if input.len() != 64 || !input.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!(
            "invalid SHA-256 `{input}`: expected exactly 64 hex characters (as printed by sha256sum)"
        );
    }
    Ok(input.to_ascii_lowercase())
}

fn infer_package_from_wasm_name(path: &Path) -> Result<String> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .context("WASM artifact path has no file name")?;
    let package = stem.replace('_', "-");
    if !CONTRACT_PACKAGES.contains(&package.as_str()) {
        bail!(
            "cannot determine a Soroban Forge contract package from `{stem}`; pass --package explicitly"
        );
    }
    Ok(package)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "soroban-forge-inspect-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn dry_run_rejects_invalid_sha256() {
        let dir = temp_dir();
        fs::create_dir_all(&dir).unwrap();
        let wasm = dir.join("soroban_forge_escrow.wasm");
        fs::write(&wasm, b"artifact").unwrap();

        let err = run_dry_run(&InspectArgs {
            contract: None,
            keys: vec![],
            json: false,
            network: "testnet".to_string(),
            source: None,
            wasm: Some(wasm.clone()),
            expected: Some("not-a-hash".to_string()),
            dry_run: true,
        })
        .unwrap_err();
        assert!(err.to_string().contains("invalid SHA-256"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dry_run_rejects_hash_mismatch() {
        let dir = temp_dir();
        fs::create_dir_all(&dir).unwrap();
        let wasm = dir.join("soroban_forge_escrow.wasm");
        fs::write(&wasm, b"artifact").unwrap();

        let err = run_dry_run(&InspectArgs {
            contract: None,
            keys: vec![],
            json: false,
            network: "testnet".to_string(),
            source: None,
            wasm: Some(wasm.clone()),
            expected: Some("a".repeat(64)),
            dry_run: true,
        })
        .unwrap_err();
        assert!(err.to_string().contains("does not match expected SHA-256"));
        fs::remove_dir_all(dir).unwrap();
    }
}
