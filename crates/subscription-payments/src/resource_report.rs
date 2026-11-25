use super::{SorobanForgeSubscriptionPaymentsClient, SubscriptionPayments};
use soroban_sdk::{testutils::Address as _, Address, Env};

#[test]
fn resource_report_scenario() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(SubscriptionPayments, ());
    let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract_id);
    let subscriber = Address::generate(&env);
    let provider = Address::generate(&env);
    let token = Address::generate(&env);

    assert_eq!(
        client.subscribe(&subscriber, &provider, &token, &500, &2_592_000),
        1
    );
    soroban_forge_test_utils::emit_resource_report(&env, "subscription-payments.subscribe");
}
