use super::{SorobanForgeVestingClient, Vesting};
use soroban_sdk::{testutils::Address as _, Address, Env};

#[test]
fn resource_report_scenario() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Vesting, ());
    let client = SorobanForgeVestingClient::new(&env, &contract_id);
    let beneficiary = Address::generate(&env);
    let token = Address::generate(&env);

    assert_eq!(
        client.create_schedule(&beneficiary, &token, &10_000, &1_000, &4_000),
        1
    );
    soroban_forge_test_utils::emit_resource_report(&env, "vesting.create_schedule");
}
