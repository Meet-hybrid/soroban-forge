//! End-to-end four-participant composition scenario asserting token conservation
//! across MultiSigWallet, Escrow, Vesting, and SubscriptionPayments simultaneously.

use crate::harness::{deploy_sac_token, setup_composition_env, ConservationLedger};
use soroban_forge_escrow::{Escrow, EscrowStatus, SorobanForgeEscrowClient};
use soroban_forge_multi_sig_wallet::{MultiSigWallet, SorobanForgeMultiSigWalletClient};
use soroban_forge_shared_utils::ForgeError;
use soroban_forge_subscription_payments::{
    SorobanForgeSubscriptionPaymentsClient, SubscriptionPayments, SubscriptionStatus,
};
use soroban_forge_vesting::{
    CreateScheduleArgs, SorobanForgeVestingClient, Vesting, VestingStatus,
};
use soroban_sdk::testutils::Ledger as _;
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

const START: u64 = 5_000_000;
const TOTAL_SUPPLY: i128 = 2_000;
const ESCROW_AMOUNT: i128 = 800;
const VESTING_AMOUNT: i128 = 1_000;
const SUB_DEPOSIT: i128 = 300;
const SUB_CHARGE: i128 = 150;
const BILLING_PERIOD: u64 = 1_000;
const REVIEW_PERIOD: u64 = 3_600;

/// Test-only router that atomically claims vested tokens and deposits a share
/// into a prepaid subscription contract.
#[contract]
struct VestingSubscriptionRouter;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
enum RouterKey {
    FailAfterClaim,
    RoutedTotal,
}

#[contractimpl]
impl VestingSubscriptionRouter {
    pub fn set_fail_after_claim(env: Env, fail: bool) {
        env.storage()
            .instance()
            .set(&RouterKey::FailAfterClaim, &fail);
    }

    pub fn claim_and_fund_subscription(
        env: Env,
        vesting_addr: Address,
        sub_addr: Address,
        schedule_id: u64,
        sub_id: u64,
        deposit_amount: i128,
    ) -> Result<i128, ForgeError> {
        let vesting = SorobanForgeVestingClient::new(&env, &vesting_addr);
        let claimed = vesting.claim(&schedule_id);

        if env
            .storage()
            .instance()
            .get(&RouterKey::FailAfterClaim)
            .unwrap_or(false)
        {
            return Err(ForgeError::ContractInvocationFailed);
        }

        if deposit_amount > claimed {
            return Err(ForgeError::InsufficientFunds);
        }

        let subs = SorobanForgeSubscriptionPaymentsClient::new(&env, &sub_addr);
        subs.deposit(&sub_id, &deposit_amount);

        let routed: i128 = env
            .storage()
            .instance()
            .get(&RouterKey::RoutedTotal)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&RouterKey::RoutedTotal, &(routed + deposit_amount));

        Ok(claimed)
    }

    pub fn routed_total(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&RouterKey::RoutedTotal)
            .unwrap_or(0)
    }
}

