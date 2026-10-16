//! Test-only contracts and SEP-41 fixtures for cross-contract dispatch tests.
//!
//! Use [`MockTarget`] when a test needs a successful `execute(Bytes)` target,
//! payload recording, or deterministic revert control. Use [`TokenFixture`]
//! for deterministic SEP-41 Stellar Asset Contract deployment, minting, and
//! balance inspection.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contracttype, token, Address, Env, Vec};

/// A fixed set of distinct mock accounts for use in tests.
///
/// Addresses are generated from the provided [`Env`] and are stable for the
/// lifetime of that environment, giving tests readable, named participants
/// (deployer, users, validator, arbiter) without hard-coding addresses.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TestAccounts {
    /// Account that deploys contracts and funds operations.
    pub deployer: Address,
    /// First regular user.
    pub user1: Address,
    /// Second regular user.
    pub user2: Address,
    /// Third regular user.
    pub user3: Address,
    /// A validator / signer role.
    pub validator: Address,
    /// A neutral arbiter role.
    pub arbiter: Address,
}

impl TestAccounts {
    /// Generate a fresh set of distinct mock addresses from `env`.
    #[must_use]
    pub fn generate(env: &Env) -> Self {
        Self {
            deployer: Address::generate(env),
            user1: Address::generate(env),
            user2: Address::generate(env),
            user3: Address::generate(env),
            validator: Address::generate(env),
            arbiter: Address::generate(env),
        }
    }

    /// All six accounts as a [`Vec`], useful for multi-party setup.
    #[must_use]
    pub fn all(&self, env: &Env) -> Vec<Address> {
        let mut v = Vec::new(env);
        v.push_back(self.deployer.clone());
        v.push_back(self.user1.clone());
        v.push_back(self.user2.clone());
        v.push_back(self.user3.clone());
        v.push_back(self.validator.clone());
        v.push_back(self.arbiter.clone());
        v
    }
}

/// Deterministic SEP-41 Stellar Asset Contract fixture for contract tests.
///
/// Wraps `Env::register_stellar_asset_contract_v2` and provides ergonomic
/// helpers for minting, transferring, and querying balances across accounts
/// and contracts.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenFixture {
    /// Registered SEP-41 token contract address.
    pub address: Address,
    /// Admin address authorized to mint tokens.
    pub admin: Address,
}

impl TokenFixture {
    /// Deploy a new SEP-41 Stellar Asset Contract controlled by `admin`.
    #[must_use]
    pub fn new(env: &Env, admin: &Address) -> Self {
        let sac = env.register_stellar_asset_contract_v2(admin.clone());
        Self {
            address: sac.address(),
            admin: admin.clone(),
        }
    }

    /// Deploy a new SEP-41 Stellar Asset Contract with a freshly generated admin.
    #[must_use]
    pub fn with_default_admin(env: &Env) -> Self {
        let admin = Address::generate(env);
        Self::new(env, &admin)
    }

    /// Construct a standard SEP-41 [`token::Client`] for this token.
    #[must_use]
    pub fn client<'a>(&self, env: &'a Env) -> token::Client<'a> {
        token::Client::new(env, &self.address)
    }

    /// Construct a [`token::StellarAssetClient`] for administrative actions.
    #[must_use]
    pub fn asset_client<'a>(&self, env: &'a Env) -> token::StellarAssetClient<'a> {
        token::StellarAssetClient::new(env, &self.address)
    }

    /// Mint `amount` tokens to `to`.
    pub fn mint(&self, env: &Env, to: &Address, amount: i128) {
        self.asset_client(env).mint(to, &amount);
    }

    /// Query the current SEP-41 balance of `account`.
    #[must_use]
    pub fn balance(&self, env: &Env, account: &Address) -> i128 {
        self.client(env).balance(account)
    }

    /// Transfer `amount` tokens from `from` to `to`.
    pub fn transfer(&self, env: &Env, from: &Address, to: &Address, amount: i128) {
        self.client(env).transfer(from, to, &amount);
    }
}

/// Mock target contract for testing cross-contract invocations.
///
/// Records the execution count and last dispatched payload in instance storage.
#[soroban_sdk::contract]
pub struct MockTarget;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MockTargetKey {
    Count,
    LastPayload,
    Revert,
}

