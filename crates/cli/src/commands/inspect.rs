use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cli::InspectArgs;

/// Stable machine-readable shape returned by contract inspection.
///
/// The JSON output is a list of entries in lexical key order, where each entry contains:
/// - `key`: the contract storage key as a string
/// - `value`: the raw JSON value rendered as a string for script-friendly display
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InspectEntry {
    pub key: String,
    pub value: String,
}

pub fn run(args: InspectArgs) -> Result<()> {
    let output = fetch_state(&args)?;
    let entries = parse_entries(&output)?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        print_table(&entries);
    }

    Ok(())
}

fn fetch_state(args: &InspectArgs) -> Result<String> {
    let mut command = std::process::Command::new("stellar");
    command
        .arg("contract")
        .arg("invoke")
        .arg("--id")
        .arg(&args.contract)
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

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted = serde_json::Map::new();
            let mut keys: Vec<_> = map.keys().cloned().collect();
            keys.sort();
            for key in keys {
                let value = map
                    .get(&key)
                    .cloned()
                    .map(canonicalize_json)
                    .unwrap_or(Value::Null);
                sorted.insert(key, value);
            }
            Value::Object(sorted)
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_json).collect()),
        other => other,
    }
}

fn parse_entries(raw: &str) -> Result<Vec<InspectEntry>> {
    let value: Value = serde_json::from_str(raw).context("failed to parse inspect output")?;
    let object = value
        .as_object()
        .context("inspect output must be a JSON object")?;

    let mut entries: Vec<_> = object
        .iter()
        .map(|(key, value)| InspectEntry {
            key: key.clone(),
            value: canonicalize_json(value.clone()).to_string(),
        })
        .collect();

    entries.sort_by(|left, right| left.key.cmp(&right.key));
    Ok(entries)
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

#[cfg(test)]
mod tests {
    use super::parse_entries;

    #[test]
    fn parse_entries_sorts_object_keys_stably() {
        let raw = r#"{"z":1,"a":{"nested":true},"m":"str"}"#;

        let entries = parse_entries(raw).expect("valid inspect object should parse");

        assert_eq!(
            entries,
            vec![
                super::InspectEntry {
                    key: "a".to_string(),
                    value: "{\"nested\":true}".to_string(),
                },
                super::InspectEntry {
                    key: "m".to_string(),
                    value: "\"str\"".to_string(),
                },
                super::InspectEntry {
                    key: "z".to_string(),
                    value: "1".to_string(),
                },
            ]
        );
    }

    #[test]
    fn parse_entries_accepts_empty_object_and_rejects_invalid_shapes() {
        assert!(parse_entries("{}").unwrap().is_empty());
        assert!(parse_entries("42").is_err());
        assert!(parse_entries("[1,2,3]").is_err());
        assert!(parse_entries("{bad").is_err());
    }

    #[test]
    fn parse_entries_preserves_nested_json_values() {
        let raw = r#"{"status":{"phase":"open","owner":"alice"},"count":2}"#;

        let entries = parse_entries(raw).expect("nested inspect values should parse");

        assert_eq!(entries[0].key, "count");
        assert_eq!(entries[0].value, "2");
        assert_eq!(entries[1].key, "status");
        assert_eq!(entries[1].value, "{\"owner\":\"alice\",\"phase\":\"open\"}");
    }
}
