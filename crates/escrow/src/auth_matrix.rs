//! # Authorization Matrix — comprehensive escrow auth test suite
//!
//! This module implements a **table-driven** authorization test matrix that:
//!
//! 1. Covers every state-changing entrypoint for both single-token and basket
//!    escrows.
//! 2. Tests every meaningful (entrypoint, role, lifecycle-state) triple.
//! 3. Verifies success paths under `mock_all_auths` and inspects `env.auths()`
//!    to confirm the exact authorized address and invocation tree.
//! 4. Verifies rejection paths with `MockAuth` (exact wrong signer), snapshotting
//!    storage, balances and events before the bad call and asserting they are
//!    identical after.
//! 5. Includes a **completeness check** that cross-references the spec trait
//!    list against the matrix, failing if any state-changing entrypoint is
//!    missing coverage.
//! 6. Explicitly demonstrates that read-only entrypoints require no auth.
//!
//! ## Why are auth-tree tests separate from exact-signer tests?
//!
//! `env.auths()` records authorizations under `mock_all_auths`.  Under
//! `mock_auths`, the envelope is validated against the provided `MockAuth`
//! entries but `env.auths()` returns empty.  Therefore:
//!
//! * **Auth-tree tests** (`_success_auth_tree`): run entirely under
//!   `mock_all_auths` (from `setup!`), call the method directly, then assert
//!   `env.auths()`.
//! * **Exact-signer rejection tests** (`_rejected`): call `env.mock_auths`
//!   with the wrong signer and assert the call aborts.

use crate::{Escrow, EscrowAsset, EscrowStatus, SorobanForgeEscrowClient};
use soroban_forge_shared_utils::ForgeError;
extern crate std;
use soroban_sdk::testutils::{
    Address as _, AuthorizedFunction, AuthorizedInvocation, Events as _, Ledger as _, MockAuth,
    MockAuthInvoke,
};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Env, IntoVal, InvokeError, Symbol, Val, Vec};

// -----------------------------------------------------------------------
// Constants
// -----------------------------------------------------------------------

const START: u64 = 1_000_000;
const TIMEOUT: u64 = 1_000;
const AMOUNT: i128 = 1_000;

// -----------------------------------------------------------------------
// Setup macros — same pattern as authz.rs so env.auths() works correctly
// -----------------------------------------------------------------------

/// Single-token environment. Returns:
/// `(env, token, token_client, contract_id, client, buyer, seller, arbiter, unrelated)`
macro_rules! setup {
    () => {{
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(START);

        let admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(admin);
        let token = sac.address();
        let token_client = soroban_sdk::token::Client::new(&env, &token);

        let accounts = soroban_forge_test_utils::TestAccounts::generate(&env);
        StellarAssetClient::new(&env, &token).mint(&accounts.user1, &AMOUNT);

        let contract_id = env.register(Escrow, ());
        let client = SorobanForgeEscrowClient::new(&env, &contract_id);

        let buyer = accounts.user1.clone();
        let seller = accounts.user2.clone();
        let arbiter = accounts.arbiter.clone();
        let unrelated = accounts.user3.clone();

        (
            env,
            token,
            token_client,
            contract_id,
            client,
            buyer,
            seller,
            arbiter,
            unrelated,
        )
    }};
}

/// Two-token basket environment. Returns:
/// `(env, token_a, token_b, tc_a, tc_b, contract_id, client, buyer, seller, arbiter, unrelated)`
macro_rules! setup_basket {
    () => {{
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(START);

        let admin = Address::generate(&env);
        let sac_a = env.register_stellar_asset_contract_v2(admin.clone());
        let token_a = sac_a.address();
        let tc_a = soroban_sdk::token::Client::new(&env, &token_a);

        let sac_b = env.register_stellar_asset_contract_v2(admin);
        let token_b = sac_b.address();
        let tc_b = soroban_sdk::token::Client::new(&env, &token_b);

        let accounts = soroban_forge_test_utils::TestAccounts::generate(&env);
        StellarAssetClient::new(&env, &token_a).mint(&accounts.user1, &AMOUNT);
        StellarAssetClient::new(&env, &token_b).mint(&accounts.user1, &AMOUNT);

        let contract_id = env.register(Escrow, ());
        let client = SorobanForgeEscrowClient::new(&env, &contract_id);

        let buyer = accounts.user1.clone();
        let seller = accounts.user2.clone();
        let arbiter = accounts.arbiter.clone();
        let unrelated = accounts.user3.clone();

        (
            env,
            token_a,
            token_b,
            tc_a,
            tc_b,
            contract_id,
            client,
            buyer,
            seller,
            arbiter,
            unrelated,
        )
    }};
}

fn basket_assets(env: &Env, token_a: &Address, token_b: &Address) -> Vec<EscrowAsset> {
    let mut v = Vec::new(env);
    v.push_back(EscrowAsset {
        token: token_a.clone(),
        amount: AMOUNT,
        released: 0,
    });
    v.push_back(EscrowAsset {
        token: token_b.clone(),
        amount: AMOUNT,
        released: 0,
    });
    v
}

fn transfer_args(env: &Env, from: &Address, to: &Address, amount: i128) -> soroban_sdk::Vec<Val> {
    (from.clone(), to.clone(), amount).into_val(env)
}

// -----------------------------------------------------------------------
// Abort assertion helper
// -----------------------------------------------------------------------

