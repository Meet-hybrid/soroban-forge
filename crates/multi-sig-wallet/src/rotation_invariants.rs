//! Owner-rotation and threshold invariants for the multi-sig wallet (issue
//! #599).
//!
//! The main suite pins the happy paths for `add_owner`, `remove_owner`, and
//! `set_threshold`; this module adds stateful coverage for the interactions
//! the issue calls out:
//!
//! - rejected owner/threshold proposals never mutate owners or threshold;
//! - a valid rotation scrubs a removed owner's in-flight confirmations and
//!   re-checks the surviving threshold at execution time;
//! - confirmation and submission authorization follow the *current* owner
//!   set, not the set captured when a tx was submitted;
//! - terminal (`Executed`/`Rejected`) governance txs reject replays and
//!   further signalling without changing their stored record;
//! - the `threshold <= owners` invariant is preserved across a deterministic
//!   matrix of one-owner and multi-owner configurations.

extern crate std;

use crate::{MultiSigWallet, SorobanForgeMultiSigWalletClient, TxKind, TxStatus, WalletTx};
use soroban_forge_shared_utils::ForgeError;
use soroban_forge_test_utils::{MockTarget, TestAccounts};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Vec};
use std::vec;

#[derive(Debug, Clone, PartialEq, Eq)]
struct WalletState {
    owners: std::vec::Vec<Address>,
    threshold: u32,
    tx_count: u64,
    paused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TxState {
    status: TxStatus,
    confirmations: std::vec::Vec<Address>,
    rejections: std::vec::Vec<Address>,
    kind: TxKind,
    expiry: Option<u64>,
}

fn owner_list(owners: &Vec<Address>) -> std::vec::Vec<Address> {
    (0..owners.len())
        .map(|index| owners.get_unchecked(index))
        .collect()
}

fn wallet_state(client: &SorobanForgeMultiSigWalletClient<'_>) -> WalletState {
    WalletState {
        owners: owner_list(&client.get_owners()),
        threshold: client.get_threshold(),
        tx_count: client.get_tx_count(),
        paused: client.is_paused(),
    }
}

fn tx_state(tx: &WalletTx) -> TxState {
    TxState {
        status: tx.status.clone(),
        confirmations: owner_list(&tx.confirmations),
        rejections: owner_list(&tx.rejections),
        kind: tx.kind.clone(),
        expiry: tx.expiry,
    }
}

fn assert_wallet_state(client: &SorobanForgeMultiSigWalletClient<'_>, expected: &WalletState) {
    assert_eq!(&wallet_state(client), expected);
}

fn assert_tx_state(client: &SorobanForgeMultiSigWalletClient<'_>, tx_id: u64, expected: &TxState) {
    assert_eq!(&tx_state(&client.get_tx(&tx_id)), expected);
}

fn new_wallet<'a>(env: &'a Env, accounts: &TestAccounts) -> SorobanForgeMultiSigWalletClient<'a> {
    let contract_id = env.register(MultiSigWallet, ());
    let client = SorobanForgeMultiSigWalletClient::new(env, &contract_id);
    client.initialize(&standard_owners(env, accounts), &2_u32);
    client
}

fn standard_owners(env: &Env, accounts: &TestAccounts) -> Vec<Address> {
    soroban_sdk::vec![
        env,
        accounts.user1.clone(),
        accounts.user2.clone(),
        accounts.user3.clone()
    ]
}

fn first_n_owners(env: &Env, accounts: &TestAccounts, count: u32) -> Vec<Address> {
    let all = accounts.all(env);
    let mut owners = Vec::new(env);
    for index in 0..count {
        owners.push_back(all.get_unchecked(index));
    }
    owners
}

fn submit_target_tx(
    env: &Env,
    client: &SorobanForgeMultiSigWalletClient<'_>,
    submitter: &Address,
) -> u64 {
    let target_id = Address::generate(env);
    env.register_at(&target_id, MockTarget, ());
    client.submit(
        submitter,
        &target_id,
        &soroban_sdk::Bytes::from_array(env, &[0x01]),
        &None,
    )
}

fn confirm_n(client: &SorobanForgeMultiSigWalletClient<'_>, tx_id: u64, signers: &[Address]) {
    for signer in signers {
        client.confirm(&tx_id, signer);
    }
}

