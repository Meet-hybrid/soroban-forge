//! Enforced authorization failure and atomic rollback tests across every
//! cross-contract boundary:
//! - MultiSigWallet -> downstream contract / token withdrawal
//! - Escrow -> MarketplaceRoyalties settlement
//! - Vesting -> SubscriptionPayments funding
//! - SubscriptionPayments -> Escrow settlement
//!
//! Setup phases run under `setup_composition_env`; every tested boundary
//! switches into strict enforced authorization mode via `env.set_auths(&[])`
//! or explicit `env.mock_auths(...)` trees so wrong signers, missing nested
//! sub-invocations, and unauthorized cross-contract hops are genuinely rejected
//! by the Soroban host with full state and token-balance rollback.

use crate::assert_auth_abort;
use crate::harness::{deploy_sac_token, setup_composition_env, ConservationLedger};
use soroban_forge_escrow::{Escrow, EscrowStatus, SorobanForgeEscrowClient};
use soroban_forge_marketplace_royalties::{
    MarketplaceRoyalties, SorobanForgeMarketplaceRoyaltiesClient,
};
use soroban_forge_multi_sig_wallet::{MultiSigWallet, SorobanForgeMultiSigWalletClient, TxStatus};
use soroban_forge_shared_utils::ForgeError;
use soroban_forge_subscription_payments::{
    SorobanForgeSubscriptionPaymentsClient, SubscriptionPayments, SubscriptionStatus,
};
use soroban_forge_vesting::{
    CreateScheduleArgs, SorobanForgeVestingClient, Vesting, VestingStatus,
};
use soroban_sdk::testutils::{Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{contract, contractimpl, contracttype, Address, Bytes, Env, IntoVal, Symbol};

const START: u64 = 6_000_000;
const SUPPLY: i128 = 1_000;
const AMOUNT: i128 = 600;
const TIMEOUT: u64 = 86_400;
const REVIEW_PERIOD: u64 = 3_600;
const BILLING_PERIOD: u64 = 1_000;
const ROYALTY_BPS: u32 = 1_000; // 10%

/// Test-only target invoked by `MultiSigWallet::execute` that mutates local
/// storage and then enforces an external signer authorization boundary.
#[contract]
struct AuthGatedWalletTarget;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
enum TargetKey {
    Executions,
    RequiredSigner,
}

#[contractimpl]
impl AuthGatedWalletTarget {
    pub fn configure(env: Env, required_signer: Address) {
        env.storage()
            .instance()
            .set(&TargetKey::RequiredSigner, &required_signer);
    }

    pub fn guarded_step(env: Env, _tag: Bytes) -> Result<(), ForgeError> {
        let count: u32 = env
            .storage()
            .instance()
            .get(&TargetKey::Executions)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&TargetKey::Executions, &(count + 1));

        let signer: Address = env
            .storage()
            .instance()
            .get(&TargetKey::RequiredSigner)
            .unwrap();
        signer.require_auth();
        Ok(())
    }

    pub fn executions(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&TargetKey::Executions)
            .unwrap_or(0)
    }
}

/// Test-only atomic bridge: releases an escrow (permissionless after review
/// window) and immediately settles proceeds through `MarketplaceRoyalties`.
#[contract]
struct AtomicEscrowRoyaltyBridge;

#[contractimpl]
impl AtomicEscrowRoyaltyBridge {
    #[allow(clippy::too_many_arguments)]
    pub fn release_and_settle(
        env: Env,
        escrow_addr: Address,
        royalties_addr: Address,
        collection: Address,
        token: Address,
        seller: Address,
        escrow_id: u64,
        amount: i128,
    ) -> Result<(), ForgeError> {
        SorobanForgeEscrowClient::new(&env, &escrow_addr).execute_release(&escrow_id);
        SorobanForgeMarketplaceRoyaltiesClient::new(&env, &royalties_addr).settle_sale(
            &collection,
            &token,
            &seller,
            &seller,
            &amount,
        );
        Ok(())
    }
}

/// Test-only atomic bridge: claims vested tokens for beneficiary and deposits
/// a portion into a prepaid `SubscriptionPayments` contract.
#[contract]
struct AtomicVestingToSubBridge;

#[contractimpl]
impl AtomicVestingToSubBridge {
    pub fn claim_and_deposit(
        env: Env,
        beneficiary: Address,
        vesting_addr: Address,
        sub_addr: Address,
        schedule_id: u64,
        sub_id: u64,
        deposit_amount: i128,
    ) -> Result<i128, ForgeError> {
        beneficiary.require_auth();
        let claimed = SorobanForgeVestingClient::new(&env, &vesting_addr).claim(&schedule_id);
        SorobanForgeSubscriptionPaymentsClient::new(&env, &sub_addr)
            .deposit(&sub_id, &deposit_amount);
        Ok(claimed)
    }
}

