use super::*;
use soroban_forge_escrow::{Escrow, EscrowStatus, SorobanForgeEscrowClient};
use soroban_forge_multi_sig_wallet::{MultiSigWallet, SorobanForgeMultiSigWalletClient, TxStatus};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::Address;

const START: u64 = 1_000_000;
const TIMEOUT: u64 = 3_600;

/// Helper to set up SAC token, MultiSigWallet, Escrow, and accounts.
fn setup_multi_sig_escrow() -> (
    Env,
    Address,
    StellarAssetClient<'static>,
    SorobanForgeMultiSigWalletClient<'static>,
    SorobanForgeEscrowClient<'static>,
    soroban_forge_test_utils::TestAccounts,
) {
    let (env, accounts) = setup_env();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);

    let token_admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(token_admin);
    let token = sac.address();
    let token_client = StellarAssetClient::new(&env, &token);

    let ms_id = env.register(MultiSigWallet, ());
    let ms_client = SorobanForgeMultiSigWalletClient::new(&env, &ms_id);

    let escrow_id = env.register(Escrow, ());
    let escrow_client = SorobanForgeEscrowClient::new(&env, &escrow_id);

    (env, token, token_client, ms_client, escrow_client, accounts)
}

#[test]
fn multi_sig_funds_escrow_and_completes_settlement() {
    let (env, token, token_client, ms_client, escrow_client, accounts) = setup_multi_sig_escrow();

    // 1. Initialize multi-sig wallet with 3 owners and threshold 2.
    let owners = soroban_sdk::vec![
        &env,
        accounts.user1.clone(),
        accounts.user2.clone(),
        accounts.arbiter.clone(),
    ];
    ms_client.initialize(&owners, &2);

    // 2. Fund multi-sig wallet treasury with 20_000 tokens.
    token_client.mint(&accounts.user1, &20_000);
    ms_client.deposit(&token, &accounts.user1, &20_000);
    assert_eq!(ms_client.balance(&token), 20_000);

    // 3. Create an escrow: Buyer is designated agent (user3), Seller is user4, Arbiter is accounts.arbiter.
    let escrow_amount = 8_000_i128;
    let escrow_id = escrow_client.create_escrow(
        &accounts.user3,
        &accounts.validator, // seller
        &accounts.arbiter,
        &token,
        &escrow_amount,
        &TIMEOUT,
    );
    assert_eq!(escrow_client.get_status(&escrow_id), EscrowStatus::Pending);

    // 4. Multi-sig owners propose a withdrawal of 8_000 tokens to fund buyer (user3).
    let tx_id =
        ms_client.submit_withdrawal(&accounts.user1, &token, &accounts.user3, &escrow_amount);
    assert_eq!(ms_client.get_tx(&tx_id).status, TxStatus::Pending);

    // 5. Owners confirm the transaction to reach threshold 2.
    ms_client.confirm(&tx_id, &accounts.user1);
    ms_client.confirm(&tx_id, &accounts.user2);

    // 6. Execute the withdrawal from multi-sig treasury.
    ms_client.execute(&tx_id);
    assert_eq!(ms_client.get_tx(&tx_id).status, TxStatus::Executed);
    assert_eq!(ms_client.balance(&token), 12_000);
    assert_eq!(token_client.balance(&accounts.user3), 8_000);

    // 7. Buyer deposits into Escrow.
    escrow_client.deposit(&escrow_id);
    assert_eq!(escrow_client.get_status(&escrow_id), EscrowStatus::Funded);
    assert_eq!(token_client.balance(&accounts.user3), 0);
    assert_eq!(token_client.balance(&escrow_client.address), 8_000);

    // 8. Seller fulfills delivery and confirms release of funds.
    escrow_client.release(&escrow_id);
    assert_eq!(
        escrow_client.get_status(&escrow_id),
        EscrowStatus::Completed
    );
    assert_eq!(token_client.balance(&accounts.validator), 8_000);
    assert_eq!(token_client.balance(&escrow_client.address), 0);

    // 9. Verify overall token conservation across all contracts and participants:
    // Initial: 20_000 minted.
    // Final: MultiSig (12_000) + Seller (8_000) = 20_000.
    assert_eq!(ms_client.balance(&token), 12_000);
    assert_eq!(token_client.balance(&accounts.validator), 8_000);
    assert_eq!(token_client.balance(&accounts.user3), 0);
}

