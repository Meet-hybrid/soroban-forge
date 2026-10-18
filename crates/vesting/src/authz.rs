//! Negative authorization tests for the vesting contract.
//!
//! The main unit suite (`tests.rs`) runs under `mock_all_auths`, which proves
//! the *call graph* of authorizations — who the contract asks to sign —
//! but never that a wrong signer or tampered arguments are rejected by the host.
//! This module covers the negative authorization domain across two layers:
//!
//! 1. **Enforce mode** (`mock_auths` / `set_auths`): each test arms
//!    authorization for exactly the signer(s) a scenario names and asserts
//!    that an unauthorized caller — or the correct signer signing over different
//!    arguments/schedule IDs — is rejected by the host (`InvokeError::Abort`),
//!    leaving lifecycle state, balances, and monotonic counters untouched.
//! 2. **Tree assertions** (`env.auths()` under `mock_all_auths`): pins the
//!    exact authorized-invocation tree the contract demands on creation and
//!    settlement claim paths.
//!
//! ## Host Mechanics (soroban-sdk 27.0.6)
//!
//! - `mock_auths` sets the invocation envelope and disables blanket mocking.
//!   Authorizations not matching a mocked auth fail and abort the transaction.
//! - `set_auths(&[])` is an empty envelope: every `require_auth` fails immediately.
//! - **Contract self-authorization is implicit**: the host auto-approves
//!   `require_auth` for the currently-executing contract during token payout.
//!   `env.auths()` records only the beneficiary's entrypoint frame with no
//!   sub-invocations, and a claim payout legitimately completes on the beneficiary's
//!   signature alone.

use crate::{SorobanForgeVestingClient, Vesting, VestingStatus};

extern crate std;
use soroban_sdk::testutils::{
    Address as _, AuthorizedFunction, AuthorizedInvocation, Ledger as _, MockAuth, MockAuthInvoke,
};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env, IntoVal, InvokeError, Symbol};

const START: u64 = 1_000_000;
const CLIFF: u64 = 1_000;
const DURATION: u64 = 4_000;
const TOTAL: i128 = 10_000;

/// Fresh env with blanket mocking for setup only (minting, contract registration).
/// The tested call re-arms the authorization envelope explicitly.
macro_rules! setup {
    () => {{
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(START);

        let admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(admin);
        let token = sac.address();
        let token_admin = StellarAssetClient::new(&env, &token);
        let token_client = TokenClient::new(&env, &token);

        let contract_id = env.register(Vesting, ());
        let client = SorobanForgeVestingClient::new(&env, &contract_id);

        let accounts = soroban_forge_test_utils::TestAccounts::generate(&env);
        // Mint tokens to vesting contract to back schedules
        token_admin.mint(&contract_id, &TOTAL);

        (env, token, token_client, contract_id, client, accounts)
    }};
}

/// Assert that a `try_` call aborted on authorization failure at the host level.
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
// create_schedule authorization & negative-auth tests
// -----------------------------------------------------------------------