fn execute_owner_change(
    client: &SorobanForgeMultiSigWalletClient<'_>,
    tx_id: u64,
    signers: &[Address],
) {
    confirm_n(client, tx_id, signers);
    client.execute(&tx_id);
}

#[test]
fn invalid_owner_and_threshold_configurations_do_not_mutate_wallet_state() {
    let env = Env::default();
    env.mock_all_auths();
    let accounts = TestAccounts::generate(&env);
    let client = new_wallet(&env, &accounts);
    let before = wallet_state(&client);

    // Invalid owner additions: an existing member and a non-owner submitter.
    let error = client
        .try_add_owner(&accounts.user1, &accounts.user2)
        .unwrap_err()
        .unwrap();
    assert_eq!(error, ForgeError::InvalidInput);
    assert_wallet_state(&client, &before);

    let error = client
        .try_add_owner(&accounts.arbiter, &Address::generate(&env))
        .unwrap_err()
        .unwrap();
    assert_eq!(error, ForgeError::Unauthorized);
    assert_wallet_state(&client, &before);

    // Invalid owner removals: a non-member and a non-owner submitter.
    let error = client
        .try_remove_owner(&accounts.user1, &accounts.arbiter)
        .unwrap_err()
        .unwrap();
    assert_eq!(error, ForgeError::InvalidInput);
    assert_wallet_state(&client, &before);

    let error = client
        .try_remove_owner(&accounts.arbiter, &accounts.user1)
        .unwrap_err()
        .unwrap();
    assert_eq!(error, ForgeError::Unauthorized);
    assert_wallet_state(&client, &before);

    // Invalid thresholds: zero and above the current owner count.
    for invalid in [0_u32, 4_u32] {
        let error = client
            .try_set_threshold(&accounts.user1, &invalid)
            .unwrap_err()
            .unwrap();
        assert_eq!(error, ForgeError::InvalidInput);
        assert_wallet_state(&client, &before);
    }

    // None of the rejected submissions allocated a transaction id.
    assert_eq!(client.get_tx_count(), before.tx_count);
}

