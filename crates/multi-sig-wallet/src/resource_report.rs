use super::{MultiSigWallet, SorobanForgeMultiSigWalletClient};
use soroban_sdk::{testutils::Address as _, Address, Env, Vec};

#[test]
fn resource_report_scenario() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(MultiSigWallet, ());
    let client = SorobanForgeMultiSigWalletClient::new(&env, &contract_id);
    let owners = Vec::from_array(&env, [Address::generate(&env), Address::generate(&env)]);

    client.initialize(&owners, &2);
    soroban_forge_test_utils::emit_resource_report(&env, "multi-sig-wallet.initialize");
}
