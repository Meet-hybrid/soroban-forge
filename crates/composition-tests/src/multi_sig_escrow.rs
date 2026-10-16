//! Multi-sig wallet + Escrow composition settlement scenarios.
//!
//! Exercises cross-contract flows between [`MultiSigWallet`] and [`Escrow`]
//! backed by a real SEP-41 [`TokenFixture`] and verified against an
//! independent [`BalanceMirror`]:
//! - Threshold-approved wallet custody withdrawals funding escrow deposits and
//!   full seller releases.
//! - Direct multi-sig cross-contract dispatch (`submit_call`) driving escrow
//!   partial releases and arbiter dispute resolution (`resolve_split`).
//! - Cross-contract authorization enforcement (call-graph verification via
//!   `env.auths()` and negative host rejections via `set_auths` / `mock_auths`).
//! - Downstream escrow revert and token failure isolation leaving upstream
//!   multi-sig transaction state, wallet custody, and token balances unchanged.
//! - Deterministic multi-seed conservation runs across [`DEFAULT_COMPOSITION_SEEDS`].

use super::*;
use soroban_forge_escrow::{Escrow, EscrowStatus, SorobanForgeEscrowClient};
use soroban_forge_multi_sig_wallet::{
    MultiSigWallet, SorobanForgeMultiSigWalletClient, TxStatus, WithdrawalLimit,
};
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::testutils::{Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal, InvokeError, Symbol, Vec};

/// Reusable setup fixture for Multi-Sig Wallet + Escrow composition tests.
///
/// Deploys a SEP-41 [`TokenFixture`], a 2-of-3 [`MultiSigWallet`] owned by
/// `(user1, user2, user3)`, and an [`Escrow`] contract, and initializes a
/// [`BalanceMirror`] covering all accounts and both contracts.
pub struct MultiSigEscrowFixture {
    pub env: Env,
    pub accounts: TestAccounts,
    pub token: TokenFixture,
    pub wallet_id: Address,
    pub escrow_id: Address,
    pub mirror: BalanceMirror,
}

impl MultiSigEscrowFixture {
    /// Deploy and initialize the multi-sig + escrow composition environment.
    pub fn new() -> Self {
        let (env, accounts, token) = setup_env();
        let wallet_id = env.register(MultiSigWallet, ());
        let escrow_id = env.register(Escrow, ());

        let wallet = SorobanForgeMultiSigWalletClient::new(&env, &wallet_id);
        let mut owners = Vec::new(&env);
        owners.push_back(accounts.user1.clone());
        owners.push_back(accounts.user2.clone());
        owners.push_back(accounts.user3.clone());
        wallet.initialize(&owners, &2);

        let participants = [
            accounts.deployer.clone(),
            accounts.user1.clone(),
            accounts.user2.clone(),
            accounts.user3.clone(),
            accounts.validator.clone(),
            accounts.arbiter.clone(),
            wallet_id.clone(),
            escrow_id.clone(),
        ];
        let mirror = BalanceMirror::new(&env, &token.address, &participants);

        Self {
            env,
            accounts,
            token,
            wallet_id,
            escrow_id,
            mirror,
        }
    }

    pub fn wallet(&self) -> SorobanForgeMultiSigWalletClient<'_> {
        SorobanForgeMultiSigWalletClient::new(&self.env, &self.wallet_id)
    }

    pub fn escrow(&self) -> SorobanForgeEscrowClient<'_> {
        SorobanForgeEscrowClient::new(&self.env, &self.escrow_id)
    }
}

