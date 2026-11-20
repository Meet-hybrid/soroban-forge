use crate::cli::VerifyArgs;
use crate::commands::build::{self, CONTRACT_PACKAGES};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// A provenance manifest produced by `scripts/provenance.sh`
/// (schema `soroban-forge/provenance@1` or `@2`).
#[derive(Debug, Deserialize)]
struct Manifest {
    schema: String,
    git_commit: Option<String>,
    source_revision: Option<String>,
    generated_at: Option<String>,
    target: Option<String>,
    cli_version: Option<String>,
    lockfile_sha256: Option<String>,
    bundle_sha256: Option<String>,
    artifacts: Vec<ManifestArtifact>,
    #[serde(default)]
    deployments: Vec<ManifestDeployment>,
}

#[derive(Debug, Deserialize)]
struct ManifestArtifact {
    name: String,
    sha256: String,
    bytes: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ManifestDeployment {
    network: Option<String>,
    contract_id: Option<String>,
    contract_name: Option<String>,
    artifact: Option<String>,
    artifact_sha256: Option<String>,
}

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
enum VerifyStatus {
    Match,
    Mismatch,
}

pub fn run(args: VerifyArgs) -> Result<()> {
    match (args.wasm.as_deref(), args.expected.as_deref()) {
        (Some(wasm), expected) => verify_wasm(wasm, expected, args.package.as_deref()),
        (None, Some(expected)) => verify_expected(expected, args.package.as_deref()),
        (None, None) => verify_manifest(&args.manifest, args.json),
    }
}

/// `--wasm <PATH>` (optionally with `--expected`): rebuild the contract that
/// matches the artifact's file name and compare hashes. The supplied artifact
/// is never overwritten; it is only read and hashed.
fn verify_wasm(wasm_path: &str, expected: Option<&str>, package: Option<&str>) -> Result<()> {
    let input_path = PathBuf::from(wasm_path);
    if !input_path.exists() {
        bail!("WASM artifact not found at {}", input_path.display());
    }
    let package = match package {
        Some(package) => package.to_owned(),
        None => infer_package_from_wasm_name(&input_path)?,
    };

    let rebuilt_path = rebuild_wasm(&package)?;
    let input_hash = sha256_hex_file(&input_path)?;
    let rebuilt_hash = sha256_hex_file(&rebuilt_path)?;

    let expected_hash = expected.map(expect_valid_sha256).transpose()?;
    let status = compare(&input_hash, &rebuilt_hash, expected_hash.as_deref());
    print_row(
        &package,
        "Input SHA256",
        &input_hash,
        "Rebuilt SHA256",
        &rebuilt_hash,
        status,
    );
    finish(status, &package)
}

/// `--expected <SHA256>` alone: rebuild the selected contract (or every
/// contract package when none is selected) and compare each rebuilt hash
/// against the supplied hash. No original WASM file is required.
fn verify_expected(expected: &str, package: Option<&str>) -> Result<()> {
    let expected_hash = expect_valid_sha256(expected)?;
    let packages: Vec<&str> = match package {
        Some(package) => {
            if !CONTRACT_PACKAGES.contains(&package) {
                bail!(
                    "`{package}` is not a known Soroban Forge contract package; expected one of: {}",
                    CONTRACT_PACKAGES.join(", ")
                );
            }
            vec![package]
        }
        None => CONTRACT_PACKAGES.to_vec(),
    };

    build::ensure_wasm_target_installed()?;
    for package in &packages {
        rebuild_package(package)?;
    }

    let mut mismatches = 0;
    print_header("Expected SHA256", "Actual SHA256");
    for package in &packages {
        let rebuilt_path = wasm_artifact_path(package)?;
        let rebuilt_hash = sha256_hex_file(&rebuilt_path)?;
        let status = compare(&expected_hash, &rebuilt_hash, None);
        print_row(
            package,
            "Expected SHA256",
            &expected_hash,
            "Actual SHA256",
            &rebuilt_hash,
            status,
        );
        if status == VerifyStatus::Mismatch {
            mismatches += 1;
        }
    }

    if mismatches > 0 {
        bail!("verification failed: {mismatches} artifact(s) did not match expected SHA-256");
    }
    Ok(())
}

/// `--manifest <PATH>`: validate every listed artifact in the provenance
/// manifest against a deterministic rebuild of the contract packages.
fn verify_manifest(manifest_path: &str, json: bool) -> Result<()> {
    let manifest = parse_manifest(Path::new(manifest_path))?;

    build::ensure_wasm_target_installed()?;
    let stdout = if json {
        std::process::Stdio::null()
    } else {
        std::process::Stdio::inherit()
    };
    build::build_contract_wasm_with_stdout(None, stdout)
        .context("rebuilding contracts for provenance verification")?;

    let summary = summarize_manifest(&manifest, &build::wasm_release_dir())?;
    if json {
        println!("{}", serde_json::to_string(&summary)?);
    } else {
        println!("Verifying provenance manifest against source rebuild");
        for (label, value) in [
            ("git_commit", &manifest.git_commit),
            ("generated_at", &manifest.generated_at),
            ("target", &manifest.target),
            ("cli_version", &manifest.cli_version),
            ("lockfile_sha256", &manifest.lockfile_sha256),
            ("bundle_sha256", &manifest.bundle_sha256),
        ] {
            if let Some(value) = value {
                println!("  {label:<13} {value}");
            }
        }
        print_header("Expected SHA256", "Actual SHA256");
        for row in &summary.artifacts {
            if row.size_drift {
                println!(
                    "  note: {:<32} size drifted: manifest {} bytes vs rebuilt {} bytes",
                    row.name,
                    row.expected_bytes.unwrap_or_default(),
                    row.actual_bytes.unwrap_or_default(),
                );
            }
            match &row.actual_sha256 {
                Some(hash) => print_row(
                    row.name,
                    "Expected SHA256",
                    row.expected_sha256,
                    "Actual SHA256",
                    hash,
                    compare(row.expected_sha256, hash, None),
                ),
                None => println!(
                    "{:<32} {:<66} {:<66} MISSING",
                    row.name, row.expected_sha256, "-"
                ),
            }
        }
    }

    summary.finish()
}

#[derive(Serialize)]
struct ManifestSummary<'a> {
    ok: bool,
    schema: &'a str,
    source_revision: Option<&'a str>,
    lockfile_sha256: Option<&'a str>,
    bundle_sha256: Option<&'a str>,
    cli_version: Option<&'a str>,
    deployments: &'a [ManifestDeployment],
    artifacts: Vec<ArtifactResult<'a>>,
}

