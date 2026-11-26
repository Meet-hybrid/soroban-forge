//! Escrow test suite.
//!
//! Covers the full lifecycle against a real Stellar Asset Contract (SAC)
//! fixture, so every balance assertion reflects actual token movement —
//! the same failure modes a live deployment would hit (missing balance,
//! failed transfer, double payout).
//!
//! Partial-release coverage includes: valid partial, multiple partials,
//! exact final partial (→ Completed), zero/negative amounts, over-remaining,
//! non-Funded state, after-completion rejection, partial→refund,
//! partial→dispute→resolve (both directions), storage compat, and conservation.
//!
//! NOTE on authorization coverage: the suite runs under `mock_all_auths`,
//! which proves the *call graph* of authorizations (who the contract
//! asks to sign) but not that a wrong signer is rejected. The one
//! logic-level access control that is enforceable without auth mocking —
//! the `dispute` claimant check, which runs before any `require_auth` —
//! is tested directly (`dispute_by_outsider_is_rejected`). Full negative
//! signature testing needs `set_auths` fixtures and is tracked in the
//! security-invariant backlog.

// The test harness links std even in a no_std crate; the event-decode test
// below needs `std::vec::Vec` for its filtered event list.
extern crate std;

use crate::{
    BasketEscrowData, Bytes, DataKey, Escrow, EscrowAsset, EscrowData, EscrowDataV1, EscrowDataV2,
    EscrowStatus, ReleaseRequest, ReleaseRequestState, SorobanForgeEscrowClient,
    MAX_ACCEPTANCE_DEADLINE, MAX_RELEASE_WINDOW,
};
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::xdr::{ContractEventBody, ScVal};
use soroban_sdk::{Address, Env, TryFromVal, Vec};

const START: u64 = 1_000_000;
const TIMEOUT: u64 = 1_000;
const AMOUNT: i128 = 1_000;

/// Fresh env: mocked auths, a SAC token with a mintable admin, the escrow
/// contract, and named accounts. Returns the pieces tests name.
macro_rules! setup {
    () => {{
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(START);

        let admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(admin.clone());
        let token = sac.address();
        let token_admin = StellarAssetClient::new(&env, &token);
        let token_client = TokenClient::new(&env, &token);

        let contract_id = env.register(Escrow, ());
        let client = SorobanForgeEscrowClient::new(&env, &contract_id);

        let accounts = soroban_forge_test_utils::TestAccounts::generate(&env);
        // Buyer starts funded; everyone else starts at zero.
        token_admin.mint(&accounts.user1, &AMOUNT);

        (env, token, token_client, contract_id, client, accounts)
    }};
}

fn create(
    client: &SorobanForgeEscrowClient<'_>,
    token: &soroban_sdk::Address,
    buyer: &soroban_sdk::Address,
    seller: &soroban_sdk::Address,
    arbiter: &soroban_sdk::Address,
    timeout: u64,
) -> u64 {
    client.create_escrow(buyer, seller, arbiter, token, &AMOUNT, &timeout, &None)
}

/// Fresh env with **two** mintable SAC tokens for the basket paths (the
/// mirror of the same macro in `authz.rs`: user1 is minted both tokens so a
/// basket can be funded).
macro_rules! setup_basket {
    () => {{
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(START);

        let admin = Address::generate(&env);
        let sac_a = env.register_stellar_asset_contract_v2(admin.clone());
        let token_a = sac_a.address();
        let sac_b = env.register_stellar_asset_contract_v2(admin);
        let token_b = sac_b.address();

        let contract_id = env.register(Escrow, ());
        let client = SorobanForgeEscrowClient::new(&env, &contract_id);

        let accounts = soroban_forge_test_utils::TestAccounts::generate(&env);
        // user1 funds baskets whose legs are AMOUNT (token A) + 2 * AMOUNT
        // (token B): mint a generous pool so multi-basket tests always
        // succeed.
        StellarAssetClient::new(&env, &token_a).mint(&accounts.user1, &(100 * AMOUNT));
        StellarAssetClient::new(&env, &token_b).mint(&accounts.user1, &(100 * AMOUNT));

        (env, token_a, token_b, contract_id, client, accounts)
    }};
}

/// Default party layout: user1 buys, user2 sells, arbiter arbitrates.
fn parties(
    accounts: &soroban_forge_test_utils::TestAccounts,
) -> (
    &soroban_sdk::Address,
    &soroban_sdk::Address,
    &soroban_sdk::Address,
) {
    (&accounts.user1, &accounts.user2, &accounts.arbiter)
}

/// Assert that a full (non-paged) read of a participant's index equals
/// `expected`, exactly in creation order, with a complete list (no next
/// cursor).
fn assert_full_index(client: &SorobanForgeEscrowClient<'_>, party: &Address, expected: &[u64]) {
    let limit = u32::try_from(expected.len()).expect("test lists fit in u32");
    let page = client.escrows_for_participant(party, &0, &limit);
    assert_eq!(page.total, limit, "index must hold the whole expected list");
    assert_eq!(page.ids.len(), limit);
    for (n, want) in expected.iter().enumerate() {
        let i = u32::try_from(n).expect("index fits in u32");
        assert_eq!(
            page.ids.get_unchecked(i),
            *want,
            "creation-order position {i}"
        );
    }
    assert_eq!(page.next_cursor, None);
}

// -----------------------------------------------------------------------
// Creation
// -----------------------------------------------------------------------

#[test]
fn create_escrow_records_parties_and_stays_pending() {
    let (_env, _token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &_token, buyer, seller, arbiter, TIMEOUT);

    let record: EscrowData = client.get_escrow(&id);
    assert_eq!(record.escrow_id, id);
    assert_eq!(&record.buyer, buyer);
    assert_eq!(&record.seller, seller);
    assert_eq!(&record.arbiter, arbiter);
    assert_eq!(record.amount, AMOUNT);
    assert_eq!(record.timeout, TIMEOUT);
    assert_eq!(record.created_at, START);
    assert_eq!(client.get_status(&id), EscrowStatus::Pending);
}

#[test]
fn escrow_ids_are_monotonic() {
    let (_env, _token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let first = create(&client, &_token, buyer, seller, arbiter, TIMEOUT);
    let second = create(&client, &_token, buyer, seller, arbiter, TIMEOUT);
    assert_eq!(first + 1, second);
}

#[test]
fn create_rejects_zero_amount() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let err = client
        .try_create_escrow(buyer, seller, arbiter, &token, &0, &TIMEOUT, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

#[test]
fn create_rejects_zero_timeout() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let err = client
        .try_create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &0, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

// -----------------------------------------------------------------------
// Deposit
// -----------------------------------------------------------------------

#[test]
fn deposit_moves_tokens_into_the_contract() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);

    client.deposit(&id);

    assert_eq!(tc.balance(buyer), 0);
    assert_eq!(tc.balance(&contract_id), AMOUNT);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn deposit_twice_is_rejected_and_moves_nothing_more() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    let err = client.try_deposit(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(tc.balance(&contract_id), AMOUNT);
}

#[test]
fn deposit_with_insufficient_balance_fails_cleanly() {
    // user3 holds no tokens; create an escrow they cannot fund.
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let poor = &accounts.user3;
    let (_, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, poor, seller, arbiter, TIMEOUT);

    let err = client.try_deposit(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::TokenTransferFailed);
    assert_eq!(tc.balance(&contract_id), 0);
    // State untouched: still Pending, retryable after topping up.
    assert_eq!(client.get_status(&id), EscrowStatus::Pending);
}

// -----------------------------------------------------------------------
// Release / refund
// -----------------------------------------------------------------------

#[test]
fn release_pays_the_seller() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.release(&id);

    assert_eq!(tc.balance(seller), AMOUNT);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Completed);
}

#[test]
fn release_requires_funded_state() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);

    let err = client.try_release(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(tc.balance(&contract_id), 0);
}

