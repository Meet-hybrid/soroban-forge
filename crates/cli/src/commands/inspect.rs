use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

use crate::cli::InspectArgs;
use crate::commands::build::CONTRACT_PACKAGES;

/// A single storage entry decoded from contract persistent or instance state.
///
/// Deterministic and stable representation for scripting and JSON output.
///
/// # JSON Shape
/// When serialized as JSON, each entry has a fixed schema with string-typed
/// `key` and `value` fields:
///
/// ```json
/// {
///   "key": "Admin",
///   "value": "GBRPYHIL2CI3FNQ4BXLFMNDLFJUNPU2HY3ZMFSHONUCEOASW7QC7OX2H"
/// }
/// ```
///
/// Script consumers can reliably depend on:
/// 1. Entries emitted as a top-level JSON array (`Vec<InspectEntry>`).
/// 2. Deterministic lexicographical sorting by `key` ascending.
/// 3. Canonical, deterministic string formatting of primitive and nested values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct InspectEntry {
    /// Storage key name or identifier.
    pub key: String,
    /// Storage value represented as a string or canonical JSON string for nested data.
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
        println!("{}", format_json(&entries)?);
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

/// Canonicalizes a JSON value so that nested objects have their keys sorted
/// recursively in ascending order, ensuring deterministic JSON formatting.
fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted = std::collections::BTreeMap::new();
            for (k, v) in map {
                sorted.insert(k.clone(), canonicalize_json(v));
            }
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonicalize_json).collect()),
        other => other.clone(),
    }
}

/// Formats a JSON value into a deterministic string representation.
///
/// - String values are unwrapped directly (without extra surrounding quotes).
/// - Objects and arrays are canonicalized recursively with sorted keys, then serialized to compact JSON.
/// - Scalars (numbers, booleans, null) are converted to their standard string representations.
fn format_value(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Object(_) | Value::Array(_) => {
            let canonical = canonicalize_json(value);
            serde_json::to_string(&canonical).unwrap_or_else(|_| value.to_string())
        }
        other => other.to_string(),
    }
}

/// Parses raw JSON inspection output into a deterministic list of [`InspectEntry`] items
/// sorted by `key` ascending.
///
/// # Supported Formats
/// - Empty or whitespace string: Treated as empty contract state, returning `Ok(vec![])`.
/// - JSON Object: Decoded into key-value pairs sorted alphabetically by key. An empty object
///   `{}` returns `Ok(vec![])`.
///
/// # Errors
/// Returns an error if:
/// - The input is not valid JSON.
/// - The JSON root is a scalar (e.g. number, string, boolean).
/// - The JSON root is a non-object (e.g. array, null).
pub fn parse_entries(raw: &str) -> Result<Vec<InspectEntry>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let value: Value = serde_json::from_str(trimmed).context("failed to parse inspect output")?;
    let object = value
        .as_object()
        .context("inspect output must be a JSON object")?;

    let mut entries: Vec<InspectEntry> = object
        .iter()
        .map(|(key, value)| InspectEntry {
            key: key.clone(),
            value: format_value(value),
        })
        .collect();

    entries.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(entries)
}

/// Formats contract inspect entries into a human-readable table string.
///
/// Table entries are emitted in deterministic key order (ascending).
pub fn format_table(entries: &[InspectEntry]) -> String {
    if entries.is_empty() {
        return "No contract state found.".to_string();
    }

    let mut sorted: Vec<&InspectEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.key.cmp(&b.key));

    let key_width = sorted
        .iter()
        .map(|entry| entry.key.len())
        .max()
        .unwrap_or(3);

    let mut out = String::new();
    out.push_str(&format!("{:<key_width$} | VALUE\n", "KEY"));
    out.push_str(&format!("{}-+-{}\n", "-".repeat(key_width), "-".repeat(20)));
    for entry in sorted {
        out.push_str(&format!("{:<key_width$} | {}\n", entry.key, entry.value));
    }
    out
}

/// Prints the formatted table to standard output.
pub fn print_table(entries: &[InspectEntry]) {
    let table = format_table(entries);
    if entries.is_empty() {
        println!("{table}");
    } else {
        print!("{table}");
    }
}

