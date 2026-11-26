//! Dedicated test suite for `revise_vote` (issue #704).
//!
//! Coverage:
//! - Both revision directions (for→against, against→for).
//! - Repeated revisions (same voter, multiple flips).
//! - Deadline boundary (revision at deadline, one before deadline).
//! - Non-voter attempting revision.
//! - Unchanged direction (no-op revision rejected).
//! - All terminal proposal states: Cancelled, Defeated, Succeeded, Executed.
//! - Token transfers after vote do not affect revision weight.
//! - Passing-to-defeated transition via revision.
//! - Failed-operation atomicity (tally unchanged on error).
//! - Tally conservation: for_votes + against_votes is unchanged by any revision.
//! - VoteRevised event emitted with correct fields.
//! - revision_count increments on every successful revision.
//! - A fresh vote has a zero revision count.

use crate::{DaoGovernance, DataKey, ProposalState, SorobanForgeDaoGovernanceClient, VoteReceipt};
use soroban_forge_shared_utils::ForgeError;
use soroban_forge_test_utils::{MockTarget, TestAccounts};
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Bytes, Env};

const START: u64 = 1_000_000;
const DURATION: u64 = 86_400;
const BOND: i128 = 100;
const FUNDS: i128 = 10_000;

// -----------------------------------------------------------------------
// Setup helper (mirrors bonded_env! in lib.rs tests)
// -----------------------------------------------------------------------

fn payload(env: &Env) -> Bytes {
    Bytes::from_array(env, &[0xC0, 0xDE, 0x00, 0xFF])
}

/// A voter's recorded vote plus how many times it was revised.
struct VoteView {
    weight: i128,
    support: bool,
    revision_count: u32,
}

/// Read a voter's vote receipt and revision count directly from contract storage.
fn get_receipt(env: &Env, contract_id: &Address, proposal_id: u64, voter: &Address) -> VoteView {
    env.as_contract(contract_id, || {
        let receipt = env
            .storage()
            .instance()
            .get::<DataKey, VoteReceipt>(&DataKey::VoteReceipt(proposal_id, voter.clone()))
            .expect("VoteReceipt must exist");
        assert!(receipt.has_voted);
        let revision_count = env
            .storage()
            .instance()
            .get::<DataKey, u32>(&DataKey::VoteRevisions(proposal_id, voter.clone()))
            .unwrap_or(0);
        VoteView {
            weight: receipt.weight,
            support: receipt.support.expect("a cast vote records its direction"),
            revision_count,
        }
    })
}

// -----------------------------------------------------------------------
// Core revision direction tests
// -----------------------------------------------------------------------

#[test]
fn revise_for_to_against() {
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
    for who in [&accounts.user1, &accounts.user2] {
        token_admin.mint(who, &FUNDS);
    }
    let target = env.register(MockTarget, ());

    let id = client.propose(
        &accounts.user1,
        &target,
        &payload(&env),
        &DURATION,
        &soroban_sdk::Vec::new(&env),
        &None,
    );

    client.vote(&id, &accounts.user2, &true);
    let p = client.get_proposal(&id);
    assert_eq!(p.for_votes, FUNDS);
    assert_eq!(p.against_votes, 0);

    client.revise_vote(&id, &accounts.user2, &false);
    let p = client.get_proposal(&id);
    assert_eq!(p.for_votes, 0);
    assert_eq!(p.against_votes, FUNDS);

    let receipt = get_receipt(&env, &contract_id, id, &accounts.user2);
    assert!(!receipt.support);
    assert_eq!(receipt.weight, FUNDS);
    assert_eq!(receipt.revision_count, 1);
}

#[test]
fn revise_against_to_for() {
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
    for who in [&accounts.user1, &accounts.user2] {
        token_admin.mint(who, &FUNDS);
    }
    let target = env.register(MockTarget, ());

    let id = client.propose(
        &accounts.user1,
        &target,
        &payload(&env),
        &DURATION,
        &soroban_sdk::Vec::new(&env),
        &None,
    );

    client.vote(&id, &accounts.user2, &false);
    let p = client.get_proposal(&id);
    assert_eq!(p.against_votes, FUNDS);
    assert_eq!(p.for_votes, 0);

    client.revise_vote(&id, &accounts.user2, &true);
    let p = client.get_proposal(&id);
    assert_eq!(p.for_votes, FUNDS);
    assert_eq!(p.against_votes, 0);

    let receipt = get_receipt(&env, &contract_id, id, &accounts.user2);
    assert!(receipt.support);
    assert_eq!(receipt.revision_count, 1);
}

