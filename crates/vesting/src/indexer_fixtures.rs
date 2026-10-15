//! Deterministic ground-truth events for vesting reassignment flows.

extern crate std;

use crate::{SorobanForgeVestingClient, Vesting};
use serde_json::Value;
use soroban_forge_test_utils::TestAccounts;
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::xdr::{self, WriteXdr};
use soroban_sdk::{Address, Env};
use std::format;
use std::path::Path;
use std::string::ToString;
use std::vec::Vec;

const XDR_LIMITS: xdr::Limits = xdr::Limits {
    depth: 500,
    len: 0x1_000_000,
};
const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../indexer/fixtures");
const FIXTURE_FILE: &str = "vesting-events.json";

fn capture(env: &Env, contract: &Address, events: &mut Vec<Value>) {
    for (index, event) in env.events().all().events().iter().enumerate() {
        let xdr::ContractEventBody::V0(body) = &event.body;
        events.push(serde_json::json!({
            "type": "contract",
            "ledger": 0,
            "ledgerClosedAt": "2026-09-23T00:00:00.000Z",
            "contractId": contract.to_string().to_string(),
            "id": format!("0-{index}"),
            "pagingToken": format!("0-{index}"),
            "inSuccessfulContractCall": true,
            "eventIndex": index,
            "transactionHash": "0000000000000000000000000000000000000000000000000000000000000000",
            "operationIndex": 0,
            "topic": body
                .topics
                .iter()
                .map(|v| v.to_xdr_base64(XDR_LIMITS).unwrap())
                .collect::<Vec<_>>(),
            "value": body.data.to_xdr_base64(XDR_LIMITS).unwrap(),
        }));
    }
}

fn generate() -> Value {
    let env = Env::default();
    env.mock_all_auths();
    let contract = env.register(Vesting, ());
    let client = SorobanForgeVestingClient::new(&env, &contract);
    let accounts = TestAccounts::generate(&env);
    let funder = accounts.user1;
    let beneficiary = accounts.user2;
    let new_beneficiary = accounts.user3;

    let schedule_id = client.create_schedule(
        &funder,
        &beneficiary,
        &env.register_stellar_asset_contract_v2(accounts.deployer.clone()).address(),
        &1_000_i128,
        &100_u64,
        &1_000_u64,
    );
    client.reassign_beneficiary(&schedule_id, &new_beneficiary);

    let mut events = Vec::new();
    capture(&env, &contract, &mut events);

    serde_json::json!({
        "schema": 1,
        "description": "Deterministic vesting reassignment events from Soroban Forge.",
        "contract": {
            "name": "soroban-forge-vesting",
            "id": contract.to_string().to_string(),
        },
        "events": events,
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
