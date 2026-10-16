//! Multi-sig wallet flow: owners submit a cross-contract transaction, collect
//! the threshold of confirmations, and the wallet dispatches it — the
//! contract's primary governance path.

use soroban_forge_multi_sig_wallet::{MultiSigWallet, SorobanForgeMultiSigWalletClient};
use soroban_forge_test_utils::{MockTarget, MockTargetClient, TestAccounts};
use soroban_sdk::{vec, Address, Bytes, Env, Vec};

use crate::metrics::ResourceSample;

const THRESHOLD: u32 = 2;

/// Runs the flow and captures the resources of the `execute` invocation.
pub(crate) fn run() -> ResourceSample {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(MultiSigWallet, ());
    let client = SorobanForgeMultiSigWalletClient::new(&env, &contract_id);
    let accounts = TestAccounts::generate(&env);
    let owners: Vec<Address> = vec![
        &env,
        accounts.user1.clone(),
        accounts.user2.clone(),
        accounts.user3.clone(),
    ];
    client.initialize(&owners, &THRESHOLD);

    let target_id = env.register(MockTarget, ());
    let payload = Bytes::from_array(&env, &[0x01, 0x02, 0x03]);
    let tx_id = client.submit(&accounts.user1, &target_id, &payload, &None);
    client.confirm(&tx_id, &accounts.user2);
    client.confirm(&tx_id, &accounts.user3);

    // The measured entrypoint: dispatch the approved call to the target.
    client.execute(&tx_id);
    let sample = ResourceSample::capture(&env);

    assert_eq!(
        MockTargetClient::new(&env, &target_id).last_payload(),
        Some(payload)
    );
    sample
}