#[test]
fn multi_sig_withdrawal_funds_escrow_and_settles_to_seller() {
    let mut fx = MultiSigEscrowFixture::new();
    let wallet = fx.wallet();
    let escrow = fx.escrow();

    let buyer = fx.accounts.validator.clone();
    let seller = fx.accounts.user3.clone();
    let arbiter = fx.accounts.arbiter.clone();

    // 1. Mint tokens to owner1 and deposit into multi-sig custody.
    fx.token.mint(&fx.env, &fx.accounts.user1, 10_000);
    fx.mirror.record_mint(&fx.accounts.user1, 10_000);
    fx.mirror.assert_conserved(&fx.env);

    wallet.deposit(&fx.token.address, &fx.accounts.user1, &7_500);
    assert_contract_event(&fx.env, &fx.wallet_id, "deposited");
    assert_sac_transfer_event(
        &fx.env,
        &fx.token.address,
        &fx.accounts.user1,
        &fx.wallet_id,
        7_500,
    );
    assert_auth_invocation(&fx.env, &fx.accounts.user1, &fx.wallet_id, "deposit");
    assert_auth_invocation(&fx.env, &fx.accounts.user1, &fx.token.address, "transfer");
    assert_eq!(wallet.balance(&fx.token.address), 7_500);
    fx.mirror
        .record_transfer(&fx.accounts.user1, &fx.wallet_id, 7_500);
    fx.mirror.assert_conserved(&fx.env);

    // 2. Buyer creates an escrow deal for 4,000 tokens.
    let deal_id =
        escrow.create_escrow(&buyer, &seller, &arbiter, &fx.token.address, &4_000, &3_600);
    assert_contract_event(&fx.env, &fx.escrow_id, "escrow_created");
    assert_auth_invocation(&fx.env, &buyer, &fx.escrow_id, "create_escrow");
    assert_eq!(escrow.get_status(&deal_id), EscrowStatus::Pending);

    // 3. Multi-sig owners submit, confirm (2-of-3), and execute a withdrawal
    //    to fund the buyer for the escrow deposit.
    let tx_id = wallet.submit_withdrawal(&fx.accounts.user1, &fx.token.address, &buyer, &4_000);
    assert_contract_event(&fx.env, &fx.wallet_id, "tx_submitted");

    wallet.confirm(&tx_id, &fx.accounts.user1);
    assert_contract_event(&fx.env, &fx.wallet_id, "tx_confirmed");
    wallet.confirm(&tx_id, &fx.accounts.user2);
    assert_contract_event(&fx.env, &fx.wallet_id, "tx_confirmed");

    wallet.execute(&tx_id);
    assert_contract_event(&fx.env, &fx.wallet_id, "tx_executed");
    assert_sac_transfer_event(&fx.env, &fx.token.address, &fx.wallet_id, &buyer, 4_000);
    assert_eq!(wallet.get_tx(&tx_id).status, TxStatus::Executed);
    assert_eq!(wallet.balance(&fx.token.address), 3_500);
    fx.mirror.record_transfer(&fx.wallet_id, &buyer, 4_000);
    fx.mirror.assert_conserved(&fx.env);

    // 4. Buyer deposits the withdrawn funds into the escrow contract.
    escrow.deposit(&deal_id);
    assert_contract_event(&fx.env, &fx.escrow_id, "deposited");
    assert_sac_transfer_event(&fx.env, &fx.token.address, &buyer, &fx.escrow_id, 4_000);
    assert_auth_invocation(&fx.env, &buyer, &fx.escrow_id, "deposit");
    assert_auth_invocation(&fx.env, &buyer, &fx.token.address, "transfer");
    assert_eq!(escrow.get_status(&deal_id), EscrowStatus::Funded);
    fx.mirror.record_transfer(&buyer, &fx.escrow_id, 4_000);
    fx.mirror.assert_conserved(&fx.env);

    // 5. Seller releases the escrow and receives the 4,000 tokens.
    escrow.release(&deal_id);
    assert_contract_event(&fx.env, &fx.escrow_id, "released");
    assert_sac_transfer_event(&fx.env, &fx.token.address, &fx.escrow_id, &seller, 4_000);
    assert_auth_invocation(&fx.env, &seller, &fx.escrow_id, "release");
    assert_eq!(escrow.get_status(&deal_id), EscrowStatus::Completed);
    fx.mirror.record_transfer(&fx.escrow_id, &seller, 4_000);
    fx.mirror.assert_conserved(&fx.env);
}

