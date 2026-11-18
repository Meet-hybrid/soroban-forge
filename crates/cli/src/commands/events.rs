use crate::cli::EventsArgs;
use anyhow::{Context, Result};

/// Parses upstream event output (JSON string or array) into stable JSONL records.
pub fn parse_events_to_jsonl(raw: &str) -> Result<Vec<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let val: serde_json::Value =
        serde_json::from_str(trimmed).context("failed to parse upstream event output as JSON")?;

    match val {
        serde_json::Value::Array(items) => {
            let mut lines = Vec::new();
            for item in items {
                lines.push(serde_json::to_string(&item)?);
            }
            Ok(lines)
        }
        serde_json::Value::Object(_) => Ok(vec![serde_json::to_string(&val)?]),
        _ => anyhow::bail!("expected array or object in upstream event output, found: {trimmed}"),
    }
}

pub fn run(args: EventsArgs) -> Result<()> {
    let mut cmd = std::process::Command::new("stellar");
    cmd.arg("events").arg("get");

    cmd.arg("--contract-ids").arg(&args.contract);
    cmd.arg("--network").arg(&args.network);

    if let Some(since) = args.since {
        cmd.arg("--start-ledger").arg(since.to_string());
    }

    if let Some(event_type) = &args.event_type {
        cmd.arg("--type").arg(event_type);
    }

    if args.jsonl {
        cmd.arg("--output").arg("json");
        let output = cmd.output().context("failed to run stellar events get")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "stellar events get failed with status {}: {stderr}",
                output.status
            );
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines = parse_events_to_jsonl(&stdout)?;
        for line in lines {
            println!("{line}");
        }
        Ok(())
    } else {
        let status = cmd.status().context("failed to run stellar events get")?;
        if !status.success() {
            std::process::exit(1);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_events_to_jsonl_empty() {
        assert_eq!(parse_events_to_jsonl("").unwrap(), Vec::<String>::new());
        assert_eq!(parse_events_to_jsonl("   ").unwrap(), Vec::<String>::new());
        assert_eq!(parse_events_to_jsonl("[]").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn test_parse_events_to_jsonl_array() {
        let raw = r#"[{"id":"1","type":"contract"},{"id":"2","type":"system"}]"#;
        let lines = parse_events_to_jsonl(raw).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], r#"{"id":"1","type":"contract"}"#);
        assert_eq!(lines[1], r#"{"id":"2","type":"system"}"#);
    }

    #[test]
    fn test_parse_events_to_jsonl_single_object() {
        let raw = r#"{"id":"1","type":"contract"}"#;
        let lines = parse_events_to_jsonl(raw).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], r#"{"id":"1","type":"contract"}"#);
    }

    #[test]
    fn test_parse_events_to_jsonl_invalid() {
        assert!(parse_events_to_jsonl("invalid json").is_err());
        assert!(parse_events_to_jsonl("123").is_err());
    }
}