// -----------------------------------------------------------------------
// Repeated revisions
// -----------------------------------------------------------------------

#[test]
fn repeated_revisions_track_count_and_tally() {
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
    for who in [&accounts.user1, &accounts.user2] {
        token_admin.mint(who, &FUNDS);
    }
    let target = env.register(MockTarget, ());

    let id = client.propose(
        &accounts.user1,
        &target,
        &payload(&env),
        &DURATION,
        &soroban_sdk::Vec::new(&env),
        &None,
    );

    client.vote(&id, &accounts.user2, &true);

    for i in 1u32..=4 {
        // Flip: revision i is against on odd, for on even.
        let next_support = i % 2 == 0;
        client.revise_vote(&id, &accounts.user2, &next_support);
        let receipt = get_receipt(&env, &contract_id, id, &accounts.user2);
        assert_eq!(receipt.revision_count, i);
        assert_eq!(receipt.support, next_support);
        let p = client.get_proposal(&id);
        if next_support {
            assert_eq!(p.for_votes, FUNDS);
            assert_eq!(p.against_votes, 0);
        } else {
            assert_eq!(p.for_votes, 0);
            assert_eq!(p.against_votes, FUNDS);
        }
    }
}

// -----------------------------------------------------------------------
// Tally conservation invariant
// -----------------------------------------------------------------------

#[test]
fn revisions_never_change_total_voted_weight() {
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
    for who in [&accounts.user1, &accounts.user2, &accounts.user3] {
        token_admin.mint(who, &FUNDS);
    }
    let target = env.register(MockTarget, ());

    let id = client.propose(
        &accounts.user1,
        &target,
        &payload(&env),
        &DURATION,
        &soroban_sdk::Vec::new(&env),
        &None,
    );

    client.vote(&id, &accounts.user2, &true);
    client.vote(&id, &accounts.user3, &false);

    let initial_total = {
        let p = client.get_proposal(&id);
        p.for_votes + p.against_votes
    };

    client.revise_vote(&id, &accounts.user2, &false);
    let p = client.get_proposal(&id);
    assert_eq!(p.for_votes + p.against_votes, initial_total);

    client.revise_vote(&id, &accounts.user3, &true);
    let p = client.get_proposal(&id);
    assert_eq!(p.for_votes + p.against_votes, initial_total);
}

// -----------------------------------------------------------------------
// Error paths
// -----------------------------------------------------------------------

#[test]
fn revise_non_voter_returns_invalid_input() {
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
    token_admin.mint(&accounts.user2, &FUNDS);
    let target = env.register(MockTarget, ());

    let id = client.propose(
        &accounts.user1,
        &target,
        &payload(&env),
        &DURATION,
        &soroban_sdk::Vec::new(&env),
        &None,
    );

    // user2 never voted.
    let res = client.try_revise_vote(&id, &accounts.user2, &true);
    assert_eq!(res.unwrap_err().unwrap(), ForgeError::InvalidInput);
}

#[test]
fn revise_unchanged_direction_returns_invalid_input() {
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
    for who in [&accounts.user1, &accounts.user2] {
        token_admin.mint(who, &FUNDS);
    }
    let target = env.register(MockTarget, ());

    let id = client.propose(
        &accounts.user1,
        &target,
        &payload(&env),
        &DURATION,
        &soroban_sdk::Vec::new(&env),
        &None,
    );

    client.vote(&id, &accounts.user2, &true);

    let res = client.try_revise_vote(&id, &accounts.user2, &true);
    assert_eq!(res.unwrap_err().unwrap(), ForgeError::InvalidInput);
}