#[test]
fn multi_sig_direct_call_drives_escrow_partial_release_and_arbitration() {
    let mut fx = MultiSigEscrowFixture::new();
    let wallet = fx.wallet();
    let escrow = fx.escrow();

    let buyer = fx.accounts.validator.clone();
    let seller = fx.accounts.user3.clone();

    // Fund multi-sig wallet and withdraw 6,000 to buyer.
    fx.token.mint(&fx.env, &fx.accounts.user1, 6_000);
    fx.mirror.record_mint(&fx.accounts.user1, 6_000);
    wallet.deposit(&fx.token.address, &fx.accounts.user1, &6_000);
    fx.mirror
        .record_transfer(&fx.accounts.user1, &fx.wallet_id, 6_000);

    let withdraw_tx =
        wallet.submit_withdrawal(&fx.accounts.user1, &fx.token.address, &buyer, &6_000);
    wallet.confirm(&withdraw_tx, &fx.accounts.user1);
    wallet.confirm(&withdraw_tx, &fx.accounts.user2);
    wallet.execute(&withdraw_tx);
    fx.mirror.record_transfer(&fx.wallet_id, &buyer, 6_000);
    fx.mirror.assert_conserved(&fx.env);

    // Deal 1: Multi-sig wallet is the SELLER; buyer funds 3,000 tokens.
    let deal_1 = escrow.create_escrow(
        &buyer,
        &fx.wallet_id,
        &fx.accounts.arbiter,
        &fx.token.address,
        &3_000,
        &3_600,
    );
    escrow.deposit(&deal_1);
    fx.mirror.record_transfer(&buyer, &fx.escrow_id, 3_000);
    fx.mirror.assert_conserved(&fx.env);

    // Multi-sig wallet dispatches `release_partial(deal_1, 1_200)` via `submit_call`.
    let partial_args = soroban_sdk::vec![
        &fx.env,
        deal_1.into_val(&fx.env),
        1_200_i128.into_val(&fx.env)
    ];
    let call_tx_1 = wallet.submit_call(
        &fx.accounts.user1,
        &fx.escrow_id,
        &Symbol::new(&fx.env, "release_partial"),
        &partial_args,
        &None,
    );
    wallet.confirm(&call_tx_1, &fx.accounts.user1);
    wallet.confirm(&call_tx_1, &fx.accounts.user2);
    wallet.execute(&call_tx_1);

    assert_contract_event(&fx.env, &fx.wallet_id, "tx_executed");
    assert_contract_event(&fx.env, &fx.escrow_id, "partially_released");
    assert_sac_transfer_event(
        &fx.env,
        &fx.token.address,
        &fx.escrow_id,
        &fx.wallet_id,
        1_200,
    );
    let record_1 = escrow.get_escrow(&deal_1);
    assert_eq!(record_1.status, EscrowStatus::Funded);
    assert_eq!(record_1.released, 1_200);
    assert_eq!(record_1.remaining(), 1_800);
    fx.mirror
        .record_transfer(&fx.escrow_id, &fx.wallet_id, 1_200);
    fx.mirror.assert_conserved(&fx.env);

    // Deal 2: Multi-sig wallet is the ARBITER; buyer funds 3,000 tokens, then
    // disputes; multi-sig resolves with a 60/40 split via `submit_call`.
    let deal_2 = escrow.create_escrow(
        &buyer,
        &seller,
        &fx.wallet_id,
        &fx.token.address,
        &3_000,
        &3_600,
    );
    escrow.deposit(&deal_2);
    fx.mirror.record_transfer(&buyer, &fx.escrow_id, 3_000);
    escrow.dispute(&deal_2, &buyer);
    assert_contract_event(&fx.env, &fx.escrow_id, "disputed");
    assert_eq!(escrow.get_status(&deal_2), EscrowStatus::Disputed);

    let split_args = soroban_sdk::vec![
        &fx.env,
        deal_2.into_val(&fx.env),
        6_000_u32.into_val(&fx.env)
    ];
    let call_tx_2 = wallet.submit_call(
        &fx.accounts.user2,
        &fx.escrow_id,
        &Symbol::new(&fx.env, "resolve_split"),
        &split_args,
        &None,
    );
    wallet.confirm(&call_tx_2, &fx.accounts.user2);
    wallet.confirm(&call_tx_2, &fx.accounts.user3);
    wallet.execute(&call_tx_2);

    assert_contract_event(&fx.env, &fx.wallet_id, "tx_executed");
    assert_contract_event(&fx.env, &fx.escrow_id, "resolved_split");
    assert_eq!(escrow.get_status(&deal_2), EscrowStatus::Completed);
    // 60% of 3,000 = 1,800 to seller, 40% = 1,200 to buyer.
    fx.mirror.record_transfer(&fx.escrow_id, &seller, 1_800);
    fx.mirror.record_transfer(&fx.escrow_id, &buyer, 1_200);
    fx.mirror.assert_conserved(&fx.env);
}