#[test]
fn release_cannot_run_twice() {
    let (_env, token, tc, _contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);
    client.release(&id);

    let err = client.try_release(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(tc.balance(seller), AMOUNT);
}

#[test]
fn refund_by_seller_before_deadline() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.refund(&id);

    assert_eq!(tc.balance(buyer), AMOUNT);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
}

#[test]
fn refund_by_buyer_after_deadline() {
    let (env, token, tc, _contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    env.ledger().set_timestamp(START + TIMEOUT + 1);
    client.refund(&id);

    assert_eq!(tc.balance(buyer), AMOUNT);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
}

#[test]
fn refund_requires_funded_state() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);

    let err = client.try_refund(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

// -----------------------------------------------------------------------
// Dispute flow
// -----------------------------------------------------------------------

#[test]
fn dispute_by_buyer_claimant() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.dispute(&id, buyer, &None);

    assert_eq!(client.get_status(&id), EscrowStatus::Disputed);
}

#[test]
fn dispute_by_seller_claimant() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.dispute(&id, seller, &None);

    assert_eq!(client.get_status(&id), EscrowStatus::Disputed);
}

#[test]
fn dispute_by_outsider_is_rejected() {
    // The claimant check is a pure value test that runs before any
    // require_auth, so this negative case is enforceable even under
    // mocked auths.
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    let err = client
        .try_dispute(&id, &accounts.user3, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn dispute_requires_funded_state() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);

    let err = client.try_dispute(&id, buyer, &None).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

#[test]
fn disputed_escrow_freezes_all_payouts() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);
    client.dispute(&id, buyer, &None);

    assert_eq!(
        client.try_release(&id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(
        client.try_refund(&id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(tc.balance(&contract_id), AMOUNT);
    assert_eq!(client.get_status(&id), EscrowStatus::Disputed);
}

#[test]
fn resolve_in_favor_of_seller_pays_out() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);
    client.dispute(&id, seller, &None);

    client.resolve(&id, &true);

    assert_eq!(tc.balance(seller), AMOUNT);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Completed);
}

#[test]
fn resolve_in_favor_of_buyer_refunds() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);
    client.dispute(&id, buyer, &None);

    client.resolve(&id, &false);

    assert_eq!(tc.balance(buyer), AMOUNT);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
}

#[test]
fn resolve_requires_disputed_state() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    let err = client.try_resolve(&id, &true).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

#[test]
fn resolve_split_pays_seller_and_buyer_in_proportion() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);
    client.dispute(&id, buyer, &None);

    client.resolve_split(&id, &5000);

    assert_eq!(tc.balance(seller), 500);
    assert_eq!(tc.balance(buyer), 500);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Completed);
}

#[test]
fn resolve_split_zero_share_refunds_buyer() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);
    client.dispute(&id, buyer, &None);

    client.resolve_split(&id, &0);

    assert_eq!(tc.balance(seller), 0);
    assert_eq!(tc.balance(buyer), AMOUNT);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
}

#[test]
fn resolve_split_rejects_invalid_basis_points() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);
    client.dispute(&id, buyer, &None);

    let err = client.try_resolve_split(&id, &10_001).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(tc.balance(&contract_id), AMOUNT);
    assert_eq!(client.get_status(&id), EscrowStatus::Disputed);
}

#[test]
fn resolve_split_uses_only_the_balance_remaining_after_partial_release() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);
    client.release_partial(&id, &200);
    client.dispute(&id, buyer, &None);

    client.resolve_split(&id, &5000);

    assert_eq!(tc.balance(seller), 600);
    assert_eq!(tc.balance(buyer), 400);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Completed);
    assert_eq!(client.get_escrow(&id).released, AMOUNT);
}

// -----------------------------------------------------------------------
// Cancel
// -----------------------------------------------------------------------

#[test]
fn cancel_pending_escrow() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    let second = create(&client, &token, buyer, seller, arbiter, TIMEOUT);

    client.cancel(&id);

    assert_eq!(client.get_status(&id), EscrowStatus::Cancelled);
    for party in [buyer, seller, arbiter] {
        assert_full_index(&client, party, &[second]);
    }
}

#[test]
fn cancel_removes_shared_role_index_once_and_double_cancel_is_noop() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, _seller, _arbiter) = parties(&accounts);
    let shared = &accounts.user3;
    let id = create(&client, &token, buyer, shared, shared, TIMEOUT);
    let other = create(&client, &token, buyer, shared, shared, TIMEOUT);

    client.cancel(&id);
    assert_full_index(&client, buyer, &[other]);
    assert_full_index(&client, shared, &[other]);

    let err = client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_full_index(&client, buyer, &[other]);
    assert_full_index(&client, shared, &[other]);
}

#[test]
fn failed_cancel_keeps_all_participant_indexes_unchanged() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    let err = client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    for party in [buyer, seller, arbiter] {
        assert_full_index(&client, party, &[id]);
    }
}

#[test]
fn other_terminal_transitions_keep_participant_index_entries() {
    let (env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    StellarAssetClient::new(&env, &token).mint(buyer, &(AMOUNT * 2));

    let released = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    let refunded = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    let resolved = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&released);
    client.release(&released);
    client.deposit(&refunded);
    client.refund(&refunded);
    client.deposit(&resolved);
    client.dispute(&resolved, buyer, &None);
    client.resolve(&resolved, &false);

    for party in [buyer, seller, arbiter] {
        assert_full_index(&client, party, &[released, refunded, resolved]);
    }
}

#[test]
fn cancel_after_deposit_is_rejected() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    let err = client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(tc.balance(&contract_id), AMOUNT);
}

// -----------------------------------------------------------------------
// Missing ids
// -----------------------------------------------------------------------

