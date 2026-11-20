use crate::cli::{EventsArgs, EventsFormat};
use crate::commands::process::{ensure_success, CommandOutput};
use anyhow::{Context, Result};
use serde_json::Value;
use std::{collections::HashSet, io, process::Command, thread, time::Duration};

const POLL_EVENT_COUNT: &str = "100";
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_RETRY_INTERVAL: Duration = Duration::from_secs(30);

/// Parses the JSON event output produced by `stellar events`.
#[cfg(test)]
pub fn parse_events_to_jsonl(raw: &str) -> Result<Vec<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let value: Value =
        serde_json::from_str(trimmed).context("failed to parse upstream event output as JSON")?;
    let events = event_values(&value)?;
    events
        .into_iter()
        .map(|event| serde_json::to_string(event).context("failed to serialize event as JSON"))
        .collect()
}

fn event_values(value: &Value) -> Result<Vec<&Value>> {
    match value {
        Value::Array(events) => Ok(events.iter().collect()),
        Value::Object(object) => match object.get("events") {
            Some(Value::Array(events)) => Ok(events.iter().collect()),
            _ => Ok(vec![value]),
        },
        _ => anyhow::bail!("expected array or object in upstream event output, found: {value}"),
    }
}

fn run_once(args: &EventsArgs, start_ledger: Option<u64>) -> Result<Vec<Value>> {
    run_once_with(args, start_ledger, |command| {
        command.output().map(CommandOutput::from_output)
    })
}

/// Fetch one batch of events through an injected command runner.
///
/// Production passes [`Command::output`]; tests pass a closure that returns a
/// fabricated [`CommandOutput`], so both the success and failure paths run
/// without spawning the Stellar CLI.
fn run_once_with<F>(args: &EventsArgs, start_ledger: Option<u64>, execute: F) -> Result<Vec<Value>>
where
    F: FnOnce(&mut Command) -> io::Result<CommandOutput>,
{
    let output =
        execute(&mut event_command(args, start_ledger)).context("failed to run stellar events")?;
    ensure_success("event retrieval", &output)?;

    let value: Value = if output.stdout.trim().is_empty() {
        Value::Array(Vec::new())
    } else {
        serde_json::from_str(output.stdout.trim())
            .context("failed to parse stellar events JSON output")?
    };
    Ok(event_values(&value)?.into_iter().cloned().collect())
}

fn event_command(args: &EventsArgs, start_ledger: Option<u64>) -> Command {
    let mut command = Command::new("stellar");
    command
        .arg("events")
        .arg("--id")
        .arg(&args.contract)
        .arg("--network")
        .arg(&args.network)
        .arg("--output")
        .arg("json")
        .arg("--count")
        .arg(POLL_EVENT_COUNT);

    if let Some(ledger) = start_ledger {
        command.arg("--start-ledger").arg(ledger.to_string());
    }
    if let Some(event_type) = &args.event_type {
        command.arg("--type").arg(event_type);
    }
    command
}

fn matches_topic(event: &Value, topic: Option<&str>) -> bool {
    let Some(topic) = topic else {
        return true;
    };

    fn contains_topic(value: &Value, topic: &str) -> bool {
        match value {
            Value::String(value) => value.eq_ignore_ascii_case(topic),
            Value::Array(values) => values.iter().any(|value| contains_topic(value, topic)),
            Value::Object(object) => object.iter().any(|(key, value)| {
                matches!(
                    key.to_ascii_lowercase().as_str(),
                    "topic" | "topics" | "name" | "eventname" | "event_name"
                ) && contains_topic(value, topic)
            }),
            _ => false,
        }
    }

    contains_topic(event, topic)
}

fn ledger_sequence(event: &Value) -> Option<u64> {
    let object = event.as_object()?;
    ["ledger", "ledger_sequence", "ledgerSequence"]
        .iter()
        .find_map(|key| object.get(*key).and_then(parse_u64))
        .or_else(|| object.get("event").and_then(ledger_sequence))
}

fn parse_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn event_identity(event: &Value) -> String {
    let identity = event.as_object().and_then(|object| {
        ["id", "pagingToken", "paging_token", "cursor"]
            .iter()
            .find_map(|key| object.get(*key).and_then(Value::as_str))
    });
    identity.map_or_else(|| event.to_string(), str::to_owned)
}

fn selected_format(args: &EventsArgs) -> EventsFormat {
    if args.jsonl {
        EventsFormat::Jsonl
    } else {
        args.format
    }
}

fn format_event(event: &Value, format: EventsFormat) -> Result<String> {
    match format {
        EventsFormat::Jsonl => Ok(serde_json::to_string(event)?),
        EventsFormat::Pretty => {
            let label = event_label(event).unwrap_or("event");
            let body = serde_json::to_string_pretty(event)?;
            Ok(format!("\x1b[1;36m[{label}]\x1b[0m\n{body}"))
        }
    }
}

fn print_event(event: &Value, format: EventsFormat) -> Result<()> {
    println!("{}", format_event(event, format)?);
    Ok(())
}

