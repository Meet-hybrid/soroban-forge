//! Negative authorization tests.
//!
//! The main suite (`tests.rs`) runs under `mock_all_auths`, which proves
//! the *call graph* of authorizations — who the contract asks to sign —
//! but never that a wrong signer is rejected. This module covers the other
//! half, in two layers:
//!
//! 1. **Enforce mode** (`mock_auths` / `set_auths`): each test arms
//!    authorization for exactly the signer(s) a scenario names and asserts
//!    that anyone else — or the right signer over the wrong arguments — is
//!    rejected by the host, with lifecycle state and balances untouched.
//! 2. **Tree assertions** (`env.auths()` under `mock_all_auths`): pins the
//!    exact authorized-invocation tree the contract demands on every payout
//!    path, per the SDK's own recommendation for tests that would otherwise
//!    prove nothing about missing `require_auth` calls.
//!
//! ## Mechanics (verified against soroban-sdk 27.0.6, not assumed)
//!
//! - `mock_auths` sets the invocation envelope **and disables blanket
//!   mocking**; authorizations not matching a mocked auth fail. A fixture
//!   must mirror the auth tree exactly: the entrypoint frame *and* any
//!   nested token frame, with arguments as the client puts them on the wire.
//! - A missing/extra/mismatched auth **at the root entrypoint** aborts the
//!   whole invocation (`Err(Err(InvokeError::Abort))` through the `try_`
//!   client). A mismatch on a **nested** cross-contract call (the SAC
//!   `transfer` pull in `deposit`) surfaces as an error *returned by the
//!   token contract*, which the escrow buckets into
//!   [`ForgeError::TokenTransferFailed`] — the error-bucketing design
//!   observable end to end.
//! - `set_auths(&[])` is a blank envelope: every `require_auth` fails.
//! - **Contract self-authorization is implicit**: the host auto-approves
//!   `require_auth` for the currently-executing contract (this is what
//!   makes the custody pattern work on-chain without a `__check_auth`).
//!   It is therefore *not* part of a signer's authorized-invocation tree —
//!   `env.auths()` records only the party's entrypoint frame with no
//!   sub-invocations, and a payout legitimately completes on the party's
//!   signature alone. The deposit pull, by contrast, authorizes the
//!   *buyer* (not the executing contract) and so is a real, matchable
//!   sub-invocation.

use crate::{BasketAsset, Escrow, EscrowStatus, SorobanForgeEscrowClient};
use soroban_forge_shared_utils::ForgeError;
// The test harness links std even in a no_std crate; AuthorizedInvocation's
// sub_invocations field is a std Vec, so re-expose std here for `vec!`.
extern crate std;
use soroban_sdk::testutils::{
    Address as _, AuthorizedFunction, AuthorizedInvocation, Ledger as _, MockAuth, MockAuthInvoke,
};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Env, IntoVal, InvokeError, Symbol, Vec};

const START: u64 = 1_000_000;
const TIMEOUT: u64 = 1_000;
const AMOUNT: i128 = 1_000;

/// Fresh env with blanket mocking for *setup only* (minting, funding).
/// The tested call re-arms the envelope afterwards.
macro_rules! setup {
    () => {{
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(START);

        let admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(admin.clone());
        let token = sac.address();
        let token_admin = StellarAssetClient::new(&env, &token);
        let token_client = soroban_sdk::token::Client::new(&env, &token);

        let contract_id = env.register(Escrow, ());
        let client = SorobanForgeEscrowClient::new(&env, &contract_id);

        let accounts = soroban_forge_test_utils::TestAccounts::generate(&env);
        token_admin.mint(&accounts.user1, &AMOUNT);

        (env, token, token_client, contract_id, client, accounts)
    }};
}

/// Args of the nested SAC `transfer` frame the contract performs.
fn transfer_args(
    env: &Env,
    from: &Address,
    to: &Address,
    amount: i128,
) -> soroban_sdk::Vec<soroban_sdk::Val> {
    (from.clone(), to.clone(), amount).into_val(env)
}

