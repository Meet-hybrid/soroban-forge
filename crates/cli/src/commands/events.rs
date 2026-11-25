use crate::cli::{EventsArgs, EventsFormat};
use crate::commands::process::{ensure_success, CommandOutput};
use anyhow::{Context, Result};
use serde_json::Value as JsonValue;
use std::collections::HashMap;

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
    event_json: &JsonValue,
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
        let mut fields = HashMap::new();

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
    fields: HashMap<String, String>,
}

fn format_value(value: &JsonValue) -> String {
    match value {
        JsonValue::String(s) => s.clone(),
        JsonValue::Number(n) => n.to_string(),
        JsonValue::Bool(b) => b.to_string(),
        JsonValue::Null => "null".to_string(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| "?".to_string()),
    }
}

pub fn run(args: EventsArgs) -> Result<()> {
    // For MVP, we'll use the stellar CLI as the transport layer
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

    // Capture output for decoding
    let output = cmd.output().context("failed to run stellar events get")?;

    if !output.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        std::process::exit(1);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Try to decode events if output is JSON-like
    let definitions = get_event_definitions();

    // Check if output is JSONL (one JSON object per line)
    let mut decoded_count = 0;
    let mut unknown_count = 0;
    let mut table_rows: Vec<(String, String, String, HashMap<String, String>)> = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Try to parse as JSON
        if let Ok(event_json) = serde_json::from_str::<JsonValue>(line) {
            match decode_event(&event_json, &definitions) {
                Ok(Some(decoded)) => {
                    let ledger = event_json
                        .get("ledger")
                        .and_then(|l| l.as_u64())
                        .map(|l| l.to_string())
                        .unwrap_or_else(|| "?".to_string());

                    table_rows.push((
                        ledger,
                        decoded.contract_type,
                        decoded.event_name,
                        decoded.fields,
                    ));
                    decoded_count += 1;
                }
                Ok(None) => {
                    // Unknown event - preserve it
                    println!("UNKNOWN EVENT:");
                    println!("  Raw XDR: {}", line);
                    unknown_count += 1;
                }
                Err(_) => {
                    // Malformed known event
                    eprintln!("ERROR: Malformed event payload");
                    eprintln!("  {}", line);
                    std::process::exit(1);
                }
            }
        } else {
            // Not JSON, just print as-is
            println!("{}", line);
        }
    }

    // Display decoded events in a table
    if !table_rows.is_empty() {
        println!("\n╔═══════════════════════════════════════════════════════════════════╗");
        println!("║                     DECODED CONTRACT EVENTS                       ║");
        println!("╚═══════════════════════════════════════════════════════════════════╝\n");

        for (ledger, contract_type, event_name, fields) in table_rows {
            println!(
                "┌─ Ledger: {} ─ Contract: {} ─ Event: {}",
                ledger, contract_type, event_name
            );
            if !fields.is_empty() {
                for (key, value) in &fields {
                    println!("│  {}: {}", key, value);
                }
            }
            println!("└────────────────────────────────────────────────────────────────\n");
        }

        println!(
            "Summary: {} decoded events, {} unknown events",
            decoded_count, unknown_count
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
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
        assert_eq!(format_value(&JsonValue::String("test".to_string())), "test");
        assert_eq!(
            format_value(&JsonValue::Number(serde_json::Number::from(42))),
            "42"
        );
        assert_eq!(format_value(&JsonValue::Bool(true)), "true");
        assert_eq!(format_value(&JsonValue::Null), "null");
    }
}
