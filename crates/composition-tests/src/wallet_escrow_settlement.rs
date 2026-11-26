//! Composition scenarios where a multi-sig wallet funds an escrow and settles
//! proceeds through downstream contracts (marketplace royalties and vesting).

use crate::harness::{deploy_sac_token, setup_composition_env, ConservationLedger};
use soroban_forge_escrow::{Escrow, EscrowStatus, SorobanForgeEscrowClient};
use soroban_forge_marketplace_royalties::{
    MarketplaceRoyalties, SorobanForgeMarketplaceRoyaltiesClient,
};
use soroban_forge_multi_sig_wallet::{MultiSigWallet, SorobanForgeMultiSigWalletClient, TxStatus};
use soroban_forge_shared_utils::ForgeError;
use soroban_forge_vesting::{
    CreateScheduleArgs, SorobanForgeVestingClient, Vesting, VestingStatus,
};
use soroban_sdk::testutils::Ledger as _;
use soroban_sdk::{contract, contractimpl, contracttype, Address, Bytes, Env, IntoVal, Symbol};

const START: u64 = 1_000_000;
const TOTAL_SUPPLY: i128 = 1_000;
const ESCROW_AMOUNT: i128 = 600;
const TIMEOUT: u64 = 86_400;
const REVIEW_PERIOD: u64 = 3_600;
const ROYALTY_BPS: u32 = 1_250; // 12.5% -> 75 on 600
const ARBITER_FEE_BPS: u32 = 500; // 5% -> 50 on 1_000

/// Test-only coordinator invoked by `MultiSigWallet::submit_call` to create
/// and fund an escrow from the wallet in a single atomic transaction.
#[contract]
struct WalletEscrowCoordinator;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
enum CoordinatorKey {
    Buyer,
    Escrow,
    Seller,
    Arbiter,
    Token,
    Amount,
    Timeout,
    ReviewPeriod,
    EscrowId,
    FailAfterCreate,
}

#[contractimpl]
impl WalletEscrowCoordinator {
    #[allow(clippy::too_many_arguments)]
    pub fn configure(
        env: Env,
        buyer: Address,
        escrow: Address,
        seller: Address,
        arbiter: Address,
        token: Address,
        amount: i128,
        timeout: u64,
        review_period: u64,
    ) {
        let storage = env.storage().instance();
        storage.set(&CoordinatorKey::Buyer, &buyer);
        storage.set(&CoordinatorKey::Escrow, &escrow);
        storage.set(&CoordinatorKey::Seller, &seller);
        storage.set(&CoordinatorKey::Arbiter, &arbiter);
        storage.set(&CoordinatorKey::Token, &token);
        storage.set(&CoordinatorKey::Amount, &amount);
        storage.set(&CoordinatorKey::Timeout, &timeout);
        storage.set(&CoordinatorKey::ReviewPeriod, &review_period);
        storage.set(&CoordinatorKey::FailAfterCreate, &false);
    }

    pub fn set_fail_after_create(env: Env, fail: bool) {
        env.storage()
            .instance()
            .set(&CoordinatorKey::FailAfterCreate, &fail);
    }

    pub fn fund_escrow(env: Env, tag: Bytes) -> Result<(), ForgeError> {
        if tag != Bytes::from_slice(&env, b"wallet-fund-escrow") {
            return Err(ForgeError::InvalidInput);
        }

        let storage = env.storage().instance();
        let buyer: Address = storage.get(&CoordinatorKey::Buyer).unwrap();
        let escrow_addr: Address = storage.get(&CoordinatorKey::Escrow).unwrap();
        let seller: Address = storage.get(&CoordinatorKey::Seller).unwrap();
        let arbiter: Address = storage.get(&CoordinatorKey::Arbiter).unwrap();
        let token: Address = storage.get(&CoordinatorKey::Token).unwrap();
        let amount: i128 = storage.get(&CoordinatorKey::Amount).unwrap();
        let timeout: u64 = storage.get(&CoordinatorKey::Timeout).unwrap();
        let review_period: u64 = storage.get(&CoordinatorKey::ReviewPeriod).unwrap();
        let fail_after_create: bool = storage
            .get(&CoordinatorKey::FailAfterCreate)
            .unwrap_or(false);

        let escrow = SorobanForgeEscrowClient::new(&env, &escrow_addr);
        let escrow_id = escrow.create_escrow_with_review(
            &buyer,
            &seller,
            &arbiter,
            &token,
            &amount,
            &timeout,
            &review_period,
        );
        storage.set(&CoordinatorKey::EscrowId, &escrow_id);

        if fail_after_create {
            return Err(ForgeError::ContractInvocationFailed);
        }

        escrow.deposit(&escrow_id);
        Ok(())
    }