impl ManifestSummary<'_> {
    fn finish(&self) -> Result<()> {
        if !self.ok {
            let failures = self
                .artifacts
                .iter()
                .filter(|row| row.status != ArtifactStatus::Match)
                .count();
            bail!("provenance verification failed: {failures} artifact(s) drifted");
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct ArtifactResult<'a> {
    name: &'a str,
    status: ArtifactStatus,
    expected_sha256: &'a str,
    actual_sha256: Option<String>,
    expected_bytes: Option<u64>,
    actual_bytes: Option<u64>,
    size_drift: bool,
}

#[derive(PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ArtifactStatus {
    Match,
    Mismatch,
    SizeDrift,
    Missing,
}

fn summarize_manifest<'a>(manifest: &'a Manifest, dir: &Path) -> Result<ManifestSummary<'a>> {
    let mut summary = ManifestSummary {
        ok: true,
        schema: &manifest.schema,
        source_revision: manifest.source_revision.as_deref(),
        lockfile_sha256: manifest.lockfile_sha256.as_deref(),
        bundle_sha256: manifest.bundle_sha256.as_deref(),
        cli_version: manifest.cli_version.as_deref(),
        deployments: &manifest.deployments,
        artifacts: Vec::with_capacity(manifest.artifacts.len()),
    };
    for artifact in &manifest.artifacts {
        let path = dir.join(&artifact.name);
        let mut row = ArtifactResult {
            name: &artifact.name,
            status: ArtifactStatus::Missing,
            expected_sha256: &artifact.sha256,
            actual_sha256: None,
            expected_bytes: artifact.bytes,
            actual_bytes: None,
            size_drift: false,
        };
        match fs::read(&path) {
            Ok(bytes) => {
                let hash = sha256_hex(&bytes);
                let len = bytes.len() as u64;
                row.size_drift = artifact.bytes.is_some_and(|expected| expected != len);
                row.status = if compare(&artifact.sha256, &hash, None) == VerifyStatus::Mismatch {
                    ArtifactStatus::Mismatch
                } else if row.size_drift {
                    ArtifactStatus::SizeDrift
                } else {
                    ArtifactStatus::Match
                };
                row.actual_sha256 = Some(hash);
                row.actual_bytes = Some(len);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("failed to read {}", path.display()))
            }
        }
        summary.ok &= row.status == ArtifactStatus::Match;
        summary.artifacts.push(row);
    }
    Ok(summary)
}