fn event_label(event: &Value) -> Option<&str> {
    let object = event.as_object()?;
    ["name", "eventName", "event_name", "type"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .or_else(|| {
            object
                .get("topic")
                .or_else(|| object.get("topics"))
                .and_then(Value::as_array)
                .and_then(|topics| topics.first())
                .and_then(Value::as_str)
        })
}

pub fn run(args: EventsArgs) -> Result<()> {
    if !args.stream {
        for event in run_once(&args, args.since)? {
            if matches_topic(&event, args.topic.as_deref()) {
                print_event(&event, selected_format(&args))?;
            }
        }
        return Ok(());
    }

    let mut latest_ledger = args.since;
    let mut latest_ledger_events = HashSet::new();
    let mut retry_interval = POLL_INTERVAL;

    loop {
        match run_once(&args, latest_ledger) {
            Ok(events) => {
                for event in events {
                    if let Some(ledger) = ledger_sequence(&event) {
                        if latest_ledger.is_none_or(|latest| ledger > latest) {
                            latest_ledger = Some(ledger);
                            latest_ledger_events.clear();
                        }
                    }

                    let identity = event_identity(&event);
                    if latest_ledger_events.insert(identity)
                        && matches_topic(&event, args.topic.as_deref())
                    {
                        print_event(&event, selected_format(&args))?;
                    }
                }

                retry_interval = POLL_INTERVAL;
                thread::sleep(POLL_INTERVAL);
            }
            Err(error) => {
                eprintln!(
                    "event polling failed: {error:#}; retrying in {} seconds",
                    retry_interval.as_secs()
                );
                thread::sleep(retry_interval);
                retry_interval = (retry_interval * 2).min(MAX_RETRY_INTERVAL);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_events_to_jsonl_accepts_empty_array_and_wrapped_events() {
        assert_eq!(parse_events_to_jsonl("").unwrap(), Vec::<String>::new());
        assert_eq!(parse_events_to_jsonl("   ").unwrap(), Vec::<String>::new());
        assert_eq!(parse_events_to_jsonl("[]").unwrap(), Vec::<String>::new());
        assert_eq!(
            parse_events_to_jsonl(r#"{"events":[{"id":"1"},{"id":"2"}]}"#).unwrap(),
            vec![r#"{"id":"1"}"#, r#"{"id":"2"}"#]
        );
    }

    #[test]
    fn parse_events_to_jsonl_accepts_single_event_object() {
        assert_eq!(
            parse_events_to_jsonl(r#"{"id":"1","type":"contract"}"#).unwrap(),
            vec![r#"{"id":"1","type":"contract"}"#]
        );
    }

    #[test]
    fn parse_events_to_jsonl_rejects_invalid_json_and_scalars() {
        assert!(parse_events_to_jsonl("invalid json").is_err());
        assert!(parse_events_to_jsonl("123").is_err());
    }

    #[test]
    fn topic_filter_matches_decoded_topic_names_case_insensitively() {
        let event = serde_json::json!({
            "topic": ["Transfer", "G...ADDRESS"],
            "value": {"amount": 12}
        });
        assert!(matches_topic(&event, Some("transfer")));
        assert!(!matches_topic(&event, Some("deposit")));
        assert!(matches_topic(&event, None));
    }

    #[test]
    fn ledger_sequence_reads_numeric_and_string_values() {
        assert_eq!(
            ledger_sequence(&serde_json::json!({"ledger": 123})),
            Some(123)
        );
        assert_eq!(
            ledger_sequence(&serde_json::json!({"event": {"ledger": "124"}})),
            Some(124)
        );
    }

    #[test]
    fn event_format_outputs_jsonl_and_colored_pretty_events() {
        let event = serde_json::json!({"topic": ["Transfer"]});
        assert_eq!(event_label(&event), Some("Transfer"));
        assert_eq!(
            format_event(&event, EventsFormat::Jsonl).unwrap(),
            r#"{"topic":["Transfer"]}"#
        );
        let pretty = format_event(&event, EventsFormat::Pretty).unwrap();
        assert!(pretty.contains("\x1b[1;36m[Transfer]\x1b[0m"));
        assert!(pretty.contains("\"topic\""));
    }

    fn events_args() -> EventsArgs {
        EventsArgs {
            contract: "C123".to_owned(),
            since: None,
            event_type: None,
            topic: None,
            network: "testnet".to_owned(),
            stream: false,
            format: EventsFormat::Pretty,
            jsonl: false,
        }
    }

    #[test]
    fn event_retrieval_success_parses_events_without_spawning_stellar() {
        let events = run_once_with(&events_args(), None, |command| {
            let args: Vec<_> = command
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            assert!(args.iter().any(|arg| arg == "events"));
            assert!(args.iter().any(|arg| arg == "C123"));
            Ok(CommandOutput::new(
                true,
                r#"{"events":[{"id":"1"},{"id":"2"}]}"#,
                "",
            ))
        })
        .unwrap();

        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["id"], "1");
    }

    #[test]
    fn event_retrieval_failure_returns_error_with_context() {
        let error = run_once_with(&events_args(), None, |_| {
            Ok(CommandOutput::new(
                false,
                "",
                "rpc unavailable: connection refused",
            ))
        })
        .unwrap_err();

        let message = format!("{error:#}");
        assert!(message.contains("event retrieval failed"));
        assert!(message.contains("rpc unavailable"));
    }

    #[test]
    fn event_retrieval_spawn_failure_returns_error_with_context() {
        let error = run_once_with(&events_args(), None, |_| {
            Err(io::Error::new(io::ErrorKind::NotFound, "stellar not found"))
        })
        .unwrap_err();

        assert!(format!("{error:#}").contains("failed to run stellar events"));
    }
}