#[test]
fn four_participant_conservation_across_wallet_escrow_vesting_and_subscription() {
    let (env, actors) = setup_composition_env(START);
    let (token_admin_id, token_id, token_admin, _token) = deploy_sac_token(&env);

    let wallet_id = env.register(MultiSigWallet, ());
    let wallet = SorobanForgeMultiSigWalletClient::new(&env, &wallet_id);
    let escrow_id = env.register(Escrow, ());
    let escrow = SorobanForgeEscrowClient::new(&env, &escrow_id);
    let vesting_id = env.register(Vesting, ());
    let vesting = SorobanForgeVestingClient::new(&env, &vesting_id);
    let sub_addr = env.register(SubscriptionPayments, ());
    let subscriptions = SorobanForgeSubscriptionPaymentsClient::new(&env, &sub_addr);
    let router_id = env.register(VestingSubscriptionRouter, ());
    let router = VestingSubscriptionRouterClient::new(&env, &router_id);

    wallet.initialize(&actors.wallet_owners(&env), &2);
    token_admin.mint(&actors.deployer, &TOTAL_SUPPLY);
    wallet.deposit(&token_id, &actors.deployer, &TOTAL_SUPPLY);

    let mut ledger = ConservationLedger::new(token_id.clone(), TOTAL_SUPPLY);
    actors.register_all(&mut ledger);
    ledger.register("token_admin", token_admin_id);
    ledger.register("token", token_id.clone());
    ledger.register("wallet", wallet_id.clone());
    ledger.register("escrow", escrow_id.clone());
    ledger.register("vesting", vesting_id.clone());
    ledger.register("subscription", sub_addr.clone());
    ledger.register("router", router_id.clone());

    assert_eq!(wallet.balance(&token_id), TOTAL_SUPPLY);
    ledger.assert_conservation(&env, &[("wallet", TOTAL_SUPPLY)]);

    // 1. Wallet funds owner1 (800 -> Escrow) and owner2 (1_000 -> Vesting), keeping 200 in custody.
    let tx_escrow =
        wallet.submit_withdrawal(&actors.owner1, &token_id, &actors.owner1, &ESCROW_AMOUNT);
    wallet.confirm(&tx_escrow, &actors.owner1);
    wallet.confirm(&tx_escrow, &actors.owner2);
    wallet.execute(&tx_escrow);

    let tx_vesting =
        wallet.submit_withdrawal(&actors.owner2, &token_id, &actors.owner2, &VESTING_AMOUNT);
    wallet.confirm(&tx_vesting, &actors.owner2);
    wallet.confirm(&tx_vesting, &actors.owner3);
    wallet.execute(&tx_vesting);

    assert_eq!(wallet.balance(&token_id), 200);
    ledger.assert_conservation(
        &env,
        &[
            ("wallet", 200),
            ("owner1", ESCROW_AMOUNT),
            ("owner2", VESTING_AMOUNT),
        ],
    );

    // Deploy active Escrow (800) and linear Vesting schedule (1_000, cliff=0, duration=4_000).
    let esc_num = escrow.create_escrow_with_review(
        &actors.owner1,
        &actors.seller,
        &actors.arbiter,
        &token_id,
        &ESCROW_AMOUNT,
        &86_400,
        &REVIEW_PERIOD,
    );
    escrow.deposit(&esc_num);

    let schedule_ids = vesting.create_schedules_batch(
        &actors.owner2,
        &soroban_sdk::vec![
            &env,
            CreateScheduleArgs {
                beneficiary: actors.beneficiary.clone(),
                token: token_id.clone(),
                total_amount: VESTING_AMOUNT,
                cliff: 0,
                duration: 4_000,
            }
        ],
    );
    let schedule_id = schedule_ids.get(0).unwrap();
    ledger.assert_conservation(
        &env,
        &[
            ("wallet", 200),
            ("escrow", ESCROW_AMOUNT),
            ("vesting", VESTING_AMOUNT),
        ],
    );

    // 2. Beneficiary opens a subscription and routes half of vesting (500 claimed -> 300 prepaid).
    let sub_id = subscriptions.subscribe(
        &actors.beneficiary,
        &actors.provider,
        &token_id,
        &SUB_CHARGE,
        &BILLING_PERIOD,
    );
    env.ledger().set_timestamp(START + 2_000);

    // First verify atomic rollback if router fails after Vesting::claim.
    router.set_fail_after_claim(&true);
    let before_route = ledger.snapshot(&env);
    assert_eq!(
        router.try_claim_and_fund_subscription(
            &vesting_id,
            &sub_addr,
            &schedule_id,
            &sub_id,
            &SUB_DEPOSIT,
        ),
        Err(Ok(ForgeError::ContractInvocationFailed))
    );
    assert_eq!(vesting.get_schedule(&schedule_id).claimed, 0);
    assert_eq!(
        subscriptions.get_subscription(&sub_id).prepaid_balance,
        None
    );
    assert_eq!(router.routed_total(), 0);
    ledger.assert_unchanged(&env, &before_route);

    // Clear failure and route 500 vested tokens -> 300 into prepaid subscription, 200 retained.
    router.set_fail_after_claim(&false);
    assert_eq!(
        router.claim_and_fund_subscription(
            &vesting_id,
            &sub_addr,
            &schedule_id,
            &sub_id,
            &SUB_DEPOSIT,
        ),
        500
    );
    assert_eq!(router.routed_total(), SUB_DEPOSIT);
    assert_eq!(vesting.get_schedule(&schedule_id).claimed, 500);
    assert_eq!(
        subscriptions.get_subscription(&sub_id).prepaid_balance,
        Some(SUB_DEPOSIT)
    );
    // All four contracts hold live balances simultaneously.
    ledger.assert_conservation(
        &env,
        &[
            ("wallet", 200),
            ("escrow", 800),
            ("vesting", 500),
            ("subscription", 300),
            ("beneficiary", 200),
        ],
    );

    // 3. Partial escrow release (300 to seller) + subscription billing period 1 (150 to provider).
    escrow.release_partial(&esc_num, &300);
    assert_eq!(escrow.get_status(&esc_num), EscrowStatus::Funded);
    assert_eq!(subscriptions.charge(&sub_id), SUB_CHARGE);
    assert_eq!(
        subscriptions.get_subscription(&sub_id).prepaid_balance,
        Some(150)
    );
    ledger.assert_conservation(
        &env,
        &[
            ("wallet", 200),
            ("escrow", 500),
            ("vesting", 500),
            ("subscription", 150),
            ("beneficiary", 200),
            ("seller", 300),
            ("provider", 150),
        ],
    );

    // 4. Revoke vesting at 50% (t = START + 2_000), return unvested 500 to owner2 -> wallet.
    vesting.revoke(&schedule_id);
    assert_eq!(vesting.get_status(&schedule_id), VestingStatus::Revoked);
    assert_eq!(vesting.refund_revoked(&schedule_id), 500);
    wallet.deposit(&token_id, &actors.owner2, &500);
    assert_eq!(wallet.balance(&token_id), 700);
    ledger.assert_conservation(
        &env,
        &[
            ("wallet", 700),
            ("escrow", 500),
            ("subscription", 150),
            ("beneficiary", 200),
            ("seller", 300),
            ("provider", 150),
        ],
    );

    // 5. Complete escrow remaining 500 via review window + charge subscription period 2 (150).
    let release_at = escrow.request_release(&esc_num, &500);
    env.ledger().set_timestamp(release_at);
    escrow.execute_release(&esc_num);
    assert_eq!(escrow.get_status(&esc_num), EscrowStatus::Completed);

    assert_eq!(subscriptions.charge(&sub_id), SUB_CHARGE);
    assert_eq!(
        subscriptions.get_subscription(&sub_id).status,
        SubscriptionStatus::Active
    );
    assert_eq!(
        subscriptions.get_subscription(&sub_id).prepaid_balance,
        Some(0)
    );
    ledger.assert_conservation(
        &env,
        &[
            ("wallet", 700),
            ("beneficiary", 200),
            ("seller", 800),
            ("provider", 300),
        ],
    );
}
