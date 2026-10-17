use super::{MarketplaceRoyalties, SorobanForgeMarketplaceRoyaltiesClient};
use soroban_sdk::{testutils::Address as _, Address, Env};

#[test]
fn resource_report_scenario() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(MarketplaceRoyalties, ());
    let client = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &contract_id);
    let collection = Address::generate(&env);
    let recipient = Address::generate(&env);

    client.set_royalty(&collection, &recipient, &500);
    soroban_forge_test_utils::emit_resource_report(&env, "marketplace-royalties.set_royalty");
}