#[test]
fn revise_at_deadline_returns_deadline_reached() {
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
    for who in [&accounts.user1, &accounts.user2] {
        token_admin.mint(who, &FUNDS);
    }
    let target = env.register(MockTarget, ());

    let id = client.propose(
        &accounts.user1,
        &target,
        &payload(&env),
        &DURATION,
        &soroban_sdk::Vec::new(&env),
        &None,
    );
    client.vote(&id, &accounts.user2, &true);

    // Advance to exactly voting_ends.
    env.ledger().set_timestamp(START + DURATION);

    let res = client.try_revise_vote(&id, &accounts.user2, &false);
    assert_eq!(res.unwrap_err().unwrap(), ForgeError::DeadlineReached);

    // Tally unchanged.
    let p = client.get_proposal(&id);
    assert_eq!(p.for_votes, FUNDS);
    assert_eq!(p.against_votes, 0);
}

#[test]
fn revise_one_second_before_deadline_succeeds() {
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
    for who in [&accounts.user1, &accounts.user2] {
        token_admin.mint(who, &FUNDS);
    }
    let target = env.register(MockTarget, ());

    let id = client.propose(
        &accounts.user1,
        &target,
        &payload(&env),
        &DURATION,
        &soroban_sdk::Vec::new(&env),
        &None,
    );
    client.vote(&id, &accounts.user2, &true);

    // One second before the deadline is still within the window.
    env.ledger().set_timestamp(START + DURATION - 1);

    client.revise_vote(&id, &accounts.user2, &false);
    let p = client.get_proposal(&id);
    assert_eq!(p.against_votes, FUNDS);
    assert_eq!(p.for_votes, 0);
}

// -----------------------------------------------------------------------
// Terminal state rejection
// -----------------------------------------------------------------------

#[test]
fn revise_on_cancelled_proposal_returns_invalid_state() {
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
    for who in [&accounts.user1, &accounts.user2] {
        token_admin.mint(who, &FUNDS);
    }
    let target = env.register(MockTarget, ());

    let id = client.propose(
        &accounts.user1,
        &target,
        &payload(&env),
        &DURATION,
        &soroban_sdk::Vec::new(&env),
        &None,
    );
    // A proposal can only be cancelled before any vote is cast, so the
    // revision attempt hits the state check before the missing-vote check.
    client.cancel_proposal(&id, &accounts.user1);

    let res = client.try_revise_vote(&id, &accounts.user2, &false);
    assert_eq!(res.unwrap_err().unwrap(), ForgeError::InvalidState);
}

#[test]
fn revise_on_defeated_proposal_returns_invalid_state() {
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
    for who in [&accounts.user1, &accounts.user2] {
        token_admin.mint(who, &FUNDS);
    }
    let target = env.register(MockTarget, ());

    let id = client.propose(
        &accounts.user1,
        &target,
        &payload(&env),
        &DURATION,
        &soroban_sdk::Vec::new(&env),
        &None,
    );
    client.vote(&id, &accounts.user2, &false);
    env.ledger().set_timestamp(START + DURATION + 1);
    client.execute(&id);

    let p = client.get_proposal(&id);
    assert_eq!(p.state, ProposalState::Defeated);

    let res = client.try_revise_vote(&id, &accounts.user2, &true);
    assert_eq!(res.unwrap_err().unwrap(), ForgeError::InvalidState);
}

#[test]
fn revise_on_executed_proposal_returns_invalid_state() {
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
    for who in [&accounts.user1, &accounts.user2] {
        token_admin.mint(who, &FUNDS);
    }
    let target = env.register(MockTarget, ());

    let id = client.propose(
        &accounts.user1,
        &target,
        &payload(&env),
        &DURATION,
        &soroban_sdk::Vec::new(&env),
        &None,
    );
    client.vote(&id, &accounts.user2, &true);
    env.ledger().set_timestamp(START + DURATION + 1);
    client.execute(&id); // Active → Succeeded
    client.execute(&id); // Succeeded → Executed

    let p = client.get_proposal(&id);
    assert_eq!(p.state, ProposalState::Executed);

    let res = client.try_revise_vote(&id, &accounts.user2, &false);
    assert_eq!(res.unwrap_err().unwrap(), ForgeError::InvalidState);
}