    pub fn escrow_id(env: Env) -> Option<u64> {
        env.storage().instance().get(&CoordinatorKey::EscrowId)
    }
}

/// Test-only settlement bridge that atomically releases an escrow and settles
/// the sale proceeds through `MarketplaceRoyalties`.
#[contract]
struct EscrowRoyaltySettler;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
enum SettlerKey {
    FailAfterRelease,
    SettledCount,
}

#[contractimpl]
impl EscrowRoyaltySettler {
    pub fn set_fail_after_release(env: Env, fail: bool) {
        env.storage()
            .instance()
            .set(&SettlerKey::FailAfterRelease, &fail);
    }

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
        let escrow = SorobanForgeEscrowClient::new(&env, &escrow_addr);
        escrow.execute_release(&escrow_id);

        if env
            .storage()
            .instance()
            .get(&SettlerKey::FailAfterRelease)
            .unwrap_or(false)
        {
            return Err(ForgeError::ContractInvocationFailed);
        }

        let royalties = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &royalties_addr);
        royalties.settle_sale(&collection, &token, &seller, &seller, &amount);

        let count: u32 = env
            .storage()
            .instance()
            .get(&SettlerKey::SettledCount)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&SettlerKey::SettledCount, &(count + 1));
        Ok(())
    }

    pub fn settled_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&SettlerKey::SettledCount)
            .unwrap_or(0)
    }
}