#[test]
fn single_owner_rotation_rejects_invalid_changes_and_allows_valid_ones() {
    let env = Env::default();
    env.mock_all_auths();
    let accounts = TestAccounts::generate(&env);
    let contract_id = env.register(MultiSigWallet, ());
    let client = SorobanForgeMultiSigWalletClient::new(&env, &contract_id);
    let owners = soroban_sdk::vec![&env, accounts.user1.clone()];
    client.initialize(&owners, &1_u32);

    let before = wallet_state(&client);
    let error = client
        .try_remove_owner(&accounts.user1, &accounts.user1)
        .unwrap_err()
        .unwrap();
    assert_eq!(error, ForgeError::InvalidInput);
    assert_wallet_state(&client, &before);

    let error = client
        .try_add_owner(&accounts.user1, &accounts.user1)
        .unwrap_err()
        .unwrap();
    assert_eq!(error, ForgeError::InvalidInput);
    assert_wallet_state(&client, &before);

    let error = client
        .try_set_threshold(&accounts.user1, &2_u32)
        .unwrap_err()
        .unwrap();
    assert_eq!(error, ForgeError::InvalidInput);
    assert_wallet_state(&client, &before);

    // Add a second owner through the threshold-1 governance path.
    let add = client.add_owner(&accounts.user1, &accounts.user2);
    client.confirm(&add, &accounts.user1);
    client.execute(&add);
    assert_eq!(client.get_owners().len(), 2);
    assert!(client.is_owner(&accounts.user2));

    // The new owner can submit, confirm, and execute a normal tx.
    let tx = submit_target_tx(&env, &client, &accounts.user2);
    client.confirm(&tx, &accounts.user2);
    client.execute(&tx);
    assert_eq!(client.get_tx(&tx).status, TxStatus::Executed);

    // Remove the original owner; the wallet remains valid at threshold 1.
    let remove = client.remove_owner(&accounts.user2, &accounts.user1);
    client.confirm(&remove, &accounts.user2);
    client.execute(&remove);
    assert_eq!(client.get_owners().len(), 1);
    assert!(!client.is_owner(&accounts.user1));
    assert_eq!(client.get_threshold(), 1_u32);

    // The rotated-out owner can no longer authorize or submit.
    let pending = submit_target_tx(&env, &client, &accounts.user2);
    let error = client
        .try_confirm(&pending, &accounts.user1)
        .unwrap_err()
        .unwrap();
    assert_eq!(error, ForgeError::Unauthorized);
    let error = client
        .try_submit(
            &accounts.user1,
            &Address::generate(&env),
            &soroban_sdk::Bytes::from_array(&env, &[0x02]),
            &None,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(error, ForgeError::Unauthorized);
}

#[test]
fn in_flight_tx_confirmations_follow_removed_owner_semantics() {
    let env = Env::default();
    env.mock_all_auths();
    let accounts = TestAccounts::generate(&env);
    let client = new_wallet(&env, &accounts);

    // A pending tx carrying user2's confirmation when user2 is rotated out.
    let tx = submit_target_tx(&env, &client, &accounts.user1);
    client.confirm(&tx, &accounts.user2);
    assert_eq!(client.get_confirmations(&tx).len(), 1);

    let remove = client.remove_owner(&accounts.user1, &accounts.user2);
    execute_owner_change(
        &client,
        remove,
        &[accounts.user3.clone(), accounts.user1.clone()],
    );
    assert!(!client.is_owner(&accounts.user2));

    // The removed owner's signature no longer counts, but the tx is still
    // live and keeps its pending status.
    let after_rotation = client.get_tx(&tx);
    assert_eq!(after_rotation.status, TxStatus::Pending);
    assert!(!client.get_confirmations(&tx).contains(&accounts.user2));
    assert_eq!(client.get_confirmations(&tx).len(), 0);

    // Below the surviving threshold (2) execution is still refused.
    let error = client.try_execute(&tx).unwrap_err().unwrap();
    assert_eq!(error, ForgeError::InvalidInput);
    assert_eq!(client.get_tx(&tx).status, TxStatus::Pending);

    // A removed owner cannot re-sign the in-flight tx.
    let error = client
        .try_confirm(&tx, &accounts.user2)
        .unwrap_err()
        .unwrap();
    assert_eq!(error, ForgeError::Unauthorized);
    assert_eq!(client.get_confirmations(&tx).len(), 0);

    // The two surviving owners can still carry it over the threshold.
    client.confirm(&tx, &accounts.user1);
    let error = client.try_execute(&tx).unwrap_err().unwrap();
    assert_eq!(error, ForgeError::InvalidInput);
    client.confirm(&tx, &accounts.user3);
    client.execute(&tx);
    assert_eq!(client.get_tx(&tx).status, TxStatus::Executed);
}

#[test]
fn rejected_governance_txs_cannot_mutate_owners_or_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let accounts = TestAccounts::generate(&env);
    let client = new_wallet(&env, &accounts);
    let newbie = Address::generate(&env);

    // Rejected add-owner: owners stay intact and the tx stays terminal.
    let add = client.add_owner(&accounts.user1, &newbie);
    client.confirm(&add, &accounts.user2);
    client.reject(&add, &accounts.user3);
    let add_state = tx_state(&client.get_tx(&add));
    let before = wallet_state(&client);

    let error = client.try_execute(&add).unwrap_err().unwrap();
    assert_eq!(error, ForgeError::InvalidInput);
    assert_wallet_state(&client, &before);
    assert!(!client.is_owner(&newbie));
    assert_tx_state(&client, add, &add_state);
    assert_eq!(add_state.status, TxStatus::Rejected);
    assert_eq!(add_state.rejections, vec![accounts.user3.clone()]);

    // Replay/signal attempts on the rejected tx change nothing.
    let error = client
        .try_confirm(&add, &accounts.user1)
        .unwrap_err()
        .unwrap();
    assert_eq!(error, ForgeError::InvalidInput);
    let error = client
        .try_reject(&add, &accounts.user1)
        .unwrap_err()
        .unwrap();
    assert_eq!(error, ForgeError::InvalidInput);
    assert_tx_state(&client, add, &add_state);

    // Rejected remove-owner: the target owner remains in the set.
    let remove = client.remove_owner(&accounts.user1, &accounts.user3);
    client.confirm(&remove, &accounts.user2);
    client.reject(&remove, &accounts.user1);
    let remove_state = tx_state(&client.get_tx(&remove));
    let before = wallet_state(&client);

    let error = client.try_execute(&remove).unwrap_err().unwrap();
    assert_eq!(error, ForgeError::InvalidInput);
    assert_wallet_state(&client, &before);
    assert!(client.is_owner(&accounts.user3));
    assert_tx_state(&client, remove, &remove_state);

    // Rejected threshold change: the configured threshold is untouched.
    let threshold = client.set_threshold(&accounts.user1, &3_u32);
    client.confirm(&threshold, &accounts.user2);
    client.reject(&threshold, &accounts.user3);
    let threshold_state = tx_state(&client.get_tx(&threshold));
    let before = wallet_state(&client);

    let error = client.try_execute(&threshold).unwrap_err().unwrap();
    assert_eq!(error, ForgeError::InvalidInput);
    assert_wallet_state(&client, &before);
    assert_eq!(client.get_threshold(), 2_u32);
    assert_tx_state(&client, threshold, &threshold_state);
}