/// Assert that a `try_` call aborted on authorization (the host aborts the
/// whole invocation when the armed envelope does not match).
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
// create_escrow
// -----------------------------------------------------------------------

#[test]
fn create_accepts_buyer_signature_with_matching_args() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;

    // The buyer authorizes create_escrow with the exact invocation args.
    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "create_escrow",
            args: (buyer, seller, arbiter, &token, AMOUNT, TIMEOUT).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let id = client
        .try_create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT)
        .expect("outer ok")
        .expect("contract ok");
    assert_eq!(client.get_status(&id), EscrowStatus::Pending);
}

#[test]
fn create_rejects_signature_from_non_buyer() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;

    // The seller (not the buyer) authorizes creation. The contract demands
    // the buyer, so the host must reject the call outright.
    env.mock_auths(&[MockAuth {
        address: seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "create_escrow",
            args: (buyer, seller, arbiter, &token, AMOUNT, TIMEOUT).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    assert_auth_abort!(res);
}

#[test]
fn create_rejects_signature_over_different_args() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;

    // Correct signer, but the armed authorization covers a *different
    // amount* than the invocation performs. A captured signature must not
    // be replayable over changed terms.
    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "create_escrow",
            args: (buyer, seller, arbiter, &token, AMOUNT + 500, TIMEOUT).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    assert_auth_abort!(res);
}

// -----------------------------------------------------------------------
// deposit
// -----------------------------------------------------------------------

#[test]
fn deposit_accepts_full_buyer_authorization_chain() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);

    // Two frames, both buyer-signed: the entrypoint and the token pull
    // into the contract's custody address.
    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "deposit",
            args: (id,).into_val(&env),
            sub_invokes: &[MockAuthInvoke {
                contract: &token,
                fn_name: "transfer",
                args: transfer_args(&env, buyer, &contract_id, AMOUNT),
                sub_invokes: &[],
            }],
        },
    }]);

    client.try_deposit(&id).expect("outer ok").unwrap();
    assert_eq!(tc.balance(buyer), 0);
    assert_eq!(tc.balance(&contract_id), AMOUNT);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn deposit_rejects_entrypoint_signature_without_token_authorization() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);

    // Only the entrypoint frame is armed; the nested SAC transfer pull has
    // no authorization. Funds must not move on an entrypoint signature
    // alone.
    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "deposit",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    // The unmatched nested auth is not a root abort: the SAC rejects the
    // pull and the escrow buckets the token error.
    let res = client.try_deposit(&id);
    assert!(matches!(res, Err(Ok(ForgeError::TokenTransferFailed))),);
    assert_eq!(tc.balance(buyer), AMOUNT);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Pending);
}

#[test]
fn deposit_rejects_token_authorization_over_a_different_amount() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);

    // The nested token authorization names a different amount than the
    // escrow pulls — the host must reject the mismatch.
    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "deposit",
            args: (id,).into_val(&env),
            sub_invokes: &[MockAuthInvoke {
                contract: &token,
                fn_name: "transfer",
                args: transfer_args(&env, buyer, &contract_id, AMOUNT - 1),
                sub_invokes: &[],
            }],
        },
    }]);

    let res = client.try_deposit(&id);
    assert!(matches!(res, Err(Ok(ForgeError::TokenTransferFailed))),);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Pending);
}

// -----------------------------------------------------------------------
// release
// -----------------------------------------------------------------------

#[test]
fn release_rejects_buyer_signature() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);

    // Only the seller confirms delivery. The buyer arming their own
    // signature must be rejected at the entrypoint.
    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "release",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_release(&id);
    assert_auth_abort!(res);
    assert_eq!(tc.balance(&contract_id), AMOUNT);
    assert_eq!(tc.balance(seller), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn release_completes_on_seller_signature_alone() {
    // The negative of the tree assertions below: contract self-auth is
    // implicit, so the seller's entrypoint signature is the *only*
    // signature a release needs. A seller signature must be sufficient —
    // and it must pay the seller, not the buyer.
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);

    env.mock_auths(&[MockAuth {
        address: seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "release",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    client.try_release(&id).expect("outer ok").unwrap();
    assert_eq!(tc.balance(seller), AMOUNT);
    assert_eq!(tc.balance(buyer), 0);
    assert_eq!(client.get_status(&id), EscrowStatus::Completed);
}

#[test]
fn release_authorization_tree_is_seller_over_transfer() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);

    client.release(&id);

    assert_eq!(
        env.auths(),
        [(
            seller.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "release"),
                    (id,).into_val(&env),
                )),
                // Contract self-auth is implicit and NOT recorded; the
                // tree is the seller's entrypoint frame only.
                sub_invocations: std::vec![],
            },
        )],
    );
}

