//! Ground-truth fixtures for the reference event indexer.
//!
//! This unit test drives the real vesting contract through a live schedule and
//! reassignment flow, captures the events it actually emits, and serialises them
//! into `indexer/fixtures/vesting-events.json` — the fixture consumed by the
//! read-only indexer in `packages/typescript-sdk` (see
//! `indexer/docs/event-schema.md`).

extern crate std;

use crate::{SorobanForgeVestingClient, Vesting};
use soroban_forge_test_utils::TestAccounts;
use soroban_sdk::address_payload::AddressPayload;
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::xdr::{self, WriteXdr};
use soroban_sdk::{Address, Env};
use std::collections::HashMap;
use std::format;
use std::path::Path;
use std::string::{String, ToString};
use std::vec::Vec;

const START: u64 = 1_000_000;
const CLIFF: u64 = 1_000;
const DURATION: u64 = 4_000;
const TOTAL: i128 = 10_000;

const XDR_LIMITS: xdr::Limits = xdr::Limits {
    depth: 500,
    len: 0x1_000_000,
};

const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../indexer/fixtures");
const FIXTURE_FILE: &str = "vesting-events.json";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn contract_hash(addr: &Address) -> [u8; 32] {
    match addr.to_payload() {
        Some(AddressPayload::ContractIdHash(hash)) => hash.to_array(),
        _ => panic!("expected a contract address, got {}", addr.to_string()),
    }
}

fn ledger_closed_at(ledger: u64) -> String {
    format!(
        "2026-09-23T00:{:02}:{:02}.000Z",
        (ledger / 60) % 60,
        ledger % 60
    )
}

struct Builder {
    events: Vec<serde_json::Value>,
    contracts: Vec<([u8; 32], String)>,
    contract_label: String,
    ledger_slots: HashMap<u64, u64>,
}

impl Builder {
    fn new() -> Self {
        Builder {
            events: Vec::new(),
            contracts: Vec::new(),
            contract_label: String::new(),
            ledger_slots: HashMap::new(),
        }
    }

    fn set_contract(&mut self, addr: &Address) {
        self.contract_label = addr.to_string().to_string();
    }

    fn register_contract(&mut self, addr: &Address) {
        self.contracts
            .push((contract_hash(addr), addr.to_string().to_string()));
    }

    fn label_for(&self, id: Option<&xdr::ContractId>) -> String {
        id.and_then(|hash| {
            self.contracts
                .iter()
                .find(|(h, _)| h == &hash.0 .0)
                .map(|(_, label)| label.clone())
        })
        .unwrap_or_else(|| {
            id.map_or_else(
                || "C-----NONE".into(),
                |hash| format!("C-----UNKNOWN-{}", hex(&hash.0 .0)),
            )
        })
    }

    fn step<T>(
        &mut self,
        env: &Env,
        contract: &Address,
        ledger: u64,
        transaction_hash: &str,
        f: impl FnOnce() -> T,
    ) -> T {
        let contract_hash = contract_hash(contract);
        let result = f();
        let events = env.events().all().events().to_vec();
        let closed_at = ledger_closed_at(ledger);
        for (i, event) in events.iter().enumerate() {
            let xdr::ContractEventBody::V0(body) = &event.body;
            let slot = self.ledger_slots.entry(ledger).or_insert(0);
            let event_index = *slot;
            *slot += 1;
            let topics = body
                .topics
                .iter()
                .map(|t| t.to_xdr_base64(XDR_LIMITS).unwrap())
                .collect::<Vec<_>>();
            let value = body.data.to_xdr_base64(XDR_LIMITS).unwrap();
            let is_contract = matches!(&event.contract_id, Some(id) if id.0 .0 == contract_hash);
            let contract_id = if is_contract {
                self.contract_label.clone()
            } else {
                self.label_for(event.contract_id.as_ref())
            };
            self.events.push(serde_json::json!({
                "type": "contract",
                "ledger": ledger,
                "ledgerClosedAt": closed_at,
                "contractId": contract_id,
                "id": format!("{ledger}-{event_index}"),
                "pagingToken": format!("{ledger}-{i}"),
                "inSuccessfulContractCall": true,
                "eventIndex": event_index,
                "transactionHash": transaction_hash,
                "operationIndex": 0,
                "topic": topics,
                "value": value,
            }));
        }
        result
    }
}

fn rig() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);

    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let token = sac.address();
    let contract = env.register(Vesting, ());
    (env, contract, token)
}

fn generate() -> serde_json::Value {
    let mut builder = Builder::new();
    let (env, contract, token) = rig();
    let client = SorobanForgeVestingClient::new(&env, &contract);
    let accounts = TestAccounts::generate(&env);
    let token_admin = StellarAssetClient::new(&env, &token);

    builder.set_contract(&contract);
    builder.register_contract(&contract);
    builder.register_contract(&token);

    token_admin.mint(&contract, &TOTAL);

    let schedule_id = builder.step(&env, &contract, 1000, "0000000000001000", || {
        client.create_schedule(
            &accounts.deployer,
            &accounts.user1,
            &token,
            &TOTAL,
            &CLIFF,
            &DURATION,
        )
    });

    env.ledger().set_timestamp(START + (DURATION / 2));
    builder.step(&env, &contract, 1001, "0000000000001001", || {
        client.reassign_beneficiary(&schedule_id, &accounts.user2)
    });

    env.ledger().set_timestamp(START + DURATION + 1);
    builder.step(&env, &contract, 1002, "0000000000001002", || {
        client.claim(&schedule_id)
    });

    serde_json::json!({
        "schema": 1,
        "description": "Deterministic lifecycle fixtures emitted by the Soroban Forge vesting contract (soroban-sdk 27 testutils). Generated and validated by crates/vesting/src/indexer_fixtures.rs; consumed by the reference indexer tests in packages/typescript-sdk.",
        "contract": { "name": "soroban-forge-vesting", "id": contract.to_string().to_string() },
        "events": builder.events,
    })
}

#[test]
fn indexer_fixtures_regenerate_and_are_stable() {
    let fixture = generate();
    assert_eq!(
        fixture,
        generate(),
        "fixture generation is not deterministic"
    );
    let path = Path::new(FIXTURE_DIR).join(FIXTURE_FILE);
    std::fs::create_dir_all(FIXTURE_DIR).unwrap();
    let pretty = format!("{}\n", serde_json::to_string_pretty(&fixture).unwrap());
    if std::fs::read_to_string(&path).ok().as_deref() != Some(pretty.as_str()) {
        std::fs::write(&path, pretty).unwrap();
        panic!("fixture changed; review and commit {}", path.display());
    }
}