/// Test-only atomic bridge: charges a prepaid subscription to `provider` and
/// immediately creates and funds an `Escrow` using the billed proceeds.
#[contract]
struct AtomicSubToEscrowBridge;

#[contractimpl]
impl AtomicSubToEscrowBridge {
    #[allow(clippy::too_many_arguments)]
    pub fn charge_and_fund_escrow(
        env: Env,
        provider: Address,
        seller: Address,
        arbiter: Address,
        token: Address,
        sub_addr: Address,
        escrow_addr: Address,
        sub_id: u64,
        timeout: u64,
    ) -> Result<u64, ForgeError> {
        provider.require_auth();
        let billed = SorobanForgeSubscriptionPaymentsClient::new(&env, &sub_addr).charge(&sub_id);
        let escrow = SorobanForgeEscrowClient::new(&env, &escrow_addr);
        let esc_id = escrow.create_escrow(&provider, &seller, &arbiter, &token, &billed, &timeout);
        escrow.deposit(&esc_id);
        Ok(esc_id)
    }
}

#[test]
fn wallet_boundary_auth_failures_and_downstream_rollback() {
    let (env, actors) = setup_composition_env(START);
    let (token_admin_id, token_id, token_admin, _token) = deploy_sac_token(&env);

    let wallet_id = env.register(MultiSigWallet, ());
    let wallet = SorobanForgeMultiSigWalletClient::new(&env, &wallet_id);
    let target_id = env.register(AuthGatedWalletTarget, ());
    let target = AuthGatedWalletTargetClient::new(&env, &target_id);

    wallet.initialize(&actors.wallet_owners(&env), &2);
    target.configure(&actors.arbiter);
    token_admin.mint(&actors.deployer, &SUPPLY);
    wallet.deposit(&token_id, &actors.deployer, &SUPPLY);

    let mut ledger = ConservationLedger::new(token_id.clone(), SUPPLY);
    actors.register_all(&mut ledger);
    ledger.register("token_admin", token_admin_id);
    ledger.register("token", token_id.clone());
    ledger.register("wallet", wallet_id.clone());
    ledger.register("target", target_id.clone());

    let snap = ledger.snapshot(&env);

    // 1. Outsider cannot submit a withdrawal even with their own signature.
    env.mock_auths(&[MockAuth {
        address: &actors.outsider,
        invoke: &MockAuthInvoke {
            contract: &wallet_id,
            fn_name: "submit_withdrawal",
            args: (&actors.outsider, &token_id, &actors.outsider, AMOUNT).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_eq!(
        wallet.try_submit_withdrawal(&actors.outsider, &token_id, &actors.outsider, &AMOUNT),
        Err(Ok(ForgeError::Unauthorized))
    );
    assert_eq!(wallet.get_tx_count(), 0);
    ledger.assert_unchanged(&env, &snap);

    // 2. Owner submission with mismatched signer aborts at host auth boundary.
    env.mock_auths(&[MockAuth {
        address: &actors.owner2,
        invoke: &MockAuthInvoke {
            contract: &wallet_id,
            fn_name: "submit_withdrawal",
            args: (&actors.owner1, &token_id, &actors.seller, AMOUNT).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(wallet.try_submit_withdrawal(
        &actors.owner1,
        &token_id,
        &actors.seller,
        &AMOUNT
    ));
    assert_eq!(wallet.get_tx_count(), 0);
    ledger.assert_unchanged(&env, &snap);

    // 3. Submit a typed call with valid owner1 auth, confirm with owner1 and owner2,
    //    reject wrong-signer confirm along the way.
    let tag = Bytes::from_slice(&env, b"guarded-call");
    let call_args = soroban_sdk::vec![&env, tag.clone().into_val(&env)];
    let fn_name = Symbol::new(&env, "guarded_step");
    env.mock_auths(&[MockAuth {
        address: &actors.owner1,
        invoke: &MockAuthInvoke {
            contract: &wallet_id,
            fn_name: "submit_call",
            args: (
                &actors.owner1,
                &target_id,
                &fn_name,
                &call_args,
                Option::<u64>::None,
            )
                .into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let tx_id = wallet.submit_call(&actors.owner1, &target_id, &fn_name, &call_args, &None);
    assert_eq!(tx_id, 1);

    // Confirm with missing signature aborts and does not record confirmation.
    env.set_auths(&[]);
    assert_auth_abort!(wallet.try_confirm(&tx_id, &actors.owner1));
    assert_eq!(wallet.get_confirmations(&tx_id).len(), 0);

    // Properly confirm with owner1 and owner2.
    env.mock_auths(&[MockAuth {
        address: &actors.owner1,
        invoke: &MockAuthInvoke {
            contract: &wallet_id,
            fn_name: "confirm",
            args: (tx_id, &actors.owner1).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    wallet.confirm(&tx_id, &actors.owner1);

    env.mock_auths(&[MockAuth {
        address: &actors.owner2,
        invoke: &MockAuthInvoke {
            contract: &wallet_id,
            fn_name: "confirm",
            args: (tx_id, &actors.owner2).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    wallet.confirm(&tx_id, &actors.owner2);

    // 4. Execute with NO downstream arbiter auth: target's `require_auth` fails,
    //    rolling back BOTH target's local counter increment AND wallet tx status.
    env.set_auths(&[]);
    assert!(wallet.try_execute(&tx_id).is_err());
    assert_eq!(wallet.get_tx(&tx_id).status, TxStatus::Pending);
    assert_eq!(target.executions(), 0);
    ledger.assert_unchanged(&env, &snap);

    // 5. Execute again WITH arbiter's authorization on the nested target call -> succeeds.
    env.mock_auths(&[MockAuth {
        address: &actors.arbiter,
        invoke: &MockAuthInvoke {
            contract: &target_id,
            fn_name: "guarded_step",
            args: (tag,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    wallet.execute(&tx_id);
    assert_eq!(wallet.get_tx(&tx_id).status, TxStatus::Executed);
    assert_eq!(target.executions(), 1);
    ledger.assert_unchanged(&env, &snap);

    // 6. Vetoed withdrawal cannot execute and preserves balances.
    env.mock_all_auths();
    let w_tx = wallet.submit_withdrawal(&actors.owner1, &token_id, &actors.seller, &AMOUNT);
    wallet.confirm(&w_tx, &actors.owner1);
    wallet.confirm(&w_tx, &actors.owner2);
    wallet.reject(&w_tx, &actors.owner3);
    assert_eq!(wallet.get_tx(&w_tx).status, TxStatus::Rejected);
    assert_eq!(wallet.try_execute(&w_tx), Err(Ok(ForgeError::InvalidInput)));
    assert_eq!(wallet.balance(&token_id), SUPPLY);
    ledger.assert_unchanged(&env, &snap);
}

#[test]
fn escrow_boundary_auth_failures_and_settlement_rollback() {
    let (env, actors) = setup_composition_env(START);
    let (token_admin_id, token_id, token_admin, _token) = deploy_sac_token(&env);

    let escrow_id = env.register(Escrow, ());
    let escrow = SorobanForgeEscrowClient::new(&env, &escrow_id);
    let royalties_id = env.register(MarketplaceRoyalties, ());
    let royalties = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &royalties_id);
    let bridge_id = env.register(AtomicEscrowRoyaltyBridge, ());
    let bridge = AtomicEscrowRoyaltyBridgeClient::new(&env, &bridge_id);

    royalties.set_royalty(&actors.collection, &actors.royalty_recipient, &ROYALTY_BPS);
    token_admin.mint(&actors.owner1, &SUPPLY);

    let mut ledger = ConservationLedger::new(token_id.clone(), SUPPLY);
    actors.register_all(&mut ledger);
    ledger.register("token_admin", token_admin_id);
    ledger.register("token", token_id.clone());
    ledger.register("escrow", escrow_id.clone());
    ledger.register("royalties", royalties_id.clone());
    ledger.register("bridge", bridge_id.clone());

    let initial_snap = ledger.snapshot(&env);

    // 1. Wrong signer (seller instead of buyer) cannot create escrow.
    env.mock_auths(&[MockAuth {
        address: &actors.seller,
        invoke: &MockAuthInvoke {
            contract: &escrow_id,
            fn_name: "create_escrow_with_review",
            args: (
                &actors.owner1,
                &actors.seller,
                &actors.arbiter,
                &token_id,
                AMOUNT,
                TIMEOUT,
                REVIEW_PERIOD,
            )
                .into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(escrow.try_create_escrow_with_review(
        &actors.owner1,
        &actors.seller,
        &actors.arbiter,
        &token_id,
        &AMOUNT,
        &TIMEOUT,
        &REVIEW_PERIOD,
    ));
    assert_eq!(escrow.try_get_status(&1), Err(Ok(ForgeError::NotFound)));
    ledger.assert_unchanged(&env, &initial_snap);

    // Create escrow with valid buyer signature.
    env.mock_auths(&[MockAuth {
        address: &actors.owner1,
        invoke: &MockAuthInvoke {
            contract: &escrow_id,
            fn_name: "create_escrow_with_review",
            args: (
                &actors.owner1,
                &actors.seller,
                &actors.arbiter,
                &token_id,
                AMOUNT,
                TIMEOUT,
                REVIEW_PERIOD,
            )
                .into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let esc_num = escrow.create_escrow_with_review(
        &actors.owner1,
        &actors.seller,
        &actors.arbiter,
        &token_id,
        &AMOUNT,
        &TIMEOUT,
        &REVIEW_PERIOD,
    );
    assert_eq!(escrow.get_status(&esc_num), EscrowStatus::Pending);

    // 2. Deposit with buyer root auth ONLY (missing nested SAC transfer sub-invoke)
    //    fails with TokenTransferFailed and keeps escrow Pending.
    env.mock_auths(&[MockAuth {
        address: &actors.owner1,
        invoke: &MockAuthInvoke {
            contract: &escrow_id,
            fn_name: "deposit",
            args: (esc_num,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_eq!(
        escrow.try_deposit(&esc_num),
        Err(Ok(ForgeError::TokenTransferFailed))
    );
    assert_eq!(escrow.get_status(&esc_num), EscrowStatus::Pending);
    ledger.assert_unchanged(&env, &initial_snap);

    // Deposit with complete auth tree (root deposit + nested SAC transfer) succeeds.
    env.mock_auths(&[MockAuth {
        address: &actors.owner1,
        invoke: &MockAuthInvoke {
            contract: &escrow_id,
            fn_name: "deposit",
            args: (esc_num,).into_val(&env),
            sub_invokes: &[MockAuthInvoke {
                contract: &token_id,
                fn_name: "transfer",
                args: (&actors.owner1, &escrow_id, AMOUNT).into_val(&env),
                sub_invokes: &[],
            }],
        },
    }]);
    escrow.deposit(&esc_num);
    assert_eq!(escrow.get_status(&esc_num), EscrowStatus::Funded);
    ledger.assert_conservation(&env, &[("owner1", SUPPLY - AMOUNT), ("escrow", AMOUNT)]);
    let funded_snap = ledger.snapshot(&env);

    // 3. Buyer cannot forge seller's `request_release`; seller cannot forge buyer's `release`.
    env.mock_auths(&[MockAuth {
        address: &actors.owner1,
        invoke: &MockAuthInvoke {
            contract: &escrow_id,
            fn_name: "request_release",
            args: (esc_num, AMOUNT).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(escrow.try_request_release(&esc_num, &AMOUNT));

    env.mock_auths(&[MockAuth {
        address: &actors.seller,
        invoke: &MockAuthInvoke {
            contract: &escrow_id,
            fn_name: "release",
            args: (esc_num,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(escrow.try_release(&esc_num));
    assert_eq!(escrow.get_status(&esc_num), EscrowStatus::Funded);
    ledger.assert_unchanged(&env, &funded_snap);

    // Seller validly requests release and review window elapses.
    env.mock_auths(&[MockAuth {
        address: &actors.seller,
        invoke: &MockAuthInvoke {
            contract: &escrow_id,
            fn_name: "request_release",
            args: (esc_num, AMOUNT).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let release_at = escrow.request_release(&esc_num, &AMOUNT);
    env.ledger().set_timestamp(release_at);

    // 4. Cross-contract auth failure rollback across Escrow -> MarketplaceRoyalties:
    //    `execute_release` succeeds inside the frame (permissionless), then `settle_sale`
    //    aborts because `collection` did not authorize! Whole frame must roll back.
    let seller_net = 540_i128;
    let royalty_share = 60_i128;
    env.mock_auths(&[MockAuth {
        address: &actors.seller,
        invoke: &MockAuthInvoke {
            contract: &royalties_id,
            fn_name: "settle_sale",
            args: (
                &actors.collection,
                &token_id,
                &actors.seller,
                &actors.seller,
                AMOUNT,
            )
                .into_val(&env),
            sub_invokes: &[
                MockAuthInvoke {
                    contract: &token_id,
                    fn_name: "transfer",
                    args: (&actors.seller, &actors.seller, seller_net).into_val(&env),
                    sub_invokes: &[],
                },
                MockAuthInvoke {
                    contract: &token_id,
                    fn_name: "transfer",
                    args: (&actors.seller, &actors.royalty_recipient, royalty_share).into_val(&env),
                    sub_invokes: &[],
                },
            ],
        },
    }]);
    assert_auth_abort!(bridge.try_release_and_settle(
        &escrow_id,
        &royalties_id,
        &actors.collection,
        &token_id,
        &actors.seller,
        &esc_num,
        &AMOUNT,
    ));
    // Upstream escrow release was completely rolled back.
    assert_eq!(escrow.get_status(&esc_num), EscrowStatus::Funded);
    assert_eq!(
        royalties.try_get_settlement_summary(&actors.collection),
        Err(Ok(ForgeError::NotFound))
    );
    ledger.assert_unchanged(&env, &funded_snap);

    // 5. Arming BOTH collection and seller auth trees allows atomic release + settlement.
    env.mock_auths(&[
        MockAuth {
            address: &actors.collection,
            invoke: &MockAuthInvoke {
                contract: &royalties_id,
                fn_name: "settle_sale",
                args: (
                    &actors.collection,
                    &token_id,
                    &actors.seller,
                    &actors.seller,
                    AMOUNT,
                )
                    .into_val(&env),
                sub_invokes: &[],
            },
        },
        MockAuth {
            address: &actors.seller,
            invoke: &MockAuthInvoke {
                contract: &royalties_id,
                fn_name: "settle_sale",
                args: (
                    &actors.collection,
                    &token_id,
                    &actors.seller,
                    &actors.seller,
                    AMOUNT,
                )
                    .into_val(&env),
                sub_invokes: &[
                    MockAuthInvoke {
                        contract: &token_id,
                        fn_name: "transfer",
                        args: (&actors.seller, &actors.seller, seller_net).into_val(&env),
                        sub_invokes: &[],
                    },
                    MockAuthInvoke {
                        contract: &token_id,
                        fn_name: "transfer",
                        args: (&actors.seller, &actors.royalty_recipient, royalty_share)
                            .into_val(&env),
                        sub_invokes: &[],
                    },
                ],
            },
        },
    ]);
    bridge.release_and_settle(
        &escrow_id,
        &royalties_id,
        &actors.collection,
        &token_id,
        &actors.seller,
        &esc_num,
        &AMOUNT,
    );
    assert_eq!(escrow.get_status(&esc_num), EscrowStatus::Completed);
    assert_eq!(
        royalties.get_settlement_summary(&actors.collection).sales,
        1
    );
    ledger.assert_conservation(
        &env,
        &[
            ("owner1", SUPPLY - AMOUNT),
            ("seller", seller_net),
            ("royalty_recipient", royalty_share),
        ],
    );
}

#[test]
fn vesting_boundary_auth_failures_and_subscription_funding_rollback() {
    let (env, actors) = setup_composition_env(START);
    let (token_admin_id, token_id, token_admin, _token) = deploy_sac_token(&env);

    let vesting_id = env.register(Vesting, ());
    let vesting = SorobanForgeVestingClient::new(&env, &vesting_id);
    let sub_addr = env.register(SubscriptionPayments, ());
    let subscriptions = SorobanForgeSubscriptionPaymentsClient::new(&env, &sub_addr);
    let bridge_id = env.register(AtomicVestingToSubBridge, ());
    let bridge = AtomicVestingToSubBridgeClient::new(&env, &bridge_id);

    token_admin.mint(&actors.owner2, &SUPPLY);

    let mut ledger = ConservationLedger::new(token_id.clone(), SUPPLY);
    actors.register_all(&mut ledger);
    ledger.register("token_admin", token_admin_id);
    ledger.register("token", token_id.clone());
    ledger.register("vesting", vesting_id.clone());
    ledger.register("subscription", sub_addr.clone());
    ledger.register("bridge", bridge_id.clone());

    let sub_id = subscriptions.subscribe(
        &actors.beneficiary,
        &actors.provider,
        &token_id,
        &200,
        &BILLING_PERIOD,
    );

    let batch = soroban_sdk::vec![
        &env,
        CreateScheduleArgs {
            beneficiary: actors.beneficiary.clone(),
            token: token_id.clone(),
            total_amount: AMOUNT,
            cliff: 0,
            duration: 2_000,
        }
    ];
    let initial_snap = ledger.snapshot(&env);

    // 1. Beneficiary cannot forge funder's `create_schedules_batch`.
    env.mock_auths(&[MockAuth {
        address: &actors.beneficiary,
        invoke: &MockAuthInvoke {
            contract: &vesting_id,
            fn_name: "create_schedules_batch",
            args: (&actors.owner2, &batch).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(vesting.try_create_schedules_batch(&actors.owner2, &batch));
    assert_eq!(vesting.try_get_status(&1), Err(Ok(ForgeError::NotFound)));
    ledger.assert_unchanged(&env, &initial_snap);

    // 2. Funder signing root `create_schedules_batch` without nested SAC transfer fails and rolls back.
    env.mock_auths(&[MockAuth {
        address: &actors.owner2,
        invoke: &MockAuthInvoke {
            contract: &vesting_id,
            fn_name: "create_schedules_batch",
            args: (&actors.owner2, &batch).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_eq!(
        vesting.try_create_schedules_batch(&actors.owner2, &batch),
        Err(Ok(ForgeError::TokenTransferFailed))
    );
    assert_eq!(vesting.try_get_status(&1), Err(Ok(ForgeError::NotFound)));
    ledger.assert_unchanged(&env, &initial_snap);

    // Properly fund batch with full auth tree.
    env.mock_auths(&[MockAuth {
        address: &actors.owner2,
        invoke: &MockAuthInvoke {
            contract: &vesting_id,
            fn_name: "create_schedules_batch",
            args: (&actors.owner2, &batch).into_val(&env),
            sub_invokes: &[MockAuthInvoke {
                contract: &token_id,
                fn_name: "transfer",
                args: (&actors.owner2, &vesting_id, AMOUNT).into_val(&env),
                sub_invokes: &[],
            }],
        },
    }]);
    let schedule_id = vesting
        .create_schedules_batch(&actors.owner2, &batch)
        .get(0)
        .unwrap();
    ledger.assert_conservation(&env, &[("owner2", SUPPLY - AMOUNT), ("vesting", AMOUNT)]);
    let vested_snap = ledger.snapshot(&env);

    // Advance halfway (300 vested out of 600).
    env.ledger().set_timestamp(START + 1_000);

    // 3. Funder cannot claim on behalf of beneficiary; beneficiary cannot revoke.
    env.mock_auths(&[MockAuth {
        address: &actors.owner2,
        invoke: &MockAuthInvoke {
            contract: &vesting_id,
            fn_name: "claim",
            args: (schedule_id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(vesting.try_claim(&schedule_id));

    env.mock_auths(&[MockAuth {
        address: &actors.beneficiary,
        invoke: &MockAuthInvoke {
            contract: &vesting_id,
            fn_name: "revoke",
            args: (schedule_id,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(vesting.try_revoke(&schedule_id));
    assert_eq!(vesting.get_status(&schedule_id), VestingStatus::Vesting);
    assert_eq!(vesting.get_schedule(&schedule_id).claimed, 0);
    ledger.assert_unchanged(&env, &vested_snap);

    // 4. Cross-contract auth failure rollback across Vesting -> SubscriptionPayments:
    //    Beneficiary authorizes bridge + vesting.claim ONLY (omits subscriptions.deposit!).
    //    `vesting.claim` pays 300 inside the frame, then `subscriptions.deposit` aborts ->
    //    entire claim rolls back cleanly!
    let deposit_amount = 200_i128;
    env.mock_auths(&[MockAuth {
        address: &actors.beneficiary,
        invoke: &MockAuthInvoke {
            contract: &bridge_id,
            fn_name: "claim_and_deposit",
            args: (
                &actors.beneficiary,
                &vesting_id,
                &sub_addr,
                schedule_id,
                sub_id,
                deposit_amount,
            )
                .into_val(&env),
            sub_invokes: &[MockAuthInvoke {
                contract: &vesting_id,
                fn_name: "claim",
                args: (schedule_id,).into_val(&env),
                sub_invokes: &[],
            }],
        },
    }]);
    assert_auth_abort!(bridge.try_claim_and_deposit(
        &actors.beneficiary,
        &vesting_id,
        &sub_addr,
        &schedule_id,
        &sub_id,
        &deposit_amount,
    ));
    assert_eq!(vesting.get_schedule(&schedule_id).claimed, 0);
    assert_eq!(
        subscriptions.get_subscription(&sub_id).prepaid_balance,
        None
    );
    ledger.assert_unchanged(&env, &vested_snap);

    // 5. Arming the full auth tree succeeds under strict enforced auth.
    env.mock_auths(&[MockAuth {
        address: &actors.beneficiary,
        invoke: &MockAuthInvoke {
            contract: &bridge_id,
            fn_name: "claim_and_deposit",
            args: (
                &actors.beneficiary,
                &vesting_id,
                &sub_addr,
                schedule_id,
                sub_id,
                deposit_amount,
            )
                .into_val(&env),
            sub_invokes: &[
                MockAuthInvoke {
                    contract: &vesting_id,
                    fn_name: "claim",
                    args: (schedule_id,).into_val(&env),
                    sub_invokes: &[],
                },
                MockAuthInvoke {
                    contract: &sub_addr,
                    fn_name: "deposit",
                    args: (sub_id, deposit_amount).into_val(&env),
                    sub_invokes: &[MockAuthInvoke {
                        contract: &token_id,
                        fn_name: "transfer",
                        args: (&actors.beneficiary, &sub_addr, deposit_amount).into_val(&env),
                        sub_invokes: &[],
                    }],
                },
            ],
        },
    }]);
    assert_eq!(
        bridge.claim_and_deposit(
            &actors.beneficiary,
            &vesting_id,
            &sub_addr,
            &schedule_id,
            &sub_id,
            &deposit_amount,
        ),
        300
    );
    assert_eq!(vesting.get_schedule(&schedule_id).claimed, 300);
    assert_eq!(
        subscriptions.get_subscription(&sub_id).prepaid_balance,
        Some(deposit_amount)
    );
    ledger.assert_conservation(
        &env,
        &[
            ("owner2", SUPPLY - AMOUNT),
            ("vesting", 300),
            ("subscription", 200),
            ("beneficiary", 100),
        ],
    );
}

#[test]
fn subscription_boundary_auth_failures_and_escrow_settlement_rollback() {
    let (env, actors) = setup_composition_env(START);
    let (token_admin_id, token_id, token_admin, _token) = deploy_sac_token(&env);

    let sub_addr = env.register(SubscriptionPayments, ());
    let subscriptions = SorobanForgeSubscriptionPaymentsClient::new(&env, &sub_addr);
    let escrow_id = env.register(Escrow, ());
    let escrow = SorobanForgeEscrowClient::new(&env, &escrow_id);
    let bridge_id = env.register(AtomicSubToEscrowBridge, ());
    let bridge = AtomicSubToEscrowBridgeClient::new(&env, &bridge_id);

    token_admin.mint(&actors.owner1, &SUPPLY);

    let mut ledger = ConservationLedger::new(token_id.clone(), SUPPLY);
    actors.register_all(&mut ledger);
    ledger.register("token_admin", token_admin_id);
    ledger.register("token", token_id.clone());
    ledger.register("subscription", sub_addr.clone());
    ledger.register("escrow", escrow_id.clone());
    ledger.register("bridge", bridge_id.clone());

    let initial_snap = ledger.snapshot(&env);

    // 1. Provider cannot subscribe on behalf of owner1 without prior opt-in.
    env.mock_auths(&[MockAuth {
        address: &actors.provider,
        invoke: &MockAuthInvoke {
            contract: &sub_addr,
            fn_name: "subscribe_on_behalf_of",
            args: (
                &actors.provider,
                &actors.owner1,
                &token_id,
                250_i128,
                BILLING_PERIOD,
            )
                .into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_eq!(
        subscriptions.try_subscribe_on_behalf_of(
            &actors.provider,
            &actors.owner1,
            &token_id,
            &250,
            &BILLING_PERIOD,
        ),
        Err(Ok(ForgeError::Unauthorized))
    );
    assert_eq!(subscriptions.get_subscription_count(), 0);
    ledger.assert_unchanged(&env, &initial_snap);

    // Setup a prepaid subscription (250/period, 500 prepaid) and a pull subscription (100/period).
    env.mock_all_auths();
    let prepaid_sub = subscriptions.subscribe(
        &actors.owner1,
        &actors.provider,
        &token_id,
        &250,
        &BILLING_PERIOD,
    );
    subscriptions.deposit(&prepaid_sub, &500);
    let pull_sub = subscriptions.subscribe(
        &actors.owner1,
        &actors.provider,
        &token_id,
        &100,
        &BILLING_PERIOD,
    );
    ledger.assert_conservation(&env, &[("owner1", 500), ("subscription", 500)]);
    let funded_snap = ledger.snapshot(&env);

    // 2. Provider cannot cancel subscriber's subscription; subscriber cannot charge.
    env.mock_auths(&[MockAuth {
        address: &actors.provider,
        invoke: &MockAuthInvoke {
            contract: &sub_addr,
            fn_name: "cancel",
            args: (prepaid_sub,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(subscriptions.try_cancel(&prepaid_sub));
    assert_eq!(
        subscriptions.get_subscription(&prepaid_sub).status,
        SubscriptionStatus::Active
    );

    env.ledger().set_timestamp(START + BILLING_PERIOD * 2);

    env.mock_auths(&[MockAuth {
        address: &actors.owner1,
        invoke: &MockAuthInvoke {
            contract: &sub_addr,
            fn_name: "charge",
            args: (prepaid_sub,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_abort!(subscriptions.try_charge(&prepaid_sub));
    assert_eq!(
        subscriptions.get_subscription(&prepaid_sub).prepaid_balance,
        Some(500)
    );
    ledger.assert_unchanged(&env, &funded_snap);

    // 3. Pull-mode `charge_catchup` with provider auth ONLY (missing subscriber's nested
    //    token transfer auth) returns TokenTransferFailed and rolls back all periods.
    env.mock_auths(&[MockAuth {
        address: &actors.provider,
        invoke: &MockAuthInvoke {
            contract: &sub_addr,
            fn_name: "charge_catchup",
            args: (pull_sub, 2_u32).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_eq!(
        subscriptions.try_charge_catchup(&pull_sub, &2),
        Err(Ok(ForgeError::TokenTransferFailed))
    );
    assert_eq!(
        subscriptions.get_subscription(&pull_sub).last_charged,
        START
    );
    ledger.assert_unchanged(&env, &funded_snap);

    // 4. Cross-contract auth failure rollback across SubscriptionPayments -> Escrow:
    //    Provider authorizes bridge + subscriptions.charge ONLY (omits escrow.create_escrow/deposit!).
    //    `subscriptions.charge` debits 250 inside the frame, then `escrow.create_escrow` aborts ->
    //    entire subscription charge rolls back atomically!
    env.mock_auths(&[MockAuth {
        address: &actors.provider,
        invoke: &MockAuthInvoke {
            contract: &bridge_id,
            fn_name: "charge_and_fund_escrow",
            args: (
                &actors.provider,
                &actors.seller,
                &actors.arbiter,
                &token_id,
                &sub_addr,
                &escrow_id,
                prepaid_sub,
                TIMEOUT,
            )
                .into_val(&env),
            sub_invokes: &[MockAuthInvoke {
                contract: &sub_addr,
                fn_name: "charge",
                args: (prepaid_sub,).into_val(&env),
                sub_invokes: &[],
            }],
        },
    }]);
    assert_auth_abort!(bridge.try_charge_and_fund_escrow(
        &actors.provider,
        &actors.seller,
        &actors.arbiter,
        &token_id,
        &sub_addr,
        &escrow_id,
        &prepaid_sub,
        &TIMEOUT,
    ));
    let sub_after_abort = subscriptions.get_subscription(&prepaid_sub);
    assert_eq!(sub_after_abort.last_charged, START);
    assert_eq!(sub_after_abort.prepaid_balance, Some(500));
    assert_eq!(escrow.try_get_status(&1), Err(Ok(ForgeError::NotFound)));
    ledger.assert_unchanged(&env, &funded_snap);

    // 5. Arming the full auth tree succeeds under strict enforced auth.
    env.mock_auths(&[MockAuth {
        address: &actors.provider,
        invoke: &MockAuthInvoke {
            contract: &bridge_id,
            fn_name: "charge_and_fund_escrow",
            args: (
                &actors.provider,
                &actors.seller,
                &actors.arbiter,
                &token_id,
                &sub_addr,
                &escrow_id,
                prepaid_sub,
                TIMEOUT,
            )
                .into_val(&env),
            sub_invokes: &[
                MockAuthInvoke {
                    contract: &sub_addr,
                    fn_name: "charge",
                    args: (prepaid_sub,).into_val(&env),
                    sub_invokes: &[],
                },
                MockAuthInvoke {
                    contract: &escrow_id,
                    fn_name: "create_escrow",
                    args: (
                        &actors.provider,
                        &actors.seller,
                        &actors.arbiter,
                        &token_id,
                        250_i128,
                        TIMEOUT,
                    )
                        .into_val(&env),
                    sub_invokes: &[],
                },
                MockAuthInvoke {
                    contract: &escrow_id,
                    fn_name: "deposit",
                    args: (1_u64,).into_val(&env),
                    sub_invokes: &[MockAuthInvoke {
                        contract: &token_id,
                        fn_name: "transfer",
                        args: (&actors.provider, &escrow_id, 250_i128).into_val(&env),
                        sub_invokes: &[],
                    }],
                },
            ],
        },
    }]);
    let created_esc_id = bridge.charge_and_fund_escrow(
        &actors.provider,
        &actors.seller,
        &actors.arbiter,
        &token_id,
        &sub_addr,
        &escrow_id,
        &prepaid_sub,
        &TIMEOUT,
    );
    assert_eq!(created_esc_id, 1);
    assert_eq!(escrow.get_status(&created_esc_id), EscrowStatus::Funded);
    assert_eq!(
        subscriptions.get_subscription(&prepaid_sub).prepaid_balance,
        Some(250)
    );
    ledger.assert_conservation(
        &env,
        &[("owner1", 500), ("subscription", 250), ("escrow", 250)],
    );
}
