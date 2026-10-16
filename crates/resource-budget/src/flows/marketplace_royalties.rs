//! Marketplace royalties flow: a collection's royalty is configured and a
//! sale is settled with an actual SEP-41 payment split between royalty
//! recipient and seller — the contract's primary settlement path.

use soroban_forge_marketplace_royalties::{
    MarketplaceRoyalties, SorobanForgeMarketplaceRoyaltiesClient,
};
use soroban_forge_test_utils::TestAccounts;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env};

use crate::metrics::ResourceSample;

const PRICE: i128 = 1_000;
/// 500 bps = 5 % royalty, the fixture used across the contract's test suite.
const BPS: u32 = 500;

/// Runs the flow and captures the resources of the `settle_sale` invocation.
pub(crate) fn run() -> ResourceSample {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin);
    let token = sac.address();
    let token_admin = StellarAssetClient::new(&env, &token);
    let token_client = TokenClient::new(&env, &token);

    let contract_id = env.register(MarketplaceRoyalties, ());
    let client = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &contract_id);
    let accounts = TestAccounts::generate(&env);
    // Layout: `arbiter` is the collection, `user2` the royalty recipient,
    // `user3` the seller, `user1` the payer.
    client.set_royalty(&accounts.arbiter, &accounts.user2, &BPS);
    token_admin.mint(&accounts.user1, &PRICE);

    // The measured entrypoint: settle a full-price sale.
    client.settle_sale(
        &accounts.arbiter,
        &token,
        &accounts.user1,
        &accounts.user3,
        &PRICE,
    );
    let sample = ResourceSample::capture(&env);

    assert_eq!(token_client.balance(&accounts.user2), PRICE * 5 / 100);
    assert_eq!(
        token_client.balance(&accounts.user3),
        PRICE - PRICE * 5 / 100
    );
    sample
}
