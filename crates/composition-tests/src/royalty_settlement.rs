//! Escrow sale proceeds are released before marketplace royalties are settled.

use super::*;
use soroban_forge_escrow::{Escrow, EscrowStatus, SorobanForgeEscrowClient};
use soroban_forge_marketplace_royalties::{
    MarketplaceRoyalties, SorobanForgeMarketplaceRoyaltiesClient,
};
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::Address;

const START: u64 = 4_000_000;
const SALE_AMOUNT: i128 = 500;
const ROYALTY_BPS: u32 = 1_000;
const REVIEW_PERIOD: u64 = 3_600;

#[test]
fn escrow_sale_proceeds_settle_to_seller_and_royalty_holder() {
    let (env, accounts) = setup_env();
    env.ledger().set_timestamp(START);

    let token_admin_id = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin_id.clone())
        .address();
    let token_admin = StellarAssetClient::new(&env, &token_id);
    let token = TokenClient::new(&env, &token_id);
    let escrow_id = env.register(Escrow, ());
    let escrow = SorobanForgeEscrowClient::new(&env, &escrow_id);
    let royalties_id = env.register(MarketplaceRoyalties, ());
    let royalties = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &royalties_id);
    let collection_id = Address::generate(&env);

    let holders = [
        accounts.deployer.clone(),
        accounts.user1.clone(),
        accounts.user2.clone(),
        accounts.user3.clone(),
        accounts.validator.clone(),
        accounts.arbiter.clone(),
        token_admin_id,
        token_id.clone(),
        escrow_id.clone(),
        royalties_id.clone(),
        collection_id.clone(),
    ];

    royalties.set_royalty(&collection_id, &accounts.validator, &ROYALTY_BPS);
    let no_tokens = [0_i128; 11];
    assert_balances(&token, 0, &holders, &no_tokens);
    token_admin.mint(&accounts.user1, &SALE_AMOUNT);
    let buyer_funded = [0, SALE_AMOUNT, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_balances(&token, SALE_AMOUNT, &holders, &buyer_funded);

    let escrow_number = escrow.create_escrow_with_review(
        &accounts.user1,
        &accounts.user2,
        &accounts.user3,
        &token_id,
        &SALE_AMOUNT,
        &86_400,
        &REVIEW_PERIOD,
    );
    assert_balances(&token, SALE_AMOUNT, &holders, &buyer_funded);

    escrow.deposit(&escrow_number);
    let escrow_funded = [0, 0, 0, 0, 0, 0, 0, 0, SALE_AMOUNT, 0, 0];
    assert_eq!(escrow.get_status(&escrow_number), EscrowStatus::Funded);
    assert_balances(&token, SALE_AMOUNT, &holders, &escrow_funded);

    let release_at = escrow.request_release(&escrow_number, &SALE_AMOUNT);
    assert_balances(&token, SALE_AMOUNT, &holders, &escrow_funded);
    env.ledger().set_timestamp(release_at - 1);
    assert_eq!(
        escrow.try_execute_release(&escrow_number),
        Err(Ok(ForgeError::DeadlineReached))
    );
    assert_balances(&token, SALE_AMOUNT, &holders, &escrow_funded);

    env.ledger().set_timestamp(release_at);
    escrow.execute_release(&escrow_number);
    let seller_paid = [0, 0, SALE_AMOUNT, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(escrow.get_status(&escrow_number), EscrowStatus::Completed);
    assert_balances(&token, SALE_AMOUNT, &holders, &seller_paid);

    let settlement = royalties.settle_sale(
        &collection_id,
        &token_id,
        &accounts.user2,
        &accounts.user2,
        &SALE_AMOUNT,
    );
    assert_eq!(settlement.seller_net, 450);
    assert_eq!(settlement.royalty_share, 50);
    let settled = [0, 0, 450, 0, 50, 0, 0, 0, 0, 0, 0];
    let summary = royalties.get_settlement_summary(&collection_id);
    assert_eq!(summary.sales, 1);
    assert_eq!(summary.gross_volume, SALE_AMOUNT);
    assert_eq!(summary.royalties_paid, 50);
    assert_balances(&token, SALE_AMOUNT, &holders, &settled);
}
