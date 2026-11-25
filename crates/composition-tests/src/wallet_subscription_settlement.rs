//! Composition scenarios where a multi-sig wallet funds subscriptions (prepaid
//! and auto-topup) and settles subscription revenue through downstream contracts.

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
use soroban_sdk::testutils::Ledger as _;
use soroban_sdk::{contract, contractimpl, contracttype, Address, Bytes, Env, IntoVal, Symbol};

const START: u64 = 2_000_000;
const TOTAL_SUPPLY: i128 = 1_000;
const PREPAID_DEPOSIT: i128 = 500;
const PERIOD_CHARGE: i128 = 200;
const BILLING_PERIOD: u64 = 1_000;
const ROYALTY_BPS: u32 = 1_000; // 10% -> 20 on 200

/// Test-only adapter invoked by `MultiSigWallet::submit_call` to create a
/// subscription and fund its prepaid custody balance in a single atomic call.
#[contract]
struct WalletSubscriptionAdapter;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
enum SubAdapterKey {
    Wallet,
    Subscriptions,
    Provider,
    Token,
    PeriodAmount,
    Period,
    PrepaidAmount,
    SubId,
    FailAfterSubscribe,
}

#[contractimpl]
impl WalletSubscriptionAdapter {
    #[allow(clippy::too_many_arguments)]
    pub fn configure(
        env: Env,
        wallet: Address,
        subscriptions: Address,
        provider: Address,
        token: Address,
        period_amount: i128,
        period: u64,
        prepaid_amount: i128,
    ) {
        let storage = env.storage().instance();
        storage.set(&SubAdapterKey::Wallet, &wallet);
        storage.set(&SubAdapterKey::Subscriptions, &subscriptions);
        storage.set(&SubAdapterKey::Provider, &provider);
        storage.set(&SubAdapterKey::Token, &token);
        storage.set(&SubAdapterKey::PeriodAmount, &period_amount);
        storage.set(&SubAdapterKey::Period, &period);
        storage.set(&SubAdapterKey::PrepaidAmount, &prepaid_amount);
        storage.set(&SubAdapterKey::FailAfterSubscribe, &false);
    }

    pub fn set_fail_after_subscribe(env: Env, fail: bool) {
        env.storage()
            .instance()
            .set(&SubAdapterKey::FailAfterSubscribe, &fail);
    }

    pub fn fund_subscription(env: Env, tag: Bytes) -> Result<(), ForgeError> {
        if tag != Bytes::from_slice(&env, b"wallet-fund-sub") {
            return Err(ForgeError::InvalidInput);
        }

        let storage = env.storage().instance();
        let wallet: Address = storage.get(&SubAdapterKey::Wallet).unwrap();
        let sub_addr: Address = storage.get(&SubAdapterKey::Subscriptions).unwrap();
        let provider: Address = storage.get(&SubAdapterKey::Provider).unwrap();
        let token: Address = storage.get(&SubAdapterKey::Token).unwrap();
        let period_amount: i128 = storage.get(&SubAdapterKey::PeriodAmount).unwrap();
        let period: u64 = storage.get(&SubAdapterKey::Period).unwrap();
        let prepaid_amount: i128 = storage.get(&SubAdapterKey::PrepaidAmount).unwrap();
        let fail_after_subscribe: bool = storage
            .get(&SubAdapterKey::FailAfterSubscribe)
            .unwrap_or(false);

        let subs = SorobanForgeSubscriptionPaymentsClient::new(&env, &sub_addr);
        let sub_id = subs.subscribe(&wallet, &provider, &token, &period_amount, &period);
        storage.set(&SubAdapterKey::SubId, &sub_id);

        if fail_after_subscribe {
            return Err(ForgeError::ContractInvocationFailed);
        }

        subs.deposit(&sub_id, &prepaid_amount);
        Ok(())
    }

    pub fn cancel_subscription(env: Env, sub_id: u64) -> Result<(), ForgeError> {
        let sub_addr: Address = env
            .storage()
            .instance()
            .get(&SubAdapterKey::Subscriptions)
            .unwrap();
        let subs = SorobanForgeSubscriptionPaymentsClient::new(&env, &sub_addr);
        subs.cancel(&sub_id);
        Ok(())
    }

    pub fn subscription_id(env: Env) -> Option<u64> {
        env.storage().instance().get(&SubAdapterKey::SubId)
    }
}

