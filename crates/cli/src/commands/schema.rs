use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

pub fn run(args: crate::cli::SchemaArgs) -> Result<()> {
    let registry = build_registry();
    let output_dir = match args.output {
        Some(dir) => Path::new(&dir),
        None => Path::new("docs/contracts/schema"),
    };
    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create schema output directory at {:?}", output_dir))?;
    let registry_path = output_dir.join("registry.json");
    let registry_text = serde_json::to_string_pretty(&registry)?;
    fs::write(&registry_path, registry_text)
        .with_context(|| format!("failed to write registry at {:?}", registry_path))?;
    println!("contract storage schema registry written to {:?}", registry_path);

    let summary_path = output_dir.join("README.md");
    let summary_text = render_summary(&registry);
    fs::write(&summary_path, summary_text)
        .with_context(|| format!("failed to write schema summary at {:?}", summary_path))?;
    println!("schema summary written to {:?}", summary_path);
    Ok(())
}

fn build_registry() -> crate::commands::schema::StorageRegistry {
    crate::commands::schema::StorageRegistry {
        generated_at: chrono_now_iso(),
        contracts: vec![
            crate::commands::schema::ContractSchema {
                crate_name: "escrow".into(),
                contract_name: "Escrow".into(),
                storage_kind: "persistent + instance".into(),
                data_keys: vec![
                    crate::commands::schema::DataKeyEntry {
                        variant: "Escrow(u64)".into(),
                        kind: "persistent entry".into(),
                        description: "Escrow record for a given escrow id; payload wrapped in StorageEntry { value: Bytes, updated_at: u64 }.".into(),
                    },
                    crate::commands::schema::DataKeyEntry {
                        variant: "Count".into(),
                        kind: "instance entry".into(),
                        description: "Monotonic escrow id counter.".into(),
                    },
                ],
                records: vec![crate::commands::schema::StoredRecord {
                    name: "EscrowData".into(),
                    storage_key: "DataKey::Escrow(id)".into(),
                    fields: vec![
                        ("escrow_id", "u64"),
                        ("buyer", "Address"),
                        ("seller", "Address"),
                        ("arbiter", "Address"),
                        ("token", "Address"),
                        ("amount", "i128"),
                        ("timeout", "u64"),
                        ("status", "EscrowStatus"),
                        ("created_at", "u64"),
                    ]
                    .into_iter()
                    .map(|(name, ty)| (name.to_string(), ty.to_string()))
                    .collect(),
                }],
                status_enum: Some("EscrowStatus".into()),
                status_variant_schema: Some(vec![
                    "Pending".into(),
                    "Funded".into(),
                    "Completed".into(),
                    "Refunded".into(),
                    "Disputed".into(),
                    "Resolved(bool)".into(),
                    "Cancelled".into(),
                ]),
            },
            crate::commands::schema::ContractSchema {
                crate_name: "dao-governance".into(),
                contract_name: "DaoGovernance".into(),
                storage_kind: "instance".into(),
                data_keys: vec![
                    crate::commands::schema::DataKeyEntry {
                        variant: "Proposal(u64)".into(),
                        kind: "instance entry".into(),
                        description: "Proposal record for a given proposal id.".into(),
                    },
                    crate::commands::schema::DataKeyEntry {
                        variant: "Vote(u64, Address)".into(),
                        kind: "instance entry".into(),
                        description: "Marks that a voter has already voted on a proposal.".into(),
                    },
                    crate::commands::schema::DataKeyEntry {
                        variant: "Count".into(),
                        kind: "instance entry".into(),
                        description: "Monotonic proposal id counter.".into(),
                    },
                ],
                records: vec![crate::commands::schema::StoredRecord {
                    name: "Proposal".into(),
                    storage_key: "DataKey::Proposal(id)".into(),
                    fields: vec![
                        ("id", "u64"),
                        ("proposer", "Address"),
                        ("title", "String"),
                        ("description", "String"),
                        ("for_votes", "u64"),
                        ("against_votes", "u64"),
                        ("state", "ProposalState"),
                        ("created_at", "u64"),
                    ]
                    .into_iter()
                    .map(|(name, ty)| (name.to_string(), ty.to_string()))
                    .collect(),
                }],
                status_enum: Some("ProposalState".into()),
                status_variant_schema: Some(vec![
                    "Pending".into(),
                    "Active".into(),
                    "Executed".into(),
                    "Defeated".into(),
                ]),
            },
            crate::commands::schema::ContractSchema {
                crate_name: "marketplace-royalties".into(),
                contract_name: "MarketplaceRoyalties".into(),
                storage_kind: "instance".into(),
                data_keys: vec![crate::commands::schema::DataKeyEntry {
                    variant: "Royalty(Address)".into(),
                    kind: "instance entry".into(),
                    description: "Royalty configuration for a collection address.".into(),
                }],
                records: vec![crate::commands::schema::StoredRecord {
                    name: "Royalty".into(),
                    storage_key: "DataKey::Royalty(collection)".into(),
                    fields: vec![
                        ("collection", "Address"),
                        ("royalty_percent", "u32"),
                        ("payout_account", "Address"),
                        ("status", "RoyaltyStatus"),
                        ("created_at", "u64"),
                    ]
                    .into_iter()
                    .map(|(name, ty)| (name.to_string(), ty.to_string()))
                    .collect(),
                }],
                status_enum: Some("RoyaltyStatus".into()),
                status_variant_schema: Some(vec![
                    "Active".into(),
                    "Revoked".into(),
                ]),
            },
            crate::commands::schema::ContractSchema {
                crate_name: "multi-sig-wallet".into(),
                contract_name: "MultiSigWallet".into(),
                storage_kind: "instance".into(),
                data_keys: vec![
                    crate::commands::schema::DataKeyEntry {
                        variant: "Tx(u64)".into(),
                        kind: "instance entry".into(),
                        description: "Pending or executed transaction record for a given tx id.".into(),
                    },
                    crate::commands::schema::DataKeyEntry {
                        variant: "Owners".into(),
                        kind: "instance entry".into(),
                        description: "The wallet's owner set.".into(),
                    },
                    crate::commands::schema::DataKeyEntry {
                        variant: "Threshold".into(),
                        kind: "instance entry".into(),
                        description: "Approval threshold required to execute.".into(),
                    },
                    crate::commands::schema::DataKeyEntry {
                        variant: "Count".into(),
                        kind: "instance entry".into(),
                        description: "Monotonic transaction id counter.".into(),
                    },
                ],
                records: vec![crate::commands::schema::StoredRecord {
                    name: "WalletTx".into(),
                    storage_key: "DataKey::Tx(id)".into(),
                    fields: vec![
                        ("tx_id", "u64"),
                        ("submitter", "Address"),
                        ("transaction", "Bytes"),
                        ("status", "TxStatus"),
                        ("confirmations", "Vec<Address>"),
                        ("created_at", "u64"),
                    ]
                    .into_iter()
                    .map(|(name, ty)| (name.to_string(), ty.to_string()))
                    .collect(),
                }],
                status_enum: Some("TxStatus".into()),
                status_variant_schema: Some(vec![
                    "Pending".into(),
                    "Confirmed".into(),
                    "Executed".into(),
                    "Cancelled".into(),
                ]),
            },
            crate::commands::schema::ContractSchema {
                crate_name: "subscription-payments".into(),
                contract_name: "SubscriptionPayments".into(),
                storage_kind: "instance".into(),
                data_keys: vec![
                    crate::commands::schema::DataKeyEntry {
                        variant: "Subscription(u64)".into(),
                        kind: "instance entry".into(),
                        description: "Subscription record for a given subscription id.".into(),
                    },
                    crate::commands::schema::DataKeyEntry {
                        variant: "Count".into(),
                        kind: "instance entry".into(),
                        description: "Monotonic subscription id counter.".into(),
                    },
                ],
                records: vec![crate::commands::schema::StoredRecord {
                    name: "Subscription".into(),
                    storage_key: "DataKey::Subscription(id)".into(),
                    fields: vec![
                        ("subscription_id", "u64"),
                        ("subscriber", "Address"),
                        ("plan", "String"),
                        ("amount", "i128"),
                        ("interval", "u64"),
                        ("status", "SubscriptionStatus"),
                        ("created_at", "u64"),
                    ]
                    .into_iter()
                    .map(|(name, ty)| (name.to_string(), ty.to_string()))
                    .collect(),
                }],
                status_enum: Some("SubscriptionStatus".into()),
                status_variant_schema: Some(vec![
                    "Active".into(),
                    "Cancelled".into(),
                    "Expired".into(),
                ]),
            },
            crate::commands::schema::ContractSchema {
                crate_name: "vesting".into(),
                contract_name: "Vesting".into(),
                storage_kind: "instance".into(),
                data_keys: vec![
                    crate::commands::schema::DataKeyEntry {
                        variant: "Schedule(u64)".into(),
                        kind: "instance entry".into(),
                        description: "Vesting schedule record for a given schedule id.".into(),
                    },
                    crate::commands::schema::DataKeyEntry {
                        variant: "Count".into(),
                        kind: "instance entry".into(),
                        description: "Monotonic vesting schedule id counter.".into(),
                    },
                ],
                records: vec![crate::commands::schema::StoredRecord {
                    name: "VestingSchedule".into(),
                    storage_key: "DataKey::Schedule(id)".into(),
                    fields: vec![
                        ("schedule_id", "u64"),
                        ("beneficiary", "Address"),
                        ("token", "Address"),
                        ("total_amount", "i128"),
                        ("interval", "u64"),
                        ("interval_count", "u64"),
                        ("status", "VestingStatus"),
                        ("created_at", "u64"),
                    ]
                    .into_iter()
                    .map(|(name, ty)| (name.to_string(), ty.to_string()))
                    .collect(),
                }],
                status_enum: Some("VestingStatus".into()),
                status_variant_schema: Some(vec![
                    "Pending".into(),
                    "Active".into(),
                    "Completed".into(),
                    "Cancelled".into(),
                ]),
            },
        ],
    }
}