#[test]
fn missing_escrow_reads_are_not_found() {
    let (_env, _token, _tc, _id, client, _accounts) = setup!();
    assert_eq!(
        client.try_get_status(&999).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
    assert_eq!(
        client.try_get_escrow(&999).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
}

#[test]
fn operations_on_missing_escrow_are_not_found() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let outsider = &accounts.user3;
    assert_eq!(
        client.try_deposit(&999).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
    assert_eq!(
        client.try_release(&999).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
    assert_eq!(
        client.try_refund(&999).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
    assert_eq!(
        client
            .try_dispute(&999, outsider, &None)
            .unwrap_err()
            .unwrap(),
        ForgeError::NotFound
    );
    assert_eq!(
        client.try_resolve(&999, &true).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
    assert_eq!(
        client.try_cancel(&999).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
    assert_eq!(
        client.try_touch_ttl(&999).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
    let _ = (token, buyer, seller, arbiter);
}

// -----------------------------------------------------------------------
// Participant index views (escrows_for_participant)
// -----------------------------------------------------------------------

#[test]
fn create_indexes_all_distinct_parties() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let first = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    let second = create(&client, &token, buyer, seller, arbiter, TIMEOUT);

    for party in [buyer, seller, arbiter] {
        let page = client.escrows_for_participant(party, &0, &10);
        assert_eq!(page.total, 2);
        assert_eq!(page.ids.len(), 2);
        assert_eq!(page.ids.get_unchecked(0), first);
        assert_eq!(page.ids.get_unchecked(1), second);
        assert_eq!(page.next_cursor, None);
    }
}

#[test]
fn repeated_role_address_is_indexed_once() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, _seller, _arbiter) = parties(&accounts);
    // The same address wears two roles (seller and arbiter): it must be
    // indexed once, or iteration would yield this id twice.
    let shared = &accounts.user3;
    let id = create(&client, &token, buyer, shared, shared, TIMEOUT);

    assert_full_index(&client, buyer, &[id]);
    let page = client.escrows_for_participant(shared, &0, &10);
    assert_eq!(page.total, 1, "one address in two roles is indexed once");
    assert_eq!(page.ids.get_unchecked(0), id);
}

#[test]
fn unknown_address_returns_empty_page() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    create(&client, &token, buyer, seller, arbiter, TIMEOUT);

    // An address this contract has never seen yields an empty result, not
    // an error.
    let page = client.escrows_for_participant(&accounts.validator, &0, &10);
    assert_eq!(page.total, 0);
    assert_eq!(page.ids.len(), 0);
    assert_eq!(page.next_cursor, None);
}

#[test]
fn index_is_creation_order_with_party_subsets() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let other_seller = &accounts.user3;

    let id1 = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    let id2 = create(&client, &token, buyer, other_seller, arbiter, TIMEOUT);
    let id3 = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    let id4 = create(&client, &token, buyer, other_seller, arbiter, TIMEOUT);

    // The arbiter appears in every escrow, in creation order.
    assert_full_index(&client, arbiter, &[id1, id2, id3, id4]);
    // Each seller sees only the escrows it is a party to, still ordered.
    assert_full_index(&client, seller, &[id1, id3]);
    assert_full_index(&client, other_seller, &[id2, id4]);
}

#[test]
fn pagination_returns_sliced_pages_with_cursors() {
    let (env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let mut expected = soroban_sdk::vec![&env];
    for _ in 0..10 {
        expected.push_back(create(&client, &token, buyer, seller, arbiter, TIMEOUT));
    }

    // A mid-list page: offset 1, limit 3 returns the 2nd..4th ids.
    let page = client.escrows_for_participant(buyer, &1, &3);
    assert_eq!(page.total, 10);
    assert_eq!(page.ids.len(), 3);
    assert_eq!(page.ids.get_unchecked(0), expected.get_unchecked(1));
    assert_eq!(page.ids.get_unchecked(2), expected.get_unchecked(3));
    assert_eq!(page.next_cursor, Some(4));

    // A page that exactly exhausts the list: one id, then no next cursor.
    let page = client.escrows_for_participant(buyer, &9, &1);
    assert_eq!(page.ids.len(), 1);
    assert_eq!(page.ids.get_unchecked(0), expected.get_unchecked(9));
    assert_eq!(page.next_cursor, None);

    // An over-run past the end is empty and terminal, not an error.
    let page = client.escrows_for_participant(buyer, &12, &5);
    assert_eq!(page.total, 10);
    assert_eq!(page.ids.len(), 0);
    assert_eq!(page.next_cursor, None);

    let page = client.escrows_for_participant(buyer, &u32::MAX, &u32::MAX);
    assert_eq!(page.total, 10);
    assert_eq!(page.ids.len(), 0);
    assert_eq!(page.next_cursor, None);
}

#[test]
fn pagination_cursor_is_live_offset_and_can_skip_after_earlier_cancel() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let ids = [
        create(&client, &token, buyer, seller, arbiter, TIMEOUT),
        create(&client, &token, buyer, seller, arbiter, TIMEOUT),
        create(&client, &token, buyer, seller, arbiter, TIMEOUT),
        create(&client, &token, buyer, seller, arbiter, TIMEOUT),
        create(&client, &token, buyer, seller, arbiter, TIMEOUT),
    ];

    let first = client.escrows_for_participant(buyer, &0, &2);
    assert_eq!(first.ids.get_unchecked(0), ids[0]);
    assert_eq!(first.ids.get_unchecked(1), ids[1]);
    assert_eq!(first.next_cursor, Some(2));

    client.cancel(&ids[0]);
    let continued = client.escrows_for_participant(buyer, &2, &2);
    assert_eq!(continued.ids.get_unchecked(0), ids[3]);
    assert_eq!(continued.ids.get_unchecked(1), ids[4]);

    // The saved live offset skipped ids[2]; restarting sees every surviving
    // id in creation order without duplicates.
    let restarted = client.escrows_for_participant(buyer, &0, &u32::MAX);
    assert_eq!(restarted.ids.len(), 4);
    assert_eq!(restarted.ids.get_unchecked(0), ids[1]);
    assert_eq!(restarted.ids.get_unchecked(1), ids[2]);
    assert_eq!(restarted.ids.get_unchecked(2), ids[3]);
    assert_eq!(restarted.ids.get_unchecked(3), ids[4]);
}

#[test]
fn pagination_iterates_every_created_id_exactly_once() {
    let (env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let mut expected = soroban_sdk::vec![&env];
    // Well past 25 so paging crosses several page boundaries; also keeps
    // this test inside CI's budget (each create is a single small write).
    for _ in 0..26 {
        expected.push_back(create(&client, &token, buyer, seller, arbiter, TIMEOUT));
    }

    let mut seen = soroban_sdk::vec![&env];
    let mut cursor = 0u32;
    let limit = 7u32;
    loop {
        let page = client.escrows_for_participant(buyer, &cursor, &limit);
        assert_eq!(page.total, 26, "total is reported on every page");
        assert!(
            page.ids.len() <= limit,
            "a page may not exceed its requested limit"
        );
        for n in 0..page.ids.len() {
            seen.push_back(page.ids.get_unchecked(n));
        }
        match page.next_cursor {
            Some(next) => {
                assert!(next > cursor, "pagination must advance past the page");
                cursor = next;
            }
            None => break,
        }
    }

    assert_eq!(
        seen, expected,
        "full iteration must yield every created id exactly once, in creation order"
    );
    assert_eq!(seen.len(), 26);
}

#[test]
fn zero_limit_returns_empty_page_without_residual_cursor() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    create(&client, &token, buyer, seller, arbiter, TIMEOUT);

    // A zero limit must not report a next cursor pointing at itself, or a
    // naive client would loop forever.
    let page = client.escrows_for_participant(buyer, &0, &0);
    assert_eq!(page.total, 1);
    assert_eq!(page.ids.len(), 0);
    assert_eq!(page.next_cursor, None);
}

#[test]
fn escrows_for_participant_is_read_only() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    let second = create(&client, &token, buyer, seller, arbiter, TIMEOUT);

    // Repeating the view must not change its answer: no state is touched
    // between two identical reads.
    let before = client.escrows_for_participant(buyer, &0, &10);
    let after = client.escrows_for_participant(buyer, &0, &10);
    assert_eq!(before, after);
    assert_eq!(after.ids.len(), 2);

    // And the id counter must not have advanced: the next escrow takes the
    // id right after the last created one, proving the view wrote nothing.
    let third = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    assert_eq!(third, second + 1);
}

// -----------------------------------------------------------------------
// TTL keeper
// -----------------------------------------------------------------------

#[test]
fn touch_ttl_extends_and_keeps_state_intact() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.touch_ttl(&id);

    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

// -----------------------------------------------------------------------
// Conservation property: every payout path returns exactly the deposit
// -----------------------------------------------------------------------

// -----------------------------------------------------------------------
// Partial release
// -----------------------------------------------------------------------

#[test]
fn release_partial_pays_seller_and_updates_accounting() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.release_partial(&id, &300);

    // 300 should have moved to seller.
    assert_eq!(tc.balance(seller), 300);
    // Contract still holds the remaining 700.
    assert_eq!(tc.balance(&contract_id), 700);
    // Escrow is still Funded.
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
    // released and remaining accounting.
    let record: EscrowData = client.get_escrow(&id);
    assert_eq!(record.released, 300);
    assert_eq!(record.remaining(), 700);
}

#[test]
fn multiple_partial_releases_accumulate_correctly() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.release_partial(&id, &200);
    client.release_partial(&id, &300);
    client.release_partial(&id, &100);

    assert_eq!(tc.balance(seller), 600);
    assert_eq!(tc.balance(&contract_id), 400);
    let record: EscrowData = client.get_escrow(&id);
    assert_eq!(record.released, 600);
    assert_eq!(record.remaining(), 400);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn exact_final_partial_release_completes_escrow() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.release_partial(&id, &400);
    // Release the exact remaining amount.
    client.release_partial(&id, &600);

    assert_eq!(tc.balance(seller), AMOUNT);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Completed);
    let record: EscrowData = client.get_escrow(&id);
    assert_eq!(record.released, AMOUNT);
    assert_eq!(record.remaining(), 0);
}