/// Test-only settler that atomically charges a subscription and splits the
/// provider's revenue through `MarketplaceRoyalties`.
#[contract]
struct SubscriptionRoyaltySettler;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
enum SubSettlerKey {
    FailAfterCharge,
    SettledCount,
}

#[contractimpl]
impl SubscriptionRoyaltySettler {
    pub fn set_fail_after_charge(env: Env, fail: bool) {
        env.storage()
            .instance()
            .set(&SubSettlerKey::FailAfterCharge, &fail);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn charge_and_settle(
        env: Env,
        sub_addr: Address,
        royalties_addr: Address,
        collection: Address,
        token: Address,
        provider: Address,
        sub_id: u64,
    ) -> Result<i128, ForgeError> {
        let subs = SorobanForgeSubscriptionPaymentsClient::new(&env, &sub_addr);
        let billed = subs.charge(&sub_id);

        if env
            .storage()
            .instance()
            .get(&SubSettlerKey::FailAfterCharge)
            .unwrap_or(false)
        {
            return Err(ForgeError::ContractInvocationFailed);
        }

        if billed > 0 {
            let royalties = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &royalties_addr);
            royalties.settle_sale(&collection, &token, &provider, &provider, &billed);
            let count: u32 = env
                .storage()
                .instance()
                .get(&SubSettlerKey::SettledCount)
                .unwrap_or(0);
            env.storage()
                .instance()
                .set(&SubSettlerKey::SettledCount, &(count + 1));
        }

        Ok(billed)
    }

    pub fn settled_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&SubSettlerKey::SettledCount)
            .unwrap_or(0)
    }
}