// -----------------------------------------------------------------------
// refund
// -----------------------------------------------------------------------

#[test]
fn refund_pre_deadline_rejects_buyer_signature() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);

    // Before the deadline only the seller may refund. The buyer arming
    // their own signature must be rejected.
    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "refund",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_refund(&id);
    assert_auth_abort!(res);
    assert_eq!(tc.balance(&contract_id), AMOUNT);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn refund_authorization_tree_is_seller_pre_deadline() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
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
                // Self-auth implicit — entrypoint frame only.
                sub_invocations: std::vec![],
            },
        )],
    );
}

// -----------------------------------------------------------------------
// dispute
// -----------------------------------------------------------------------

#[test]
fn dispute_accepts_claimant_signature() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);

    // Dispute performs no token transfer, so the claimant's entrypoint
    // signature alone completes it — the enforce-mode positive control.
    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "dispute",
            args: (id, buyer).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    client.try_dispute(&id, buyer).expect("outer ok").unwrap();
    assert_eq!(client.get_status(&id), EscrowStatus::Disputed);
}

#[test]
fn dispute_rejects_signature_from_a_non_claimant() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);

    // A third party signs, but the claim on-chain is the buyer's. The
    // contract requires authorization of the *claimant parameter*, so a
    // stranger's signature must not freeze the escrow.
    env.mock_auths(&[MockAuth {
        address: &accounts.validator,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "dispute",
            args: (id, buyer).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_dispute(&id, buyer);
    assert_auth_abort!(res);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn dispute_rejects_other_party_signing_as_claimant() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);

    // The seller signs a call whose claimant is the buyer: the signature
    // exists but belongs to the wrong address for the claimed identity.
    env.mock_auths(&[MockAuth {
        address: seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "dispute",
            args: (id, buyer).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_dispute(&id, buyer);
    assert_auth_abort!(res);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

// -----------------------------------------------------------------------
// resolve
// -----------------------------------------------------------------------

#[test]
fn resolve_rejects_non_arbiter_signature() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    client.dispute(&id, buyer);

    // A party (buyer) signs the resolve instead of the arbiter.
    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "resolve",
            args: (id, true).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_resolve(&id, &true);
    assert_auth_abort!(res);
    assert_eq!(client.get_status(&id), EscrowStatus::Disputed);
    assert_eq!(tc.balance(&contract_id), AMOUNT);
}

#[test]
fn resolve_authorization_tree_is_arbiter_over_transfer() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);
    client.dispute(&id, buyer);

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
                // Self-auth implicit — entrypoint frame only.
                sub_invocations: std::vec![],
            },
        )],
    );
}

// -----------------------------------------------------------------------
// cancel
// -----------------------------------------------------------------------