fn render_summary(registry: &crate::commands::schema::StorageRegistry) -> String {
    let mut md = String::new();
    md.push_str("# Contract Storage Schema Registry\n\n");
    md.push_str(&format!(
        "Generated from the six Soroban Forge contracts at {}\n\n",
        registry.generated_at
    ));
    md.push_str(
        "This registry mirrors the contract `DataKey` enums and the serialized record shapes\n",
    );
    md.push_str(
        "each contract persists in Soroban instance or persistent storage.\n\n",
    );
    md.push_str("Each contract below lists:\n\n");
    md.push_str("- `storage_kind`: whether records are stored in instance or persistent storage;\n");
    md.push_str(
        "- `data_keys`: every `DataKey` variant, with its kind and description;\n",
    );
    md.push_str(
        "- `records`: the main stored record shape for each contract, including field types;\n",
    );
    md.push_str(
        "- `status_enum`: the enum used for lifecycle state, when one exists;\n",
    );
    md.push_str(
        "- `status_variant_schema`: the variants of that status enum.\n\n",
    );
    md.push_str("## Contracts\n\n");
    for contract in &registry.contracts {
        md.push_str(&format!("### `{}`\n\n", contract.crate_name));
        md.push_str(&format!("- **Contract**: {}\n", contract.contract_name));
        md.push_str(&format!("- **Storage kind**: {}\n\n", contract.storage_kind));

        md.push_str("#### Data keys\n\n");
        md.push_str("| Variant | Kind | Description |\n");
        md.push_str("| --- | --- | --- |\n");
        for key in &contract.data_keys {
            md.push_str(&format!(
                "| `{}` | {} | {} |\n",
                key.variant, key.kind, key.description
            ));
        }
        md.push_str("\n");

        if !contract.records.is_empty() {
            md.push_str("#### Stored records\n\n");
            for record in &contract.records {
                md.push_str(&format!(
                    "**{}** stored at `{}\n\n",
                    record.name, record.storage_key
                ));
                md.push_str("| Field | Type |\n");
                md.push_str("| --- | --- |\n");
                for (name, ty) in &record.fields {
                    md.push_str(&format!("| {} | {} |\n", name, ty));
                }
                md.push_str("\n");
            }
        }

        if let Some(status) = &contract.status_enum {
            md.push_str(&format!("#### Status enum: {}\n\n", status));
            let variants: Vec<&str> = contract
                .status_variant_schema
                .as_ref()
                .map(|v| v.iter().map(|s| s.as_str()).collect())
                .unwrap_or_default();
            if variants.is_empty() {
                md.push_str("- (no variants enumerated in this registry)\n");
            } else {
                md.push_str("- ");
                md.push_str(&variants.join(" `, "));
                md.push_str(" `)\n");
            }
            md.push_str("\n");
        }
    }

    md
}