/// Load and validate a provenance manifest, normalizing every listed SHA-256
/// to lowercase so comparisons are stable.
fn parse_manifest(path: &Path) -> Result<Manifest> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read provenance manifest at {}", path.display()))?;
    let mut manifest: Manifest = serde_json::from_str(&raw)
        .with_context(|| format!("malformed provenance manifest at {}", path.display()))?;
    if manifest.schema != "soroban-forge/provenance@1"
        && manifest.schema != "soroban-forge/provenance@2"
    {
        bail!(
            "unsupported provenance manifest schema `{}` (expected `soroban-forge/provenance@1` or `@2`)",
            manifest.schema
        );
    }
    for artifact in &mut manifest.artifacts {
        artifact.sha256 = expect_valid_sha256(&artifact.sha256)?;
    }
    Ok(manifest)
}

/// Pure hash comparison: when an explicit `expected` hash is present the
/// result matches only if both the input artifact hash and the rebuilt hash
/// equal it; otherwise the input artifact hash must equal the rebuilt hash.
fn compare(input_hash: &str, rebuilt_hash: &str, expected: Option<&str>) -> VerifyStatus {
    let matches = match expected {
        Some(expected) => {
            input_hash.eq_ignore_ascii_case(expected) && rebuilt_hash.eq_ignore_ascii_case(expected)
        }
        None => input_hash.eq_ignore_ascii_case(rebuilt_hash),
    };
    if matches {
        VerifyStatus::Match
    } else {
        VerifyStatus::Mismatch
    }
}

/// SHA-256 of `bytes`, hex-encoded lowercase — the same representation used
/// by `scripts/provenance.sh` (`sha256sum`).
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256_hex_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(sha256_hex(&bytes))
}

/// Validate a SHA-256 argument and normalize it to lowercase hex.
fn expect_valid_sha256(input: &str) -> Result<String> {
    let input = input.trim();
    if input.len() != 64 || !input.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!(
            "invalid SHA-256 `{input}`: expected exactly 64 hex characters (as printed by sha256sum)"
        );
    }
    Ok(input.to_ascii_lowercase())
}

/// Map a WASM artifact file name back to its contract package using the
/// repository's naming convention (`soroban_forge_escrow.wasm` →
/// `soroban-forge-escrow`).
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

/// Rebuild `package` for `wasm32v1-none` in release mode and return the
/// rebuilt artifact's path.
fn rebuild_wasm(package: &str) -> Result<PathBuf> {
    build::ensure_wasm_target_installed()?;
    rebuild_package(package)?;
    wasm_artifact_path(package)
}