#[test]
fn multi_sig_funds_escrow_with_seller_refund_returned_to_treasury() {
    let (env, token, token_client, ms_client, escrow_client, accounts) = setup_multi_sig_escrow();

    // 1. Initialize multi-sig wallet with threshold 2.
    let owners = soroban_sdk::vec![
        &env,
        accounts.user1.clone(),
        accounts.user2.clone(),
        accounts.arbiter.clone(),
    ];
    ms_client.initialize(&owners, &2);

    // 2. Fund multi-sig wallet with 15_000 tokens.
    token_client.mint(&accounts.user1, &15_000);
    ms_client.deposit(&token, &accounts.user1, &15_000);
    assert_eq!(ms_client.balance(&token), 15_000);

    // 3. Create escrow for 6_000 tokens.
    let escrow_amount = 6_000_i128;
    let escrow_id = escrow_client.create_escrow(
        &accounts.user3,
        &accounts.validator,
        &accounts.arbiter,
        &token,
        &escrow_amount,
        &TIMEOUT,
    );

    // 4. Approve and execute withdrawal to buyer.
    let tx_id =
        ms_client.submit_withdrawal(&accounts.user1, &token, &accounts.user3, &escrow_amount);
    ms_client.confirm(&tx_id, &accounts.user1);
    ms_client.confirm(&tx_id, &accounts.user2);
    ms_client.execute(&tx_id);

    // 5. Buyer deposits into escrow.
    escrow_client.deposit(&escrow_id);
    assert_eq!(escrow_client.get_status(&escrow_id), EscrowStatus::Funded);

    // 6. Seller triggers refund before deadline.
    escrow_client.refund(&escrow_id);
    assert_eq!(escrow_client.get_status(&escrow_id), EscrowStatus::Refunded);
    assert_eq!(token_client.balance(&accounts.user3), 6_000);

    // 7. Buyer returns refunded tokens to the multi-sig treasury.
    ms_client.deposit(&token, &accounts.user3, &6_000);

    // 8. Verify multi-sig treasury is completely restored to 15_000 tokens.
    assert_eq!(ms_client.balance(&token), 15_000);
    assert_eq!(token_client.balance(&accounts.user3), 0);
    assert_eq!(token_client.balance(&accounts.validator), 0);
    assert_eq!(token_client.balance(&escrow_client.address), 0);
}

#[test]
fn multi_sig_funds_escrow_disputed_and_split_resolved() {
    let (env, token, token_client, ms_client, escrow_client, accounts) = setup_multi_sig_escrow();

    let owners = soroban_sdk::vec![
        &env,
        accounts.user1.clone(),
        accounts.user2.clone(),
        accounts.arbiter.clone(),
    ];
    ms_client.initialize(&owners, &2);

    token_client.mint(&accounts.user1, &10_000);
    ms_client.deposit(&token, &accounts.user1, &10_000);

    let escrow_amount = 10_000_i128;
    let escrow_id = escrow_client.create_escrow(
        &accounts.user3,
        &accounts.validator,
        &accounts.arbiter,
        &token,
        &escrow_amount,
        &TIMEOUT,
    );

    let tx_id =
        ms_client.submit_withdrawal(&accounts.user1, &token, &accounts.user3, &escrow_amount);
    ms_client.confirm(&tx_id, &accounts.user1);
    ms_client.confirm(&tx_id, &accounts.user2);
    ms_client.execute(&tx_id);
    escrow_client.deposit(&escrow_id);

    // Buyer raises a dispute.
    escrow_client.dispute(&escrow_id, &accounts.user3);
    assert_eq!(escrow_client.get_status(&escrow_id), EscrowStatus::Disputed);

    // Arbiter resolves dispute with 60% (6_000 bps) to seller, 40% (4_000) to buyer.
    escrow_client.resolve_split(&escrow_id, &6_000);
    assert_eq!(
        escrow_client.get_status(&escrow_id),
        EscrowStatus::Completed
    );
    assert_eq!(token_client.balance(&accounts.validator), 6_000);
    assert_eq!(token_client.balance(&accounts.user3), 4_000);

    // Buyer returns their remaining 4_000 to the multi-sig treasury.
    ms_client.deposit(&token, &accounts.user3, &4_000);
    assert_eq!(ms_client.balance(&token), 4_000);
    assert_eq!(token_client.balance(&accounts.user3), 0);
}