#[test]
fn create_schedule_accepts_beneficiary_signature_with_matching_args() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let beneficiary = &accounts.user1;

    env.mock_auths(&[MockAuth {
        address: beneficiary,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "create_schedule",
            args: (
                &accounts.deployer,
                beneficiary,
                &token,
                TOTAL,
                CLIFF,
                DURATION,
            )
                .into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let id = client
        .try_create_schedule(
            &accounts.deployer,
            beneficiary,
            &token,
            &TOTAL,
            &CLIFF,
            &DURATION,
        )
        .expect("outer ok")
        .expect("contract ok");
    assert_eq!(id, 1);
    assert_eq!(client.get_status(&id), VestingStatus::Locked);
}

#[test]
fn create_schedule_rejects_signature_from_non_beneficiary() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let beneficiary = &accounts.user1;
    let attacker = &accounts.user2;

    // Attacker signs, but beneficiary is named in the call
    env.mock_auths(&[MockAuth {
        address: attacker,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "create_schedule",
            args: (
                &accounts.deployer,
                beneficiary,
                &token,
                TOTAL,
                CLIFF,
                DURATION,
            )
                .into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_create_schedule(
        &accounts.deployer,
        beneficiary,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );
    assert_auth_abort!(res);
}

#[test]
fn create_schedule_rejects_signature_over_different_amount() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let beneficiary = &accounts.user1;

    // Beneficiary signed for TOTAL, but transaction attempts TOTAL + 5000
    env.mock_auths(&[MockAuth {
        address: beneficiary,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "create_schedule",
            args: (
                &accounts.deployer,
                beneficiary,
                &token,
                TOTAL,
                CLIFF,
                DURATION,
            )
                .into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_create_schedule(
        &accounts.deployer,
        beneficiary,
        &token,
        &(TOTAL + 5000),
        &CLIFF,
        &DURATION,
    );
    assert_auth_abort!(res);
}

#[test]
fn create_schedule_rejects_signature_over_different_cliff_or_duration() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let beneficiary = &accounts.user1;

    // Beneficiary signed for CLIFF=1000, DURATION=4000
    env.mock_auths(&[MockAuth {
        address: beneficiary,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "create_schedule",
            args: (
                &accounts.deployer,
                beneficiary,
                &token,
                TOTAL,
                CLIFF,
                DURATION,
            )
                .into_val(&env),
            sub_invokes: &[],
        },
    }]);

    // Attempted call alters duration to 2000
    let res = client.try_create_schedule(
        &accounts.deployer,
        beneficiary,
        &token,
        &TOTAL,
        &CLIFF,
        &2000u64,
    );
    assert_auth_abort!(res);
}

#[test]
fn create_schedule_rejects_signature_over_different_token() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let beneficiary = &accounts.user1;
    let other_token = &accounts.validator;

    // Beneficiary signed for token, but call uses other_token
    env.mock_auths(&[MockAuth {
        address: beneficiary,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "create_schedule",
            args: (
                &accounts.deployer,
                beneficiary,
                &token,
                TOTAL,
                CLIFF,
                DURATION,
            )
                .into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_create_schedule(
        &accounts.deployer,
        beneficiary,
        other_token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );
    assert_auth_abort!(res);
}

#[test]
fn create_schedule_authorization_tree_is_beneficiary_root() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let beneficiary = &accounts.user1;

    let id = client.create_schedule(
        &accounts.deployer,
        beneficiary,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );

    assert_eq!(
        env.auths(),
        [(
            beneficiary.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "create_schedule"),
                    (
                        &accounts.deployer,
                        beneficiary,
                        &token,
                        TOTAL,
                        CLIFF,
                        DURATION
                    )
                        .into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
    assert_eq!(id, 1);
}

// -----------------------------------------------------------------------
// claim authorization & negative-auth tests
// -----------------------------------------------------------------------

#[test]
fn claim_accepts_beneficiary_signature_alone() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let beneficiary = &accounts.user1;
    let id = client.create_schedule(
        &accounts.deployer,
        beneficiary,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );

    env.ledger().set_timestamp(START + DURATION);

    // Enforce mode: arm exact beneficiary claim authorization
    env.mock_auths(&[MockAuth {
        address: beneficiary,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "claim",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let paid = client.try_claim(&id).expect("outer ok").expect("claim ok");
    assert_eq!(paid, TOTAL);
    assert_eq!(tc.balance(beneficiary), TOTAL);
    assert_eq!(tc.balance(&contract_id), 0);
    assert_eq!(client.get_status(&id), VestingStatus::Completed);
}

#[test]
fn claim_rejects_signature_from_non_beneficiary() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let beneficiary = &accounts.user1;
    let attacker = &accounts.user2;
    let id = client.create_schedule(
        &accounts.deployer,
        beneficiary,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );

    env.ledger().set_timestamp(START + DURATION);

    // Attacker attempts to claim beneficiary's schedule
    env.mock_auths(&[MockAuth {
        address: attacker,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "claim",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_claim(&id);
    assert_auth_abort!(res);
    assert_eq!(tc.balance(beneficiary), 0);
    assert_eq!(tc.balance(&contract_id), TOTAL);
    assert_eq!(client.get_status(&id), VestingStatus::Vesting);
}

#[test]
fn claim_rejects_signature_replayed_for_different_schedule_id() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let beneficiary = &accounts.user1;
    let id1 = client.create_schedule(
        &accounts.deployer,
        beneficiary,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );
    let id2 = client.create_schedule(
        &accounts.deployer,
        beneficiary,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );

    env.ledger().set_timestamp(START + DURATION);

    // Signature armed specifically for id1
    env.mock_auths(&[MockAuth {
        address: beneficiary,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "claim",
            args: (id1,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    // Attempting to use id1 auth on id2 must fail
    let res = client.try_claim(&id2);
    assert_auth_abort!(res);
    assert_eq!(tc.balance(beneficiary), 0);
}

#[test]
fn claim_authorization_tree_is_beneficiary_root_entrypoint() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let beneficiary = &accounts.user1;
    let id = client.create_schedule(
        &accounts.deployer,
        beneficiary,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );

    env.ledger().set_timestamp(START + DURATION);
    client.claim(&id);

    // Contract self-authorization is implicit in the Soroban host; the recorded
    // tree contains the beneficiary's entrypoint frame only with 0 sub-invocations.
    assert_eq!(
        env.auths(),
        [(
            beneficiary.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "claim"),
                    (id,).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
}

// -----------------------------------------------------------------------
// revoke authorization & negative-auth tests (issue #67)
// -----------------------------------------------------------------------

#[test]
fn revoke_accepts_funder_signature_alone() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let funder = &accounts.deployer;
    let beneficiary = &accounts.user1;
    let id = client.create_schedule(funder, beneficiary, &token, &TOTAL, &CLIFF, &DURATION);

    env.ledger().set_timestamp(START + DURATION / 2);

    // Enforce mode: arm exactly the funder's revoke authorization.
    env.mock_auths(&[MockAuth {
        address: funder,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "revoke",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    client
        .try_revoke(&id)
        .expect("outer ok")
        .expect("revoke ok");
    assert_eq!(client.get_status(&id), VestingStatus::Revoked);
}

#[test]
fn revoke_rejects_signature_from_non_funder() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let funder = &accounts.deployer;
    let beneficiary = &accounts.user1;
    let attacker = &accounts.user2;
    let id = client.create_schedule(funder, beneficiary, &token, &TOTAL, &CLIFF, &DURATION);

    env.ledger().set_timestamp(START + DURATION / 2);

    // A third party signs the revoke attempt; the funder does not.
    env.mock_auths(&[MockAuth {
        address: attacker,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "revoke",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_revoke(&id);
    assert_auth_abort!(res);
    // The schedule is untouched: still Vesting, nothing frozen.
    assert_eq!(client.get_status(&id), VestingStatus::Vesting);
    assert_eq!(client.get_schedule(&id).revoked_vested, None);
}

#[test]
fn revoke_rejects_signature_from_beneficiary() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let funder = &accounts.deployer;
    let beneficiary = &accounts.user1;
    let id = client.create_schedule(funder, beneficiary, &token, &TOTAL, &CLIFF, &DURATION);

    env.ledger().set_timestamp(START + DURATION / 2);

    // The beneficiary cannot revoke their own grant, even with a valid
    // signature over the exact call.
    env.mock_auths(&[MockAuth {
        address: beneficiary,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "revoke",
            args: (id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let res = client.try_revoke(&id);
    assert_auth_abort!(res);
    assert_eq!(client.get_status(&id), VestingStatus::Vesting);
    assert_eq!(client.get_schedule(&id).revoked_vested, None);
}

#[test]
fn revoke_rejects_signature_over_different_schedule_id() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let funder = &accounts.deployer;
    let beneficiary = &accounts.user1;
    let id1 = client.create_schedule(funder, beneficiary, &token, &TOTAL, &CLIFF, &DURATION);
    let id2 = client.create_schedule(funder, beneficiary, &token, &TOTAL, &CLIFF, &DURATION);

    env.ledger().set_timestamp(START + DURATION / 2);

    // Funder authorization armed specifically for id1.
    env.mock_auths(&[MockAuth {
        address: funder,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "revoke",
            args: (id1,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    // Using id1's auth on id2 must fail.
    let res = client.try_revoke(&id2);
    assert_auth_abort!(res);
    assert_eq!(client.get_schedule(&id2).revoked_vested, None);
}

#[test]
fn revoke_authorization_tree_is_funder_root_entrypoint() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let funder = &accounts.deployer;
    let beneficiary = &accounts.user1;
    let id = client.create_schedule(funder, beneficiary, &token, &TOTAL, &CLIFF, &DURATION);

    env.ledger().set_timestamp(START + DURATION / 2);
    client.revoke(&id);

    // The recorded tree contains the funder's entrypoint frame only, with no
    // sub-invocations: revocation moves no tokens.
    assert_eq!(
        env.auths(),
        [(
            funder.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    contract_id.clone(),
                    Symbol::new(&env, "revoke"),
                    (id,).into_val(&env),
                )),
                sub_invocations: std::vec![],
            },
        )],
    );
}

#[test]
fn blank_envelope_aborts_revoke_and_preserves_schedule() {
    let (env, token, _tc, _contract_id, client, accounts) = setup!();
    let funder = &accounts.deployer;
    let beneficiary = &accounts.user1;
    let id = client.create_schedule(funder, beneficiary, &token, &TOTAL, &CLIFF, &DURATION);

    env.ledger().set_timestamp(START + DURATION / 2);

    // Empty envelope: every require_auth fails immediately.
    env.set_auths(&[]);

    let res = client.try_revoke(&id);
    assert_auth_abort!(res);
    assert_eq!(client.get_status(&id), VestingStatus::Vesting);
    assert_eq!(client.get_schedule(&id).revoked_vested, None);

    // Re-arming proves the id space and storage were left untouched.
    env.mock_all_auths();
    client.revoke(&id);
    assert_eq!(client.get_status(&id), VestingStatus::Revoked);
}

// -----------------------------------------------------------------------
// Blank envelope tests (no authorizations armed)
// -----------------------------------------------------------------------

#[test]
fn blank_envelope_aborts_create_schedule_and_writes_nothing() {
    let (env, token, _tc, _contract_id, client, accounts) = setup!();
    let beneficiary = &accounts.user1;

    // Empty envelope
    env.set_auths(&[]);

    let res = client.try_create_schedule(
        &accounts.deployer,
        beneficiary,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );
    assert_auth_abort!(res);

    // Re-arming proves no ID counter was consumed
    env.mock_all_auths();
    let id = client.create_schedule(
        &accounts.deployer,
        beneficiary,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );
    assert_eq!(id, 1);
}

#[test]
fn blank_envelope_aborts_claim_and_preserves_custody() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let beneficiary = &accounts.user1;
    let id = client.create_schedule(
        &accounts.deployer,
        beneficiary,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );

    env.ledger().set_timestamp(START + DURATION);

    // Blank envelope
    env.set_auths(&[]);

    let res = client.try_claim(&id);
    assert_auth_abort!(res);
    assert_eq!(tc.balance(beneficiary), 0);
    assert_eq!(tc.balance(&contract_id), TOTAL);
    assert_eq!(client.get_status(&id), VestingStatus::Vesting);
}

#[test]
fn reassignment_accepts_funder_signature_alone() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let funder = &accounts.deployer;
    let id = client.create_schedule(funder, &accounts.user1, &token, &TOTAL, &CLIFF, &DURATION);
    let new_beneficiary = &accounts.user2;

    env.mock_auths(&[MockAuth {
        address: funder,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "reassign_beneficiary",
            args: (id, new_beneficiary).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    client
        .try_reassign_beneficiary(&id, new_beneficiary)
        .expect("outer ok")
        .expect("reassignment ok");
    assert_eq!(client.get_schedule(&id).beneficiary, *new_beneficiary);
    assert_eq!(client.get_schedule(&id).reassignment_count, 1);
}

#[test]
fn reassignment_rejects_old_new_and_outsider_signatures() {
    let (env, token, _tc, contract_id, client, accounts) = setup!();
    let id = client.create_schedule(
        &accounts.deployer,
        &accounts.user1,
        &token,
        &TOTAL,
        &CLIFF,
        &DURATION,
    );
    let new_beneficiary = accounts.user2.clone();
    let outsider = Address::generate(&env);

    for caller in [accounts.user1.clone(), accounts.user2.clone(), outsider] {
        env.mock_auths(&[MockAuth {
            address: &caller,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "reassign_beneficiary",
                args: (id, &new_beneficiary).into_val(&env),
                sub_invokes: &[],
            },
        }]);

        let res = client.try_reassign_beneficiary(&id, &new_beneficiary);
        assert_auth_abort!(res);
        assert_eq!(client.get_schedule(&id).beneficiary, accounts.user1);
        assert_eq!(client.get_schedule(&id).reassignment_count, 0);
    }
}

// -----------------------------------------------------------------------
// Schedule creation and cancellation authorization matrix (Issue #451)
// -----------------------------------------------------------------------

#[test]
fn test_creation_and_cancellation_authorization_matrix() {
    let (env, token, tc, contract_id, client, accounts) = setup!();
    let creator = &accounts.deployer;
    let beneficiary = &accounts.user1;
    let third_party = &accounts.user2;
    let attacker = &accounts.user3;

    // 1. CREATION AUTHORIZATION MATRIX
    // A. Unauthorized creator / third-party / attacker signing alone must abort at host level
    for unauthorized_signer in [creator, third_party, attacker] {
        env.mock_auths(&[MockAuth {
            address: unauthorized_signer,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "create_schedule",
                args: (creator, beneficiary, &token, TOTAL, CLIFF, DURATION).into_val(&env),
                sub_invokes: &[],
            },
        }]);

        let res =
            client.try_create_schedule(creator, beneficiary, &token, &TOTAL, &CLIFF, &DURATION);
        assert_auth_abort!(res);
    }

    // Storage and monotonic counter unchanged: schedule 1 must not exist
    assert_eq!(
        client.try_get_schedule(&1).unwrap_err().unwrap(),
        soroban_forge_shared_utils::ForgeError::NotFound
    );
    assert_eq!(
        client.try_get_status(&1).unwrap_err().unwrap(),
        soroban_forge_shared_utils::ForgeError::NotFound
    );

    // B. Authorized beneficiary signature creates the schedule
    env.mock_auths(&[MockAuth {
        address: beneficiary,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "create_schedule",
            args: (creator, beneficiary, &token, TOTAL, CLIFF, DURATION).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let sched_id = client
        .try_create_schedule(creator, beneficiary, &token, &TOTAL, &CLIFF, &DURATION)
        .expect("outer ok")
        .expect("contract ok");
    assert_eq!(sched_id, 1);
    assert_eq!(client.get_status(&sched_id), VestingStatus::Locked);
    let sched_record = client.get_schedule(&sched_id);
    assert_eq!(sched_record.funder, *creator);
    assert_eq!(sched_record.beneficiary, *beneficiary);
    assert_eq!(sched_record.total_amount, TOTAL);
    assert_eq!(sched_record.claimed, 0);

    // 2. CANCELLATION / REVOCATION AUTHORIZATION MATRIX
    env.ledger().set_timestamp(START + CLIFF + 500); // midway vesting

    // A. Non-creator roles (beneficiary, third-party, attacker) are rejected
    for unauthorized_caller in [beneficiary, third_party, attacker] {
        env.mock_auths(&[MockAuth {
            address: unauthorized_caller,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "revoke",
                args: (sched_id,).into_val(&env),
                sub_invokes: &[],
            },
        }]);

        let res = client.try_revoke(&sched_id);
        assert_auth_abort!(res);

        // Assert relevant state remains completely unchanged on rejected calls
        assert_eq!(client.get_status(&sched_id), VestingStatus::Vesting);
        let s = client.get_schedule(&sched_id);
        assert_eq!(s.status, VestingStatus::Locked); // stored status updated on claim/revoke
        assert_eq!(s.revoked_vested, None);
        assert_eq!(tc.balance(&contract_id), TOTAL);
        assert_eq!(tc.balance(creator), 0);
        assert_eq!(tc.balance(beneficiary), 0);
    }

    // B. Non-existent schedule revocation rejected with NotFound
    env.mock_auths(&[MockAuth {
        address: creator,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "revoke",
            args: (999u64,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_eq!(
        client.try_revoke(&999u64).unwrap_err().unwrap(),
        soroban_forge_shared_utils::ForgeError::NotFound
    );

    // C. Valid creator/funder signature successfully revokes the schedule
    env.mock_auths(&[MockAuth {
        address: creator,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "revoke",
            args: (sched_id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    client
        .try_revoke(&sched_id)
        .expect("outer ok")
        .expect("revoke ok");
    assert_eq!(client.get_status(&sched_id), VestingStatus::Revoked);
    let revoked_record = client.get_schedule(&sched_id);
    assert_eq!(revoked_record.status, VestingStatus::Revoked);
    assert!(revoked_record.revoked_vested.is_some());

    // D. Second revocation attempt on already revoked schedule fails with InvalidInput, state preserved
    env.mock_auths(&[MockAuth {
        address: creator,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "revoke",
            args: (sched_id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let double_revoke_res = client.try_revoke(&sched_id).unwrap_err().unwrap();
    assert_eq!(
        double_revoke_res,
        soroban_forge_shared_utils::ForgeError::InvalidInput
    );
    assert_eq!(client.get_status(&sched_id), VestingStatus::Revoked);
    assert_eq!(
        client.get_schedule(&sched_id).revoked_vested,
        revoked_record.revoked_vested
    );

    // 3. CLAIM AUTHORIZATION MATRIX ON REVOKED SCHEDULE
    // A. Creator, third-party, and attacker cannot claim beneficiary's vested tokens
    for unauthorized_claimer in [creator, third_party, attacker] {
        env.mock_auths(&[MockAuth {
            address: unauthorized_claimer,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "claim",
                args: (sched_id,).into_val(&env),
                sub_invokes: &[],
            },
        }]);

        let claim_res = client.try_claim(&sched_id);
        assert_auth_abort!(claim_res);
        assert_eq!(tc.balance(beneficiary), 0);
        assert_eq!(tc.balance(&contract_id), TOTAL);
    }

    // B. Beneficiary can claim up to the frozen revoked amount
    let claimable_amount = client.claimable(&sched_id);
    assert!(claimable_amount > 0);
    env.mock_auths(&[MockAuth {
        address: beneficiary,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "claim",
            args: (sched_id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let claimed = client
        .try_claim(&sched_id)
        .expect("outer ok")
        .expect("claim ok");
    assert_eq!(claimed, claimable_amount);
    assert_eq!(tc.balance(beneficiary), claimed);
    assert_eq!(tc.balance(&contract_id), TOTAL - claimed);
    assert_eq!(client.claimable(&sched_id), 0);
}