#[test]
fn wallet_funds_escrow_and_settles_through_royalties_atomically() {
    let (env, actors) = setup_composition_env(START);
    let (token_admin_id, token_id, token_admin, _token) = deploy_sac_token(&env);

    let wallet_id = env.register(MultiSigWallet, ());
    let wallet = SorobanForgeMultiSigWalletClient::new(&env, &wallet_id);
    let escrow_id = env.register(Escrow, ());
    let escrow = SorobanForgeEscrowClient::new(&env, &escrow_id);
    let royalties_id = env.register(MarketplaceRoyalties, ());
    let royalties = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &royalties_id);
    let coordinator_id = env.register(WalletEscrowCoordinator, ());
    let coordinator = WalletEscrowCoordinatorClient::new(&env, &coordinator_id);
    let settler_id = env.register(EscrowRoyaltySettler, ());
    let settler = EscrowRoyaltySettlerClient::new(&env, &settler_id);

    wallet.initialize(&actors.wallet_owners(&env), &2);
    royalties.set_royalty(&actors.collection, &actors.royalty_recipient, &ROYALTY_BPS);
    coordinator.configure(
        &wallet_id,
        &escrow_id,
        &actors.seller,
        &actors.arbiter,
        &token_id,
        &ESCROW_AMOUNT,
        &TIMEOUT,
        &REVIEW_PERIOD,
    );

    token_admin.mint(&actors.deployer, &TOTAL_SUPPLY);

    let mut ledger = ConservationLedger::new(token_id.clone(), TOTAL_SUPPLY);
    actors.register_all(&mut ledger);
    ledger.register("token_admin", token_admin_id);
    ledger.register("token", token_id.clone());
    ledger.register("wallet", wallet_id.clone());
    ledger.register("escrow", escrow_id.clone());
    ledger.register("royalties", royalties_id.clone());
    ledger.register("coordinator", coordinator_id.clone());
    ledger.register("settler", settler_id.clone());

    ledger.assert_conservation(&env, &[("deployer", TOTAL_SUPPLY)]);

    wallet.deposit(&token_id, &actors.deployer, &TOTAL_SUPPLY);
    assert_eq!(wallet.balance(&token_id), TOTAL_SUPPLY);
    ledger.assert_conservation(&env, &[("wallet", TOTAL_SUPPLY)]);

    // Submit typed wallet call to create and fund escrow.
    let tag = Bytes::from_slice(&env, b"wallet-fund-escrow");
    let args = soroban_sdk::vec![&env, tag.into_val(&env)];
    let tx_id = wallet.submit_call(
        &actors.owner1,
        &coordinator_id,
        &Symbol::new(&env, "fund_escrow"),
        &args,
        &None,
    );
    wallet.confirm(&tx_id, &actors.owner1);
    assert_eq!(
        wallet.try_execute(&tx_id),
        Err(Ok(ForgeError::InvalidInput))
    );
    ledger.assert_conservation(&env, &[("wallet", TOTAL_SUPPLY)]);

    // Threshold reached, execute wallet -> coordinator -> escrow deposit.
    wallet.confirm(&tx_id, &actors.owner2);
    wallet.execute(&tx_id);
    assert_eq!(wallet.get_tx(&tx_id).status, TxStatus::Executed);

    let created_escrow_id = coordinator.escrow_id().expect("escrow id recorded");
    assert_eq!(escrow.get_status(&created_escrow_id), EscrowStatus::Funded);
    ledger.assert_conservation(
        &env,
        &[
            ("wallet", TOTAL_SUPPLY - ESCROW_AMOUNT),
            ("escrow", ESCROW_AMOUNT),
        ],
    );

    // Seller requests release; review period elapses.
    let release_at = escrow.request_release(&created_escrow_id, &ESCROW_AMOUNT);
    assert_eq!(release_at, START + REVIEW_PERIOD);
    env.ledger().set_timestamp(release_at);

    // Atomic settlement through EscrowRoyaltySettler -> Escrow + MarketplaceRoyalties.
    settler.release_and_settle(
        &escrow_id,
        &royalties_id,
        &actors.collection,
        &token_id,
        &actors.seller,
        &created_escrow_id,
        &ESCROW_AMOUNT,
    );

    assert_eq!(
        escrow.get_status(&created_escrow_id),
        EscrowStatus::Completed
    );
    assert_eq!(settler.settled_count(), 1);
    let summary = royalties.get_settlement_summary(&actors.collection);
    assert_eq!(summary.sales, 1);
    assert_eq!(summary.gross_volume, ESCROW_AMOUNT);
    assert_eq!(summary.royalties_paid, 75);

    ledger.assert_conservation(
        &env,
        &[
            ("wallet", TOTAL_SUPPLY - ESCROW_AMOUNT),
            ("seller", 525),
            ("royalty_recipient", 75),
        ],
    );
}