#[test]
fn threshold_increases_apply_to_in_flight_transactions() {
    let env = Env::default();
    env.mock_all_auths();
    let accounts = TestAccounts::generate(&env);
    let client = new_wallet(&env, &accounts);

    let in_flight = submit_target_tx(&env, &client, &accounts.user1);
    client.confirm(&in_flight, &accounts.user2);

    // Raising the threshold is authorized by the old threshold (2).
    let raise = client.set_threshold(&accounts.user1, &3_u32);
    execute_owner_change(
        &client,
        raise,
        &[accounts.user2.clone(), accounts.user3.clone()],
    );
    assert_eq!(client.get_threshold(), 3_u32);

    // The in-flight tx keeps its confirmation, but two approvals no longer
    // meet the new threshold.
    assert_eq!(client.get_confirmations(&in_flight).len(), 1);
    let error = client.try_execute(&in_flight).unwrap_err().unwrap();
    assert_eq!(error, ForgeError::InvalidInput);
    assert_eq!(client.get_tx(&in_flight).status, TxStatus::Pending);

    // Adding the remaining two approvals executes it under threshold 3.
    client.confirm(&in_flight, &accounts.user1);
    client.confirm(&in_flight, &accounts.user3);
    client.execute(&in_flight);
    assert_eq!(client.get_tx(&in_flight).status, TxStatus::Executed);
}

#[test]
fn threshold_decreases_apply_to_in_flight_transactions() {
    let env = Env::default();
    env.mock_all_auths();
    let accounts = TestAccounts::generate(&env);
    let contract_id = env.register(MultiSigWallet, ());
    let client = SorobanForgeMultiSigWalletClient::new(&env, &contract_id);
    client.initialize(&standard_owners(&env, &accounts), &3_u32);

    let in_flight = submit_target_tx(&env, &client, &accounts.user1);
    client.confirm(&in_flight, &accounts.user2);
    client.confirm(&in_flight, &accounts.user1);

    // Lowering the threshold still needs all three old-threshold approvals.
    let lower = client.set_threshold(&accounts.user1, &2_u32);
    execute_owner_change(
        &client,
        lower,
        &[
            accounts.user2.clone(),
            accounts.user1.clone(),
            accounts.user3.clone(),
        ],
    );
    assert_eq!(client.get_threshold(), 2_u32);

    // The two existing confirmations now satisfy the wallet.
    client.execute(&in_flight);
    assert_eq!(client.get_tx(&in_flight).status, TxStatus::Executed);
}