#[soroban_sdk::contractimpl]
impl MockTarget {
    /// Dispatched by contracts executing actions via cross-contract calls.
    ///
    /// # Panics
    ///
    /// Panics if `set_revert(true)` has been configured on this instance.
    pub fn execute(env: Env, payload: soroban_sdk::Bytes) {
        assert!(
            !env.storage()
                .instance()
                .get(&MockTargetKey::Revert)
                .unwrap_or(false),
            "mock target reverted"
        );
        let count: u32 = env
            .storage()
            .instance()
            .get(&MockTargetKey::Count)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&MockTargetKey::Count, &(count + 1));
        env.storage()
            .instance()
            .set(&MockTargetKey::LastPayload, &payload);
    }

    /// Make subsequent `execute` calls revert when enabled.
    pub fn set_revert(env: Env, revert: bool) {
        env.storage()
            .instance()
            .set(&MockTargetKey::Revert, &revert);
    }

    /// Read the number of times `execute` was called.
    #[must_use]
    pub fn count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&MockTargetKey::Count)
            .unwrap_or(0)
    }

    /// Read the last payload dispatched to `execute`.
    #[must_use]
    pub fn last_payload(env: Env) -> Option<soroban_sdk::Bytes> {
        env.storage().instance().get(&MockTargetKey::LastPayload)
    }
}

/// A mock target whose `execute` panics, simulating a target revert.
#[soroban_sdk::contract]
pub struct RevertingTarget;

#[soroban_sdk::contractimpl]
impl RevertingTarget {
    /// Panics on invocation, simulating a target revert.
    ///
    /// # Panics
    ///
    /// Always panics with `"target reverted"`.
    pub fn execute(_env: Env, _payload: soroban_sdk::Bytes) {
        panic!("target reverted");
    }
}

#[cfg(test)]
mod tests {
    use super::{MockTarget, MockTargetClient, TestAccounts, TokenFixture};
    use soroban_sdk::{Bytes, Env};

    #[test]
    fn target_succeeds_by_default_and_records_payload() {
        let env = Env::default();
        let target = env.register(MockTarget, ());
        let client = MockTargetClient::new(&env, &target);
        let payload = Bytes::from_slice(&env, b"first");

        client.execute(&payload);

        assert_eq!(client.count(), 1);
        assert_eq!(client.last_payload(), Some(payload));
    }

    #[test]
    fn forced_revert_is_controllable_and_does_not_record_failed_payload() {
        let env = Env::default();
        let target = env.register(MockTarget, ());
        let client = MockTargetClient::new(&env, &target);
        client.set_revert(&true);

        assert!(client
            .try_execute(&Bytes::from_slice(&env, b"revert"))
            .is_err());
        assert_eq!(client.count(), 0);
        assert_eq!(client.last_payload(), None);

        client.set_revert(&false);
        client.execute(&Bytes::from_slice(&env, b"success"));
        assert_eq!(client.count(), 1);
    }

    #[test]
    fn records_the_most_recent_payload_across_multiple_calls() {
        let env = Env::default();
        let target = env.register(MockTarget, ());
        let client = MockTargetClient::new(&env, &target);
        let first = Bytes::from_slice(&env, b"first");
        let second = Bytes::from_slice(&env, b"second");

        client.execute(&first);
        client.execute(&second);

        assert_eq!(client.count(), 2);
        assert_eq!(client.last_payload(), Some(second));
    }

    #[test]
    fn instances_keep_revert_state_and_payloads_independent() {
        let env = Env::default();
        let first = env.register(MockTarget, ());
        let second = env.register(MockTarget, ());
        let first_client = MockTargetClient::new(&env, &first);
        let second_client = MockTargetClient::new(&env, &second);
        first_client.set_revert(&true);

        assert!(first_client
            .try_execute(&Bytes::from_slice(&env, b"blocked"))
            .is_err());
        second_client.execute(&Bytes::from_slice(&env, b"allowed"));

        assert_eq!(first_client.count(), 0);
        assert_eq!(second_client.count(), 1);
        assert_eq!(
            second_client.last_payload(),
            Some(Bytes::from_slice(&env, b"allowed"))
        );
    }

    #[test]
    fn token_fixture_mints_transfers_and_queries_balances() {
        let env = crate::new_env();
        let accounts = TestAccounts::generate(&env);
        let token = TokenFixture::new(&env, &accounts.deployer);

        assert_eq!(token.balance(&env, &accounts.user1), 0);
        token.mint(&env, &accounts.user1, 1_000);
        assert_eq!(token.balance(&env, &accounts.user1), 1_000);

        token.transfer(&env, &accounts.user1, &accounts.user2, 350);
        assert_eq!(token.balance(&env, &accounts.user1), 650);
        assert_eq!(token.balance(&env, &accounts.user2), 350);
    }
}