#[test]
fn cross_contract_authorization_enforced_across_multi_sig_and_escrow() {
    let mut fx = MultiSigEscrowFixture::new();
    let wallet = fx.wallet();
    let escrow = fx.escrow();

    fx.token.mint(&fx.env, &fx.accounts.user1, 5_000);
    fx.mirror.record_mint(&fx.accounts.user1, 5_000);
    wallet.deposit(&fx.token.address, &fx.accounts.user1, &5_000);
    fx.mirror
        .record_transfer(&fx.accounts.user1, &fx.wallet_id, 5_000);
    fx.mirror.assert_conserved(&fx.env);

    // 1. Outsider (validator) cannot submit a withdrawal or call on the wallet.
    let snap = fx.mirror.snapshot();
    let err = wallet
        .try_submit_withdrawal(
            &fx.accounts.validator,
            &fx.token.address,
            &fx.accounts.validator,
            &1_000,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ForgeError::Unauthorized);
    assert_eq!(wallet.get_tx_count(), 0);
    fx.mirror.assert_unchanged(&fx.env, &snap);

    // 2. Sub-threshold execution is rejected without moving custody tokens.
    let tx_id = wallet.submit_withdrawal(
        &fx.accounts.user1,
        &fx.token.address,
        &fx.accounts.validator,
        &2_000,
    );
    wallet.confirm(&tx_id, &fx.accounts.user1);
    let snap = fx.mirror.snapshot();
    let err = wallet.try_execute(&tx_id).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::InvalidInput);
    assert_eq!(wallet.get_tx(&tx_id).status, TxStatus::Pending);
    fx.mirror.assert_unchanged(&fx.env, &snap);

    // Complete 2-of-3 approval and fund buyer (`validator`).
    wallet.confirm(&tx_id, &fx.accounts.user2);
    wallet.execute(&tx_id);
    fx.mirror
        .record_transfer(&fx.wallet_id, &fx.accounts.validator, 2_000);
    fx.mirror.assert_conserved(&fx.env);

    // Create and fund an escrow deal where `validator` is buyer and `user3` is seller.
    let deal_id = escrow.create_escrow(
        &fx.accounts.validator,
        &fx.accounts.user3,
        &fx.accounts.arbiter,
        &fx.token.address,
        &2_000,
        &3_600,
    );
    escrow.deposit(&deal_id);
    fx.mirror
        .record_transfer(&fx.accounts.validator, &fx.escrow_id, 2_000);
    fx.mirror.assert_conserved(&fx.env);

    // 3. Enforce-mode auth check: buyer (`validator`) cannot authorize seller-only `release`.
    let snap = fx.mirror.snapshot();
    fx.env.mock_auths(&[MockAuth {
        address: &fx.accounts.validator,
        invoke: &MockAuthInvoke {
            contract: &fx.escrow_id,
            fn_name: "release",
            args: (deal_id,).into_val(&fx.env),
            sub_invokes: &[],
        },
    }]);
    let res = escrow.try_release(&deal_id);
    assert!(matches!(res, Err(Err(InvokeError::Abort))));
    fx.env.mock_all_auths();
    assert_eq!(escrow.get_status(&deal_id), EscrowStatus::Funded);
    fx.mirror.assert_unchanged(&fx.env, &snap);

    // 4. Blank auth envelope rejects `wallet.confirm` at the host layer.
    let tx_2 = wallet.submit_withdrawal(
        &fx.accounts.user1,
        &fx.token.address,
        &fx.accounts.validator,
        &500,
    );
    fx.env.set_auths(&[]);
    let res = wallet.try_confirm(&tx_2, &fx.accounts.user1);
    assert!(matches!(res, Err(Err(InvokeError::Abort))));
    fx.env.mock_all_auths();
    assert!(wallet.get_confirmations(&tx_2).is_empty());
    fx.mirror.assert_unchanged(&fx.env, &snap);
}