#[test]
fn added_owner_can_authorize_and_removed_owner_cannot() {
    let env = Env::default();
    env.mock_all_auths();
    let accounts = TestAccounts::generate(&env);
    let client = new_wallet(&env, &accounts);
    let newbie = Address::generate(&env);

    let add = client.add_owner(&accounts.user1, &newbie);
    execute_owner_change(
        &client,
        add,
        &[accounts.user2.clone(), accounts.user3.clone()],
    );
    assert!(client.is_owner(&newbie));

    // The added owner's confirmation counts towards a pending tx.
    let tx = submit_target_tx(&env, &client, &accounts.user1);
    client.confirm(&tx, &newbie);
    client.confirm(&tx, &accounts.user1);
    client.execute(&tx);
    assert_eq!(client.get_tx(&tx).status, TxStatus::Executed);
    assert!(client.get_confirmations(&tx).contains(&newbie));

    // Rotate the added owner out again.
    let remove = client.remove_owner(&accounts.user1, &newbie);
    execute_owner_change(
        &client,
        remove,
        &[accounts.user2.clone(), accounts.user3.clone()],
    );
    assert!(!client.is_owner(&newbie));

    let pending = submit_target_tx(&env, &client, &accounts.user1);
    let error = client.try_confirm(&pending, &newbie).unwrap_err().unwrap();
    assert_eq!(error, ForgeError::Unauthorized);
    let error = client
        .try_submit(
            &newbie,
            &Address::generate(&env),
            &soroban_sdk::Bytes::from_array(&env, &[0x03]),
            &None,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(error, ForgeError::Unauthorized);
}

#[test]
fn terminal_rotation_txs_reject_replays_without_mutation() {
    let env = Env::default();
    env.mock_all_auths();
    let accounts = TestAccounts::generate(&env);
    let client = new_wallet(&env, &accounts);
    let newbie = Address::generate(&env);

    let add = client.add_owner(&accounts.user1, &newbie);
    execute_owner_change(
        &client,
        add,
        &[accounts.user2.clone(), accounts.user3.clone()],
    );
    let executed = tx_state(&client.get_tx(&add));
    assert_eq!(executed.status, TxStatus::Executed);
    let before = wallet_state(&client);

    // Replays and late signals are rejected and leave the record intact.
    let error = client.try_execute(&add).unwrap_err().unwrap();
    assert_eq!(error, ForgeError::InvalidInput);
    let error = client
        .try_confirm(&add, &accounts.user1)
        .unwrap_err()
        .unwrap();
    assert_eq!(error, ForgeError::InvalidInput);
    let error = client
        .try_reject(&add, &accounts.user1)
        .unwrap_err()
        .unwrap();
    assert_eq!(error, ForgeError::InvalidInput);
    assert_wallet_state(&client, &before);
    assert_tx_state(&client, add, &executed);
}

#[test]
fn rotation_matrix_preserves_threshold_owner_invariant() {
    for owner_count in 1_u32..=4 {
        for threshold in 1_u32..=owner_count {
            let env = Env::default();
            env.mock_all_auths();
            let accounts = TestAccounts::generate(&env);
            let contract_id = env.register(MultiSigWallet, ());
            let client = SorobanForgeMultiSigWalletClient::new(&env, &contract_id);
            let owners = first_n_owners(&env, &accounts, owner_count);
            client.initialize(&owners, &threshold);

            assert_eq!(client.get_owners().len(), owner_count);
            assert_eq!(client.get_threshold(), threshold);

            if owner_count == 1 {
                // The final owner can never be removed.
                let before = wallet_state(&client);
                let error = client
                    .try_remove_owner(&owners.get_unchecked(0), &owners.get_unchecked(0))
                    .unwrap_err()
                    .unwrap();
                assert_eq!(error, ForgeError::InvalidInput);
                assert_wallet_state(&client, &before);
                continue;
            }

            // Removing an owner must either succeed while preserving
            // `threshold <= owners`, or fail with the wallet untouched.
            let removed = owners.get_unchecked(owner_count - 1);
            let remove = client.remove_owner(&owners.get_unchecked(0), &removed);
            for index in 0..threshold {
                client.confirm(&remove, &owners.get_unchecked(index));
            }

            let before = wallet_state(&client);
            if threshold > owner_count - 1 {
                let error = client.try_execute(&remove).unwrap_err().unwrap();
                assert_eq!(error, ForgeError::InvalidInput);
                assert_wallet_state(&client, &before);
                assert!(client.is_owner(&removed));
            } else {
                client.execute(&remove);
                assert_eq!(client.get_owners().len(), owner_count - 1);
                assert_eq!(client.get_threshold(), threshold);
                assert!(!client.is_owner(&removed));
                assert!(client.get_threshold() <= client.get_owners().len());
            }

            // Adding an owner always preserves the invariant.
            let added = Address::generate(&env);
            let add = client.add_owner(&owners.get_unchecked(0), &added);
            for index in 0..threshold {
                client.confirm(&add, &owners.get_unchecked(index));
            }
            client.execute(&add);
            let expected_owners = if threshold > owner_count - 1 {
                owner_count
            } else {
                owner_count - 1
            };
            assert_eq!(client.get_owners().len(), expected_owners + 1);
            assert_eq!(client.get_threshold(), threshold);
            assert!(client.is_owner(&added));
            assert!(client.get_threshold() <= client.get_owners().len());
        }
    }
}