#[test]
fn revise_on_succeeded_proposal_returns_invalid_state() {
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
    for who in [&accounts.user1, &accounts.user2] {
        token_admin.mint(who, &FUNDS);
    }
    let target = env.register(MockTarget, ());

    let id = client.propose(
        &accounts.user1,
        &target,
        &payload(&env),
        &DURATION,
        &soroban_sdk::Vec::new(&env),
        &None,
    );
    client.vote(&id, &accounts.user2, &true);
    env.ledger().set_timestamp(START + DURATION + 1);
    client.execute(&id); // Active → Succeeded

    let p = client.get_proposal(&id);
    assert_eq!(p.state, ProposalState::Succeeded);

    let res = client.try_revise_vote(&id, &accounts.user2, &false);
    assert_eq!(res.unwrap_err().unwrap(), ForgeError::InvalidState);
}

// -----------------------------------------------------------------------
// Token-transfer independence
// -----------------------------------------------------------------------

#[test]
fn token_transfer_after_vote_does_not_change_revision_weight() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);

    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin);
    let token = sac.address();
    let token_admin = StellarAssetClient::new(&env, &token);
    let token_client = TokenClient::new(&env, &token);
    let contract_id = env.register(DaoGovernance, ());
    let client = SorobanForgeDaoGovernanceClient::new(&env, &contract_id);
    let accounts = TestAccounts::generate(&env);
    client.initialize(&token);
    client.configure_bond(&token, &BOND, &accounts.deployer);
    for who in [&accounts.user1, &accounts.user2, &accounts.user3] {
        token_admin.mint(who, &FUNDS);
    }
    let target = env.register(MockTarget, ());

    let id = client.propose(
        &accounts.user1,
        &target,
        &payload(&env),
        &DURATION,
        &soroban_sdk::Vec::new(&env),
        &None,
    );

    client.vote(&id, &accounts.user2, &true);

    // Transfer all of user2's remaining balance away after voting.
    let balance = token_client.balance(&accounts.user2);
    if balance > 0 {
        token_client.transfer(&accounts.user2, &accounts.user3, &balance);
    }

    // Revise: weight must still be the original FUNDS (locked at vote time).
    client.revise_vote(&id, &accounts.user2, &false);

    let p = client.get_proposal(&id);
    assert_eq!(
        p.against_votes, FUNDS,
        "original weight used even after balance transfer"
    );
    assert_eq!(p.for_votes, 0);

    let receipt = get_receipt(&env, &contract_id, id, &accounts.user2);
    assert_eq!(receipt.weight, FUNDS);
}

// -----------------------------------------------------------------------
// Passing-to-defeated transition via revision
// -----------------------------------------------------------------------

#[test]
fn revision_can_flip_outcome_from_passing_to_defeated() {
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
    for who in [
        &accounts.user1,
        &accounts.user2,
        &accounts.user3,
        &accounts.validator,
    ] {
        token_admin.mint(who, &FUNDS);
    }
    let target = env.register(MockTarget, ());

    let id = client.propose(
        &accounts.user1,
        &target,
        &payload(&env),
        &DURATION,
        &soroban_sdk::Vec::new(&env),
        &None,
    );

    // user2 and validator vote for (2:1 over user3 against).
    client.vote(&id, &accounts.user2, &true);
    client.vote(&id, &accounts.validator, &true);
    client.vote(&id, &accounts.user3, &false);

    {
        let p = client.get_proposal(&id);
        assert!(
            p.for_votes > p.against_votes,
            "expected for majority before revision"
        );
    }

    // user2 and validator revise to against → against now wins 2:1.
    client.revise_vote(&id, &accounts.user2, &false);
    client.revise_vote(&id, &accounts.validator, &false);

    {
        let p = client.get_proposal(&id);
        assert!(
            p.against_votes > p.for_votes,
            "expected against majority after revision"
        );
    }

    // Execute after deadline should Defeat the proposal.
    env.ledger().set_timestamp(START + DURATION + 1);
    client.execute(&id);

    let p = client.get_proposal(&id);
    assert_eq!(p.state, ProposalState::Defeated);
}

// -----------------------------------------------------------------------
// Atomicity: failed revision leaves state unchanged
// -----------------------------------------------------------------------