#[test]
fn wallet_escrow_settlement_failure_rolls_back_and_retry_succeeds() {
    let (env, actors) = setup_composition_env(START);
    let (token_admin_id, token_id, token_admin, _token) = deploy_sac_token(&env);

    let wallet_id = env.register(MultiSigWallet, ());
    let wallet = SorobanForgeMultiSigWalletClient::new(&env, &wallet_id);
    let escrow_id = env.register(Escrow, ());
    let escrow = SorobanForgeEscrowClient::new(&env, &escrow_id);
    let royalties_id = env.register(MarketplaceRoyalties, ());
    let royalties = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &royalties_id);
    let coordinator_id = env.register(WalletEscrowCoordinator, ());
    let coordinator = WalletEscrowCoordinatorClient::new(&env, &coordinator_id);
    let settler_id = env.register(EscrowRoyaltySettler, ());
    let settler = EscrowRoyaltySettlerClient::new(&env, &settler_id);

    wallet.initialize(&actors.wallet_owners(&env), &2);
    royalties.set_royalty(&actors.collection, &actors.royalty_recipient, &ROYALTY_BPS);
    coordinator.configure(
        &wallet_id,
        &escrow_id,
        &actors.seller,
        &actors.arbiter,
        &token_id,
        &ESCROW_AMOUNT,
        &TIMEOUT,
        &REVIEW_PERIOD,
    );
    token_admin.mint(&actors.deployer, &TOTAL_SUPPLY);
    wallet.deposit(&token_id, &actors.deployer, &TOTAL_SUPPLY);

    let mut ledger = ConservationLedger::new(token_id.clone(), TOTAL_SUPPLY);
    actors.register_all(&mut ledger);
    ledger.register("token_admin", token_admin_id);
    ledger.register("token", token_id.clone());
    ledger.register("wallet", wallet_id.clone());
    ledger.register("escrow", escrow_id.clone());
    ledger.register("royalties", royalties_id.clone());
    ledger.register("coordinator", coordinator_id.clone());
    ledger.register("settler", settler_id.clone());

    // 1. Downstream failure inside wallet->coordinator->escrow rolls back tx + escrow creation.
    coordinator.set_fail_after_create(&true);
    let tag = Bytes::from_slice(&env, b"wallet-fund-escrow");
    let args = soroban_sdk::vec![&env, tag.into_val(&env)];
    let tx_id = wallet.submit_call(
        &actors.owner1,
        &coordinator_id,
        &Symbol::new(&env, "fund_escrow"),
        &args,
        &None,
    );
    wallet.confirm(&tx_id, &actors.owner1);
    wallet.confirm(&tx_id, &actors.owner2);

    let before_wallet_exec = ledger.snapshot(&env);
    assert_eq!(
        wallet.try_execute(&tx_id),
        Err(Ok(ForgeError::ContractInvocationFailed))
    );
    assert_eq!(wallet.get_tx(&tx_id).status, TxStatus::Pending);
    assert_eq!(coordinator.escrow_id(), None);
    assert_eq!(escrow.try_get_status(&1), Err(Ok(ForgeError::NotFound)));
    ledger.assert_unchanged(&env, &before_wallet_exec);

    // Clear failure and retry wallet execution.
    coordinator.set_fail_after_create(&false);
    wallet.execute(&tx_id);
    let created_escrow_id = coordinator.escrow_id().expect("escrow created on retry");
    assert_eq!(created_escrow_id, 1);
    assert_eq!(escrow.get_status(&created_escrow_id), EscrowStatus::Funded);
    ledger.assert_conservation(
        &env,
        &[
            ("wallet", TOTAL_SUPPLY - ESCROW_AMOUNT),
            ("escrow", ESCROW_AMOUNT),
        ],
    );

    // 2. Downstream failure after escrow release inside settler rolls back release + transfer.
    let release_at = escrow.request_release(&created_escrow_id, &ESCROW_AMOUNT);
    env.ledger().set_timestamp(release_at);
    settler.set_fail_after_release(&true);

    let before_settle = ledger.snapshot(&env);
    assert_eq!(
        settler.try_release_and_settle(
            &escrow_id,
            &royalties_id,
            &actors.collection,
            &token_id,
            &actors.seller,
            &created_escrow_id,
            &ESCROW_AMOUNT,
        ),
        Err(Ok(ForgeError::ContractInvocationFailed))
    );
    assert_eq!(escrow.get_status(&created_escrow_id), EscrowStatus::Funded);
    assert_eq!(settler.settled_count(), 0);
    assert_eq!(
        royalties.try_get_settlement_summary(&actors.collection),
        Err(Ok(ForgeError::NotFound))
    );
    ledger.assert_unchanged(&env, &before_settle);

    // Clear failure and retry settlement.
    settler.set_fail_after_release(&false);
    settler.release_and_settle(
        &escrow_id,
        &royalties_id,
        &actors.collection,
        &token_id,
        &actors.seller,
        &created_escrow_id,
        &ESCROW_AMOUNT,
    );
    assert_eq!(
        escrow.get_status(&created_escrow_id),
        EscrowStatus::Completed
    );
    assert_eq!(settler.settled_count(), 1);
    ledger.assert_conservation(
        &env,
        &[
            ("wallet", TOTAL_SUPPLY - ESCROW_AMOUNT),
            ("seller", 525),
            ("royalty_recipient", 75),
        ],
    );
}

