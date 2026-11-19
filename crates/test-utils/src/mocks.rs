//! Test-only contracts for cross-contract dispatch tests.
//!
//! Use [`MockTarget`] when a test needs a successful `execute(Bytes)` target,
//! payload recording, or deterministic revert control, including failure on a
//! selected successful-call ordinal. Use a token fixture for SEP-41 behavior;
//! these targets intentionally do not model token balances or production
//! authorization.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contracttype, Address, Env, Val, Vec};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureBoundary {
    BeforeTransfer,
    AfterAuthorization,
    BeforeBatchStep,
    ResultDecoding,
    AuthorizationMismatch,
}

impl FailureBoundary {
    fn tag(self) -> u64 {
        match self {
            Self::BeforeTransfer => 1,
            Self::AfterAuthorization => 2,
            Self::BeforeBatchStep => 3,
            Self::ResultDecoding => 4,
            Self::AuthorizationMismatch => 5,
        }
    }
}

pub fn failure_step_for_seed(seed: u64, boundary: FailureBoundary) -> u32 {
    let mut state = seed ^ boundary.tag();
    state ^= state >> 30;
    state = state.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    state ^= state >> 27;
    state = state.wrapping_mul(0x94d0_49bb_1331_11eb);
    state ^= state >> 31;
    match boundary {
        FailureBoundary::BeforeBatchStep => (state % 8) as u32 + 1,
        _ => 1,
    }
}

impl FailureSchedule {
    pub fn seeded(seed: u64, boundary: FailureBoundary) -> Self {
        match boundary {
            FailureBoundary::BeforeTransfer => Self::BeforeTransfer,
            FailureBoundary::AfterAuthorization => Self::AfterAuthorization,
            FailureBoundary::BeforeBatchStep => {
                Self::BeforeCall(failure_step_for_seed(seed, boundary))
            }
            FailureBoundary::ResultDecoding => Self::MalformedResult,
            FailureBoundary::AuthorizationMismatch => Self::AuthorizationMismatch,
        }
    }

    pub fn failing_step(&self) -> Option<u32> {
        match self {
            Self::None | Self::MalformedResult => None,
            Self::BeforeCall(step) => Some(*step),
            Self::BeforeTransfer | Self::AfterAuthorization | Self::AuthorizationMismatch => {
                Some(1)
            }
        }
    }

    pub fn attempt_trace(&self, attempts: u32) -> std::vec::Vec<u32> {
        (1..=attempts).collect()
    }
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
    RequiredAuth,
    MismatchedAuth,
}