macro_rules! assert_auth_abort {
    ($res:expr) => {
        assert!(
            matches!($res, Err(Err(InvokeError::Abort))),
            "expected auth abort, got {:?}",
            $res
        );
    };
}

// -----------------------------------------------------------------------
// Matrix completeness check
// -----------------------------------------------------------------------

const STATE_CHANGING_ENTRYPOINTS: &[&str] = &[
    "create_escrow",
    "deposit",
    "release",
    "release_partial",
    "refund",
    "refund_expired",
    "dispute",
    "resolve",
    "resolve_split",
    "cancel",
    "create_basket",
    "deposit_basket",
    "release_basket",
    "release_partial_basket",
    "refund_basket",
    "dispute_basket",
    "resolve_basket",
    "cancel_basket",
];

const MATRIX_COVERAGE: &[&str] = &[
    "create_escrow",
    "deposit",
    "release",
    "release_partial",
    "refund",
    "refund_expired",
    "dispute",
    "resolve",
    "resolve_split",
    "cancel",
    "create_basket",
    "deposit_basket",
    "release_basket",
    "release_partial_basket",
    "refund_basket",
    "dispute_basket",
    "resolve_basket",
    "cancel_basket",
];

#[test]
fn completeness_check_all_state_changing_entrypoints_are_covered() {
    let mut spec = STATE_CHANGING_ENTRYPOINTS.to_vec();
    spec.sort_unstable();
    let mut matrix = MATRIX_COVERAGE.to_vec();
    matrix.sort_unstable();
    assert_eq!(
        spec,
        matrix,
        "MATRIX_COVERAGE does not match STATE_CHANGING_ENTRYPOINTS.\n\
         Missing from matrix: {:?}\n\
         Extra in matrix: {:?}",
        {
            let mut v: std::vec::Vec<&str> = spec
                .iter()
                .filter(|e| !matrix.contains(e))
                .copied()
                .collect();
            v.sort_unstable();
            v
        },
        {
            let mut v: std::vec::Vec<&str> = matrix
                .iter()
                .filter(|e| !spec.contains(e))
                .copied()
                .collect();
            v.sort_unstable();
            v
        },
    );
}

// -----------------------------------------------------------------------
// Read-only entrypoints: no authorization required
// -----------------------------------------------------------------------

#[test]
fn read_only_entrypoints_require_no_authorization() {
    let (env, token, _tc, _cid, client, buyer, seller, arbiter, _unrelated) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);

    env.set_auths(&[]);
    let escrow = client.get_escrow(&id);
    assert_eq!(escrow.escrow_id, id);
    let status = client.get_status(&id);
    assert_eq!(status, EscrowStatus::Funded);
    let page = client.escrows_for_participant(&buyer, &0, &10);
    assert!(page.total >= 1);
    client.touch_ttl(&id); // permissionless keeper

    assert!(
        env.auths().is_empty(),
        "read-only/permissionless entrypoints must not appear in env.auths()"
    );
}

#[test]
fn read_only_basket_entrypoints_require_no_authorization() {
    let (env, token_a, token_b, _tc_a, _tc_b, _cid, client, buyer, seller, arbiter, _unrelated) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.deposit_basket(&id);

    env.set_auths(&[]);
    let basket = client.get_basket(&id);
    assert_eq!(basket.escrow_id, id);
    let status = client.get_status(&id);
    assert_eq!(status, EscrowStatus::Funded);
    client.touch_ttl(&id);

    assert!(env.auths().is_empty());
}

// -----------------------------------------------------------------------
// ==== SINGLE-TOKEN ESCROW ====
// -----------------------------------------------------------------------

// -----------------------------------------------------------------------
// create_escrow
// -----------------------------------------------------------------------