fn chrono_now_iso() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = now.as_secs();
    let days = total_secs / 86_400;
    let remaining = total_secs % 86_400;
    let hours = remaining / 3_600;
    let minutes = (remaining % 3_600) / 60;
    let seconds = remaining % 60;
    let (y, m, d) = days_to_ymd(days as i64);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y, m, d, hours, minutes, seconds
    )
}

fn days_to_ymd(days: i64) -> (i64, u32, u32) {
    let total = days + 719_468;
    let year = if total >= 0 {
        (total as f64 / 365.2425).floor() as i64
    } else {
        (total as f64 / 365.2425).ceil() as i64 - 1
    };
    let mut year = year.max(1);
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        let next_start = cumulative_days_before_year(year + 1);
        if total < next_start {
            break;
        }
        year += 1;
    }
    let start = cumulative_days_before_year(year);
    let doy = (total - start) as u32;
    let month = month_from_doy(year, doy);
    let day = doy - cumulative_days_before_month(year, month) + 1;
    (year, month, day)
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0) && (year % 100 != 0 || year % 400 == 0)
}

fn cumulative_days_before_year(year: i64) -> i64 {
    let y = year - 1;
    y * 365 + y / 4 - y / 100 + y / 400
}

fn month_from_doy(year: i64, doy: u32) -> u32 {
    let cumulative: [u32; 12] = [
        0,
        31,
        59,
        90,
        120,
        151,
        181,
        212,
        243,
        273,
        304,
        334,
    ];
    let leap_adj = if is_leap(year) && doy >= 59 { 1 } else { 0 };
    let mut month = 0;
    for (i, &c) in cumulative.iter().enumerate() {
        if doy >= c + leap_adj {
            month = i as u32;
        }
    }
    if month >= 11 {
        11
    } else {
        month + 1
    }
}

fn cumulative_days_before_month(year: i64, month: u32) -> u32 {
    let cumulative: [u32; 12] = [
        0,
        31,
        59,
        90,
        120,
        151,
        181,
        212,
        243,
        273,
        304,
        334,
    ];
    let leap_adj = if is_leap(year) && month > 2 { 1 } else { 0 };
    cumulative[(month - 1) as usize] + leap_adj
}