#[soroban_sdk::contractimpl]
impl MockTarget {
    /// Dispatched by contracts executing actions via cross-contract calls.
    pub fn execute(env: Env, payload: soroban_sdk::Bytes) -> Val {
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
        if schedule == FailureSchedule::AuthorizationMismatch {
            match env
                .storage()
                .instance()
                .get::<_, Address>(&MockTargetKey::MismatchedAuth)
            {
                Some(signer) => signer.require_auth(),
                None => panic!("mock target authorization mismatch"),
            }
        } else if let Some(required) = env
            .storage()
            .instance()
            .get::<_, Address>(&MockTargetKey::RequiredAuth)
        {
            required.require_auth();
        }
        match schedule {
            FailureSchedule::BeforeCall(call) if call == next_call => {
                panic!("mock target failed before call {call}");
            }
            FailureSchedule::BeforeTransfer => {
                panic!("mock target rejected transfer");
            }
            FailureSchedule::AfterAuthorization => {
                panic!("mock target failed after nested authorization");
            }
            FailureSchedule::AuthorizationMismatch => {
                panic!("authorization mismatch aborts before dispatch")
            }
            FailureSchedule::MalformedResult => {}
            FailureSchedule::BeforeCall(_) | FailureSchedule::None => {}
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
        match schedule {
            FailureSchedule::MalformedResult => Val::from(1_u32),
            _ => Val::from(()),
        }
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

    pub fn set_required_auth(env: Env, signer: Address) {
        env.storage()
            .instance()
            .set(&MockTargetKey::RequiredAuth, &signer);
    }

    pub fn set_mismatched_auth(env: Env, signer: Address) {
        env.storage()
            .instance()
            .set(&MockTargetKey::MismatchedAuth, &signer);
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
    use super::{
        failure_step_for_seed, FailureBoundary, FailureSchedule, FailureToken, FailureTokenClient,
        MockTarget, MockTargetClient,
    };
    use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
    use soroban_sdk::{Address, Bytes, Env, IntoVal};

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
    fn seeded_schedules_reproduce_the_same_failure_trace() {
        let seed = 42;
        let step = failure_step_for_seed(seed, FailureBoundary::BeforeBatchStep);
        assert!((1..=8).contains(&step));
        assert_eq!(
            FailureSchedule::seeded(seed, FailureBoundary::BeforeBatchStep),
            FailureSchedule::BeforeCall(step)
        );

        let schedule = FailureSchedule::BeforeCall(step);
        let mut committed = std::vec::Vec::new();
        let mut attempts = std::vec::Vec::new();
        for _ in 0..2 {
            let env = Env::default();
            let token = env.register(FailureToken, ());
            let client = FailureTokenClient::new(&env, &token);
            let from = Address::generate(&env);
            let to = Address::generate(&env);
            client.mint(&from, &100);
            client.set_fail_at_transfer(&step);
            for _ in 1..step {
                assert!(client.try_transfer(&from, &to, &1).is_ok());
            }
            assert!(client.try_transfer(&from, &to, &1).is_err());
            committed.push(
                client
                    .transfer_trace()
                    .iter()
                    .collect::<std::vec::Vec<u32>>(),
            );
            attempts.push(schedule.attempt_trace(step));
        }
        assert_eq!(committed[0], committed[1]);
        assert_eq!(attempts[0], attempts[1]);
        assert_eq!(attempts[0], (1..=step).collect::<std::vec::Vec<u32>>());
    }

    #[test]
    fn target_exposes_after_authorization_and_result_decoding_failure_points() {
        let env = Env::default();
        env.mock_all_auths();
        let target = env.register(MockTarget, ());
        let client = MockTargetClient::new(&env, &target);
        let signer = Address::generate(&env);
        let payload = Bytes::from_slice(&env, b"dispatch");
        client.set_required_auth(&signer);
        client.set_failure_schedule(&FailureSchedule::AfterAuthorization);

        assert!(client.try_execute(&payload).is_err());
        assert_eq!(client.count(), 0);
        assert!(client.trace().is_empty());

        client.set_failure_schedule(&FailureSchedule::None);
        client.execute(&payload);
        assert_eq!(client.count(), 1);
        assert_eq!(client.trace(), soroban_sdk::Vec::from_slice(&env, &[1]));

        let malformed = env.register(MockTarget, ());
        let malformed_client = MockTargetClient::new(&env, &malformed);
        malformed_client.set_failure_schedule(&FailureSchedule::MalformedResult);
        assert!(malformed_client.try_execute(&payload).is_ok());
        assert_eq!(malformed_client.count(), 1);
        assert_eq!(
            malformed_client.trace(),
            soroban_sdk::Vec::from_slice(&env, &[1])
        );
    }

    #[test]
    fn target_rejects_an_authorization_mismatch() {
        let env = Env::default();
        let target = env.register(MockTarget, ());
        let client = MockTargetClient::new(&env, &target);
        let signer = Address::generate(&env);
        let wrong = Address::generate(&env);
        let payload = Bytes::from_slice(&env, b"dispatch");
        client.set_required_auth(&signer);
        client.set_mismatched_auth(&wrong);
        client.set_failure_schedule(&FailureSchedule::AuthorizationMismatch);
        env.mock_auths(&[MockAuth {
            address: &signer,
            invoke: &MockAuthInvoke {
                contract: &target,
                fn_name: "execute",
                args: (payload.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }]);

        assert!(client.try_execute(&payload).is_err());
        assert_eq!(client.count(), 0);
        assert!(client.trace().is_empty());
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
            soroban_sdk::Vec::from_slice(&env, &[1])
        );

        client.set_fail_at_transfer(&0);
        client.set_fail_before_transfer(&true);
        assert!(client.try_transfer(&from, &to, &25).is_err());
        assert_eq!(client.balance(&from), 75);
        assert_eq!(client.balance(&to), 25);
        assert_eq!(
            client.transfer_trace(),
            soroban_sdk::Vec::from_slice(&env, &[1])
        );
    }
}
