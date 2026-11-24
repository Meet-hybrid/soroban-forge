use crate::cli::{EventsArgs, EventsFormat};
use crate::commands::process::{ensure_success, CommandOutput};
use anyhow::{Context, Result};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet}, io, process::Command, thread, time::Duration};

/// Known event types from Soroban Forge contracts
#[derive(Debug, Clone)]
struct EventDefinition {
    contract_type: &'static str,
    event_name: &'static str,
    topics: Vec<&'static str>,
    data_fields: Vec<&'static str>,
}

/// Get event definitions for all supported Soroban Forge contracts
fn get_event_definitions() -> HashMap<&'static str, EventDefinition> {
    let mut defs = HashMap::new();

    // Escrow events
    defs.insert(
        "escrow_created",
        EventDefinition {
            contract_type: "escrow",
            event_name: "EscrowCreated",
            topics: vec!["escrow_id"],
            data_fields: vec!["depositor", "beneficiary", "token", "amount", "arbiter"],
        },
    );

    defs.insert(
        "deposited",
        EventDefinition {
            contract_type: "escrow",
            event_name: "Deposited",
            topics: vec!["escrow_id"],
            data_fields: vec!["amount"],
        },
    );

    defs.insert(
        "released",
        EventDefinition {
            contract_type: "escrow",
            event_name: "Released",
            topics: vec!["escrow_id"],
            data_fields: vec!["amount"],
        },
    );

    defs.insert(
        "partially_released",
        EventDefinition {
            contract_type: "escrow",
            event_name: "PartiallyReleased",
            topics: vec!["escrow_id"],
            data_fields: vec!["amount", "data"],
        },
    );

    defs.insert(
        "refunded",
        EventDefinition {
            contract_type: "escrow",
            event_name: "Refunded",
            topics: vec!["escrow_id"],
            data_fields: vec!["amount"],
        },
    );

    defs.insert(
        "refund_expired",
        EventDefinition {
            contract_type: "escrow",
            event_name: "RefundExpired",
            topics: vec!["escrow_id"],
            data_fields: vec![],
        },
    );

    defs.insert(
        "disputed",
        EventDefinition {
            contract_type: "escrow",
            event_name: "Disputed",
            topics: vec!["escrow_id"],
            data_fields: vec![],
        },
    );

    defs.insert(
        "resolved",
        EventDefinition {
            contract_type: "escrow",
            event_name: "Resolved",
            topics: vec!["escrow_id"],
            data_fields: vec!["to_beneficiary"],
        },
    );

    defs.insert(
        "resolved_split",
        EventDefinition {
            contract_type: "escrow",
            event_name: "ResolvedSplit",
            topics: vec!["escrow_id"],
            data_fields: vec!["to_depositor", "to_beneficiary"],
        },
    );

    defs.insert(
        "cancelled",
        EventDefinition {
            contract_type: "escrow",
            event_name: "Cancelled",
            topics: vec!["escrow_id"],
            data_fields: vec![],
        },
    );

    // Basket escrow events
    defs.insert(
        "basket_created",
        EventDefinition {
            contract_type: "escrow",
            event_name: "BasketCreated",
            topics: vec!["escrow_id"],
            data_fields: vec![],
        },
    );

    defs.insert(
        "basket_deposited",
        EventDefinition {
            contract_type: "escrow",
            event_name: "BasketDeposited",
            topics: vec!["escrow_id"],
            data_fields: vec![],
        },
    );

    defs.insert(
        "basket_released",
        EventDefinition {
            contract_type: "escrow",
            event_name: "BasketReleased",
            topics: vec!["escrow_id"],
            data_fields: vec![],
        },
    );

    defs.insert(
        "basket_partially_released",
        EventDefinition {
            contract_type: "escrow",
            event_name: "BasketPartiallyReleased",
            topics: vec!["escrow_id"],
            data_fields: vec!["leg", "amount", "data"],
        },
    );

    defs.insert(
        "basket_refunded",
        EventDefinition {
            contract_type: "escrow",
            event_name: "BasketRefunded",
            topics: vec!["escrow_id"],
            data_fields: vec![],
        },
    );

    defs.insert(
        "basket_disputed",
        EventDefinition {
            contract_type: "escrow",
            event_name: "BasketDisputed",
            topics: vec!["escrow_id"],
            data_fields: vec![],
        },
    );

    defs.insert(
        "basket_resolved",
        EventDefinition {
            contract_type: "escrow",
            event_name: "BasketResolved",
            topics: vec!["escrow_id"],
            data_fields: vec!["to_beneficiary"],
        },
    );

    defs.insert(
        "basket_cancelled",
        EventDefinition {
            contract_type: "escrow",
            event_name: "BasketCancelled",
            topics: vec!["escrow_id"],
            data_fields: vec![],
        },
    );

    // Marketplace royalties events
    defs.insert(
        "royalty_configured",
        EventDefinition {
            contract_type: "marketplace-royalties",
            event_name: "RoyaltyConfigured",
            topics: vec!["collection"],
            data_fields: vec!["recipient", "bps"],
        },
    );

    defs.insert(
        "sale_settled",
        EventDefinition {
            contract_type: "marketplace-royalties",
            event_name: "SaleSettled",
            topics: vec!["collection"],
            data_fields: vec![
                "token",
                "payer",
                "seller",
                "royalty_recipient",
                "gross_amount",
                "seller_net",
                "royalty_share",
            ],
        },
    );

    // DAO governance events
    defs.insert(
        "proposed",
        EventDefinition {
            contract_type: "dao-governance",
            event_name: "Proposed",
            topics: vec!["proposal_id"],
            data_fields: vec!["proposer"],
        },
    );

    defs.insert(
        "vote_cast",
        EventDefinition {
            contract_type: "dao-governance",
            event_name: "VoteCast",
            topics: vec!["proposal_id"],
            data_fields: vec!["voter", "vote"],
        },
    );

    defs.insert(
        "finalised",
        EventDefinition {
            contract_type: "dao-governance",
            event_name: "Finalised",
            topics: vec!["proposal_id"],
            data_fields: vec!["outcome"],
        },
    );

    defs.insert(
        "bond_posted",
        EventDefinition {
            contract_type: "dao-governance",
            event_name: "BondPosted",
            topics: vec!["proposal_id"],
            data_fields: vec!["proposer", "token", "amount"],
        },
    );

    defs.insert(
        "bond_released",
        EventDefinition {
            contract_type: "dao-governance",
            event_name: "BondReleased",
            topics: vec!["proposal_id"],
            data_fields: vec!["to", "token", "amount", "forfeited"],
        },
    );

    defs.insert(
        "quorum_updated",
        EventDefinition {
            contract_type: "dao-governance",
            event_name: "QuorumUpdated",
            topics: vec![],
            data_fields: vec!["old_quorum_bps", "new_quorum_bps"],
        },
    );

    // Vesting events
    defs.insert(
        "beneficiary_reassigned_from",
        EventDefinition {
            contract_type: "vesting",
            event_name: "BeneficiaryReassignedFrom",
            topics: vec!["schedule_id"],
            data_fields: vec!["from_beneficiary", "to_beneficiary"],
        },
    );

    defs.insert(
        "beneficiary_reassigned_to",
        EventDefinition {
            contract_type: "vesting",
            event_name: "BeneficiaryReassignedTo",
            topics: vec!["schedule_id"],
            data_fields: vec!["from_beneficiary", "to_beneficiary"],
        },
    );

    // Multi-sig wallet events (commonly includes transaction events)
    defs.insert(
        "transaction_proposed",
        EventDefinition {
            contract_type: "multi-sig-wallet",
            event_name: "TransactionProposed",
            topics: vec!["transaction_id"],
            data_fields: vec!["proposer"],
        },
    );

    defs.insert(
        "transaction_approved",
        EventDefinition {
            contract_type: "multi-sig-wallet",
            event_name: "TransactionApproved",
            topics: vec!["transaction_id"],
            data_fields: vec!["approver"],
        },
    );

    defs.insert(
        "transaction_executed",
        EventDefinition {
            contract_type: "multi-sig-wallet",
            event_name: "TransactionExecuted",
            topics: vec!["transaction_id"],
            data_fields: vec![],
        },
    );

    // Subscription payments events (commonly includes subscription lifecycle)
    defs.insert(
        "subscription_created",
        EventDefinition {
            contract_type: "subscription-payments",
            event_name: "SubscriptionCreated",
            topics: vec!["subscription_id"],
            data_fields: vec!["subscriber", "amount", "interval"],
        },
    );

    defs.insert(
        "payment_collected",
        EventDefinition {
            contract_type: "subscription-payments",
            event_name: "PaymentCollected",
            topics: vec!["subscription_id"],
            data_fields: vec!["amount"],
        },
    );

    defs.insert(
        "subscription_cancelled",
        EventDefinition {
            contract_type: "subscription-payments",
            event_name: "SubscriptionCancelled",
            topics: vec!["subscription_id"],
            data_fields: vec![],
        },
    );

    defs
}

