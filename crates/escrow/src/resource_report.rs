use super::{Escrow, SorobanForgeEscrowClient};
use soroban_sdk::{testutils::Address as _, Address, Env};

#[test]
fn resource_report_scenario() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Escrow, ());
    let client = SorobanForgeEscrowClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let arbiter = Address::generate(&env);
    let token = Address::generate(&env);

    assert_eq!(
        client.create_escrow(&buyer, &seller, &arbiter, &token, &1_000, &3_600),
        1
    );
    soroban_forge_test_utils::emit_resource_report(&env, "escrow.create_escrow");
}
