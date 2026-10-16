//! DAO governance flow: a bonded proposal is voted on and executed — the
//! contract's primary lifecycle, ending in a cross-contract dispatch and
//! bond release.

use soroban_forge_dao_governance::{DaoGovernance, SorobanForgeDaoGovernanceClient};
use soroban_forge_test_utils::{MockTarget, MockTargetClient, TestAccounts};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Bytes, Env};

use crate::metrics::ResourceSample;

const START: u64 = 1_000_000;
const DURATION: u64 = 86_400;
/// Bond amount configured for every proposal in the fixture.
const BOND: i128 = 100;
/// Starting balance minted to the proposer (covers the bond and gives the
/// vote a non-zero weight).
const FUNDS: i128 = 10_000;

/// Runs the flow and captures the resources of the `execute` invocation.
pub(crate) fn run() -> ResourceSample {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);

    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin);
    let token = sac.address();
    let token_admin = StellarAssetClient::new(&env, &token);

    let contract_id = env.register(DaoGovernance, ());
    let client = SorobanForgeDaoGovernanceClient::new(&env, &contract_id);
    let accounts = TestAccounts::generate(&env);
    client.initialize(&token);
    client.configure_bond(&token, &BOND, &accounts.deployer);
    token_admin.mint(&accounts.user1, &FUNDS);

    let target_id = env.register(MockTarget, ());
    let action = Bytes::from_array(&env, &[0xC0, 0xDE, 0x00, 0xFF]);
    let no_deps = soroban_sdk::Vec::new(&env);
    let proposal_id = client.propose(
        &accounts.user1,
        &target_id,
        &action,
        &DURATION,
        &no_deps,
        &None,
    );
    client.vote(&proposal_id, &accounts.user1, &true);

    // Past the voting window: the proposal is ready for execution.
    env.ledger().set_timestamp(START + DURATION + 1);

    // Fixture step: the first `execute` finalises the past-deadline proposal
    // (`Active` -> `Succeeded`); it deliberately does not dispatch yet.
    client.execute(&proposal_id);

    // The measured entrypoint: the second `execute` dispatches the action to
    // the target contract and releases the bond to the proposer.
    client.execute(&proposal_id);
    let sample = ResourceSample::capture(&env);

    assert_eq!(MockTargetClient::new(&env, &target_id).count(), 1);
    sample
}