#[test]
fn release_partial_zero_amount_is_rejected() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    let err = client.try_release_partial(&id, &0).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(tc.balance(&contract_id), AMOUNT);
    assert_eq!(tc.balance(seller), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn release_partial_negative_amount_is_rejected() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    let err = client.try_release_partial(&id, &-100).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(tc.balance(&contract_id), AMOUNT);
    assert_eq!(tc.balance(seller), 0);
}

#[test]
fn release_partial_exceeding_remaining_is_rejected() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.release_partial(&id, &400);

    // Try to release more than what's left (remaining = 600).
    let err = client.try_release_partial(&id, &601).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    // Balances unchanged after rejected call.
    assert_eq!(tc.balance(seller), 400);
    assert_eq!(tc.balance(&contract_id), 600);
}

#[test]
fn release_partial_on_pending_escrow_is_rejected() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);

    let err = client.try_release_partial(&id, &100).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

#[test]
fn release_partial_after_completion_is_rejected() {
    let (_env, token, tc, _contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);
    // Complete the escrow via full partial release.
    client.release_partial(&id, &AMOUNT);

    assert_eq!(client.get_status(&id), EscrowStatus::Completed);
    let err = client.try_release_partial(&id, &1).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(tc.balance(seller), AMOUNT);
}

#[test]
fn partial_release_then_refund_pays_only_remaining() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.release_partial(&id, &400);
    // Seller refunds the remaining 600 to buyer.
    client.refund(&id);

    assert_eq!(tc.balance(seller), 400);
    assert_eq!(tc.balance(buyer), 600);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
}

#[test]
fn partial_release_then_dispute_then_resolve_for_seller() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.release_partial(&id, &300);
    client.dispute(&id, buyer, &None);
    // Arbiter resolves in favor of seller: remaining 700 goes to seller.
    client.resolve(&id, &true);

    assert_eq!(tc.balance(seller), AMOUNT); // 300 partial + 700 resolved
    assert_eq!(tc.balance(buyer), 0);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Completed);
}

#[test]
fn partial_release_then_dispute_then_resolve_for_buyer() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.release_partial(&id, &300);
    client.dispute(&id, seller, &None);
    // Arbiter resolves in favor of buyer: remaining 700 goes back to buyer.
    client.resolve(&id, &false);

    assert_eq!(tc.balance(seller), 300);
    assert_eq!(tc.balance(buyer), 700);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
}

#[test]
fn full_release_after_partial_releases_remaining_only() {
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.release_partial(&id, &250);
    // Call the full release (should pay only the remaining 750).
    client.release(&id);

    assert_eq!(tc.balance(seller), AMOUNT);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Completed);
}

#[test]
fn storage_compat_new_records_have_released_zero() {
    // Verify that a freshly created record has released = 0 and
    // remaining == amount.
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    let record: EscrowData = client.get_escrow(&id);
    assert_eq!(record.released, 0);
    assert_eq!(record.remaining(), AMOUNT);
    assert_eq!(record.amount, AMOUNT);
}