/// Formats contract inspect entries as a deterministic, pretty-printed JSON string.
///
/// The output is guaranteed to be a JSON array of [`InspectEntry`] objects
/// ordered deterministically by `key` ascending.
pub fn format_json(entries: &[InspectEntry]) -> Result<String> {
    let mut sorted: Vec<&InspectEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.key.cmp(&b.key));
    serde_json::to_string_pretty(&sorted).context("failed to serialize inspect entries as JSON")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Returns the lowercase SHA-256 of an artifact file, for deterministic reporting.
pub fn sha256_hex_file(path: &Path) -> Result<String> {
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

/// Infers the Forge contract package name from a WASM artifact file name.
pub fn infer_package_from_wasm_name(path: &Path) -> Result<String> {
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

    #[test]
    fn table_entries_emitted_in_deterministic_key_order() {
        let entries = vec![
            InspectEntry {
                key: "zeta".to_string(),
                value: "last".to_string(),
            },
            InspectEntry {
                key: "alpha".to_string(),
                value: "first".to_string(),
            },
            InspectEntry {
                key: "mid".to_string(),
                value: "middle".to_string(),
            },
        ];

        let table = format_table(&entries);
        let alpha_idx = table.find("alpha").expect("table must contain alpha");
        let mid_idx = table.find("mid").expect("table must contain mid");
        let zeta_idx = table.find("zeta").expect("table must contain zeta");

        assert!(
            alpha_idx < mid_idx && mid_idx < zeta_idx,
            "table entries must be ordered by key ascending: alpha < mid < zeta"
        );
    }

    #[test]
    fn parse_entries_deterministic_ordering() {
        let json_input = r#"{
            "zebra": "z_val",
            "apple": "a_val",
            "mango": "m_val",
            "banana": "b_val"
        }"#;

        let entries = parse_entries(json_input).expect("parsing valid object should succeed");
        let keys: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, vec!["apple", "banana", "mango", "zebra"]);
    }

    #[test]
    fn json_output_stable_shape_and_roundtrip() {
        let entries = vec![
            InspectEntry {
                key: "Admin".to_string(),
                value: "GBRPYHIL2CI3FNQ4BXLFMNDLFJUNPU2HY3ZMFSHONUCEOASW7QC7OX2H".to_string(),
            },
            InspectEntry {
                key: "Counter".to_string(),
                value: "42".to_string(),
            },
        ];

        let json_str = format_json(&entries).expect("format_json should succeed");
        let parsed_val: Value = serde_json::from_str(&json_str).expect("output must be valid JSON");

        // Documented stable shape: top-level array of objects with "key" and "value" string fields
        let arr = parsed_val.as_array().expect("root must be a JSON array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["key"], "Admin");
        assert_eq!(
            arr[0]["value"],
            "GBRPYHIL2CI3FNQ4BXLFMNDLFJUNPU2HY3ZMFSHONUCEOASW7QC7OX2H"
        );
        assert_eq!(arr[1]["key"], "Counter");
        assert_eq!(arr[1]["value"], "42");

        // Roundtrip deserialization
        let roundtrip: Vec<InspectEntry> =
            serde_json::from_str(&json_str).expect("must roundtrip back to Vec<InspectEntry>");
        assert_eq!(roundtrip, entries);
    }

    #[test]
    fn empty_inputs_tested() {
        // Empty string
        let empty_str_entries = parse_entries("").expect("empty string should return empty vec");
        assert!(empty_str_entries.is_empty());

        // Whitespace-only string
        let ws_entries =
            parse_entries("   \n\t  ").expect("whitespace string should return empty vec");
        assert!(ws_entries.is_empty());

        // Empty JSON object
        let empty_obj_entries =
            parse_entries("{}").expect("empty JSON object should return empty vec");
        assert!(empty_obj_entries.is_empty());

        // Empty table output is human-actionable
        let empty_table = format_table(&[]);
        assert_eq!(empty_table, "No contract state found.");

        // Empty JSON output is a valid empty array
        let empty_json = format_json(&[]).expect("empty format_json should succeed");
        assert_eq!(empty_json, "[]");
    }

    #[test]
    fn scalar_inputs_tested() {
        // Integer scalar
        let err_int = parse_entries("42").unwrap_err();
        assert!(
            err_int
                .to_string()
                .contains("inspect output must be a JSON object"),
            "expected object error for int scalar, got: {err_int}"
        );

        // String scalar
        let err_str = parse_entries(r#""scalar string""#).unwrap_err();
        assert!(
            err_str
                .to_string()
                .contains("inspect output must be a JSON object"),
            "expected object error for string scalar, got: {err_str}"
        );

        // Boolean scalar
        let err_bool = parse_entries("true").unwrap_err();
        assert!(
            err_bool
                .to_string()
                .contains("inspect output must be a JSON object"),
            "expected object error for bool scalar, got: {err_bool}"
        );

        // Float scalar
        let err_float = parse_entries("3.14159").unwrap_err();
        assert!(
            err_float
                .to_string()
                .contains("inspect output must be a JSON object"),
            "expected object error for float scalar, got: {err_float}"
        );
    }

    #[test]
    fn nested_inputs_tested() {
        // Nested object with arbitrary key order
        let json_nested = r#"{
            "settings": {
                "timeout": 30,
                "active": true,
                "credentials": {
                    "token": "xyz",
                    "id": 1
                }
            },
            "admin": "GAAA"
        }"#;

        let entries = parse_entries(json_nested).expect("nested JSON object should parse");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "admin");
        assert_eq!(entries[0].value, "GAAA");

        assert_eq!(entries[1].key, "settings");
        // Nested object must have keys deterministically sorted: active, credentials (id, token), timeout
        assert_eq!(
            entries[1].value,
            r#"{"active":true,"credentials":{"id":1,"token":"xyz"},"timeout":30}"#
        );

        // Deeply nested input with array of objects
        let json_deep = r#"{
            "records": [
                {"z": 2, "a": 1},
                {"b": 4, "a": 3}
            ]
        }"#;
        let entries_deep = parse_entries(json_deep).expect("nested array should parse");
        assert_eq!(entries_deep.len(), 1);
        assert_eq!(entries_deep[0].key, "records");
        assert_eq!(entries_deep[0].value, r#"[{"a":1,"z":2},{"a":3,"b":4}]"#);
    }

    #[test]
    fn invalid_inputs_tested() {
        // Syntax error - completely invalid tokens
        let err1 = parse_entries("not a json value").unwrap_err();
        assert!(err1.to_string().contains("failed to parse inspect output"));

        // Syntax error - unclosed brace
        let err2 = parse_entries(r#"{"key": "value""#).unwrap_err();
        assert!(err2.to_string().contains("failed to parse inspect output"));

        // Syntax error - missing value
        let err3 = parse_entries(r#"{"key": }"#).unwrap_err();
        assert!(err3.to_string().contains("failed to parse inspect output"));

        // Syntax error - trailing unexpected tokens
        let err4 = parse_entries(r#"{"key": "value"} extra"#).unwrap_err();
        assert!(err4.to_string().contains("failed to parse inspect output"));
    }

    #[test]
    fn non_object_inputs_tested() {
        // JSON array of primitives
        let err_arr = parse_entries("[1, 2, 3]").unwrap_err();
        assert!(
            err_arr
                .to_string()
                .contains("inspect output must be a JSON object"),
            "expected object error for array input, got: {err_arr}"
        );

        // JSON array of objects
        let err_arr_obj = parse_entries(r#"[{"key": "a", "value": "b"}]"#).unwrap_err();
        assert!(
            err_arr_obj
                .to_string()
                .contains("inspect output must be a JSON object"),
            "expected object error for array of objects, got: {err_arr_obj}"
        );

        // JSON null literal
        let err_null = parse_entries("null").unwrap_err();
        assert!(
            err_null
                .to_string()
                .contains("inspect output must be a JSON object"),
            "expected object error for null, got: {err_null}"
        );
    }
}