#[test]
fn cancel_accepts_buyer_rejects_seller() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);

    // Seller cannot cancel a pending escrow.
    env.mock_auths(&[MockAuth {
        address: seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "cancel",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let res = client.try_cancel(&id);
    assert_auth_abort!(res);
    assert_eq!(client.get_status(&id), EscrowStatus::Pending);

    // The buyer's signature completes it (positive control).
    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "cancel",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    client.try_cancel(&id).expect("outer ok").unwrap();
    assert_eq!(client.get_status(&id), EscrowStatus::Cancelled);
}

// -----------------------------------------------------------------------
// Blank envelope: no authorizations at all
// -----------------------------------------------------------------------

#[test]
fn blank_envelope_aborts_release_and_preserves_custody() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);

    // No authorization entries whatsoever: every require_auth fails.
    env.set_auths(&[]);

    let res = client.try_release(&id);
    assert_auth_abort!(res);
    assert_eq!(tc.balance(&contract_id), AMOUNT);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn blank_envelope_aborts_create_and_writes_nothing() {
    let (env, token, _tc, _contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;

    env.set_auths(&[]);

    let res = client.try_create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    assert_auth_abort!(res);
    // The id counter must not have advanced: the next escrow is id 1.
    env.mock_all_auths();
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    assert_eq!(id, 1);
}

// -----------------------------------------------------------------------
// release_partial
// -----------------------------------------------------------------------

#[test]
fn release_partial_rejects_buyer_signature() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);

    // Buyer attempts to collect a partial release — must be rejected.
    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "release_partial",
            args: (id, AMOUNT / 2).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_release_partial(&id, &(AMOUNT / 2));
    assert_auth_abort!(res);
    assert_eq!(tc.balance(&contract_id), AMOUNT);
    assert_eq!(tc.balance(seller), 0);
    assert_eq!(client.get_status(&id), crate::EscrowStatus::Funded);
}

#[test]
fn release_partial_accepts_seller_signature() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);

    // Positive control: seller's signature is sufficient.
    env.mock_auths(&[MockAuth {
        address: seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "release_partial",
            args: (id, AMOUNT / 2).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    client
        .try_release_partial(&id, &(AMOUNT / 2))
        .expect("outer ok")
        .unwrap();
    assert_eq!(tc.balance(seller), AMOUNT / 2);
    assert_eq!(tc.balance(&contract_id), AMOUNT - AMOUNT / 2);
}

#[test]
fn release_partial_rejects_arbiter_signature() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);

    env.mock_auths(&[MockAuth {
        address: arbiter,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "release_partial",
            args: (id, AMOUNT / 2).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_release_partial(&id, &(AMOUNT / 2));
    assert_auth_abort!(res);
    assert_eq!(tc.balance(&contract_id), AMOUNT);
}

#[test]
fn release_partial_rejects_signature_over_different_amount() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);

    // Seller signs for one amount but the invocation sends a different amount.
    env.mock_auths(&[MockAuth {
        address: seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "release_partial",
            args: (id, AMOUNT / 2 + 100).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_release_partial(&id, &(AMOUNT / 2));
    assert_auth_abort!(res);
    assert_eq!(tc.balance(&contract_id), AMOUNT);
}

#[test]
fn release_partial_authorization_tree_is_seller_only() {
    // Contract self-auth is implicit; the seller's entrypoint frame is the
    // only entry in env.auths() — same as release and refund.
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);

    client.release_partial(&id, &(AMOUNT / 3));

    assert_eq!(
        env.auths(),
        [(
            seller.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "release_partial"),
                    (id, AMOUNT / 3).into_val(&env),
                )),
                // Self-auth is implicit; entrypoint frame only.
                sub_invocations: std::vec![],
            },
        )],
    );
}

#[test]
fn blank_envelope_aborts_release_partial() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);

    env.set_auths(&[]);

    let res = client.try_release_partial(&id, &(AMOUNT / 2));
    assert_auth_abort!(res);
    assert_eq!(tc.balance(&contract_id), AMOUNT);
    assert_eq!(tc.balance(seller), 0);
    assert_eq!(client.get_status(&id), crate::EscrowStatus::Funded);
}

// -----------------------------------------------------------------------
// Mutation verification for release_partial require_auth
//
// Procedure:
// 1. This test confirms the NEGATIVE path: when require_auth is absent,
//    a buyer signature on release_partial must be rejected by the host.
// 2. The POSITIVE path is covered by release_partial_accepts_seller_signature.
//
// To manually verify the mutation test:
//   a. Temporarily comment out `escrow.seller.require_auth();` in
//      `release_partial` in lib.rs.
//   b. Run: cargo test -p soroban-forge-escrow authz::release_partial_rejects_buyer_signature
//      → The test should FAIL (buyer's call succeeds without the guard).
//   c. Restore the require_auth line.
//   d. Run the same test again → it should PASS.
//
// The test below documents this invariant in CI. It uses a blank envelope
// (set_auths(&[])) which exercises the same code path: without require_auth,
// no auth check occurs and the call would succeed; with require_auth, the
// host aborts.
// -----------------------------------------------------------------------