/// Decode a known event from JSON
fn decode_event(
    event_json: &Value,
    definitions: &HashMap<&'static str, EventDefinition>,
) -> Result<Option<DecodedEvent>> {
    // Try to extract event name from topics
    let topics = event_json.get("topics").and_then(|t| t.as_array());
    if topics.is_none() {
        return Ok(None);
    }

    let topics = topics.unwrap();
    if topics.is_empty() {
        return Ok(None);
    }

    // First topic is typically the event name symbol
    let event_name_str = topics[0]
        .as_str()
        .unwrap_or("")
        .to_lowercase()
        .replace("\"", "");

    if let Some(def) = definitions.get(event_name_str.as_str()) {
        let mut fields = BTreeMap::new();

        // Extract topic fields
        for (i, topic_name) in def.topics.iter().enumerate() {
            if let Some(value) = topics.get(i + 1) {
                fields.insert(topic_name.to_string(), format_value(value));
            }
        }

        // Extract data fields
        if let Some(data) = event_json.get("data") {
            if let Some(data_obj) = data.as_object() {
                for field_name in &def.data_fields {
                    if let Some(value) = data_obj.get(*field_name) {
                        fields.insert(field_name.to_string(), format_value(value));
                    }
                }
            }
        }

        Ok(Some(DecodedEvent {
            contract_type: def.contract_type.to_string(),
            event_name: def.event_name.to_string(),
            fields,
        }))
    } else {
        Ok(None)
    }
}

