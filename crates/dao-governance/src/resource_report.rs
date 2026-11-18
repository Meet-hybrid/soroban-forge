use super::{DaoGovernance, SorobanForgeDaoGovernanceClient};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{testutils::Address as _, Address, Bytes, Env, Vec};

#[test]
fn resource_report_scenario() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(DaoGovernance, ());
    let client = SorobanForgeDaoGovernanceClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(admin).address();
    let token_admin = StellarAssetClient::new(&env, &token);
    let proposer = Address::generate(&env);
    let target = Address::generate(&env);
    let action = Bytes::from_slice(&env, &[1, 2, 3]);
    let treasury = Address::generate(&env);

    client.initialize(&token);
    client.configure_bond(&token, &100, &treasury);
    token_admin.mint(&proposer, &100);
    assert_eq!(
        client.propose(&proposer, &target, &action, &86_400, &Vec::new(&env), &None,),
        1
    );
    soroban_forge_test_utils::emit_resource_report(&env, "dao-governance.propose");
}