#[test]
fn wallet_funds_prepaid_subscription_and_settles_charges_through_royalties() {
    let (env, actors) = setup_composition_env(START);
    let (token_admin_id, token_id, token_admin, _token) = deploy_sac_token(&env);

    let wallet_id = env.register(MultiSigWallet, ());
    let wallet = SorobanForgeMultiSigWalletClient::new(&env, &wallet_id);
    let sub_id_addr = env.register(SubscriptionPayments, ());
    let subscriptions = SorobanForgeSubscriptionPaymentsClient::new(&env, &sub_id_addr);
    let royalties_id = env.register(MarketplaceRoyalties, ());
    let royalties = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &royalties_id);
    let adapter_id = env.register(WalletSubscriptionAdapter, ());
    let adapter = WalletSubscriptionAdapterClient::new(&env, &adapter_id);
    let settler_id = env.register(SubscriptionRoyaltySettler, ());
    let settler = SubscriptionRoyaltySettlerClient::new(&env, &settler_id);

    wallet.initialize(&actors.wallet_owners(&env), &2);
    royalties.set_royalty(&actors.collection, &actors.royalty_recipient, &ROYALTY_BPS);
    adapter.configure(
        &wallet_id,
        &sub_id_addr,
        &actors.provider,
        &token_id,
        &PERIOD_CHARGE,
        &BILLING_PERIOD,
        &PREPAID_DEPOSIT,
    );

    token_admin.mint(&actors.deployer, &TOTAL_SUPPLY);
    wallet.deposit(&token_id, &actors.deployer, &TOTAL_SUPPLY);

    let mut ledger = ConservationLedger::new(token_id.clone(), TOTAL_SUPPLY);
    actors.register_all(&mut ledger);
    ledger.register("token_admin", token_admin_id);
    ledger.register("token", token_id.clone());
    ledger.register("wallet", wallet_id.clone());
    ledger.register("subscription", sub_id_addr.clone());
    ledger.register("royalties", royalties_id.clone());
    ledger.register("adapter", adapter_id.clone());
    ledger.register("settler", settler_id.clone());

    ledger.assert_conservation(&env, &[("wallet", TOTAL_SUPPLY)]);

    // Wallet funds prepaid subscription via threshold-approved typed call.
    let tag = Bytes::from_slice(&env, b"wallet-fund-sub");
    let args = soroban_sdk::vec![&env, tag.into_val(&env)];
    let tx_id = wallet.submit_call(
        &actors.owner1,
        &adapter_id,
        &Symbol::new(&env, "fund_subscription"),
        &args,
        &None,
    );
    wallet.confirm(&tx_id, &actors.owner1);
    wallet.confirm(&tx_id, &actors.owner2);
    wallet.execute(&tx_id);

    let sub_id = adapter.subscription_id().expect("subscription created");
    let sub_record = subscriptions.get_subscription(&sub_id);
    assert_eq!(sub_record.status, SubscriptionStatus::Active);
    assert_eq!(sub_record.prepaid_balance, Some(PREPAID_DEPOSIT));
    ledger.assert_conservation(
        &env,
        &[
            ("wallet", TOTAL_SUPPLY - PREPAID_DEPOSIT),
            ("subscription", PREPAID_DEPOSIT),
        ],
    );

    // Billing period 1: charge 200 -> settle 180 to provider, 20 to royalty_recipient.
    env.ledger().set_timestamp(START + BILLING_PERIOD);
    assert_eq!(
        settler.charge_and_settle(
            &sub_id_addr,
            &royalties_id,
            &actors.collection,
            &token_id,
            &actors.provider,
            &sub_id,
        ),
        PERIOD_CHARGE
    );
    assert_eq!(
        subscriptions.get_subscription(&sub_id).prepaid_balance,
        Some(300)
    );
    ledger.assert_conservation(
        &env,
        &[
            ("wallet", 500),
            ("subscription", 300),
            ("provider", 180),
            ("royalty_recipient", 20),
        ],
    );

    // Billing period 2: charge 200 -> settle 180 to provider, 20 to royalty_recipient.
    env.ledger().set_timestamp(START + BILLING_PERIOD * 2);
    assert_eq!(
        settler.charge_and_settle(
            &sub_id_addr,
            &royalties_id,
            &actors.collection,
            &token_id,
            &actors.provider,
            &sub_id,
        ),
        PERIOD_CHARGE
    );
    assert_eq!(
        subscriptions.get_subscription(&sub_id).prepaid_balance,
        Some(100)
    );
    assert_eq!(settler.settled_count(), 2);
    ledger.assert_conservation(
        &env,
        &[
            ("wallet", 500),
            ("subscription", 100),
            ("provider", 360),
            ("royalty_recipient", 40),
        ],
    );

    // Billing period 3: remaining prepaid (100 < 200) moves subscription to PastDue.
    env.ledger().set_timestamp(START + BILLING_PERIOD * 3);
    assert_eq!(
        settler.charge_and_settle(
            &sub_id_addr,
            &royalties_id,
            &actors.collection,
            &token_id,
            &actors.provider,
            &sub_id,
        ),
        0
    );
    let past_due = subscriptions.get_subscription(&sub_id);
    assert_eq!(past_due.status, SubscriptionStatus::PastDue);
    assert_eq!(past_due.failed_attempts, 1);
    ledger.assert_conservation(
        &env,
        &[
            ("wallet", 500),
            ("subscription", 100),
            ("provider", 360),
            ("royalty_recipient", 40),
        ],
    );

    // Wallet cancels subscription via typed call; unused prepaid 100 returns to wallet.
    let cancel_args = soroban_sdk::vec![&env, sub_id.into_val(&env)];
    let cancel_tx = wallet.submit_call(
        &actors.owner2,
        &adapter_id,
        &Symbol::new(&env, "cancel_subscription"),
        &cancel_args,
        &None,
    );
    wallet.confirm(&cancel_tx, &actors.owner2);
    wallet.confirm(&cancel_tx, &actors.owner3);
    wallet.execute(&cancel_tx);

    let cancelled = subscriptions.get_subscription(&sub_id);
    assert_eq!(cancelled.status, SubscriptionStatus::Cancelled);
    assert_eq!(cancelled.prepaid_balance, Some(0));
    let summary = royalties.get_settlement_summary(&actors.collection);
    assert_eq!(summary.sales, 2);
    assert_eq!(summary.gross_volume, 400);
    assert_eq!(summary.royalties_paid, 40);
    ledger.assert_conservation(
        &env,
        &[
            ("wallet", 600),
            ("provider", 360),
            ("royalty_recipient", 40),
        ],
    );
}