#[derive(Debug)]
struct DecodedEvent {
    contract_type: String,
    event_name: String,
    fields: BTreeMap<String, String>,
}

fn format_value(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| "?".to_string()),
    }
}

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
    let mut command = event_command(args, start_ledger)?;
    let output = execute(&mut command).context("failed to run stellar events")?;
    ensure_success("event retrieval", &output)?;

    let value: Value = if output.stdout.trim().is_empty() {
        Value::Array(Vec::new())
    } else {
        serde_json::from_str(output.stdout.trim())
            .context("failed to parse stellar events JSON output")?
    };
    Ok(event_values(&value)?.into_iter().cloned().collect())
}

fn event_command(args: &EventsArgs, start_ledger: Option<u64>) -> Result<Command> {
    let network = crate::network_profiles::resolve_network(
        args.network.as_deref(),
        args.network_profile.as_deref(),
        args.rpc_url.as_deref(),
        args.network_passphrase.as_deref(),
    )?;
    let mut command = Command::new("stellar");
    command
        .arg("events")
        .arg("--id")
        .arg(&args.contract)
        .arg("--network")
        .arg(network.name)
        .arg("--output")
        .arg("json")
        .arg("--count")
        .arg(POLL_EVENT_COUNT);

    if let Some(rpc_url) = network.rpc_url {
        command.arg("--rpc-url").arg(rpc_url);
    }
    if let Some(passphrase) = network.network_passphrase {
        command.arg("--network-passphrase").arg(passphrase);
    }

    if let Some(ledger) = start_ledger {
        command.arg("--start-ledger").arg(ledger.to_string());
    }
    if let Some(event_type) = &args.event_type {
        command.arg("--type").arg(event_type);
    }
    Ok(command)
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
            let body = serde_json::to_string_pretty(event)?;
            // Known Soroban Forge events get a decoded, human-readable header;
            // unknown events fall back to their raw label.
            if let Some(decoded) = decode_event(event, &get_event_definitions())? {
                let mut header = format!(
                    "\x1b[1;36m[{} / {}]\x1b[0m",
                    decoded.contract_type, decoded.event_name
                );
                for (key, value) in &decoded.fields {
                    header.push_str(&format!("\n  {key}: {value}"));
                }
                return Ok(format!("{header}\n{body}"));
            }
            let label = event_label(event).unwrap_or("event");
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
            network: Some("testnet".to_owned()),
            network_profile: None,
            rpc_url: None,
            network_passphrase: None,
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

    fn event_definitions_cover_all_contracts() {
        let defs = get_event_definitions();

        // Verify we have definitions for each contract type
        let contract_types: Vec<&str> = defs.values().map(|d| d.contract_type).collect();

        assert!(contract_types.contains(&"escrow"));
        assert!(contract_types.contains(&"marketplace-royalties"));
        assert!(contract_types.contains(&"dao-governance"));
        assert!(contract_types.contains(&"vesting"));
        assert!(contract_types.contains(&"multi-sig-wallet"));
        assert!(contract_types.contains(&"subscription-payments"));
    }

    #[test]
    fn format_value_handles_types() {
        assert_eq!(format_value(&Value::String("test".to_string())), "test");
        assert_eq!(
            format_value(&Value::Number(serde_json::Number::from(42))),
            "42"
        );
        assert_eq!(format_value(&Value::Bool(true)), "true");
        assert_eq!(format_value(&Value::Null), "null");
    }

    #[test]
    fn pretty_format_decodes_known_events_and_keeps_unknown_raw() {
        let known = serde_json::json!({
            "topics": ["escrow_created", 7],
            "data": { "depositor": "GDEPOSITOR", "amount": 500 }
        });
        let pretty = format_event(&known, EventsFormat::Pretty).unwrap();
        assert!(pretty.contains("[escrow / EscrowCreated]"));
        assert!(pretty.contains("escrow_id: 7"));
        assert!(pretty.contains("depositor: GDEPOSITOR"));
        assert!(pretty.contains("amount: 500"));

        let unknown = serde_json::json!({ "topics": ["not_a_forge_event"] });
        let pretty = format_event(&unknown, EventsFormat::Pretty).unwrap();
        assert!(pretty.contains("[not_a_forge_event]"));
    }
}
