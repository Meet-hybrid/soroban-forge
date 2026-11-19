//! Test-only contracts for cross-contract dispatch tests.
//!
//! Use [`MockTarget`] when a test needs a successful `execute(Bytes)` target,
//! payload recording, or deterministic revert control, including failure on a
//! selected successful-call ordinal. Use a token fixture for SEP-41 behavior;
//! these targets intentionally do not model token balances or production
//! authorization.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contracttype, Address, Env, Vec};

/// A fixed set of distinct mock accounts for use in tests.
///
/// Addresses are generated from the provided [`Env`] and are stable for the
/// lifetime of that environment, giving tests readable, named participants
/// (deployer, users, validator, arbiter) without hard-coding addresses.
#[contracttype]
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

/// Deterministic external-call failure schedule shared by test contracts.
///
/// The values are deliberately finite and explicit so a test can reproduce the
/// same invocation trace under the same seed. They are never used by
/// production contract code.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FailureSchedule {
    None,
    BeforeTransfer,
    BeforeCall(u32),
    AfterAuthorization,
    MalformedResult,
    AuthorizationMismatch,
}

impl TestAccounts {
    /// Generate a fresh set of distinct mock addresses from `env`.
    pub fn generate(env: &Env) -> Self {
        TestAccounts {
            deployer: Address::generate(env),
            user1: Address::generate(env),
            user2: Address::generate(env),
            user3: Address::generate(env),
            validator: Address::generate(env),
            arbiter: Address::generate(env),
        }
    }

    /// All six accounts as a [`Vec`], useful for multi-party setup.
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
    FailAt,
    Schedule,
    Trace,
}

#[soroban_sdk::contractimpl]
impl MockTarget {
    /// Dispatched by contracts executing actions via cross-contract calls.
    pub fn execute(env: Env, payload: soroban_sdk::Bytes) {
        let count: u32 = env
            .storage()
            .instance()
            .get(&MockTargetKey::Count)
            .unwrap_or(0);
        let next_call = count.saturating_add(1);
        let mut trace: soroban_sdk::Vec<u32> = env
            .storage()
            .instance()
            .get(&MockTargetKey::Trace)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env));
        trace.push_back(next_call);
        env.storage().instance().set(&MockTargetKey::Trace, &trace);
        let schedule: FailureSchedule = env
            .storage()
            .instance()
            .get(&MockTargetKey::Schedule)
            .unwrap_or(FailureSchedule::None);
        match schedule {
            FailureSchedule::BeforeCall(call) if call == next_call => {
                panic!("mock target failed before call {call}");
            }
            FailureSchedule::BeforeTransfer => {
                panic!("mock target rejected transfer");
            }
            FailureSchedule::AfterAuthorization => {
                panic!("mock target authorization mismatch");
            }
            FailureSchedule::MalformedResult => {
                panic!("mock target returned malformed data");
            }
            FailureSchedule::AuthorizationMismatch => {
                panic!("mock target authorization mismatch");
            }
            _ => {}
        }
        if env
            .storage()
            .instance()
            .get(&MockTargetKey::Revert)
            .unwrap_or(false)
        {
            panic!("mock target reverted");
        }
        let fail_at: u32 = env
            .storage()
            .instance()
            .get(&MockTargetKey::FailAt)
            .unwrap_or(0);
        if fail_at != 0 && fail_at == next_call {
            panic!("mock target failed at configured call {fail_at}");
        }
        env.storage()
            .instance()
            .set(&MockTargetKey::Count, &next_call);
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

    /// Fail when `execute` reaches the configured one-based successful-call
    /// ordinal. Pass `0` to disable the failure point. Since a reverted call
    /// does not advance the successful-call count, the failure remains armed
    /// until it is cleared or moved.
    pub fn set_fail_at(env: Env, call_number: u32) {
        env.storage()
            .instance()
            .set(&MockTargetKey::FailAt, &call_number);
    }

    /// Configure a deterministic failure schedule for subsequent calls.
    pub fn set_failure_schedule(env: Env, schedule: FailureSchedule) {
        env.storage()
            .instance()
            .set(&MockTargetKey::Schedule, &schedule);
    }

    /// Return the invocation trace in one-based call/order form.
    pub fn trace(env: Env) -> soroban_sdk::Vec<u32> {
        env.storage()
            .instance()
            .get(&MockTargetKey::Trace)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env))
    }

    /// Read the number of times `execute` was called.
    pub fn count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&MockTargetKey::Count)
            .unwrap_or(0)
    }

    /// Read the last payload dispatched to `execute`.
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
    pub fn execute(_env: Env, _payload: soroban_sdk::Bytes) {
        panic!("target reverted");
    }
}

/// Minimal SEP-41-shaped test token with deterministic transfer failure points.
///
/// It exists only in `test-utils` and is not production contract code. The
/// fixture supports pre-transfer rejection and one-based transfer ordinal
/// rejection, with balances and a reusable invocation trace.
#[soroban_sdk::contract]
pub struct FailureToken;