#[test]
fn wallet_subscription_and_settlement_failures_roll_back_atomically() {
    let (env, actors) = setup_composition_env(START);
    let (token_admin_id, token_id, token_admin, _token) = deploy_sac_token(&env);

    let wallet_id = env.register(MultiSigWallet, ());
    let wallet = SorobanForgeMultiSigWalletClient::new(&env, &wallet_id);
    let sub_addr = env.register(SubscriptionPayments, ());
    let subscriptions = SorobanForgeSubscriptionPaymentsClient::new(&env, &sub_addr);
    let royalties_id = env.register(MarketplaceRoyalties, ());
    let royalties = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &royalties_id);
    let adapter_id = env.register(WalletSubscriptionAdapter, ());
    let adapter = WalletSubscriptionAdapterClient::new(&env, &adapter_id);
    let settler_id = env.register(SubscriptionRoyaltySettler, ());
    let settler = SubscriptionRoyaltySettlerClient::new(&env, &settler_id);

    wallet.initialize(&actors.wallet_owners(&env), &2);
    royalties.set_royalty(&actors.collection, &actors.royalty_recipient, &ROYALTY_BPS);
    adapter.configure(
        &wallet_id,
        &sub_addr,
        &actors.provider,
        &token_id,
        &PERIOD_CHARGE,
        &BILLING_PERIOD,
        &PREPAID_DEPOSIT,
    );
    token_admin.mint(&actors.deployer, &TOTAL_SUPPLY);
    wallet.deposit(&token_id, &actors.deployer, &TOTAL_SUPPLY);

    let mut ledger = ConservationLedger::new(token_id.clone(), TOTAL_SUPPLY);
    actors.register_all(&mut ledger);
    ledger.register("token_admin", token_admin_id);
    ledger.register("token", token_id.clone());
    ledger.register("wallet", wallet_id.clone());
    ledger.register("subscription", sub_addr.clone());
    ledger.register("royalties", royalties_id.clone());
    ledger.register("adapter", adapter_id.clone());
    ledger.register("settler", settler_id.clone());

    // 1. Failure after subscribe inside adapter rolls back wallet tx and subscription creation.
    adapter.set_fail_after_subscribe(&true);
    let tag = Bytes::from_slice(&env, b"wallet-fund-sub");
    let args = soroban_sdk::vec![&env, tag.into_val(&env)];
    let tx_id = wallet.submit_call(
        &actors.owner1,
        &adapter_id,
        &Symbol::new(&env, "fund_subscription"),
        &args,
        &None,
    );
    wallet.confirm(&tx_id, &actors.owner1);
    wallet.confirm(&tx_id, &actors.owner2);

    let before_exec = ledger.snapshot(&env);
    assert_eq!(
        wallet.try_execute(&tx_id),
        Err(Ok(ForgeError::ContractInvocationFailed))
    );
    assert_eq!(wallet.get_tx(&tx_id).status, TxStatus::Pending);
    assert_eq!(adapter.subscription_id(), None);
    assert_eq!(subscriptions.get_subscription_count(), 0);
    ledger.assert_unchanged(&env, &before_exec);

    // Clear failure and retry wallet execution.
    adapter.set_fail_after_subscribe(&false);
    wallet.execute(&tx_id);
    let sub_id = adapter.subscription_id().expect("subscription created");
    assert_eq!(subscriptions.get_subscription_count(), 1);
    ledger.assert_conservation(
        &env,
        &[
            ("wallet", TOTAL_SUPPLY - PREPAID_DEPOSIT),
            ("subscription", PREPAID_DEPOSIT),
        ],
    );

    // 2. Failure after charge inside settler rolls back charge, last_charged, and prepaid debit.
    env.ledger().set_timestamp(START + BILLING_PERIOD);
    settler.set_fail_after_charge(&true);
    let before_charge = ledger.snapshot(&env);
    let sub_before = subscriptions.get_subscription(&sub_id);

    assert_eq!(
        settler.try_charge_and_settle(
            &sub_addr,
            &royalties_id,
            &actors.collection,
            &token_id,
            &actors.provider,
            &sub_id,
        ),
        Err(Ok(ForgeError::ContractInvocationFailed))
    );
    let sub_after = subscriptions.get_subscription(&sub_id);
    assert_eq!(sub_after.last_charged, sub_before.last_charged);
    assert_eq!(sub_after.prepaid_balance, sub_before.prepaid_balance);
    assert_eq!(sub_after.status, SubscriptionStatus::Active);
    assert_eq!(settler.settled_count(), 0);
    assert_eq!(
        royalties.try_get_settlement_summary(&actors.collection),
        Err(Ok(ForgeError::NotFound))
    );
    ledger.assert_unchanged(&env, &before_charge);

    // Clear failure and retry charge + settlement.
    settler.set_fail_after_charge(&false);
    assert_eq!(
        settler.charge_and_settle(
            &sub_addr,
            &royalties_id,
            &actors.collection,
            &token_id,
            &actors.provider,
            &sub_id,
        ),
        PERIOD_CHARGE
    );
    assert_eq!(settler.settled_count(), 1);
    ledger.assert_conservation(
        &env,
        &[
            ("wallet", 500),
            ("subscription", 300),
            ("provider", 180),
            ("royalty_recipient", 20),
        ],
    );
}