fn rebuild_package(package: &str) -> Result<()> {
    build::build_contract_wasm(Some(package))
}

fn wasm_artifact_path(package: &str) -> Result<PathBuf> {
    if !CONTRACT_PACKAGES.contains(&package) {
        bail!(
            "`{package}` is not a known Soroban Forge contract package; expected one of: {}",
            CONTRACT_PACKAGES.join(", ")
        );
    }
    let stem = package.replace('-', "_");
    let path = build::wasm_release_dir().join(format!("{stem}.wasm"));
    if !path.exists() {
        bail!(
            "rebuilt WASM artifact not found at {}; did the build succeed?",
            path.display()
        );
    }
    Ok(path)
}

fn print_header(expected_label: &str, actual_label: &str) {
    println!(
        "{:<32} {:<66} {:<66} Status",
        "Contract", expected_label, actual_label
    );
    println!("{:-<32} {:-<66} {:-<66} {:-<6}", "", "", "", "");
}

fn print_row(
    contract: &str,
    _expected_label: &str,
    expected_hash: &str,
    _actual_label: &str,
    actual_hash: &str,
    status: VerifyStatus,
) {
    println!(
        "{:<32} {:<66} {:<66} {}",
        contract,
        expected_hash,
        actual_hash,
        match status {
            VerifyStatus::Match => "MATCH",
            VerifyStatus::Mismatch => "MISMATCH",
        }
    );
}

