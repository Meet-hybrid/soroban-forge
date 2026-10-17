use super::{DaoGovernance, SorobanForgeDaoGovernanceClient};
use soroban_sdk::{testutils::Address as _, Address, Bytes, Env};

#[test]
fn resource_report_scenario() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(DaoGovernance, ());
    let client = SorobanForgeDaoGovernanceClient::new(&env, &contract_id);
    let proposer = Address::generate(&env);
    let target = Address::generate(&env);
    let action = Bytes::from_slice(&env, &[1, 2, 3]);

    assert_eq!(client.propose(&proposer, &target, &action, &86_400), 1);
    soroban_forge_test_utils::emit_resource_report(&env, "dao-governance.propose");
}