#[test]
fn wallet_funds_subscription_with_auto_topup_and_provider_settles_into_escrow() {
    let (env, actors) = setup_composition_env(START);
    let (token_admin_id, token_id, token_admin, _token) = deploy_sac_token(&env);

    let wallet_id = env.register(MultiSigWallet, ());
    let wallet = SorobanForgeMultiSigWalletClient::new(&env, &wallet_id);
    let sub_addr = env.register(SubscriptionPayments, ());
    let subscriptions = SorobanForgeSubscriptionPaymentsClient::new(&env, &sub_addr);
    let escrow_id = env.register(Escrow, ());
    let escrow = SorobanForgeEscrowClient::new(&env, &escrow_id);

    wallet.initialize(&actors.wallet_owners(&env), &2);
    token_admin.mint(&actors.deployer, &TOTAL_SUPPLY);
    wallet.deposit(&token_id, &actors.deployer, &TOTAL_SUPPLY);

    let mut ledger = ConservationLedger::new(token_id.clone(), TOTAL_SUPPLY);
    actors.register_all(&mut ledger);
    ledger.register("token_admin", token_admin_id);
    ledger.register("token", token_id.clone());
    ledger.register("wallet", wallet_id.clone());
    ledger.register("subscription", sub_addr.clone());
    ledger.register("escrow", escrow_id.clone());

    // Wallet withdraws 700 to owner1 to fund subscription + auto-topup reserve.
    let w_tx = wallet.submit_withdrawal(&actors.owner1, &token_id, &actors.owner1, &700);
    wallet.confirm(&w_tx, &actors.owner1);
    wallet.confirm(&w_tx, &actors.owner2);
    wallet.execute(&w_tx);
    assert_eq!(wallet.balance(&token_id), 300);
    ledger.assert_conservation(&env, &[("wallet", 300), ("owner1", 700)]);

    // Owner1 subscribes (200/period), deposits 250 prepaid, configures auto-topup (+300 when < 200).
    let sub_id = subscriptions.subscribe(
        &actors.owner1,
        &actors.provider,
        &token_id,
        &PERIOD_CHARGE,
        &BILLING_PERIOD,
    );
    subscriptions.deposit(&sub_id, &250);
    subscriptions.configure_auto_topup(&sub_id, &200, &300);
    ledger.assert_conservation(
        &env,
        &[("wallet", 300), ("owner1", 450), ("subscription", 250)],
    );

    // Period 1: prepaid balance 250 >= 200 and >= trigger 200 -> no topup; balance becomes 50.
    env.ledger().set_timestamp(START + BILLING_PERIOD);
    assert_eq!(subscriptions.charge(&sub_id), PERIOD_CHARGE);
    assert_eq!(
        subscriptions.get_subscription(&sub_id).prepaid_balance,
        Some(50)
    );
    ledger.assert_conservation(
        &env,
        &[
            ("wallet", 300),
            ("owner1", 450),
            ("subscription", 50),
            ("provider", 200),
        ],
    );

    // Period 2: prepaid balance 50 < 200 -> auto-topup pulls 300 from owner1 (balance 350),
    // then debits 200 to provider (remaining prepaid 150).
    env.ledger().set_timestamp(START + BILLING_PERIOD * 2);
    assert_eq!(subscriptions.charge(&sub_id), PERIOD_CHARGE);
    assert_eq!(
        subscriptions.get_subscription(&sub_id).prepaid_balance,
        Some(150)
    );
    ledger.assert_conservation(
        &env,
        &[
            ("wallet", 300),
            ("owner1", 150),
            ("subscription", 150),
            ("provider", 400),
        ],
    );

    // Provider settles accumulated 400 revenue into an escrow paying seller.
    let esc_num = escrow.create_escrow(
        &actors.provider,
        &actors.seller,
        &actors.arbiter,
        &token_id,
        &400,
        &86_400,
    );
    escrow.deposit(&esc_num);
    assert_eq!(escrow.get_status(&esc_num), EscrowStatus::Funded);
    ledger.assert_conservation(
        &env,
        &[
            ("wallet", 300),
            ("owner1", 150),
            ("subscription", 150),
            ("escrow", 400),
        ],
    );

    escrow.request_release(&esc_num, &400);
    escrow.approve_release(&esc_num);
    assert_eq!(escrow.get_status(&esc_num), EscrowStatus::Completed);
    ledger.assert_conservation(
        &env,
        &[
            ("wallet", 300),
            ("owner1", 150),
            ("subscription", 150),
            ("seller", 400),
        ],
    );
}
