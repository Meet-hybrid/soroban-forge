//! Vesting flow: a schedule is created with a cliff, then claimed mid-way
//! through the vesting window — the contract's primary transfer path.

use soroban_forge_test_utils::TestAccounts;
use soroban_forge_vesting::{SorobanForgeVestingClient, Vesting};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Env};

use crate::metrics::ResourceSample;

const START: u64 = 1_000_000;
const CLIFF: u64 = 1_000;
const DURATION: u64 = 4_000;
const TOTAL: i128 = 10_000;

/// Runs the flow and captures the resources of the `claim` invocation.
pub(crate) fn run() -> ResourceSample {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);

    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin);
    let token = sac.address();
    let token_admin = StellarAssetClient::new(&env, &token);

    let contract_id = env.register(Vesting, ());
    let client = SorobanForgeVestingClient::new(&env, &contract_id);
    let accounts = TestAccounts::generate(&env);
    // Top the schedule contract up with the full allocation, the way a
    // deployment is topped up before schedules run.
    token_admin.mint(&contract_id, &TOTAL);

    let schedule_id = client.create_schedule(
        &accounts.deployer,
        &accounts.user1,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );

    // Halfway through the vesting window (past the cliff): exactly half the
    // allocation is claimable.
    env.ledger()
        .set_timestamp(START + CLIFF + (DURATION - CLIFF) / 2);

    // The measured entrypoint.
    let claimed = client.claim(&schedule_id);
    let sample = ResourceSample::capture(&env);

    assert_eq!(claimed, TOTAL / 2);
    sample
}
