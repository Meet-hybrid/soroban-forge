//! Escrow flow: buyer creates an escrow, deposits, and the seller's release
//! pays out — the contract's primary path through SEP-41 token settlement.

use soroban_forge_escrow::{Escrow, SorobanForgeEscrowClient};
use soroban_forge_test_utils::TestAccounts;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env};

use crate::metrics::ResourceSample;

const START: u64 = 1_000_000;
const TIMEOUT: u64 = 1_000;
const AMOUNT: i128 = 1_000;

/// Runs the flow and captures the resources of the `release` invocation.
pub(crate) fn run() -> ResourceSample {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);

    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin);
    let token = sac.address();
    let token_admin = StellarAssetClient::new(&env, &token);
    let token_client = TokenClient::new(&env, &token);

    let contract_id = env.register(Escrow, ());
    let client = SorobanForgeEscrowClient::new(&env, &contract_id);
    let accounts = TestAccounts::generate(&env);
    token_admin.mint(&accounts.user1, &AMOUNT);

    let escrow_id = client.create_escrow(
        &accounts.user1,
        &accounts.user2,
        &accounts.arbiter,
        &token,
        &AMOUNT,
        &TIMEOUT,
    );
    client.deposit(&escrow_id);

    // The measured entrypoint: release pays the seller out of custody.
    client.release(&escrow_id);
    let sample = ResourceSample::capture(&env);

    assert_eq!(token_client.balance(&accounts.user2), AMOUNT);
    sample
}