fn finish(status: VerifyStatus, package: &str) -> Result<()> {
    if status == VerifyStatus::Mismatch {
        bail!("verification failed: hashes do not match for {package}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "soroban-forge-verify-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn manifest_summary_serializes_success_and_drift() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("manifest.json");
        let hash = sha256_hex(b"abc");
        let good = serde_json::json!({"name": "good.wasm", "sha256": hash, "bytes": 3});
        let mut data = serde_json::json!({
            "schema": "soroban-forge/provenance@1", "artifacts": [good]
        });
        fs::write(dir.path().join("good.wasm"), b"abc").unwrap();
        fs::write(&manifest_path, data.to_string()).unwrap();
        let manifest = parse_manifest(&manifest_path).unwrap();
        let summary = summarize_manifest(&manifest, dir.path()).unwrap();
        let output = serde_json::to_value(&summary).unwrap();
        assert!(summary.finish().is_ok());
        assert_eq!(output["ok"], true);
        assert_eq!(output["artifacts"][0]["status"], "match");

        fs::write(dir.path().join("mismatch.wasm"), b"other").unwrap();
        fs::write(dir.path().join("size.wasm"), b"abc").unwrap();
        data["artifacts"] = serde_json::json!([
            good,
            {"name": "mismatch.wasm", "sha256": hash, "bytes": 3},
            {"name": "size.wasm", "sha256": hash, "bytes": 4},
            {"name": "missing.wasm", "sha256": hash}
        ]);
        fs::write(&manifest_path, data.to_string()).unwrap();
        let manifest = parse_manifest(&manifest_path).unwrap();
        let summary = summarize_manifest(&manifest, dir.path()).unwrap();
        let output = serde_json::to_value(&summary).unwrap();
        assert!(summary.finish().is_err());
        assert_eq!(output["ok"], false);
        let rows = &output["artifacts"];
        assert_eq!(rows[1]["status"], "mismatch");
        assert_eq!(rows[1]["size_drift"], true);
        assert_eq!(rows[1]["actual_bytes"], 5);
        assert_eq!(rows[2]["status"], "size_drift");
        assert_eq!(rows[3]["status"], "missing");
        assert!(rows[3]["actual_sha256"].is_null());
    }

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn hash_comparison_reports_match_and_mismatch() {
        let hash = sha256_hex(b"payload");
        // Identical hashes match.
        assert_eq!(compare(&hash, &hash, None), VerifyStatus::Match);
        // Explicating the same hash as expected and comparing equal rebuilt hashes matches.
        assert_eq!(compare(&hash, &hash, Some(&hash)), VerifyStatus::Match);
        // Divergent hashes mismatch, with or without an expected value.
        let other = sha256_hex(b"different");
        assert_eq!(compare(&hash, &other, None), VerifyStatus::Mismatch);
        assert_eq!(compare(&hash, &other, Some(&hash)), VerifyStatus::Mismatch);
    }

    #[test]
    fn invalid_sha256_is_rejected() {
        assert!(expect_valid_sha256("not-a-hash").is_err());
        assert!(expect_valid_sha256("abc").is_err());
        // 63 hex characters is too short.
        assert!(expect_valid_sha256(&"a".repeat(63)).is_err());
        assert!(expect_valid_sha256(&"g".repeat(64)).is_err());
        // 64 hex characters normalize to lowercase.
        let normalized = expect_valid_sha256(&"A".repeat(64)).unwrap();
        assert_eq!(normalized, "a".repeat(64));
    }

    #[test]
    fn wasm_file_name_maps_to_package() {
        let dir = temp_dir();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("soroban_forge_escrow.wasm");
        fs::write(&path, vec![0; 4]).unwrap();
        assert_eq!(
            infer_package_from_wasm_name(&path).unwrap(),
            "soroban-forge-escrow"
        );

        // Unknown names fail actionably rather than guessing.
        let unknown = dir.join("random_contract.wasm");
        fs::write(&unknown, vec![0; 4]).unwrap();
        let err = infer_package_from_wasm_name(&unknown).unwrap_err();
        assert!(err.to_string().contains("--package"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parses_valid_manifest() {
        let dir = temp_dir();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("provenance-manifest.json");
        fs::write(
            &path,
            r#"{
                "schema": "soroban-forge/provenance@1",
                "git_commit": "abc123",
                "target": "wasm32v1-none",
                "artifacts": [
                    { "name": "soroban_forge_escrow.wasm", "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", "bytes": 123 }
                ]
            }"#,
        )
        .unwrap();
        let manifest = parse_manifest(&path).unwrap();
        assert_eq!(manifest.schema, "soroban-forge/provenance@1");
        assert_eq!(manifest.artifacts.len(), 1);
        assert_eq!(manifest.artifacts[0].name, "soroban_forge_escrow.wasm");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn malformed_manifest_is_rejected() {
        let dir = temp_dir();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("provenance-manifest.json");
        fs::write(&path, "this is not json").unwrap();
        let err = parse_manifest(&path).unwrap_err();
        assert!(err.to_string().contains("malformed provenance manifest"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_manifest_is_actionable() {
        let path = temp_dir().join("does-not-exist.json");
        let err = parse_manifest(&path).unwrap_err();
        assert!(err
            .to_string()
            .contains("failed to read provenance manifest"));
    }

    #[test]
    fn manifest_with_invalid_sha_is_rejected() {
        let dir = temp_dir();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("provenance-manifest.json");
        fs::write(
            &path,
            r#"{
                "schema": "soroban-forge/provenance@1",
                "artifacts": [
                    { "name": "soroban_forge_escrow.wasm", "sha256": "nope", "bytes": 123 }
                ]
            }"#,
        )
        .unwrap();
        let err = parse_manifest(&path).unwrap_err();
        assert!(err.to_string().contains("invalid SHA-256"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_wasm_artifact_is_actionable() {
        let dir = temp_dir();
        fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("soroban_forge_escrow.wasm");
        let err = verify_wasm(&missing.display().to_string(), None, None).unwrap_err();
        assert!(err.to_string().contains("WASM artifact not found"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unknown_package_for_expected_is_rejected_before_building() {
        let err = verify_expected(&"a".repeat(64), Some("not-a-contract")).unwrap_err();
        assert!(err
            .to_string()
            .contains("not a known Soroban Forge contract package"));
    }
}