#[test]
fn release_partial_conservation_holds() {
    // After multiple partial releases the sum buyer+seller+contract must
    // always equal AMOUNT (the mint).
    let (_env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    let amounts = [100i128, 200, 300, 400];
    let mut total_paid = 0i128;
    for &amt in &amounts {
        // Only release if it doesn't exceed remaining.
        let record: EscrowData = client.get_escrow(&id);
        if amt <= record.remaining() {
            client.release_partial(&id, &amt);
            total_paid += amt;
            assert_eq!(
                tc.balance(buyer) + tc.balance(seller) + tc.balance(&contract_id),
                AMOUNT,
                "conservation must hold after each partial release"
            );
        }
    }
    let record: EscrowData = client.get_escrow(&id);
    assert_eq!(record.released, total_paid);
}

/// For every reachable terminal path × timeout combination, the contract
/// ends holding exactly zero and the parties' combined balance equals the
/// original mint: `buyer_start == buyer_end + seller_end`. One escrow, one
/// payout, so the pooled balance must return to zero on every path.
// The path table's function-pointer type is verbose by design — it reads
// as a table of scenarios, not a data structure to be abstracted.
#[allow(clippy::type_complexity)]
#[test]
fn conservation_holds_on_every_terminal_path() {
    let paths: &[&dyn Fn(
        &Env,
        &SorobanForgeEscrowClient<'_>,
        u64,
        &Address,
        &Address,
        &Address,
    )] = &[
        // Release to the seller.
        &|_env, client, id, _buyer, _seller, _arbiter| {
            client.release(&id);
        },
        // Seller refund before the deadline.
        &|_env, client, id, _buyer, _seller, _arbiter| {
            client.refund(&id);
        },
        // Buyer reclaim after the deadline.
        &|env, client, id, _buyer, _seller, _arbiter| {
            env.ledger().set_timestamp(START + TIMEOUT + 1);
            client.refund(&id);
        },
        // Dispute raised by the buyer; arbiter sides with the seller.
        &|_env, client, id, buyer, _seller, _arbiter| {
            client.dispute(&id, buyer, &None);
            client.resolve(&id, &true);
        },
        // Dispute raised by the seller; arbiter sides with the buyer.
        &|_env, client, id, _buyer, seller, _arbiter| {
            client.dispute(&id, seller, &None);
            client.resolve(&id, &false);
        },
        // Partial release (half) then full release of remainder.
        &|_env, client, id, _buyer, _seller, _arbiter| {
            client.release_partial(&id, &(AMOUNT / 2));
            client.release(&id);
        },
        // Partial release (partial) then refund of remaining.
        &|_env, client, id, _buyer, _seller, _arbiter| {
            client.release_partial(&id, &(AMOUNT / 3));
            client.refund(&id);
        },
        // Partial release then dispute then resolve for seller.
        &|_env, client, id, buyer, _seller, _arbiter| {
            client.release_partial(&id, &(AMOUNT / 4));
            client.dispute(&id, buyer, &None);
            client.resolve(&id, &true);
        },
        // Partial release then dispute then resolve for buyer.
        &|_env, client, id, _buyer, seller, _arbiter| {
            client.release_partial(&id, &(AMOUNT / 4));
            client.dispute(&id, seller, &None);
            client.resolve(&id, &false);
        },
        // Exact final partial release → Completed.
        &|_env, client, id, _buyer, _seller, _arbiter| {
            client.release_partial(&id, &(AMOUNT / 2));
            client.release_partial(&id, &(AMOUNT - AMOUNT / 2));
        },
    ];

    for timeout in [1u64, 10, TIMEOUT, 100_000] {
        for (i, path) in paths.iter().enumerate() {
            let (env, token, tc, contract_id, client, accounts) = setup!();
            let (buyer, seller, arbiter) = parties(&accounts);
            let id = create(&client, &token, buyer, seller, arbiter, timeout);
            client.deposit(&id);

            path(&env, &client, id, buyer, seller, arbiter);

            assert_eq!(
                tc.balance(&contract_id),
                0,
                "path {i}: contract must not retain dust (timeout {timeout})"
            );
            assert_eq!(
                tc.balance(buyer) + tc.balance(seller),
                AMOUNT,
                "path {i}: buyer+seller must sum to the deposit (timeout {timeout})"
            );
            assert_eq!(
                client.try_deposit(&id).unwrap_err().unwrap(),
                ForgeError::InvalidInput,
                "path {i}: terminal escrow must not be refundable again"
            );
        }
    }
}

// -----------------------------------------------------------------------
// Issue #729 — Atomic basket settlement summaries
// -----------------------------------------------------------------------

/// Two-leg basket assets: leg A is `AMOUNT` of `token_a`, leg B is
/// `2 * AMOUNT` of `token_b` (creation order: A then B).
fn basket_legs(env: &Env, token_a: &Address, token_b: &Address) -> Vec<EscrowAsset> {
    let mut assets = Vec::new(env);
    assets.push_back(EscrowAsset {
        token: token_a.clone(),
        amount: AMOUNT,
        released: 0,
    });
    assets.push_back(EscrowAsset {
        token: token_b.clone(),
        amount: 2 * AMOUNT,
        released: 0,
    });
    assets
}

/// Create + fund a basket over [`basket_legs`] with the default deadline.
#[allow(clippy::too_many_arguments)]
fn basket_fund(
    env: &Env,
    client: &SorobanForgeEscrowClient<'_>,
    token_a: &Address,
    token_b: &Address,
    buyer: &Address,
    seller: &Address,
    arbiter: &Address,
) -> u64 {
    let id = client.create_basket(
        buyer,
        seller,
        arbiter,
        &basket_legs(env, token_a, token_b),
        &TIMEOUT,
        &None,
    );
    client.deposit_basket(&id);
    id
}

#[test]
fn basket_summary_of_new_basket_reports_full_hold() {
    // "Empty" case: a funded basket with no settlement yet — every component
    // is entirely held, nothing released or refunded.
    let (env, token_a, token_b, _contract_id, client, accounts) = setup_basket!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = basket_fund(&env, &client, &token_a, &token_b, buyer, seller, arbiter);

    let summary = client.get_basket_summary(&id);
    assert_eq!(summary.escrow_id, id);
    assert_eq!(summary.status, EscrowStatus::Funded);
    assert_eq!(summary.acceptance_deadline, None);
    assert_eq!(summary.total_amount, AMOUNT + 2 * AMOUNT);
    assert_eq!(summary.held, AMOUNT + 2 * AMOUNT);
    assert_eq!(summary.released, 0);
    assert_eq!(summary.refunded, 0);
    assert_eq!(summary.components.len(), 2);

    let leg_a = summary.components.get_unchecked(0);
    assert_eq!(leg_a.token, token_a);
    assert_eq!(leg_a.amount, AMOUNT);
    assert_eq!(leg_a.held, AMOUNT);
    assert_eq!(leg_a.released, 0);
    assert_eq!(leg_a.refunded, 0);

    let leg_b = summary.components.get_unchecked(1);
    assert_eq!(leg_b.token, token_b);
    assert_eq!(leg_b.amount, 2 * AMOUNT);
    assert_eq!(leg_b.held, 2 * AMOUNT);
    assert_eq!(leg_b.released, 0);
    assert_eq!(leg_b.refunded, 0);
}

#[test]
fn basket_summary_accounts_partial_release_per_component() {
    let (env, token_a, token_b, _contract_id, client, accounts) = setup_basket!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = basket_fund(&env, &client, &token_a, &token_b, buyer, seller, arbiter);

    client.release_partial_basket(&id, &token_a, &300);

    let summary = client.get_basket_summary(&id);
    // Leg A: 700 held, 300 released. Leg B: fully held.
    assert_eq!(summary.held, 700 + 2 * AMOUNT);
    assert_eq!(summary.released, 300);
    assert_eq!(summary.refunded, 0);
    assert_eq!(summary.total_amount, AMOUNT + 2 * AMOUNT);
    // Disjoint partition: held + released == total.
    assert_eq!(summary.held + summary.released, summary.total_amount);

    let leg_a = summary.components.get_unchecked(0);
    assert_eq!(leg_a.held, 700);
    assert_eq!(leg_a.released, 300);
}

#[test]
fn basket_summary_on_mixed_terminal_states() {
    let (env, token_a, token_b, _contract_id, client, accounts) = setup_basket!();
    let (buyer, seller, arbiter) = parties(&accounts);

    // 1. Completed via `release_basket`.
    let id_released = basket_fund(&env, &client, &token_a, &token_b, buyer, seller, arbiter);
    client.release_basket(&id_released);
    let summary = client.get_basket_summary(&id_released);
    assert_eq!(summary.status, EscrowStatus::Completed);
    assert_eq!(summary.held, 0);
    assert_eq!(summary.released, AMOUNT + 2 * AMOUNT);
    assert_eq!(summary.refunded, 0);

    // 2. Refunded via `refund_basket`.
    let id_refunded = basket_fund(&env, &client, &token_a, &token_b, buyer, seller, arbiter);
    client.refund_basket(&id_refunded);
    let summary = client.get_basket_summary(&id_refunded);
    assert_eq!(summary.status, EscrowStatus::Refunded);
    assert_eq!(summary.held, 0);
    // All value came back to the buyer: reported as refunded, never as
    // released — the two are disjoint.
    assert_eq!(summary.released, 0);
    assert_eq!(summary.refunded, AMOUNT + 2 * AMOUNT);

    // 3. Completed via `resolve_basket(in_favor_of_seller)`.
    let id_resolved_seller = basket_fund(&env, &client, &token_a, &token_b, buyer, seller, arbiter);
    client.dispute_basket(&id_resolved_seller, buyer, &None);
    client.resolve_basket(&id_resolved_seller, &true);
    let summary = client.get_basket_summary(&id_resolved_seller);
    assert_eq!(summary.status, EscrowStatus::Completed);
    assert_eq!(summary.released, AMOUNT + 2 * AMOUNT);
    assert_eq!(summary.refunded, 0);

    // 4. Refunded via `resolve_basket(in_favor_of_buyer)`.
    let id_resolved_buyer = basket_fund(&env, &client, &token_a, &token_b, buyer, seller, arbiter);
    client.dispute_basket(&id_resolved_buyer, seller, &None);
    client.resolve_basket(&id_resolved_buyer, &false);
    let summary = client.get_basket_summary(&id_resolved_buyer);
    assert_eq!(summary.status, EscrowStatus::Refunded);
    assert_eq!(summary.held, 0);
    assert_eq!(summary.released, 0);
    assert_eq!(summary.refunded, AMOUNT + 2 * AMOUNT);
}

#[test]
fn basket_summary_equals_sum_of_component_escrow_values() {
    // The summary is derived from the basket's component records: per-leg
    // `released`/`remaining` are summed into the totals.
    let (env, token_a, token_b, _contract_id, client, accounts) = setup_basket!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = basket_fund(&env, &client, &token_a, &token_b, buyer, seller, arbiter);

    client.release_partial_basket(&id, &token_a, &300);
    client.dispute_basket(&id, buyer, &None);

    let basket = client.get_basket(&id);
    let summary = client.get_basket_summary(&id);
    let mut sum_held = 0_i128;
    let mut sum_released = 0_i128;
    for n in 0..basket.assets.len() {
        let leg = basket.assets.get_unchecked(n);
        sum_held += leg.remaining();
        sum_released += leg.released;
        assert_eq!(summary.components.get_unchecked(n).amount, leg.amount);
        assert_eq!(summary.components.get_unchecked(n).released, leg.released);
        assert_eq!(summary.components.get_unchecked(n).held, leg.remaining());
    }
    assert_eq!(summary.held, sum_held);
    assert_eq!(summary.released, sum_released);
    assert_eq!(summary.held + summary.released, summary.total_amount);
}

#[test]
fn basket_summary_rejects_corrupt_or_missing_components() {
    let (env, token_a, token_b, contract_id, client, accounts) = setup_basket!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = basket_fund(&env, &client, &token_a, &token_b, buyer, seller, arbiter);

    // Tamper: leg A claims to have released more than its amount. The
    // summary must reject the component rather than silently miscount it.
    env.as_contract(&contract_id, || {
        let mut basket: BasketEscrowData = env
            .storage()
            .persistent()
            .get(&DataKey::Basket(id))
            .expect("basket exists");
        let mut legs = Vec::new(&env);
        for leg in basket.assets.iter() {
            legs.push_back(EscrowAsset {
                token: leg.token.clone(),
                amount: leg.amount,
                released: leg.amount + 1,
            });
        }
        basket.assets = legs;
        env.storage()
            .persistent()
            .set(&DataKey::Basket(id), &basket);
    });
    assert_eq!(
        client.try_get_basket_summary(&id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );

    // A missing basket is a NotFound, never an empty summary.
    assert_eq!(
        client.try_get_basket_summary(&999).unwrap_err().unwrap(),
        ForgeError::NotFound
    );
}

#[test]
fn basket_summary_is_read_only() {
    let (env, token_a, token_b, _contract_id, client, accounts) = setup_basket!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = basket_fund(&env, &client, &token_a, &token_b, buyer, seller, arbiter);
    client.release_partial_basket(&id, &token_a, &300);

    // Repeat the query: identical answers on every call.
    let summary_a = client.get_basket_summary(&id);
    let summary_b = client.get_basket_summary(&id);
    assert_eq!(summary_a, summary_b);
    assert_eq!(summary_a.held, 700 + 2 * AMOUNT);
    assert_eq!(summary_a.released, 300);

    // The underlying basket record was not rewritten by the queries.
    let basket: BasketEscrowData = client.get_basket(&id);
    assert_eq!(basket.status, EscrowStatus::Funded);
    assert_eq!(basket.assets.get_unchecked(0).released, 300);
    assert_eq!(basket.assets.get_unchecked(0).remaining(), 700);
    assert_eq!(basket.assets.get_unchecked(1).released, 0);

    // No state changed: the id counter was not advanced by the views, so
    // the next escrow id is the one right after `id`.
    let next = create(&client, &token_a, buyer, seller, arbiter, TIMEOUT);
    assert_eq!(next, id + 1);
}

// -----------------------------------------------------------------------
// Issue #728 — Seller acceptance deadlines for funded escrows
// -----------------------------------------------------------------------

#[test]
fn create_escrow_rejects_zero_acceptance_deadline() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let err = client
        .try_create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT, &Some(0))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

#[test]
fn create_escrow_rejects_deadline_exceeding_cap() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    // One second past the documented cap is rejected.
    let too_far = START + MAX_ACCEPTANCE_DEADLINE + 1;
    let err = client
        .try_create_escrow(
            buyer,
            seller,
            arbiter,
            &token,
            &AMOUNT,
            &TIMEOUT,
            &Some(too_far),
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    // At exactly the cap it is valid: deadline <= created_at + MAX.
    let at_cap = START + MAX_ACCEPTANCE_DEADLINE;
    client.create_escrow(
        buyer,
        seller,
        arbiter,
        &token,
        &AMOUNT,
        &TIMEOUT,
        &Some(at_cap),
    );
}

#[test]
fn create_escrow_records_valid_acceptance_deadline() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let deadline = START + 1_000;
    let id = client.create_escrow(
        buyer,
        seller,
        arbiter,
        &token,
        &AMOUNT,
        &TIMEOUT,
        &Some(deadline),
    );
    let record: EscrowData = client.get_escrow(&id);
    assert_eq!(record.acceptance_deadline, Some(deadline));
}