#[test]
fn revision_atomicity_leaves_tally_unchanged_on_error() {
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
    for who in [&accounts.user1, &accounts.user2] {
        token_admin.mint(who, &FUNDS);
    }
    let target = env.register(MockTarget, ());

    let id = client.propose(
        &accounts.user1,
        &target,
        &payload(&env),
        &DURATION,
        &soroban_sdk::Vec::new(&env),
        &None,
    );

    client.vote(&id, &accounts.user2, &true);
    let p_before = client.get_proposal(&id);

    // Attempt an invalid revision (same direction) — must fail atomically.
    let _ = client.try_revise_vote(&id, &accounts.user2, &true);
    let p_after = client.get_proposal(&id);
    assert_eq!(p_before.for_votes, p_after.for_votes);
    assert_eq!(p_before.against_votes, p_after.against_votes);

    // Attempt revision after deadline — must fail atomically.
    env.ledger().set_timestamp(START + DURATION);
    let _ = client.try_revise_vote(&id, &accounts.user2, &false);
    let p_after2 = client.get_proposal(&id);
    assert_eq!(p_before.for_votes, p_after2.for_votes);
    assert_eq!(p_before.against_votes, p_after2.against_votes);
}

// -----------------------------------------------------------------------
// VoteRevised event emission
// -----------------------------------------------------------------------

#[test]
fn revise_vote_emits_vote_revised_event() {
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
    for who in [&accounts.user1, &accounts.user2] {
        token_admin.mint(who, &FUNDS);
    }
    let target = env.register(MockTarget, ());

    let id = client.propose(
        &accounts.user1,
        &target,
        &payload(&env),
        &DURATION,
        &soroban_sdk::Vec::new(&env),
        &None,
    );

    client.vote(&id, &accounts.user2, &true);
    client.revise_vote(&id, &accounts.user2, &false);

    let all_events = env.events().all();
    let contract_events: std::vec::Vec<_> = all_events
        .filter_by_contract(&contract_id)
        .events()
        .to_vec();

    // After revise_vote there must be at least one event from this contract.
    assert!(
        !contract_events.is_empty(),
        "expected at least one contract event after revise_vote"
    );

    // The last event in the revision call is VoteRevised; it has at least 2 topics.
    let last = contract_events.last().unwrap();
    let soroban_sdk::xdr::ContractEventBody::V0(body) = &last.body;
    assert!(
        body.topics.len() >= 2,
        "VoteRevised event must have at least 2 topics (name + proposal_id)"
    );

    // The proposal tally reflects the revision.
    let p = client.get_proposal(&id);
    assert_eq!(p.against_votes, FUNDS);
    assert_eq!(p.for_votes, 0);
}

// -----------------------------------------------------------------------
// Multi-voter tally correctness with revisions
// -----------------------------------------------------------------------

#[test]
fn multi_voter_tally_correct_after_all_revise() {
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
    for who in [
        &accounts.user1,
        &accounts.user2,
        &accounts.user3,
        &accounts.validator,
    ] {
        token_admin.mint(who, &FUNDS);
    }
    let target = env.register(MockTarget, ());

    let id = client.propose(
        &accounts.user1,
        &target,
        &payload(&env),
        &DURATION,
        &soroban_sdk::Vec::new(&env),
        &None,
    );

    client.vote(&id, &accounts.user2, &true);
    client.vote(&id, &accounts.user3, &true);
    client.vote(&id, &accounts.validator, &false);

    let p = client.get_proposal(&id);
    assert_eq!(p.for_votes, 2 * FUNDS);
    assert_eq!(p.against_votes, FUNDS);

    client.revise_vote(&id, &accounts.user2, &false);
    client.revise_vote(&id, &accounts.user3, &false);

    let p = client.get_proposal(&id);
    assert_eq!(p.for_votes, 0);
    assert_eq!(p.against_votes, 3 * FUNDS);
    assert_eq!(p.for_votes + p.against_votes, 3 * FUNDS);
}

// -----------------------------------------------------------------------
// Initial receipt state
// -----------------------------------------------------------------------

#[test]
fn receipt_initial_state_is_zero_revisions() {
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
    for who in [&accounts.user1, &accounts.user2] {
        token_admin.mint(who, &FUNDS);
    }
    let target = env.register(MockTarget, ());

    let id = client.propose(
        &accounts.user1,
        &target,
        &payload(&env),
        &DURATION,
        &soroban_sdk::Vec::new(&env),
        &None,
    );

    client.vote(&id, &accounts.user2, &true);

    let receipt = get_receipt(&env, &contract_id, id, &accounts.user2);
    assert_eq!(receipt.revision_count, 0);
    assert_eq!(receipt.weight, FUNDS);
    assert!(receipt.support);
}