#[test]
fn wallet_withdrawal_funds_disputed_escrow_and_settles_into_vesting() {
    let (env, actors) = setup_composition_env(START);
    let (token_admin_id, token_id, token_admin, _token) = deploy_sac_token(&env);

    let wallet_id = env.register(MultiSigWallet, ());
    let wallet = SorobanForgeMultiSigWalletClient::new(&env, &wallet_id);
    let escrow_id = env.register(Escrow, ());
    let escrow = SorobanForgeEscrowClient::new(&env, &escrow_id);
    let vesting_id = env.register(Vesting, ());
    let vesting = SorobanForgeVestingClient::new(&env, &vesting_id);

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

    assert_eq!(wallet.balance(&token_id), TOTAL_SUPPLY);
    ledger.assert_conservation(&env, &[("wallet", TOTAL_SUPPLY)]);

    // Wallet executes a threshold-approved withdrawal to owner1, who funds the escrow.
    let w_tx = wallet.submit_withdrawal(&actors.owner1, &token_id, &actors.owner1, &TOTAL_SUPPLY);
    wallet.confirm(&w_tx, &actors.owner1);
    wallet.confirm(&w_tx, &actors.owner2);
    wallet.execute(&w_tx);
    assert_eq!(wallet.balance(&token_id), 0);
    ledger.assert_conservation(&env, &[("owner1", TOTAL_SUPPLY)]);

    let esc_num = escrow.create_escrow_with_arbiter_fee(
        &actors.owner1,
        &actors.seller,
        &actors.arbiter,
        &token_id,
        &TOTAL_SUPPLY,
        &TIMEOUT,
        &ARBITER_FEE_BPS,
    );
    escrow.deposit(&esc_num);
    assert_eq!(escrow.get_status(&esc_num), EscrowStatus::Funded);
    ledger.assert_conservation(&env, &[("escrow", TOTAL_SUPPLY)]);

    // Buyer disputes; arbiter resolves 60/40 split after 5% arbiter fee (50).
    // Distributable = 950 -> seller gets 570, buyer (owner1) gets 380.
    escrow.dispute(&esc_num, &actors.owner1);
    assert_eq!(escrow.get_status(&esc_num), EscrowStatus::Disputed);
    escrow.resolve_split(&esc_num, &6_000);
    assert_eq!(escrow.get_status(&esc_num), EscrowStatus::Completed);
    ledger.assert_conservation(&env, &[("arbiter", 50), ("seller", 570), ("owner1", 380)]);

    // Owner1 re-deposits their 380 refund back into the multi-sig wallet.
    wallet.deposit(&token_id, &actors.owner1, &380);
    assert_eq!(wallet.balance(&token_id), 380);
    ledger.assert_conservation(&env, &[("arbiter", 50), ("seller", 570), ("wallet", 380)]);

    // Seller locks their 570 settlement proceeds into a linear vesting schedule for beneficiary.
    let batch = soroban_sdk::vec![
        &env,
        CreateScheduleArgs {
            beneficiary: actors.beneficiary.clone(),
            token: token_id.clone(),
            total_amount: 570,
            cliff: 100,
            duration: 400,
        }
    ];
    let schedule_ids = vesting.create_schedules_batch(&actors.seller, &batch);
    let schedule_id = schedule_ids.get(0).unwrap();
    assert_eq!(vesting.get_status(&schedule_id), VestingStatus::Locked);
    ledger.assert_conservation(&env, &[("arbiter", 50), ("vesting", 570), ("wallet", 380)]);

    // Halfway through linear ramp (t = START + 250): 570 * (150 / 300) = 285 vested.
    env.ledger().set_timestamp(START + 250);
    assert_eq!(vesting.claim(&schedule_id), 285);
    ledger.assert_conservation(
        &env,
        &[
            ("arbiter", 50),
            ("beneficiary", 285),
            ("vesting", 285),
            ("wallet", 380),
        ],
    );

    // Seller revokes at t = START + 250 and reclaims unvested 285.
    vesting.revoke(&schedule_id);
    assert_eq!(vesting.get_status(&schedule_id), VestingStatus::Revoked);
    assert_eq!(vesting.refund_revoked(&schedule_id), 285);
    ledger.assert_conservation(
        &env,
        &[
            ("arbiter", 50),
            ("beneficiary", 285),
            ("seller", 285),
            ("wallet", 380),
        ],
    );
}