#[test]
fn refund_expired_rejected_before_acceptance_deadline() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let deadline = START + 100;
    let id = client.create_escrow(
        buyer,
        seller,
        arbiter,
        &token,
        &AMOUNT,
        &TIMEOUT,
        &Some(deadline),
    );
    client.deposit(&id);

    // One second before the deadline the buyer cannot reclaim yet.
    env.ledger().set_timestamp(deadline - 1);
    let err = client.try_refund_expired(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::DeadlineReached);
    // Failed call left the state untouched.
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
    assert_eq!(tc.balance(&contract_id), AMOUNT);
    assert_eq!(tc.balance(buyer), 0);
}

#[test]
fn refund_expired_at_deadline_succeeds() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let deadline = START + 100;
    let id = client.create_escrow(
        buyer,
        seller,
        arbiter,
        &token,
        &AMOUNT,
        &TIMEOUT,
        &Some(deadline),
    );
    client.deposit(&id);

    // Boundary: "at and after the deadline" — `now >= acceptance_deadline`.
    env.ledger().set_timestamp(deadline);
    client.refund_expired(&id);

    assert_eq!(tc.balance(buyer), AMOUNT);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
}

#[test]
fn refund_expired_after_deadline_succeeds() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let deadline = START + 100;
    let id = client.create_escrow(
        buyer,
        seller,
        arbiter,
        &token,
        &AMOUNT,
        &TIMEOUT,
        &Some(deadline),
    );
    client.deposit(&id);

    env.ledger().set_timestamp(deadline + 1);
    client.refund_expired(&id);

    assert_eq!(tc.balance(buyer), AMOUNT);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
}

#[test]
fn refund_expired_cannot_run_twice() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let deadline = START + 100;
    let id = client.create_escrow(
        buyer,
        seller,
        arbiter,
        &token,
        &AMOUNT,
        &TIMEOUT,
        &Some(deadline),
    );
    client.deposit(&id);

    env.ledger().set_timestamp(deadline);
    client.refund_expired(&id);
    assert_eq!(tc.balance(buyer), AMOUNT);

    // Repeated deadline refunds are rejected on the terminal status.
    let err = client.try_refund_expired(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
}

#[test]
fn deadline_refunds_pay_only_the_still_held_amount() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let deadline = START + 100;
    let id = client.create_escrow(
        buyer,
        seller,
        arbiter,
        &token,
        &AMOUNT,
        &TIMEOUT,
        &Some(deadline),
    );
    client.deposit(&id);
    // A partial release moves 300 to the seller before the deadline.
    client.release_partial(&id, &300);

    env.ledger().set_timestamp(deadline + 1);
    client.refund_expired(&id);

    // The buyer is refunded the still-held amount only: 300 already paid to
    // the seller and never refunded (released and refunded never overlap).
    assert_eq!(tc.balance(seller), 300);
    assert_eq!(tc.balance(buyer), AMOUNT - 300);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
    // The escrow's `released` stays at the seller's 300; the buyer's 700 is
    // a refund of the still-held balance, never double-counted as released.
    let record: EscrowData = client.get_escrow(&id);
    assert_eq!(record.released, 300);
}

