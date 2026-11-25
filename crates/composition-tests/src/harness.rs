//! Deterministic actors, token conservation tracking, and auth-assertion helpers
//! shared by cross-contract composition scenarios.

use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env};
use std::collections::BTreeMap;
use std::vec::Vec;

/// Deterministic named participants generated in a fixed order per isolated
/// Soroban test environment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeterministicActors {
    pub deployer: Address,
    pub owner1: Address,
    pub owner2: Address,
    pub owner3: Address,
    pub seller: Address,
    pub arbiter: Address,
    pub provider: Address,
    pub beneficiary: Address,
    pub royalty_recipient: Address,
    pub collection: Address,
    pub outsider: Address,
}

impl DeterministicActors {
    /// Generate all deterministic actors in a fixed order from `env`.
    pub fn generate(env: &Env) -> Self {
        Self {
            deployer: Address::generate(env),
            owner1: Address::generate(env),
            owner2: Address::generate(env),
            owner3: Address::generate(env),
            seller: Address::generate(env),
            arbiter: Address::generate(env),
            provider: Address::generate(env),
            beneficiary: Address::generate(env),
            royalty_recipient: Address::generate(env),
            collection: Address::generate(env),
            outsider: Address::generate(env),
        }
    }

    /// Return the standard 3-owner multisig signer vector `(owner1, owner2, owner3)`.
    pub fn wallet_owners(&self, env: &Env) -> soroban_sdk::Vec<Address> {
        soroban_sdk::vec![
            env,
            self.owner1.clone(),
            self.owner2.clone(),
            self.owner3.clone()
        ]
    }

    /// Register all actor addresses into a [`ConservationLedger`].
    pub fn register_all(&self, ledger: &mut ConservationLedger) {
        ledger.register("deployer", self.deployer.clone());
        ledger.register("owner1", self.owner1.clone());
        ledger.register("owner2", self.owner2.clone());
        ledger.register("owner3", self.owner3.clone());
        ledger.register("seller", self.seller.clone());
        ledger.register("arbiter", self.arbiter.clone());
        ledger.register("provider", self.provider.clone());
        ledger.register("beneficiary", self.beneficiary.clone());
        ledger.register("royalty_recipient", self.royalty_recipient.clone());
        ledger.register("collection", self.collection.clone());
        ledger.register("outsider", self.outsider.clone());
    }
}

/// Create an isolated Soroban environment pinned to `start_timestamp` with
/// deterministic actors.
///
/// Setup mocking allows nested contract calls while fixtures are deployed.
/// Boundary-authorization tests explicitly replace this envelope via
/// [`Env::set_auths`] or [`Env::mock_auths`].
pub fn setup_composition_env(start_timestamp: u64) -> (Env, DeterministicActors) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    env.ledger().set_timestamp(start_timestamp);
    let actors = DeterministicActors::generate(&env);
    (env, actors)
}

/// Deploy a real Stellar Asset Contract token for a scenario.
pub fn deploy_sac_token(env: &Env) -> (Address, Address, StellarAssetClient<'_>, TokenClient<'_>) {
    let admin_id = Address::generate(env);
    let token_id = env
        .register_stellar_asset_contract_v2(admin_id.clone())
        .address();
    let admin = StellarAssetClient::new(env, &token_id);
    let token = TokenClient::new(env, &token_id);
    (admin_id, token_id, admin, token)
}

/// Tracks every participant address in a composition scenario and asserts both
/// per-holder balances and global token supply conservation after every step.
pub struct ConservationLedger {
    token_id: Address,
    minted_supply: i128,
    participants: Vec<(&'static str, Address)>,
}

impl ConservationLedger {
    pub fn new(token_id: Address, minted_supply: i128) -> Self {
        Self {
            token_id,
            minted_supply,
            participants: Vec::new(),
        }
    }

    pub fn register(&mut self, label: &'static str, address: Address) {
        assert!(
            !self.participants.iter().any(|(name, _)| *name == label),
            "duplicate conservation participant label: {label}"
        );
        self.participants.push((label, address));
    }

    /// Assert every tracked participant holds its exact expected balance
    /// (participants omitted from `non_zero_expected` must hold `0`) and that
    /// total holdings equal `minted_supply`.
    pub fn assert_conservation(&self, env: &Env, non_zero_expected: &[(&str, i128)]) {
        let token = TokenClient::new(env, &self.token_id);
        let mut expected_map: BTreeMap<&str, i128> = BTreeMap::new();
        for &(label, amount) in non_zero_expected {
            assert!(
                self.participants.iter().any(|(name, _)| *name == label),
                "unknown conservation participant label: {label}"
            );
            assert!(
                expected_map.insert(label, amount).is_none(),
                "duplicate expected balance for label: {label}"
            );
        }

        let mut total = 0_i128;
        for (label, address) in &self.participants {
            let actual = token.balance(address);
            let expected = expected_map.get(label).copied().unwrap_or(0);
            assert_eq!(
                actual, expected,
                "token balance mismatch for participant `{label}`"
            );
            total = total
                .checked_add(actual)
                .expect("token balance sum overflowed i128");
        }

        assert_eq!(
            total, self.minted_supply,
            "token conservation violated: tracked total {total} != minted supply {}",
            self.minted_supply
        );
    }

    /// Snapshot every participant balance prior to a call expected to fail or
    /// roll back.
    pub fn snapshot(&self, env: &Env) -> Vec<(&'static str, i128)> {
        let token = TokenClient::new(env, &self.token_id);
        self.participants
            .iter()
            .map(|(label, addr)| (*label, token.balance(addr)))
            .collect()
    }

    /// Assert that no tracked participant balance changed relative to `before`
    /// and total supply remains conserved.
    pub fn assert_unchanged(&self, env: &Env, before: &[(&'static str, i128)]) {
        self.assert_conservation(env, before);
    }
}

/// Assert that a `try_` contract invocation aborted at the host authorization
/// boundary.
#[macro_export]
macro_rules! assert_auth_abort {
    ($res:expr) => {
        assert!(
            matches!($res, Err(Err(soroban_sdk::InvokeError::Abort))),
            "expected host authorization abort, got {:?}",
            $res
        );
    };
}

#[test]
fn deterministic_actors_reproduce_identically_across_isolated_envs() {
    let (_env_a, actors_a) = setup_composition_env(1_000_000);
    let (_env_b, actors_b) = setup_composition_env(1_000_000);

    assert_eq!(actors_a, actors_b);
    assert_eq!(actors_a.wallet_owners(&_env_a).len(), 3);
    assert_ne!(actors_a.owner1, actors_a.owner2);
    assert_ne!(actors_a.owner2, actors_a.owner3);
    assert_ne!(actors_a.seller, actors_a.provider);
    assert_ne!(actors_a.beneficiary, actors_a.outsider);
}

#[test]
fn conservation_ledger_tracks_transfers_and_snapshots() {
    let (env, actors) = setup_composition_env(1_000_000);
    let (token_admin_id, token_id, token_admin, token) = deploy_sac_token(&env);
    let supply = 1_000_i128;
    token_admin.mint(&actors.deployer, &supply);

    let mut ledger = ConservationLedger::new(token_id.clone(), supply);
    actors.register_all(&mut ledger);
    ledger.register("token_admin", token_admin_id);
    ledger.register("token", token_id);

    ledger.assert_conservation(&env, &[("deployer", supply)]);
    let snap = ledger.snapshot(&env);
    ledger.assert_unchanged(&env, &snap);

    token.transfer(&actors.deployer, &actors.seller, &400);
    ledger.assert_conservation(&env, &[("deployer", 600), ("seller", 400)]);
}