#[test]
fn matrix_create_escrow_buyer_success_auth_tree() {
    let (env, token, _tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    // env.auths() must be checked before any other contract call, which would push a new snapshot.

    assert_eq!(
        env.auths(),
        [(
            buyer.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "create_escrow"),
                    (&buyer, &seller, &arbiter, &token, AMOUNT, TIMEOUT).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
    assert_eq!(client.get_status(&id), EscrowStatus::Pending);
}

#[test]
fn matrix_create_escrow_seller_rejected_nothing_written() {
    let (env, token, _tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    env.mock_auths(&[MockAuth {
        address: &seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "create_escrow",
            args: (&buyer, &seller, &arbiter, &token, AMOUNT, TIMEOUT).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(
        client.try_create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT)
    );
    env.mock_all_auths();
    assert_eq!(
        client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT),
        1
    );
}

#[test]
fn matrix_create_escrow_arbiter_rejected() {
    let (env, token, _tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    env.mock_auths(&[MockAuth {
        address: &arbiter,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "create_escrow",
            args: (&buyer, &seller, &arbiter, &token, AMOUNT, TIMEOUT).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(
        client.try_create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT)
    );
}

#[test]
fn matrix_create_escrow_unrelated_rejected() {
    let (env, token, _tc, contract_id, client, buyer, seller, arbiter, unrelated) = setup!();
    env.mock_auths(&[MockAuth {
        address: &unrelated,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "create_escrow",
            args: (&buyer, &seller, &arbiter, &token, AMOUNT, TIMEOUT).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(
        client.try_create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT)
    );
}

#[test]
fn matrix_create_escrow_buyer_wrong_args_rejected() {
    let (env, token, _tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    env.mock_auths(&[MockAuth {
        address: &buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "create_escrow",
            args: (&buyer, &seller, &arbiter, &token, AMOUNT + 1, TIMEOUT).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(
        client.try_create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT)
    );
}

// -----------------------------------------------------------------------
// deposit
// -----------------------------------------------------------------------

#[test]
fn matrix_deposit_buyer_success_auth_tree() {
    let (env, token, tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);

    assert_eq!(
        env.auths(),
        [(
            buyer.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "deposit"),
                    (id,).into_val(&env),
                )),
                sub_invocations: std::vec![AuthorizedInvocation {
                    function: AuthorizedFunction::Contract((
                        token.clone(),
                        Symbol::new(&env, "transfer"),
                        transfer_args(&env, &buyer, &contract_id, AMOUNT),
                    )),
                    sub_invocations: std::vec![],
                }],
            },
        )],
    );
    assert_eq!(tc.balance(&buyer), 0);
    assert_eq!(tc.balance(&contract_id), AMOUNT);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn matrix_deposit_seller_rejected_snapshot_unchanged() {
    let (env, token, tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    let bal_before = tc.balance(&contract_id);
    let status_before = client.get_status(&id);
    let events_before = env.events().all().events().len();
    env.mock_auths(&[MockAuth {
        address: &seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "deposit",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_deposit(&id));
    assert_eq!(tc.balance(&contract_id), bal_before);
    assert_eq!(client.get_status(&id), status_before);
    assert_eq!(env.events().all().events().len(), events_before);
}

#[test]
fn matrix_deposit_buyer_missing_nested_token_auth_snapshot_unchanged() {
    let (env, token, tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    let bal_before = tc.balance(&buyer);
    env.mock_auths(&[MockAuth {
        address: &buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "deposit",
            args: (id,).into_val(&env),
            sub_invokes: &[], // no nested token auth
        },
    }]);
    assert!(
        matches!(
            client.try_deposit(&id),
            Err(Ok(ForgeError::TokenTransferFailed))
        ),
        "missing nested auth must surface as TokenTransferFailed"
    );
    assert_eq!(tc.balance(&buyer), bal_before);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Pending);
}

#[test]
fn matrix_deposit_unrelated_rejected() {
    let (env, _token, _tc, contract_id, client, buyer, seller, arbiter, unrelated) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &_token, &AMOUNT, &TIMEOUT);
    env.mock_auths(&[MockAuth {
        address: &unrelated,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "deposit",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_deposit(&id));
    assert_eq!(client.get_status(&id), EscrowStatus::Pending);
}

// -----------------------------------------------------------------------
// release
// -----------------------------------------------------------------------

#[test]
fn matrix_release_buyer_success_auth_tree() {
    let (env, token, tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    client.release(&id);

    assert_eq!(
        env.auths(),
        [(
            buyer.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "release"),
                    (id,).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
    assert_eq!(tc.balance(&seller), AMOUNT);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Completed);
}

#[test]
fn matrix_release_seller_rejected_snapshot_unchanged() {
    let (env, token, tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    let bal = tc.balance(&contract_id);
    let status = client.get_status(&id);
    let events = env.events().all().events().len();
    env.mock_auths(&[MockAuth {
        address: &seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "release",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_release(&id));
    assert_eq!(tc.balance(&contract_id), bal);
    assert_eq!(client.get_status(&id), status);
    assert_eq!(env.events().all().events().len(), events);
}

#[test]
fn matrix_release_arbiter_rejected() {
    let (env, token, _tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    env.mock_auths(&[MockAuth {
        address: &arbiter,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "release",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_release(&id));
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn matrix_release_unrelated_rejected() {
    let (env, token, _tc, contract_id, client, buyer, seller, arbiter, unrelated) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    env.mock_auths(&[MockAuth {
        address: &unrelated,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "release",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_release(&id));
}

// -----------------------------------------------------------------------
// release_partial
// -----------------------------------------------------------------------

#[test]
fn matrix_release_partial_buyer_success_auth_tree() {
    let (env, token, tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    let partial = AMOUNT / 2;
    client.release_partial(&id, &partial);

    assert_eq!(
        env.auths(),
        [(
            buyer.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "release_partial"),
                    (id, partial).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
    assert_eq!(tc.balance(&seller), partial);
    assert_eq!(tc.balance(&contract_id), AMOUNT - partial);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn matrix_release_partial_seller_rejected_snapshot_unchanged() {
    let (env, token, tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    let partial = AMOUNT / 2;
    let bal = tc.balance(&contract_id);
    let status = client.get_status(&id);
    let events = env.events().all().events().len();
    env.mock_auths(&[MockAuth {
        address: &seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "release_partial",
            args: (id, partial).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_release_partial(&id, &partial));
    assert_eq!(tc.balance(&contract_id), bal);
    assert_eq!(client.get_status(&id), status);
    assert_eq!(env.events().all().events().len(), events);
}

#[test]
fn matrix_release_partial_arbiter_rejected() {
    let (env, token, _tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    let partial = AMOUNT / 2;
    env.mock_auths(&[MockAuth {
        address: &arbiter,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "release_partial",
            args: (id, partial).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_release_partial(&id, &partial));
}

#[test]
fn matrix_release_partial_wrong_amount_rejected() {
    let (env, token, tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    let partial = AMOUNT / 2;
    env.mock_auths(&[MockAuth {
        address: &seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "release_partial",
            args: (id, partial + 1).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_release_partial(&id, &partial));
    assert_eq!(tc.balance(&contract_id), AMOUNT);
}

// -----------------------------------------------------------------------
// refund
// -----------------------------------------------------------------------

#[test]
fn matrix_refund_pre_deadline_seller_success_auth_tree() {
    let (env, token, tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    client.refund(&id);

    assert_eq!(
        env.auths(),
        [(
            seller.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "refund"),
                    (id,).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
    assert_eq!(tc.balance(&buyer), AMOUNT);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
}

#[test]
fn matrix_refund_pre_deadline_buyer_rejected_snapshot_unchanged() {
    let (env, token, tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    let bal = tc.balance(&contract_id);
    let status = client.get_status(&id);
    let events = env.events().all().events().len();
    env.mock_auths(&[MockAuth {
        address: &buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "refund",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_refund(&id));
    assert_eq!(tc.balance(&contract_id), bal);
    assert_eq!(client.get_status(&id), status);
    assert_eq!(env.events().all().events().len(), events);
}

#[test]
fn matrix_refund_post_deadline_buyer_success_auth_tree() {
    let (env, token, tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    env.ledger().set_timestamp(START + TIMEOUT + 1);
    client.refund(&id);

    assert_eq!(
        env.auths(),
        [(
            buyer.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "refund"),
                    (id,).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
    assert_eq!(tc.balance(&buyer), AMOUNT);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
}

#[test]
fn matrix_refund_post_deadline_seller_rejected_snapshot_unchanged() {
    let (env, token, tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    env.ledger().set_timestamp(START + TIMEOUT + 1);
    let bal = tc.balance(&contract_id);
    let status = client.get_status(&id);
    let events = env.events().all().events().len();
    env.mock_auths(&[MockAuth {
        address: &seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "refund",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_refund(&id));
    assert_eq!(tc.balance(&contract_id), bal);
    assert_eq!(client.get_status(&id), status);
    assert_eq!(env.events().all().events().len(), events);
}

// -----------------------------------------------------------------------
// refund_expired  (permissionless)
// -----------------------------------------------------------------------

#[test]
fn matrix_refund_expired_permissionless_no_auth_recorded() {
    let (env, token, tc, _cid, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    env.ledger().set_timestamp(START + TIMEOUT + 1);
    env.set_auths(&[]);
    client
        .try_refund_expired(&id)
        .expect("outer ok")
        .expect("permissionless must succeed");
    assert_eq!(tc.balance(&buyer), AMOUNT);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
    assert!(
        env.auths().is_empty(),
        "refund_expired must not appear in env.auths()"
    );
}

#[test]
fn matrix_refund_expired_before_deadline_rejected() {
    let (env, token, _tc, _cid, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    env.set_auths(&[]);
    assert!(
        matches!(
            client.try_refund_expired(&id),
            Err(Ok(ForgeError::DeadlineReached))
        ),
        "expected DeadlineReached before deadline"
    );
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

// -----------------------------------------------------------------------
// dispute
// -----------------------------------------------------------------------

#[test]
fn matrix_dispute_buyer_claimant_success_auth_tree() {
    let (env, token, _tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    client.dispute(&id, &buyer);

    assert_eq!(
        env.auths(),
        [(
            buyer.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "dispute"),
                    (id, &buyer).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
    assert_eq!(client.get_status(&id), EscrowStatus::Disputed);
}

#[test]
fn matrix_dispute_seller_claimant_success_auth_tree() {
    let (env, token, _tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    client.dispute(&id, &seller);

    assert_eq!(
        env.auths(),
        [(
            seller.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "dispute"),
                    (id, &seller).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
    assert_eq!(client.get_status(&id), EscrowStatus::Disputed);
}

#[test]
fn matrix_dispute_unrelated_rejected_snapshot_unchanged() {
    let (env, token, tc, contract_id, client, buyer, seller, arbiter, unrelated) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    let bal = tc.balance(&contract_id);
    let status = client.get_status(&id);
    let events = env.events().all().events().len();
    env.mock_auths(&[MockAuth {
        address: &unrelated,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "dispute",
            args: (id, &buyer).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_dispute(&id, &buyer));
    assert_eq!(tc.balance(&contract_id), bal);
    assert_eq!(client.get_status(&id), status);
    assert_eq!(env.events().all().events().len(), events);
}

#[test]
fn matrix_dispute_wrong_signer_for_claimant_rejected() {
    let (env, token, _tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    env.mock_auths(&[MockAuth {
        address: &seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "dispute",
            args: (id, &buyer).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_dispute(&id, &buyer));
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

// -----------------------------------------------------------------------
// resolve
// -----------------------------------------------------------------------

#[test]
fn matrix_resolve_arbiter_seller_wins_success_auth_tree() {
    let (env, token, tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    client.dispute(&id, &buyer);
    client.resolve(&id, &true);

    assert_eq!(
        env.auths(),
        [(
            arbiter.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "resolve"),
                    (id, true).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
    assert_eq!(tc.balance(&seller), AMOUNT);
    assert_eq!(tc.balance(&buyer), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Completed);
}

#[test]
fn matrix_resolve_arbiter_buyer_wins_success() {
    let (_env, token, tc, _cid, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    client.dispute(&id, &seller);
    client.resolve(&id, &false);
    assert_eq!(tc.balance(&buyer), AMOUNT);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
}

#[test]
fn matrix_resolve_buyer_rejected_snapshot_unchanged() {
    let (env, token, tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    client.dispute(&id, &buyer);
    let bal = tc.balance(&contract_id);
    let status = client.get_status(&id);
    let events = env.events().all().events().len();
    env.mock_auths(&[MockAuth {
        address: &buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "resolve",
            args: (id, true).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_resolve(&id, &true));
    assert_eq!(tc.balance(&contract_id), bal);
    assert_eq!(client.get_status(&id), status);
    assert_eq!(env.events().all().events().len(), events);
}

#[test]
fn matrix_resolve_seller_rejected() {
    let (env, token, _tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    client.dispute(&id, &buyer);
    env.mock_auths(&[MockAuth {
        address: &seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "resolve",
            args: (id, true).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_resolve(&id, &true));
    assert_eq!(client.get_status(&id), EscrowStatus::Disputed);
}

#[test]
fn matrix_resolve_unrelated_rejected() {
    let (env, token, _tc, contract_id, client, buyer, seller, arbiter, unrelated) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    client.dispute(&id, &buyer);
    env.mock_auths(&[MockAuth {
        address: &unrelated,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "resolve",
            args: (id, true).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_resolve(&id, &true));
}

// -----------------------------------------------------------------------
// resolve_split
// -----------------------------------------------------------------------

#[test]
fn matrix_resolve_split_arbiter_success_auth_tree() {
    let (env, token, tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    client.dispute(&id, &buyer);
    let bps: u32 = 5_000;
    client.resolve_split(&id, &bps);
    let seller_share = AMOUNT / 2;
    let buyer_share = AMOUNT - seller_share;

    assert_eq!(
        env.auths(),
        [(
            arbiter.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "resolve_split"),
                    (id, bps).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
    assert_eq!(tc.balance(&seller), seller_share);
    assert_eq!(tc.balance(&buyer), buyer_share);
    assert_eq!(tc.balance(&contract_id), 0);
}

#[test]
fn matrix_resolve_split_buyer_rejected() {
    let (env, token, _tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    client.dispute(&id, &buyer);
    env.mock_auths(&[MockAuth {
        address: &buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "resolve_split",
            args: (id, 5000u32).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_resolve_split(&id, &5000));
    assert_eq!(client.get_status(&id), EscrowStatus::Disputed);
}

#[test]
fn matrix_resolve_split_seller_rejected() {
    let (env, token, _tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    client.dispute(&id, &buyer);
    env.mock_auths(&[MockAuth {
        address: &seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "resolve_split",
            args: (id, 5000u32).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_resolve_split(&id, &5000));
}

#[test]
fn matrix_resolve_split_unrelated_rejected_snapshot_unchanged() {
    let (env, token, tc, contract_id, client, buyer, seller, arbiter, unrelated) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    client.dispute(&id, &buyer);
    let bal = tc.balance(&contract_id);
    let status = client.get_status(&id);
    let events = env.events().all().events().len();
    env.mock_auths(&[MockAuth {
        address: &unrelated,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "resolve_split",
            args: (id, 5000u32).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_resolve_split(&id, &5000));
    assert_eq!(tc.balance(&contract_id), bal);
    assert_eq!(client.get_status(&id), status);
    assert_eq!(env.events().all().events().len(), events);
}

// -----------------------------------------------------------------------
// cancel
// -----------------------------------------------------------------------

#[test]
fn matrix_cancel_buyer_success_auth_tree() {
    let (env, token, _tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    client.cancel(&id);

    assert_eq!(
        env.auths(),
        [(
            buyer.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "cancel"),
                    (id,).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
    assert_eq!(client.get_status(&id), EscrowStatus::Cancelled);
}

#[test]
fn matrix_cancel_seller_rejected_snapshot_unchanged() {
    let (env, token, _tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    let status = client.get_status(&id);
    let events = env.events().all().events().len();
    env.mock_auths(&[MockAuth {
        address: &seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "cancel",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_cancel(&id));
    assert_eq!(client.get_status(&id), status);
    assert_eq!(env.events().all().events().len(), events);
}

#[test]
fn matrix_cancel_arbiter_rejected() {
    let (env, token, _tc, contract_id, client, buyer, seller, arbiter, _) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    env.mock_auths(&[MockAuth {
        address: &arbiter,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "cancel",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_cancel(&id));
    assert_eq!(client.get_status(&id), EscrowStatus::Pending);
}

#[test]
fn matrix_cancel_unrelated_rejected() {
    let (env, token, _tc, contract_id, client, buyer, seller, arbiter, unrelated) = setup!();
    let id = client.create_escrow(&buyer, &seller, &arbiter, &token, &AMOUNT, &TIMEOUT);
    env.mock_auths(&[MockAuth {
        address: &unrelated,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "cancel",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_cancel(&id));
}

// -----------------------------------------------------------------------
// ==== BASKET ESCROW ====
// -----------------------------------------------------------------------

// -----------------------------------------------------------------------
// create_basket
// -----------------------------------------------------------------------

#[test]
fn matrix_create_basket_buyer_success_auth_tree() {
    let (env, token_a, token_b, _tc_a, _tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);

    assert_eq!(
        env.auths(),
        [(
            buyer.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "create_basket"),
                    (&buyer, &seller, &arbiter, &assets, TIMEOUT).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
    assert_eq!(client.get_status(&id), EscrowStatus::Pending);
}

#[test]
fn matrix_create_basket_seller_rejected() {
    let (env, token_a, token_b, _tc_a, _tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    env.mock_auths(&[MockAuth {
        address: &seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "create_basket",
            args: (&buyer, &seller, &arbiter, &assets, TIMEOUT).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT));
    env.mock_all_auths();
    assert_eq!(
        client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT),
        1
    );
}

#[test]
fn matrix_create_basket_unrelated_rejected() {
    let (
        env,
        token_a,
        token_b,
        _tc_a,
        _tc_b,
        contract_id,
        client,
        buyer,
        seller,
        arbiter,
        unrelated,
    ) = setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    env.mock_auths(&[MockAuth {
        address: &unrelated,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "create_basket",
            args: (&buyer, &seller, &arbiter, &assets, TIMEOUT).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT));
}

#[test]
fn matrix_create_basket_wrong_assets_rejected() {
    let (env, token_a, token_b, _tc_a, _tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let mut wrong = Vec::new(&env);
    wrong.push_back(EscrowAsset {
        token: token_a.clone(),
        amount: AMOUNT + 1,
        released: 0,
    });
    wrong.push_back(EscrowAsset {
        token: token_b.clone(),
        amount: AMOUNT,
        released: 0,
    });
    env.mock_auths(&[MockAuth {
        address: &buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "create_basket",
            args: (&buyer, &seller, &arbiter, &wrong, TIMEOUT).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT));
}

// -----------------------------------------------------------------------
// deposit_basket
// -----------------------------------------------------------------------

#[test]
fn matrix_deposit_basket_buyer_success_auth_tree() {
    let (env, token_a, token_b, tc_a, tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.deposit_basket(&id);

    assert_eq!(
        env.auths(),
        [(
            buyer.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "deposit_basket"),
                    (id,).into_val(&env),
                )),
                sub_invocations: std::vec![
                    AuthorizedInvocation {
                        function: AuthorizedFunction::Contract((
                            token_a.clone(),
                            Symbol::new(&env, "transfer"),
                            transfer_args(&env, &buyer, &contract_id, AMOUNT),
                        )),
                        sub_invocations: std::vec![],
                    },
                    AuthorizedInvocation {
                        function: AuthorizedFunction::Contract((
                            token_b.clone(),
                            Symbol::new(&env, "transfer"),
                            transfer_args(&env, &buyer, &contract_id, AMOUNT),
                        )),
                        sub_invocations: std::vec![],
                    },
                ],
            },
        )],
    );
    assert_eq!(tc_a.balance(&contract_id), AMOUNT);
    assert_eq!(tc_b.balance(&contract_id), AMOUNT);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn matrix_deposit_basket_seller_rejected() {
    let (env, token_a, token_b, _tc_a, _tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    env.mock_auths(&[MockAuth {
        address: &seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "deposit_basket",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_deposit_basket(&id));
    assert_eq!(client.get_status(&id), EscrowStatus::Pending);
}

#[test]
fn matrix_deposit_basket_partial_leg_auth_rejected_no_partial_custody() {
    let (env, token_a, token_b, tc_a, tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    env.mock_auths(&[MockAuth {
        address: &buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "deposit_basket",
            args: (id,).into_val(&env),
            sub_invokes: &[MockAuthInvoke {
                contract: &token_a,
                fn_name: "transfer",
                args: transfer_args(&env, &buyer, &contract_id, AMOUNT),
                sub_invokes: &[],
            }],
        },
    }]);
    assert!(
        matches!(
            client.try_deposit_basket(&id),
            Err(Ok(ForgeError::TokenTransferFailed))
        ),
        "partial auth must surface as TokenTransferFailed"
    );
    assert_eq!(tc_a.balance(&contract_id), 0, "leg A must roll back");
    assert_eq!(tc_b.balance(&contract_id), 0, "leg B must not land");
    assert_eq!(client.get_status(&id), EscrowStatus::Pending);
}

// -----------------------------------------------------------------------
// release_basket
// -----------------------------------------------------------------------

#[test]
fn matrix_release_basket_buyer_success_auth_tree() {
    let (env, token_a, token_b, tc_a, tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.deposit_basket(&id);
    client.release_basket(&id);

    assert_eq!(
        env.auths(),
        [(
            buyer.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "release_basket"),
                    (id,).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
    assert_eq!(tc_a.balance(&seller), AMOUNT);
    assert_eq!(tc_b.balance(&seller), AMOUNT);
    assert_eq!(tc_a.balance(&contract_id), 0);
    assert_eq!(tc_b.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Completed);
}

#[test]
fn matrix_release_basket_seller_rejected_snapshot_unchanged() {
    let (env, token_a, token_b, tc_a, tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.deposit_basket(&id);
    let bal_a = tc_a.balance(&contract_id);
    let bal_b = tc_b.balance(&contract_id);
    let status = client.get_status(&id);
    let events = env.events().all().events().len();
    env.mock_auths(&[MockAuth {
        address: &seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "release_basket",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_release_basket(&id));
    assert_eq!(tc_a.balance(&contract_id), bal_a);
    assert_eq!(tc_b.balance(&contract_id), bal_b);
    assert_eq!(client.get_status(&id), status);
    assert_eq!(env.events().all().events().len(), events);
}

#[test]
fn matrix_release_basket_arbiter_rejected() {
    let (env, token_a, token_b, _tc_a, _tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.deposit_basket(&id);
    env.mock_auths(&[MockAuth {
        address: &arbiter,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "release_basket",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_release_basket(&id));
}

// -----------------------------------------------------------------------
// release_partial_basket
// -----------------------------------------------------------------------

#[test]
fn matrix_release_partial_basket_buyer_success_auth_tree() {
    let (env, token_a, token_b, tc_a, tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.deposit_basket(&id);
    let partial = AMOUNT / 2;
    client.release_partial_basket(&id, &token_a, &partial);

    assert_eq!(
        env.auths(),
        [(
            buyer.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "release_partial_basket"),
                    (id, &token_a, partial).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
    assert_eq!(tc_a.balance(&seller), partial);
    assert_eq!(tc_a.balance(&contract_id), AMOUNT - partial);
    assert_eq!(tc_b.balance(&contract_id), AMOUNT);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn matrix_release_partial_basket_seller_rejected() {
    let (env, token_a, token_b, tc_a, _tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.deposit_basket(&id);
    let partial = AMOUNT / 2;
    env.mock_auths(&[MockAuth {
        address: &seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "release_partial_basket",
            args: (id, &token_a, partial).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_release_partial_basket(&id, &token_a, &partial));
    assert_eq!(tc_a.balance(&contract_id), AMOUNT);
}

#[test]
fn matrix_release_partial_basket_wrong_leg_rejected() {
    let (env, token_a, token_b, tc_a, tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.deposit_basket(&id);
    let partial = AMOUNT / 2;
    env.mock_auths(&[MockAuth {
        address: &seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "release_partial_basket",
            args: (id, &token_b, partial).into_val(&env),
            sub_invokes: &[], // signed B, paying A
        },
    }]);
    assert_auth_abort!(client.try_release_partial_basket(&id, &token_a, &partial));
    assert_eq!(tc_a.balance(&contract_id), AMOUNT);
    assert_eq!(tc_b.balance(&contract_id), AMOUNT);
}

// -----------------------------------------------------------------------
// refund_basket
// -----------------------------------------------------------------------

#[test]
fn matrix_refund_basket_pre_deadline_seller_success_auth_tree() {
    let (env, token_a, token_b, tc_a, tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.deposit_basket(&id);
    client.refund_basket(&id);

    assert_eq!(
        env.auths(),
        [(
            seller.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "refund_basket"),
                    (id,).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
    assert_eq!(tc_a.balance(&buyer), AMOUNT);
    assert_eq!(tc_b.balance(&buyer), AMOUNT);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
}

#[test]
fn matrix_refund_basket_pre_deadline_buyer_rejected() {
    let (env, token_a, token_b, _tc_a, _tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.deposit_basket(&id);
    env.mock_auths(&[MockAuth {
        address: &buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "refund_basket",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_refund_basket(&id));
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn matrix_refund_basket_post_deadline_buyer_success_auth_tree() {
    let (env, token_a, token_b, tc_a, tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.deposit_basket(&id);
    env.ledger().set_timestamp(START + TIMEOUT + 1);
    client.refund_basket(&id);

    assert_eq!(
        env.auths(),
        [(
            buyer.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "refund_basket"),
                    (id,).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
    assert_eq!(tc_a.balance(&buyer), AMOUNT);
    assert_eq!(tc_b.balance(&buyer), AMOUNT);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
}

#[test]
fn matrix_refund_basket_post_deadline_seller_rejected_snapshot_unchanged() {
    let (env, token_a, token_b, tc_a, _tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.deposit_basket(&id);
    env.ledger().set_timestamp(START + TIMEOUT + 1);
    let bal = tc_a.balance(&contract_id);
    let status = client.get_status(&id);
    let events = env.events().all().events().len();
    env.mock_auths(&[MockAuth {
        address: &seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "refund_basket",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_refund_basket(&id));
    assert_eq!(tc_a.balance(&contract_id), bal);
    assert_eq!(client.get_status(&id), status);
    assert_eq!(env.events().all().events().len(), events);
}

// -----------------------------------------------------------------------
// dispute_basket
// -----------------------------------------------------------------------

#[test]
fn matrix_dispute_basket_buyer_claimant_success_auth_tree() {
    let (env, token_a, token_b, _tc_a, _tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.deposit_basket(&id);
    client.dispute_basket(&id, &buyer);

    assert_eq!(
        env.auths(),
        [(
            buyer.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "dispute_basket"),
                    (id, &buyer).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
    assert_eq!(client.get_status(&id), EscrowStatus::Disputed);
}

#[test]
fn matrix_dispute_basket_seller_claimant_success_auth_tree() {
    let (env, token_a, token_b, _tc_a, _tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.deposit_basket(&id);
    client.dispute_basket(&id, &seller);

    assert_eq!(
        env.auths(),
        [(
            seller.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "dispute_basket"),
                    (id, &seller).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
    assert_eq!(client.get_status(&id), EscrowStatus::Disputed);
}

#[test]
fn matrix_dispute_basket_unrelated_rejected_snapshot_unchanged() {
    let (
        env,
        token_a,
        token_b,
        tc_a,
        _tc_b,
        contract_id,
        client,
        buyer,
        seller,
        arbiter,
        unrelated,
    ) = setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.deposit_basket(&id);
    let bal = tc_a.balance(&contract_id);
    let status = client.get_status(&id);
    let events = env.events().all().events().len();
    env.mock_auths(&[MockAuth {
        address: &unrelated,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "dispute_basket",
            args: (id, &buyer).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_dispute_basket(&id, &buyer));
    assert_eq!(tc_a.balance(&contract_id), bal);
    assert_eq!(client.get_status(&id), status);
    assert_eq!(env.events().all().events().len(), events);
}

#[test]
fn matrix_dispute_basket_wrong_signer_for_claimant_rejected() {
    let (env, token_a, token_b, _tc_a, _tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.deposit_basket(&id);
    env.mock_auths(&[MockAuth {
        address: &seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "dispute_basket",
            args: (id, &buyer).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_dispute_basket(&id, &buyer));
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

// -----------------------------------------------------------------------
// resolve_basket
// -----------------------------------------------------------------------

#[test]
fn matrix_resolve_basket_arbiter_seller_wins_success_auth_tree() {
    let (env, token_a, token_b, tc_a, tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.deposit_basket(&id);
    client.dispute_basket(&id, &buyer);
    client.resolve_basket(&id, &true);

    assert_eq!(
        env.auths(),
        [(
            arbiter.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "resolve_basket"),
                    (id, true).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
    assert_eq!(tc_a.balance(&seller), AMOUNT);
    assert_eq!(tc_b.balance(&seller), AMOUNT);
    assert_eq!(client.get_status(&id), EscrowStatus::Completed);
}

#[test]
fn matrix_resolve_basket_arbiter_buyer_wins_success() {
    let (_env, token_a, token_b, tc_a, tc_b, _cid, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&_env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.deposit_basket(&id);
    client.dispute_basket(&id, &seller);
    client.resolve_basket(&id, &false);
    assert_eq!(tc_a.balance(&buyer), AMOUNT);
    assert_eq!(tc_b.balance(&buyer), AMOUNT);
    assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
}

#[test]
fn matrix_resolve_basket_buyer_rejected_snapshot_unchanged() {
    let (env, token_a, token_b, tc_a, tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.deposit_basket(&id);
    client.dispute_basket(&id, &buyer);
    let bal_a = tc_a.balance(&contract_id);
    let bal_b = tc_b.balance(&contract_id);
    let status = client.get_status(&id);
    let events = env.events().all().events().len();
    env.mock_auths(&[MockAuth {
        address: &buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "resolve_basket",
            args: (id, true).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_resolve_basket(&id, &true));
    assert_eq!(tc_a.balance(&contract_id), bal_a);
    assert_eq!(tc_b.balance(&contract_id), bal_b);
    assert_eq!(client.get_status(&id), status);
    assert_eq!(env.events().all().events().len(), events);
}

#[test]
fn matrix_resolve_basket_seller_rejected() {
    let (env, token_a, token_b, _tc_a, _tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.deposit_basket(&id);
    client.dispute_basket(&id, &buyer);
    env.mock_auths(&[MockAuth {
        address: &seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "resolve_basket",
            args: (id, true).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_resolve_basket(&id, &true));
    assert_eq!(client.get_status(&id), EscrowStatus::Disputed);
}

// -----------------------------------------------------------------------
// cancel_basket
// -----------------------------------------------------------------------

#[test]
fn matrix_cancel_basket_buyer_success_auth_tree() {
    let (env, token_a, token_b, _tc_a, _tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    client.cancel_basket(&id);

    assert_eq!(
        env.auths(),
        [(
            buyer.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "cancel_basket"),
                    (id,).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
    assert_eq!(client.get_status(&id), EscrowStatus::Cancelled);
}

#[test]
fn matrix_cancel_basket_seller_rejected_snapshot_unchanged() {
    let (env, token_a, token_b, _tc_a, _tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    let status = client.get_status(&id);
    let events = env.events().all().events().len();
    env.mock_auths(&[MockAuth {
        address: &seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "cancel_basket",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_cancel_basket(&id));
    assert_eq!(client.get_status(&id), status);
    assert_eq!(env.events().all().events().len(), events);
}

#[test]
fn matrix_cancel_basket_arbiter_rejected() {
    let (env, token_a, token_b, _tc_a, _tc_b, contract_id, client, buyer, seller, arbiter, _) =
        setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    env.mock_auths(&[MockAuth {
        address: &arbiter,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "cancel_basket",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_cancel_basket(&id));
    assert_eq!(client.get_status(&id), EscrowStatus::Pending);
}

#[test]
fn matrix_cancel_basket_unrelated_rejected() {
    let (
        env,
        token_a,
        token_b,
        _tc_a,
        _tc_b,
        contract_id,
        client,
        buyer,
        seller,
        arbiter,
        unrelated,
    ) = setup_basket!();
    let assets = basket_assets(&env, &token_a, &token_b);
    let id = client.create_basket(&buyer, &seller, &arbiter, &assets, &TIMEOUT);
    env.mock_auths(&[MockAuth {
        address: &unrelated,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "cancel_basket",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(client.try_cancel_basket(&id));
}