#[test]
fn downstream_escrow_revert_rolls_back_multi_sig_execution_and_preserves_balances() {
    let mut fx = MultiSigEscrowFixture::new();
    let wallet = fx.wallet();
    let escrow = fx.escrow();

    let buyer = fx.accounts.validator.clone();

    // Fund buyer with 2,500 tokens via multi-sig wallet custody.
    fx.token.mint(&fx.env, &fx.accounts.user1, 5_000);
    fx.mirror.record_mint(&fx.accounts.user1, 5_000);
    wallet.deposit(&fx.token.address, &fx.accounts.user1, &5_000);
    fx.mirror
        .record_transfer(&fx.accounts.user1, &fx.wallet_id, 5_000);

    let fund_buyer_tx =
        wallet.submit_withdrawal(&fx.accounts.user1, &fx.token.address, &buyer, &2_500);
    wallet.confirm(&fund_buyer_tx, &fx.accounts.user1);
    wallet.confirm(&fund_buyer_tx, &fx.accounts.user2);
    wallet.execute(&fund_buyer_tx);
    fx.mirror.record_transfer(&fx.wallet_id, &buyer, 2_500);
    fx.mirror.assert_conserved(&fx.env);

    // Case 1: Buyer attempts to deposit into a 4,000-token escrow with only 2,500 tokens.
    let over_deal = escrow.create_escrow(
        &buyer,
        &fx.wallet_id,
        &fx.accounts.arbiter,
        &fx.token.address,
        &4_000,
        &3_600,
    );
    let snap = fx.mirror.snapshot();
    let err = escrow.try_deposit(&over_deal).unwrap_err().unwrap();
    assert_eq!(err, ForgeError::TokenTransferFailed);
    assert_eq!(escrow.get_status(&over_deal), EscrowStatus::Pending);
    fx.mirror.assert_unchanged(&fx.env, &snap);

    // Case 2: Create and fund a valid 2,500-token escrow with `wallet_id` as seller.
    let deal_id = escrow.create_escrow(
        &buyer,
        &fx.wallet_id,
        &fx.accounts.arbiter,
        &fx.token.address,
        &2_500,
        &3_600,
    );
    escrow.deposit(&deal_id);
    fx.mirror.record_transfer(&buyer, &fx.escrow_id, 2_500);
    fx.mirror.assert_conserved(&fx.env);

    // Multi-sig submits and confirms a call to `release_partial(deal_id, 3_000)`
    // (exceeding the 2,500 escrowed amount).
    let bad_args = soroban_sdk::vec![
        &fx.env,
        deal_id.into_val(&fx.env),
        3_000_i128.into_val(&fx.env)
    ];
    let bad_call_tx = wallet.submit_call(
        &fx.accounts.user1,
        &fx.escrow_id,
        &Symbol::new(&fx.env, "release_partial"),
        &bad_args,
        &None,
    );
    wallet.confirm(&bad_call_tx, &fx.accounts.user1);
    wallet.confirm(&bad_call_tx, &fx.accounts.user2);

    let snap = fx.mirror.snapshot();
    let exec_err = wallet.try_execute(&bad_call_tx).unwrap_err().unwrap();
    assert_eq!(exec_err, ForgeError::ContractInvocationFailed);
    assert_no_contract_events(&fx.env, &fx.escrow_id);

    // Upstream multi-sig tx remains Pending; downstream escrow remains Funded with 0 released.
    assert_eq!(wallet.get_tx(&bad_call_tx).status, TxStatus::Pending);
    let deal_state = escrow.get_escrow(&deal_id);
    assert_eq!(deal_state.status, EscrowStatus::Funded);
    assert_eq!(deal_state.released, 0);
    fx.mirror.assert_unchanged(&fx.env, &snap);

    // Case 3: Configure a rolling withdrawal limit of 1,000 on the wallet;
    // submitting a 1,500 withdrawal is rejected without altering wallet custody.
    let limit = WithdrawalLimit {
        token: fx.token.address.clone(),
        amount: 1_000,
        window_seconds: 3_600,
    };
    let limit_args = soroban_sdk::vec![&fx.env, limit.into_val(&fx.env)];
    let limit_tx = wallet.submit_call(
        &fx.accounts.user1,
        &fx.wallet_id,
        &Symbol::new(&fx.env, "set_withdrawal_limit"),
        &limit_args,
        &None,
    );
    wallet.confirm(&limit_tx, &fx.accounts.user1);
    wallet.confirm(&limit_tx, &fx.accounts.user2);
    wallet.execute(&limit_tx);

    let snap = fx.mirror.snapshot();
    let over_limit_err = wallet
        .try_submit_withdrawal(&fx.accounts.user1, &fx.token.address, &buyer, &1_500)
        .unwrap_err()
        .unwrap();
    assert_eq!(over_limit_err, ForgeError::InsufficientFunds);
    assert_eq!(wallet.balance(&fx.token.address), 2_500);
    fx.mirror.assert_unchanged(&fx.env, &snap);
}