#[test]
fn refund_expired_legacy_boundary_is_unchanged() {
    // Without an acceptance deadline the legacy rules stand: strictly after
    // `created_at + timeout`, the buyer may reclaim.
    let (env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    env.ledger().set_timestamp(START + TIMEOUT); // exactly at the deadline
    assert_eq!(
        client.try_refund_expired(&id).unwrap_err().unwrap(),
        ForgeError::DeadlineReached
    );
    env.ledger().set_timestamp(START + TIMEOUT + 1);
    client.refund_expired(&id);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
}

#[test]
fn basket_deadline_refund_expired_pays_buyer() {
    let (env, token_a, token_b, contract_id, client, accounts) = setup_basket!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let deadline = START + 100;
    let id = client.create_basket(
        buyer,
        seller,
        arbiter,
        &basket_legs(&env, &token_a, &token_b),
        &TIMEOUT,
        &Some(deadline),
    );
    client.deposit_basket(&id);

    // Rejected before the deadline; allowed at and after it.
    env.ledger().set_timestamp(deadline - 1);
    assert_eq!(
        client.try_refund_expired(&id).unwrap_err().unwrap(),
        ForgeError::DeadlineReached
    );
    let tc_a = TokenClient::new(&env, &token_a);
    let tc_b = TokenClient::new(&env, &token_b);
    let buyer_a_before = tc_a.balance(buyer);
    let buyer_b_before = tc_b.balance(buyer);
    env.ledger().set_timestamp(deadline);
    client.refund_expired(&id);

    // The buyer gets every leg's still-held amount back; the contract holds
    // nothing of either token.
    assert_eq!(tc_a.balance(&contract_id), 0);
    assert_eq!(tc_b.balance(&contract_id), 0);
    assert_eq!(tc_a.balance(buyer), buyer_a_before + AMOUNT);
    assert_eq!(tc_b.balance(buyer), buyer_b_before + 2 * AMOUNT);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
    // Repeated attempts fail on the terminal status.
    assert_eq!(
        client.try_refund_expired(&id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
}

#[test]
fn basket_deadline_zero_and_over_cap_are_rejected() {
    let (env, token_a, token_b, _contract_id, client, accounts) = setup_basket!();
    let (buyer, seller, arbiter) = parties(&accounts);

    let err = client
        .try_create_basket(
            buyer,
            seller,
            arbiter,
            &basket_legs(&env, &token_a, &token_b),
            &TIMEOUT,
            &Some(0),
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);

    let err = client
        .try_create_basket(
            buyer,
            seller,
            arbiter,
            &basket_legs(&env, &token_a, &token_b),
            &TIMEOUT,
            &Some(START + MAX_ACCEPTANCE_DEADLINE + 1),
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

#[test]
fn basket_summary_reports_acceptance_deadline() {
    let (env, token_a, token_b, _contract_id, client, accounts) = setup_basket!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let deadline = START + 100;
    let id = client.create_basket(
        buyer,
        seller,
        arbiter,
        &basket_legs(&env, &token_a, &token_b),
        &TIMEOUT,
        &Some(deadline),
    );
    client.deposit_basket(&id);

    let summary = client.get_basket_summary(&id);
    assert_eq!(summary.acceptance_deadline, Some(deadline));
    assert_eq!(summary.status, EscrowStatus::Funded);
    assert_eq!(summary.held, 3 * AMOUNT);
}

// -----------------------------------------------------------------------
// Issue #727 — Tamper-evident dispute evidence commitments
// -----------------------------------------------------------------------

/// A fixed non-empty evidence commitment (a stand-in "hash").
fn commitment_bytes(env: &Env) -> Bytes {
    Bytes::from_slice(env, &[0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE])
}

#[test]
fn dispute_stores_evidence_commitment() {
    let (env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    let commitment = commitment_bytes(&env);
    client.dispute(&id, buyer, &Some(commitment.clone()));

    // Exposed through the escrow query.
    let record: EscrowData = client.get_escrow(&id);
    assert_eq!(record.evidence_commitment, Some(commitment));
}

#[test]
fn dispute_without_commitment_stores_none() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.dispute(&id, buyer, &None);

    let record: EscrowData = client.get_escrow(&id);
    assert_eq!(record.evidence_commitment, None);
    assert_eq!(client.get_status(&id), EscrowStatus::Disputed);
}

#[test]
fn dispute_commitment_is_a_deterministic_round_trip() {
    let (env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    let commitment = commitment_bytes(&env);
    client.dispute(&id, buyer, &Some(commitment.clone()));

    // Reading the record twice yields the same commitment (no mutation or
    // lossy transformation on the way in or out).
    assert_eq!(
        client.get_escrow(&id).evidence_commitment,
        Some(commitment.clone())
    );
    assert_eq!(client.get_escrow(&id).evidence_commitment, Some(commitment));
}

#[test]
fn dispute_rejects_empty_evidence_commitment() {
    let (env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    let err = client
        .try_dispute(&id, buyer, &Some(Bytes::new(&env)))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    // The escrow is untouched: still Funded, no commitment stored.
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
    assert_eq!(client.get_escrow(&id).evidence_commitment, None);
}

#[test]
fn dispute_commitment_survives_seller_resolution() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    let commitment = commitment_bytes(&_env);
    client.dispute(&id, seller, &Some(commitment.clone()));
    client.resolve(&id, &true);

    assert_eq!(client.get_status(&id), EscrowStatus::Completed);
    assert_eq!(client.get_escrow(&id).evidence_commitment, Some(commitment));
}

#[test]
fn dispute_commitment_survives_buyer_resolution() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    let commitment = commitment_bytes(&_env);
    client.dispute(&id, buyer, &Some(commitment.clone()));
    client.resolve(&id, &false);

    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
    assert_eq!(client.get_escrow(&id).evidence_commitment, Some(commitment));
}

#[test]
fn dispute_by_outsider_is_rejected_with_commitment() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    let commitment = commitment_bytes(&_env);
    let err = client
        .try_dispute(&id, &accounts.user3, &Some(commitment.clone()))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
    assert_eq!(client.get_escrow(&id).evidence_commitment, None);
}

#[test]
fn duplicate_dispute_rejected_with_commitment() {
    let (_env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    let commitment = commitment_bytes(&_env);
    client.dispute(&id, buyer, &Some(commitment.clone()));
    // A second dispute is invalid: the escrow is no longer Funded, and no
    // new commitment may overwrite the recorded one.
    let other = commitment_bytes(&_env);
    let err = client
        .try_dispute(&id, buyer, &Some(other))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(client.get_escrow(&id).evidence_commitment, Some(commitment));
}

#[test]
fn basket_dispute_stores_evidence_commitment() {
    let (env, token_a, token_b, _contract_id, client, accounts) = setup_basket!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = basket_fund(&env, &client, &token_a, &token_b, buyer, seller, arbiter);

    let commitment = commitment_bytes(&env);
    client.dispute_basket(&id, buyer, &Some(commitment.clone()));
    assert_eq!(
        client.get_basket(&id).evidence_commitment,
        Some(commitment.clone())
    );

    // Survives resolution.
    client.resolve_basket(&id, &true);
    assert_eq!(client.get_basket(&id).evidence_commitment, Some(commitment));
}

#[test]
fn basket_dispute_rejects_empty_evidence_commitment() {
    let (env, token_a, token_b, _contract_id, client, accounts) = setup_basket!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = basket_fund(&env, &client, &token_a, &token_b, buyer, seller, arbiter);

    let err = client
        .try_dispute_basket(&id, buyer, &Some(Bytes::new(&env)))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(client.get_basket(&id).evidence_commitment, None);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn legacy_escrow_records_decode_with_default_fields() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);

    // Schema V2: the pre-#728/#727 shape — 10 fields (with `released`,
    // without `acceptance_deadline`/`evidence_commitment`).
    env.as_contract(&contract_id, || {
        let v2 = EscrowDataV2 {
            escrow_id: id,
            buyer: buyer.clone(),
            seller: seller.clone(),
            arbiter: arbiter.clone(),
            token: token.clone(),
            amount: AMOUNT,
            released: 123,
            timeout: TIMEOUT,
            status: EscrowStatus::Funded,
            created_at: START,
        };
        env.storage().persistent().set(&DataKey::Escrow(id), &v2);
    });
    let record: EscrowData = client.get_escrow(&id);
    assert_eq!(record.escrow_id, id);
    assert_eq!(record.released, 123);
    assert_eq!(record.acceptance_deadline, None);
    assert_eq!(record.evidence_commitment, None);
    assert_eq!(record.status, EscrowStatus::Funded);
    assert_eq!(record.remaining(), AMOUNT - 123);

    // Schema V1: the pre-partial-release shape — 9 fields (no `released`).
    let id_v1 = id + 1;
    env.as_contract(&contract_id, || {
        let v1 = EscrowDataV1 {
            escrow_id: id_v1,
            buyer: buyer.clone(),
            seller: seller.clone(),
            arbiter: arbiter.clone(),
            token: token.clone(),
            amount: AMOUNT,
            timeout: TIMEOUT,
            status: EscrowStatus::Funded,
            created_at: START,
        };
        env.storage().persistent().set(&DataKey::Escrow(id_v1), &v1);
    });
    let record: EscrowData = client.get_escrow(&id_v1);
    assert_eq!(record.escrow_id, id_v1);
    assert_eq!(record.released, 0);
    assert_eq!(record.acceptance_deadline, None);
    assert_eq!(record.evidence_commitment, None);
    assert_eq!(record.status, EscrowStatus::Funded);
}

#[test]
fn dispute_commitment_is_included_in_the_dispute_event() {
    let (env, token, _tc, _id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    let commitment = commitment_bytes(&env);
    client.dispute(&id, buyer, &Some(commitment.clone()));

    // The Disputed event carries the full record — the commitment rides
    // along in its data payload.
    let events = env.events().all().events().to_vec();
    let disputed: std::vec::Vec<&soroban_sdk::xdr::ContractEvent> = events
        .iter()
        .filter(|event| {
            let ContractEventBody::V0(body) = &event.body;
            body.topics.iter().next().is_some_and(|topic| {
                matches!(
                    topic,
                    ScVal::Symbol(sym) if sym.to_utf8_string().is_ok_and(|name| name == "Disputed")
                )
            })
        })
        .collect();
    assert_eq!(disputed.len(), 1, "exactly one Disputed event was emitted");
    let ContractEventBody::V0(body) = &disputed[0].body;
    let record: EscrowData = EscrowData::try_from_val(&env, &body.data).expect("data decodes");
    assert_eq!(record.evidence_commitment, Some(commitment));
}

// -----------------------------------------------------------------------
// Issue #726 — Review-window release requests with expiry
// -----------------------------------------------------------------------

/// Read the stored release request for `id`, or `None` if none is stored.
fn stored_release_request(env: &Env, contract_id: &Address, id: u64) -> Option<ReleaseRequest> {
    env.as_contract(contract_id, || {
        env.storage().persistent().get(&DataKey::ReleaseRequest(id))
    })
}

#[test]
fn request_release_creates_pending_request() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.request_release(&id, &100);

    // Stored and pending; the request itself changes no accounting.
    let request = stored_release_request(&env, &contract_id, id).expect("request stored");
    assert_eq!(request.expiry, START + 100);
    assert_eq!(request.state, ReleaseRequestState::Pending);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn request_release_rejects_zero_window() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    let err = client.try_request_release(&id, &0).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(stored_release_request(&env, &contract_id, id), None);
}

#[test]
fn request_release_rejects_window_over_cap() {
    let (_env, token, _tc, _contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    let err = client
        .try_request_release(&id, &(MAX_RELEASE_WINDOW + 1))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

#[test]
fn request_release_rejects_when_not_funded() {
    let (_env, token, _tc, _contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);

    // Pending (never funded).
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    let err = client.try_request_release(&id, &100).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);

    // Completed (terminal).
    let id_done = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id_done);
    client.release(&id_done);
    let err = client
        .try_request_release(&id_done, &100)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

#[test]
fn request_release_rejects_repeated_while_pending() {
    let (_env, token, _tc, _contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.request_release(&id, &100);
    let err = client.try_request_release(&id, &100).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    let err = client.try_request_release(&id, &50).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

#[test]
fn approve_release_request_pays_seller_and_completes() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.request_release(&id, &1000);
    client.approve_release_request(&id);

    // Approval pays the remaining balance to the seller through the
    // standard release accounting; nothing stays in the contract.
    assert_eq!(tc.balance(seller), AMOUNT);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Completed);
    let request = stored_release_request(&env, &contract_id, id).expect("request stored");
    assert_eq!(request.state, ReleaseRequestState::Approved);
}

#[test]
fn approve_release_request_at_expiry_is_rejected() {
    let (env, token, _tc, _contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.request_release(&id, &100);
    env.ledger().set_timestamp(START + 100); // at the expiry boundary
    let err = client
        .try_approve_release_request(&id)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn approve_release_request_after_expiry_is_rejected() {
    let (env, token, _tc, _contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.request_release(&id, &100);
    env.ledger().set_timestamp(START + 101);
    let err = client
        .try_approve_release_request(&id)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn approve_release_request_repeated_is_rejected() {
    let (_env, token, _tc, _contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.request_release(&id, &1000);
    client.approve_release_request(&id);
    let err = client
        .try_approve_release_request(&id)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

#[test]
fn expire_release_request_at_expiry_boundary_settles() {
    // At the expiry instant the request is settled: `now >= expiry` is the
    // boundary for expiry settlement (approval requires strictly before).
    let (env, token, tc, _contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.request_release(&id, &100);
    env.ledger().set_timestamp(START + 100);
    client.expire_release_request(&id);
    assert_eq!(tc.balance(buyer), AMOUNT);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
}

#[test]
fn expire_release_request_refunds_buyer_still_held() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);
    // A partial release moves 300 to the seller first.
    client.release_partial(&id, &300);

    client.request_release(&id, &100);
    env.ledger().set_timestamp(START + 100); // at expiry
    client.expire_release_request(&id);

    // The buyer is refunded the still-held amount: 300 already went to the
    // seller, the rest returns; the contract holds nothing.
    assert_eq!(tc.balance(seller), 300);
    assert_eq!(tc.balance(buyer), AMOUNT - 300);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
    let request = stored_release_request(&env, &contract_id, id).expect("request stored");
    assert_eq!(request.state, ReleaseRequestState::Expired);
}

#[test]
fn expire_release_request_requires_expiry() {
    let (env, token, _tc, _contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.request_release(&id, &100);
    env.ledger().set_timestamp(START + 50); // before expiry
    let err = client.try_expire_release_request(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn expire_release_request_repeated_is_rejected() {
    let (env, token, _tc, _contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.request_release(&id, &100);
    env.ledger().set_timestamp(START + 100);
    client.expire_release_request(&id);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
    let err = client.try_expire_release_request(&id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
}

#[test]
fn expiry_settlement_blocks_further_settlement() {
    let (env, token, _tc, _contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);

    client.request_release(&id, &100);
    env.ledger().set_timestamp(START + 100);
    client.expire_release_request(&id); // settles as Refunded

    // No other path may now settle the escrow: the terminal status blocks
    // every lifecycle operation.
    assert_eq!(
        client
            .try_approve_release_request(&id)
            .unwrap_err()
            .unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(
        client.try_expire_release_request(&id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(
        client.try_release(&id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(
        client.try_request_release(&id, &100).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
}

#[test]
fn approve_settlement_blocks_expiry_and_vice_versa() {
    // After approval settles (Completed), expiry cannot run; and vice versa:
    // after expiry settles (Refunded), approval cannot run. Two escrows: the
    // buyer's mint covers both.
    let (env, token, _tc, _contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    StellarAssetClient::new(&env, &token).mint(buyer, &(2 * AMOUNT));
    let id = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id);
    client.request_release(&id, &1000);
    client.approve_release_request(&id);
    assert_eq!(
        client.try_expire_release_request(&id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(client.get_status(&id), EscrowStatus::Completed);

    let id2 = create(&client, &token, buyer, seller, arbiter, TIMEOUT);
    client.deposit(&id2);
    client.request_release(&id2, &100);
    env.ledger().set_timestamp(START + 100);
    client.expire_release_request(&id2);
    assert_eq!(
        client
            .try_approve_release_request(&id2)
            .unwrap_err()
            .unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(client.get_status(&id2), EscrowStatus::Refunded);
}

#[test]
fn expiry_and_deadline_refunds_settle_once() {
    // Cross-feature: an acceptance deadline (#728) and a review-window
    // expiry (#726) settle the escrow exactly once — whichever runs first
    // blocks the other.
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let (buyer, seller, arbiter) = parties(&accounts);
    let deadline = START + 100;
    let id = client.create_escrow(
        buyer,
        seller,
        arbiter,
        &token,
        &AMOUNT,
        &TIMEOUT,
        &Some(deadline),
    );
    client.deposit(&id);

    client.request_release(&id, &1000);
    // At the acceptance deadline, the buyer may reclaim via either the
    // deadline path (#728) or expiry settlement (#726); the first one wins.
    env.ledger().set_timestamp(deadline);
    client.refund_expired(&id);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
    assert_eq!(
        client.try_expire_release_request(&id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(tc.balance(buyer), AMOUNT);

    // Same, but expiry settles first.
    let id2 = client.create_escrow(
        buyer,
        seller,
        arbiter,
        &token,
        &AMOUNT,
        &TIMEOUT,
        &Some(deadline),
    );
    client.deposit(&id2);
    client.request_release(&id2, &100);
    env.ledger().set_timestamp(deadline + 100);
    client.expire_release_request(&id2);
    assert_eq!(client.get_status(&id2), EscrowStatus::Refunded);
    assert_eq!(
        client.try_refund_expired(&id2).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    // Conservation: the buyer gets back exactly what they deposited.
    assert_eq!(tc.balance(buyer), AMOUNT);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(tc.balance(seller), 0);
}