#[test]
fn release_partial_mutation_test_no_auth_aborts() {
    // With require_auth in place:
    // - A blank auth envelope must abort the call.
    // Without require_auth (mutation):
    // - The call would succeed — this is what the mutation test detects.
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let id = client.create_escrow(buyer, seller, arbiter, &token, &AMOUNT, &TIMEOUT);
    client.deposit(&id);

    // Blank envelope: every require_auth fails.
    env.set_auths(&[]);

    let res = client.try_release_partial(&id, &(AMOUNT / 2));
    // MUST be Abort: if require_auth were removed, this would be Ok(Ok(())).
    assert_auth_abort!(res);

    // Storage and balances must be untouched.
    assert_eq!(tc.balance(&contract_id), AMOUNT, "custody must be intact");
    assert_eq!(
        tc.balance(seller),
        0,
        "seller must not have received anything"
    );

    // Re-arm with the correct seller signature to prove the positive path
    // (and that the escrow itself is intact and can still be used).
    env.mock_auths(&[MockAuth {
        address: seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "release_partial",
            args: (id, AMOUNT / 2).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    client
        .try_release_partial(&id, &(AMOUNT / 2))
        .expect("outer ok")
        .expect("seller auth must succeed after auth is restored");
    assert_eq!(tc.balance(seller), AMOUNT / 2);
}

// -----------------------------------------------------------------------
// Multi-Asset Basket Authorization Tests
// -----------------------------------------------------------------------

fn setup_multi(
    env: &Env,
    count: usize,
) -> (
    std::vec::Vec<Address>,
    std::vec::Vec<soroban_sdk::token::Client<'static>>,
    std::vec::Vec<StellarAssetClient<'static>>,
) {
    let mut tokens = std::vec::Vec::new();
    let mut clients = std::vec::Vec::new();
    let mut admins = std::vec::Vec::new();
    for _ in 0..count {
        let admin = Address::generate(env);
        let sac = env.register_stellar_asset_contract_v2(admin);
        let token = sac.address();
        let token_admin = StellarAssetClient::new(env, &token);
        let token_client = soroban_sdk::token::Client::new(env, &token);
        tokens.push(token);
        clients.push(token_client);
        admins.push(token_admin);
    }
    (tokens, clients, admins)
}

#[test]
fn create_escrow_multi_accepts_buyer_signature_with_matching_args() {
    let (env, _token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let (tokens, _, _) = setup_multi(&env, 2);

    let mut assets = Vec::new(&env);
    assets.push_back(BasketAsset {
        token: tokens[0].clone(),
        amount: 500,
    });
    assets.push_back(BasketAsset {
        token: tokens[1].clone(),
        amount: 1000,
    });

    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "create_escrow_multi",
            args: (buyer, seller, arbiter, assets.clone(), TIMEOUT).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let id = client
        .try_create_escrow_multi(buyer, seller, arbiter, &assets, &TIMEOUT)
        .expect("outer ok")
        .expect("contract ok");
    assert_eq!(client.get_status(&id), EscrowStatus::Pending);
}

#[test]
fn create_escrow_multi_rejects_signature_from_non_buyer() {
    let (env, _token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let (tokens, _, _) = setup_multi(&env, 2);

    let mut assets = Vec::new(&env);
    assets.push_back(BasketAsset {
        token: tokens[0].clone(),
        amount: 500,
    });
    assets.push_back(BasketAsset {
        token: tokens[1].clone(),
        amount: 1000,
    });

    // Seller signs instead of buyer
    env.mock_auths(&[MockAuth {
        address: seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "create_escrow_multi",
            args: (buyer, seller, arbiter, assets.clone(), TIMEOUT).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_create_escrow_multi(buyer, seller, arbiter, &assets, &TIMEOUT);
    assert_auth_abort!(res);
}

#[test]
fn create_escrow_multi_rejects_signature_over_different_args() {
    let (env, _token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let (tokens, _, _) = setup_multi(&env, 2);

    let mut assets = Vec::new(&env);
    assets.push_back(BasketAsset {
        token: tokens[0].clone(),
        amount: 500,
    });
    assets.push_back(BasketAsset {
        token: tokens[1].clone(),
        amount: 1000,
    });

    let mut tampered_assets = Vec::new(&env);
    tampered_assets.push_back(BasketAsset {
        token: tokens[0].clone(),
        amount: 600, // tampered amount
    });
    tampered_assets.push_back(BasketAsset {
        token: tokens[1].clone(),
        amount: 1000,
    });

    // Signature armed for tampered args
    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "create_escrow_multi",
            args: (buyer, seller, arbiter, tampered_assets, TIMEOUT).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_create_escrow_multi(buyer, seller, arbiter, &assets, &TIMEOUT);
    assert_auth_abort!(res);
}

#[test]
fn deposit_multi_accepts_full_buyer_authorization_chain() {
    let (env, _token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let (tokens, token_clients, token_admins) = setup_multi(&env, 2);

    token_admins[0].mint(buyer, &500);
    token_admins[1].mint(buyer, &1000);

    let mut assets = Vec::new(&env);
    assets.push_back(BasketAsset {
        token: tokens[0].clone(),
        amount: 500,
    });
    assets.push_back(BasketAsset {
        token: tokens[1].clone(),
        amount: 1000,
    });

    let id = client.create_escrow_multi(buyer, seller, arbiter, &assets, &TIMEOUT);

    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "deposit_multi",
            args: (id,).into_val(&env),
            sub_invokes: &[
                MockAuthInvoke {
                    contract: &tokens[0],
                    fn_name: "transfer",
                    args: transfer_args(&env, buyer, &contract_id, 500),
                    sub_invokes: &[],
                },
                MockAuthInvoke {
                    contract: &tokens[1],
                    fn_name: "transfer",
                    args: transfer_args(&env, buyer, &contract_id, 1000),
                    sub_invokes: &[],
                },
            ],
        },
    }]);

    client.try_deposit_multi(&id).expect("outer ok").unwrap();
    assert_eq!(token_clients[0].balance(buyer), 0);
    assert_eq!(token_clients[0].balance(&contract_id), 500);
    assert_eq!(token_clients[1].balance(buyer), 0);
    assert_eq!(token_clients[1].balance(&contract_id), 1000);
    assert_eq!(client.get_status(&id), EscrowStatus::Funded);
}

#[test]
fn deposit_multi_rejects_non_buyer() {
    let (env, _token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let (tokens, _, token_admins) = setup_multi(&env, 1);
    token_admins[0].mint(buyer, &500);

    let mut assets = Vec::new(&env);
    assets.push_back(BasketAsset {
        token: tokens[0].clone(),
        amount: 500,
    });

    let id = client.create_escrow_multi(buyer, seller, arbiter, &assets, &TIMEOUT);

    // Armed for seller instead of buyer
    env.mock_auths(&[MockAuth {
        address: seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "deposit_multi",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_deposit_multi(&id);
    assert_auth_abort!(res);
}

#[test]
fn deposit_multi_rejects_missing_token_authorizations() {
    let (env, _token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let (tokens, _, token_admins) = setup_multi(&env, 2);
    token_admins[0].mint(buyer, &500);
    token_admins[1].mint(buyer, &1000);

    let mut assets = Vec::new(&env);
    assets.push_back(BasketAsset {
        token: tokens[0].clone(),
        amount: 500,
    });
    assets.push_back(BasketAsset {
        token: tokens[1].clone(),
        amount: 1000,
    });

    let id = client.create_escrow_multi(buyer, seller, arbiter, &assets, &TIMEOUT);

    // Only armed at root entrypoint, no sub_invokes for tokens
    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "deposit_multi",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_deposit_multi(&id);
    assert!(matches!(res, Err(Ok(ForgeError::TokenTransferFailed))));
}

#[test]
fn deposit_multi_rejects_token_authorization_over_different_amount() {
    let (env, _token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let (tokens, _, token_admins) = setup_multi(&env, 1);
    token_admins[0].mint(buyer, &500);

    let mut assets = Vec::new(&env);
    assets.push_back(BasketAsset {
        token: tokens[0].clone(),
        amount: 500,
    });

    let id = client.create_escrow_multi(buyer, seller, arbiter, &assets, &TIMEOUT);

    // Armed for transfer of 400 instead of 500
    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "deposit_multi",
            args: (id,).into_val(&env),
            sub_invokes: &[MockAuthInvoke {
                contract: &tokens[0],
                fn_name: "transfer",
                args: transfer_args(&env, buyer, &contract_id, 400),
                sub_invokes: &[],
            }],
        },
    }]);

    let res = client.try_deposit_multi(&id);
    assert!(matches!(res, Err(Ok(ForgeError::TokenTransferFailed))));
}

#[test]
fn release_multi_accepts_seller_signature() {
    let (env, _token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let (tokens, token_clients, token_admins) = setup_multi(&env, 2);
    token_admins[0].mint(buyer, &500);
    token_admins[1].mint(buyer, &1000);

    let mut assets = Vec::new(&env);
    assets.push_back(BasketAsset {
        token: tokens[0].clone(),
        amount: 500,
    });
    assets.push_back(BasketAsset {
        token: tokens[1].clone(),
        amount: 1000,
    });

    let id = client.create_escrow_multi(buyer, seller, arbiter, &assets, &TIMEOUT);
    client.deposit_multi(&id);

    env.mock_auths(&[MockAuth {
        address: seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "release_multi",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    client.try_release_multi(&id).expect("outer ok").unwrap();
    assert_eq!(token_clients[0].balance(seller), 500);
    assert_eq!(token_clients[1].balance(seller), 1000);
    assert_eq!(client.get_status(&id), EscrowStatus::Completed);
}

#[test]
fn release_multi_rejects_non_seller() {
    let (env, _token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let (tokens, _, token_admins) = setup_multi(&env, 1);
    token_admins[0].mint(buyer, &500);

    let mut assets = Vec::new(&env);
    assets.push_back(BasketAsset {
        token: tokens[0].clone(),
        amount: 500,
    });

    let id = client.create_escrow_multi(buyer, seller, arbiter, &assets, &TIMEOUT);
    client.deposit_multi(&id);

    // Buyer attempts to authorize release_multi
    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "release_multi",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_release_multi(&id);
    assert_auth_abort!(res);
}

#[test]
fn release_multi_blank_envelope_aborts() {
    let (env, _token, _tc, _contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let (tokens, _, token_admins) = setup_multi(&env, 1);
    token_admins[0].mint(buyer, &500);

    let mut assets = Vec::new(&env);
    assets.push_back(BasketAsset {
        token: tokens[0].clone(),
        amount: 500,
    });

    let id = client.create_escrow_multi(buyer, seller, arbiter, &assets, &TIMEOUT);
    client.deposit_multi(&id);

    env.set_auths(&[]);
    let res = client.try_release_multi(&id);
    assert_auth_abort!(res);
}

#[test]
fn refund_multi_pre_deadline_accepts_seller_and_rejects_buyer() {
    let (env, _token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let (tokens, token_clients, token_admins) = setup_multi(&env, 1);
    token_admins[0].mint(buyer, &500);

    let mut assets = Vec::new(&env);
    assets.push_back(BasketAsset {
        token: tokens[0].clone(),
        amount: 500,
    });

    let id = client.create_escrow_multi(buyer, seller, arbiter, &assets, &TIMEOUT);
    client.deposit_multi(&id);

    // Pre-deadline: buyer auth is rejected
    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "refund_multi",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let res = client.try_refund_multi(&id);
    assert_auth_abort!(res);

    // Seller auth succeeds
    env.mock_auths(&[MockAuth {
        address: seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "refund_multi",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    client.try_refund_multi(&id).expect("outer ok").unwrap();
    assert_eq!(token_clients[0].balance(buyer), 500);
}

#[test]
fn refund_multi_post_deadline_accepts_buyer_and_rejects_seller() {
    let (env, _token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let (tokens, token_clients, token_admins) = setup_multi(&env, 1);
    token_admins[0].mint(buyer, &500);

    let mut assets = Vec::new(&env);
    assets.push_back(BasketAsset {
        token: tokens[0].clone(),
        amount: 500,
    });

    let id = client.create_escrow_multi(buyer, seller, arbiter, &assets, &TIMEOUT);
    client.deposit_multi(&id);

    env.ledger().set_timestamp(START + TIMEOUT + 10);

    // Post-deadline: seller auth is rejected
    env.mock_auths(&[MockAuth {
        address: seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "refund_multi",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let res = client.try_refund_multi(&id);
    assert_auth_abort!(res);

    // Buyer auth succeeds
    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "refund_multi",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    client.try_refund_multi(&id).expect("outer ok").unwrap();
    assert_eq!(token_clients[0].balance(buyer), 500);
}

#[test]
fn dispute_multi_accepts_claimant_and_rejects_non_claimant() {
    let (env, _token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let (tokens, _, token_admins) = setup_multi(&env, 1);
    token_admins[0].mint(buyer, &500);

    let mut assets = Vec::new(&env);
    assets.push_back(BasketAsset {
        token: tokens[0].clone(),
        amount: 500,
    });

    let id = client.create_escrow_multi(buyer, seller, arbiter, &assets, &TIMEOUT);
    client.deposit_multi(&id);

    // Claimant named is buyer, but seller signs
    env.mock_auths(&[MockAuth {
        address: seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "dispute_multi",
            args: (id, buyer).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let res = client.try_dispute_multi(&id, buyer);
    assert_auth_abort!(res);

    // Claimant named is buyer, buyer signs -> succeeds
    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "dispute_multi",
            args: (id, buyer).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    client
        .try_dispute_multi(&id, buyer)
        .expect("outer ok")
        .unwrap();
    assert_eq!(client.get_status(&id), EscrowStatus::Disputed);
}

#[test]
fn resolve_multi_accepts_arbiter_and_rejects_non_arbiter() {
    let (env, _token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let (tokens, token_clients, token_admins) = setup_multi(&env, 1);
    token_admins[0].mint(buyer, &500);

    let mut assets = Vec::new(&env);
    assets.push_back(BasketAsset {
        token: tokens[0].clone(),
        amount: 500,
    });

    let id = client.create_escrow_multi(buyer, seller, arbiter, &assets, &TIMEOUT);
    client.deposit_multi(&id);
    client.dispute_multi(&id, buyer);

    // Seller attempts to resolve -> rejected
    env.mock_auths(&[MockAuth {
        address: seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "resolve_multi",
            args: (id, true).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let res = client.try_resolve_multi(&id, &true);
    assert_auth_abort!(res);

    // Arbiter resolves -> succeeds
    env.mock_auths(&[MockAuth {
        address: arbiter,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "resolve_multi",
            args: (id, true).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    client
        .try_resolve_multi(&id, &true)
        .expect("outer ok")
        .unwrap();
    assert_eq!(token_clients[0].balance(seller), 500);
}

#[test]
fn cancel_multi_accepts_buyer_and_rejects_non_buyer() {
    let (env, _token, _tc, contract_id, client, accounts) = setup!();
    let buyer = &accounts.user1;
    let seller = &accounts.user2;
    let arbiter = &accounts.arbiter;
    let (tokens, _, _) = setup_multi(&env, 1);

    let mut assets = Vec::new(&env);
    assets.push_back(BasketAsset {
        token: tokens[0].clone(),
        amount: 500,
    });

    let id = client.create_escrow_multi(buyer, seller, arbiter, &assets, &TIMEOUT);

    // Seller attempts to cancel
    env.mock_auths(&[MockAuth {
        address: seller,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "cancel_multi",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let res = client.try_cancel_multi(&id);
    assert_auth_abort!(res);

    // Buyer cancels -> succeeds
    env.mock_auths(&[MockAuth {
        address: buyer,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "cancel_multi",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    client.try_cancel_multi(&id).expect("outer ok").unwrap();
    assert_eq!(client.get_status(&id), EscrowStatus::Cancelled);
}