#[test]
fn multi_sig_escrow_conservation_holds_across_deterministic_seeds() {
    for seed in DEFAULT_COMPOSITION_SEEDS {
        let mut rng = SeededRng::new(seed);
        let mut fx = MultiSigEscrowFixture::new();
        let wallet = fx.wallet();
        let escrow = fx.escrow();

        let buyer = fx.accounts.validator.clone();
        let seller = fx.accounts.user3.clone();
        let arbiter = fx.accounts.arbiter.clone();

        let initial_mint = rng.range_i128(5_000, 50_000);
        let wallet_deposit = rng.range_i128(2_000, initial_mint);
        let deal_amount = rng.range_i128(1_000, wallet_deposit);
        let timeout = rng.range_u64(600, 7_200);

        fx.token.mint(&fx.env, &fx.accounts.user1, initial_mint);
        fx.mirror.record_mint(&fx.accounts.user1, initial_mint);
        fx.mirror.assert_conserved(&fx.env);

        wallet.deposit(&fx.token.address, &fx.accounts.user1, &wallet_deposit);
        fx.mirror
            .record_transfer(&fx.accounts.user1, &fx.wallet_id, wallet_deposit);
        fx.mirror.assert_conserved(&fx.env);

        let withdraw_tx =
            wallet.submit_withdrawal(&fx.accounts.user1, &fx.token.address, &buyer, &deal_amount);
        wallet.confirm(&withdraw_tx, &fx.accounts.user1);
        wallet.confirm(&withdraw_tx, &fx.accounts.user2);
        wallet.execute(&withdraw_tx);
        fx.mirror
            .record_transfer(&fx.wallet_id, &buyer, deal_amount);
        fx.mirror.assert_conserved(&fx.env);

        let deal_id = escrow.create_escrow(
            &buyer,
            &seller,
            &arbiter,
            &fx.token.address,
            &deal_amount,
            &timeout,
        );
        escrow.deposit(&deal_id);
        fx.mirror
            .record_transfer(&buyer, &fx.escrow_id, deal_amount);
        fx.mirror.assert_conserved(&fx.env);

        let first_partial = rng.range_i128(1, deal_amount / 2);
        escrow.release_partial(&deal_id, &first_partial);
        fx.mirror
            .record_transfer(&fx.escrow_id, &seller, first_partial);
        fx.mirror.assert_conserved(&fx.env);

        let remaining = deal_amount - first_partial;
        match rng.range_u32(0, 2) {
            0 => {
                // Full release of remaining balance to seller.
                escrow.release(&deal_id);
                assert_eq!(escrow.get_status(&deal_id), EscrowStatus::Completed);
                fx.mirror.record_transfer(&fx.escrow_id, &seller, remaining);
            }
            1 => {
                // Advance past deadline and refund remaining balance to buyer.
                let now = fx.env.ledger().timestamp();
                fx.env.ledger().set_timestamp(now + timeout + 1);
                escrow.refund_expired(&deal_id);
                assert_eq!(escrow.get_status(&deal_id), EscrowStatus::Refunded);
                fx.mirror.record_transfer(&fx.escrow_id, &buyer, remaining);
            }
            _ => {
                // Dispute and arbiter split resolution.
                let seller_bps = rng.range_u32(1_000, 9_000);
                escrow.dispute(&deal_id, &buyer);
                escrow.resolve_split(&deal_id, &seller_bps);
                assert_eq!(escrow.get_status(&deal_id), EscrowStatus::Completed);
                let seller_share = (remaining * i128::from(seller_bps)) / 10_000;
                let buyer_share = remaining - seller_share;
                fx.mirror
                    .record_transfer(&fx.escrow_id, &seller, seller_share);
                fx.mirror
                    .record_transfer(&fx.escrow_id, &buyer, buyer_share);
            }
        }

        fx.mirror.assert_conserved(&fx.env);
    }
}