#[contracttype]
enum FailureTokenKey {
    Balance(Address),
    FailBeforeTransfer,
    FailAtTransfer,
    TransferCount,
    TransferTrace,
}

#[soroban_sdk::contractimpl]
impl FailureToken {
    pub fn mint(env: Env, to: Address, amount: i128) {
        let key = FailureTokenKey::Balance(to);
        let balance: i128 = env.storage().instance().get(&key).unwrap_or(0);
        env.storage()
            .instance()
            .set(&key, &(balance.saturating_add(amount)));
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        let key = FailureTokenKey::Balance(id);
        env.storage().instance().get(&key).unwrap_or(0)
    }

    pub fn set_fail_before_transfer(env: Env, enabled: bool) {
        env.storage()
            .instance()
            .set(&FailureTokenKey::FailBeforeTransfer, &enabled);
    }

    pub fn set_fail_at_transfer(env: Env, transfer_number: u32) {
        env.storage()
            .instance()
            .set(&FailureTokenKey::FailAtTransfer, &transfer_number);
    }

    pub fn transfer_trace(env: Env) -> soroban_sdk::Vec<u32> {
        env.storage()
            .instance()
            .get(&FailureTokenKey::TransferTrace)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env))
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        let failed_before: bool = env
            .storage()
            .instance()
            .get(&FailureTokenKey::FailBeforeTransfer)
            .unwrap_or(false);
        if failed_before {
            panic!("failure token rejected before transfer");
        }

        let count: u32 = env
            .storage()
            .instance()
            .get(&FailureTokenKey::TransferCount)
            .unwrap_or(0);
        let next = count.saturating_add(1);
        let mut trace: soroban_sdk::Vec<u32> = env
            .storage()
            .instance()
            .get(&FailureTokenKey::TransferTrace)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env));
        trace.push_back(next);
        env.storage()
            .instance()
            .set(&FailureTokenKey::TransferTrace, &trace);
        let fail_at: u32 = env
            .storage()
            .instance()
            .get(&FailureTokenKey::FailAtTransfer)
            .unwrap_or(0);
        if fail_at != 0 && fail_at == next {
            panic!("failure token rejected transfer {next}");
        }

        let from_key = FailureTokenKey::Balance(from);
        let to_key = FailureTokenKey::Balance(to);
        let from_balance: i128 = env.storage().instance().get(&from_key).unwrap_or(0);
        if amount < 0 || from_balance < amount {
            panic!("failure token has insufficient funds");
        }
        env.storage()
            .instance()
            .set(&from_key, &(from_balance - amount));
        let to_balance: i128 = env.storage().instance().get(&to_key).unwrap_or(0);
        env.storage()
            .instance()
            .set(&to_key, &(to_balance.saturating_add(amount)));
        env.storage()
            .instance()
            .set(&FailureTokenKey::TransferCount, &next);
    }
}

#[cfg(test)]
mod tests {
    use super::{FailureToken, FailureTokenClient, MockTarget, MockTargetClient};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Bytes, Env};

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
    fn configured_failure_point_is_reproducible_and_can_be_cleared() {
        let env = Env::default();
        let target = env.register(MockTarget, ());
        let client = MockTargetClient::new(&env, &target);
        client.execute(&Bytes::from_slice(&env, b"first"));
        client.execute(&Bytes::from_slice(&env, b"second"));
        client.set_fail_at(&3);

        let third = Bytes::from_slice(&env, b"third");
        assert!(client.try_execute(&third).is_err());
        assert!(client.try_execute(&third).is_err());
        assert_eq!(client.count(), 2);
        assert_eq!(
            client.last_payload(),
            Some(Bytes::from_slice(&env, b"second"))
        );

        client.set_fail_at(&0);
        client.execute(&third);
        assert_eq!(client.count(), 3);
        assert_eq!(client.last_payload(), Some(third));
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
    fn failure_token_reproducibly_traces_pre_and_later_transfer_failures() {
        let env = Env::default();
        let token = env.register(FailureToken, ());
        let client = FailureTokenClient::new(&env, &token);
        let from = Address::generate(&env);
        let to = Address::generate(&env);
        client.mint(&from, &100);

        client.set_fail_at_transfer(&2);
        assert!(client.try_transfer(&from, &to, &25).is_ok());
        assert!(client.try_transfer(&from, &to, &25).is_err());
        assert_eq!(client.balance(&from), 75);
        assert_eq!(client.balance(&to), 25);
        assert_eq!(
            client.transfer_trace(),
            soroban_sdk::Vec::from_slice(&env, &[1, 2])
        );

        client.set_fail_at_transfer(&0);
        client.set_fail_before_transfer(&true);
        assert!(client.try_transfer(&from, &to, &25).is_err());
        assert_eq!(client.balance(&from), 75);
        assert_eq!(client.balance(&to), 25);
    }
}
