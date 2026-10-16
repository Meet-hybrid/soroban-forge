//! Cross-contract composition test helpers.
//!
//! Provides reusable primitives for multi-contract settlement scenarios:
//! - [`BalanceMirror`]: an independent off-ledger balance bookkeeper and
//!   conservation verifier that tracks expected SEP-41 balances across accounts
//!   and contract custody addresses and asserts exact agreement with on-chain
//!   token state.
//! - [`SeededRng`] and [`DEFAULT_COMPOSITION_SEEDS`]: a deterministic
//!   `SplitMix64` generator and canonical seed schedule so multi-contract
//!   settlement suites execute repeatedly with reproducible inputs.
//! - Event and authorization inspection helpers
//!   ([`contract_event_count`], [`assert_contract_event`],
//!   [`assert_no_contract_events`], [`assert_sac_transfer_event`],
//!   [`has_auth_invocation`], [`assert_auth_invocation`]) for verifying
//!   cross-contract call graphs and emitted events.

extern crate alloc;

use alloc::vec::Vec;
use soroban_sdk::testutils::{AuthorizedFunction, AuthorizedInvocation, Events as _};
use soroban_sdk::xdr::{ContractEventBody, Int128Parts, ScVal};
use soroban_sdk::{token, Address, Env, Symbol};

/// Canonical deterministic seeds used to run composition scenarios repeatedly.
pub const DEFAULT_COMPOSITION_SEEDS: [u64; 8] = [
    0x50F0_BA40_0001_0001,
    0x50F0_BA40_0002_0002,
    0x50F0_BA40_0003_0003,
    0x50F0_BA40_0004_0004,
    0x50F0_BA40_0005_0005,
    0x50F0_BA40_0006_0006,
    0x50F0_BA40_0007_0007,
    0x50F0_BA40_0008_0008,
];

/// Deterministic `SplitMix64` pseudo-random number generator for composition
/// test parameter generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SeededRng {
    state: u64,
}

impl SeededRng {
    /// Construct a new deterministic generator from `seed`.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    /// Produce the next 64-bit pseudo-random word.
    pub const fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniformly sample a `u64` in `[min, max_inclusive]`.
    ///
    /// # Panics
    ///
    /// Panics if `min > max_inclusive`.
    pub fn range_u64(&mut self, min: u64, max_inclusive: u64) -> u64 {
        assert!(min <= max_inclusive, "invalid range_u64 bounds");
        let span = max_inclusive - min;
        if span == u64::MAX {
            return self.next_u64();
        }
        min + (self.next_u64() % (span + 1))
    }

    /// Uniformly sample a `u32` in `[min, max_inclusive]`.
    ///
    /// # Panics
    ///
    /// Panics if `min > max_inclusive`.
    pub fn range_u32(&mut self, min: u32, max_inclusive: u32) -> u32 {
        assert!(min <= max_inclusive, "invalid range_u32 bounds");
        let sampled = self.range_u64(u64::from(min), u64::from(max_inclusive));
        u32::try_from(sampled).expect("sampled value fits in u32")
    }

    /// Uniformly sample an `i128` in `[min, max_inclusive]` where both bounds
    /// are non-negative and fit in `u64`.
    ///
    /// # Panics
    ///
    /// Panics if `min < 0`, `min > max_inclusive`, or bounds exceed `u64::MAX`.
    pub fn range_i128(&mut self, min: i128, max_inclusive: i128) -> i128 {
        assert!(
            min >= 0 && min <= max_inclusive,
            "invalid range_i128 bounds"
        );
        let min_u64 = u64::try_from(min).expect("min fits in u64");
        let max_u64 = u64::try_from(max_inclusive).expect("max fits in u64");
        i128::from(self.range_u64(min_u64, max_u64))
    }

    /// Sample a pseudo-random boolean.
    pub const fn choose_bool(&mut self) -> bool {
        (self.next_u64() & 1) == 1
    }
}

/// Independent off-chain balance mirror and conservation verifier for a
/// single SEP-41 token across multiple accounts and contracts.
///
/// Tracks expected balances independently of the on-chain token contract so
/// composition tests can verify both per-participant balance accuracy and
/// global value conservation after every settlement or rollback.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BalanceMirror {
    token: Address,
    entries: Vec<(Address, i128)>,
    total_supply: i128,
}

impl BalanceMirror {
    /// Snapshot the current on-chain SEP-41 balances of `participants` for
    /// `token` as the initial state of the mirror.
    ///
    /// Duplicate addresses in `participants` are deduplicated so each
    /// participant is counted exactly once.
    #[must_use]
    pub fn new(env: &Env, token: &Address, participants: &[Address]) -> Self {
        let client = token::Client::new(env, token);
        let mut entries = Vec::with_capacity(participants.len());
        let mut total_supply = 0_i128;

        for participant in participants {
            if entries.iter().any(|(addr, _)| addr == participant) {
                continue;
            }
            let bal = client.balance(participant);
            total_supply = total_supply
                .checked_add(bal)
                .expect("initial total supply overflow");
            entries.push((participant.clone(), bal));
        }

        Self {
            token: token.clone(),
            entries,
            total_supply,
        }
    }

    /// Register an additional participant by reading its current on-chain
    /// balance and folding it into the tracked total. No-op if already tracked.
    ///
    /// # Panics
    ///
    /// Panics if adding the participant's balance overflows `i128`.
    pub fn add_participant(&mut self, env: &Env, participant: &Address) {
        if self.entries.iter().any(|(addr, _)| addr == participant) {
            return;
        }
        let bal = token::Client::new(env, &self.token).balance(participant);
        self.total_supply = self
            .total_supply
            .checked_add(bal)
            .expect("total supply overflow");
        self.entries.push((participant.clone(), bal));
    }

    /// Record an expected mint of `amount` to `to` in the off-chain mirror.
    ///
    /// # Panics
    ///
    /// Panics if `amount < 0`, `to` is not tracked, or balances overflow.
    pub fn record_mint(&mut self, to: &Address, amount: i128) {
        assert!(amount >= 0, "mint amount must be non-negative");
        let slot = self
            .entries
            .iter_mut()
            .find(|(addr, _)| addr == to)
            .expect("mint recipient must be tracked in BalanceMirror");
        slot.1 = slot.1.checked_add(amount).expect("mirror balance overflow");
        self.total_supply = self
            .total_supply
            .checked_add(amount)
            .expect("mirror total supply overflow");
    }

    /// Record an expected value-conserving transfer of `amount` from `from` to
    /// `to` in the off-chain mirror.
    ///
    /// # Panics
    ///
    /// Panics if `amount < 0`, either party is untracked, or `from` has
    /// insufficient expected balance.
    pub fn record_transfer(&mut self, from: &Address, to: &Address, amount: i128) {
        assert!(amount >= 0, "transfer amount must be non-negative");
        if from == to || amount == 0 {
            return;
        }
        let from_slot = self
            .entries
            .iter_mut()
            .find(|(addr, _)| addr == from)
            .expect("transfer sender must be tracked in BalanceMirror");
        assert!(
            from_slot.1 >= amount,
            "mirror transfer underflow: expected >= {amount}, had {}",
            from_slot.1
        );
        from_slot.1 -= amount;

        let to_slot = self
            .entries
            .iter_mut()
            .find(|(addr, _)| addr == to)
            .expect("transfer recipient must be tracked in BalanceMirror");
        to_slot.1 = to_slot
            .1
            .checked_add(amount)
            .expect("mirror recipient balance overflow");
    }

    /// Return the expected balance recorded for `account`.
    ///
    /// # Panics
    ///
    /// Panics if `account` is not tracked by this mirror.
    #[must_use]
    pub fn expected_balance(&self, account: &Address) -> i128 {
        self.entries
            .iter()
            .find(|(addr, _)| addr == account)
            .map(|(_, bal)| *bal)
            .expect("account must be tracked in BalanceMirror")
    }

    /// Return the invariant total supply tracked across all participants.
    #[must_use]
    pub const fn total_supply(&self) -> i128 {
        self.total_supply
    }

    /// Take a clone of the current expected balances for rollback verification.
    #[must_use]
    pub fn snapshot(&self) -> Vec<(Address, i128)> {
        self.entries.clone()
    }

    /// Assert that every tracked participant's on-chain SEP-41 balance matches
    /// the mirror's expected balance and that the sum of on-chain balances
    /// equals the invariant total supply.
    ///
    /// # Panics
    ///
    /// Panics if any participant's on-chain balance diverges from the mirror or
    /// if the total supply is not conserved.
    pub fn assert_conserved(&self, env: &Env) {
        let client = token::Client::new(env, &self.token);
        let mut actual_sum = 0_i128;
        let mut expected_sum = 0_i128;

        for (addr, expected) in &self.entries {
            let actual = client.balance(addr);
            assert_eq!(
                actual, *expected,
                "balance mismatch for participant {addr:?}: on-chain={actual}, mirror={expected}"
            );
            actual_sum = actual_sum
                .checked_add(actual)
                .expect("on-chain sum overflow");
            expected_sum = expected_sum
                .checked_add(*expected)
                .expect("mirror sum overflow");
        }

        assert_eq!(
            expected_sum, self.total_supply,
            "mirror internal sum ({expected_sum}) diverged from total_supply ({})",
            self.total_supply
        );
        assert_eq!(
            actual_sum, self.total_supply,
            "on-chain total balance ({actual_sum}) diverged from conserved supply ({})",
            self.total_supply
        );
    }

    /// Assert that on-chain balances match `snapshot` (taken before a reverted
    /// or rejected call) and that the mirror remains conserved.
    ///
    /// # Panics
    ///
    /// Panics if any on-chain balance changed relative to `snapshot` or the
    /// mirror conservation check fails.
    pub fn assert_unchanged(&self, env: &Env, snapshot: &[(Address, i128)]) {
        let client = token::Client::new(env, &self.token);
        for (addr, before_bal) in snapshot {
            let actual = client.balance(addr);
            assert_eq!(
                actual, *before_bal,
                "rollback violation for participant {addr:?}: before={before_bal}, after={actual}"
            );
        }
        self.assert_conserved(env);
    }
}

/// Count how many events emitted by `contract` in the most recent invocation
/// have `topic_symbol` as their first topic symbol.
///
/// # Panics
///
/// Panics if `topic_symbol` cannot be converted into an XDR `ScSymbol`.
#[must_use]
pub fn contract_event_count(env: &Env, contract: &Address, topic_symbol: &str) -> usize {
    let expected_topic = ScVal::Symbol(
        topic_symbol
            .try_into()
            .expect("valid XDR symbol for event topic"),
    );
    let filtered = env.events().all().filter_by_contract(contract);
    filtered
        .events()
        .iter()
        .filter(|event| {
            matches!(
                &event.body,
                ContractEventBody::V0(body) if body.topics.first() == Some(&expected_topic)
            )
        })
        .count()
}

/// Assert that `contract` emitted at least one event with `topic_symbol` as
/// its first topic in the most recent invocation.
///
/// # Panics
///
/// Panics if no matching event was emitted by `contract`.
pub fn assert_contract_event(env: &Env, contract: &Address, topic_symbol: &str) {
    let count = contract_event_count(env, contract, topic_symbol);
    assert!(
        count > 0,
        "expected contract {contract:?} to emit event '{topic_symbol}', but count was 0"
    );
}

/// Assert that `contract` emitted zero events in the most recent invocation
/// (e.g., when a call reverted or was rejected before reaching `contract`).
///
/// # Panics
///
/// Panics if `contract` emitted any events in the most recent invocation.
pub fn assert_no_contract_events(env: &Env, contract: &Address) {
    let filtered = env.events().all().filter_by_contract(contract);
    assert!(
        filtered.events().is_empty(),
        "expected zero events from {contract:?}, found {}",
        filtered.events().len()
    );
}

/// Assert that `token` emitted a SEP-41 `transfer(from, to, amount)` event in
/// the most recent invocation.
///
/// # Panics
///
/// Panics if no matching `transfer` event was emitted by `token`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn assert_sac_transfer_event(
    env: &Env,
    token: &Address,
    from: &Address,
    to: &Address,
    amount: i128,
) {
    let transfer_topic = ScVal::Symbol("transfer".try_into().expect("valid transfer symbol"));
    let from_val = ScVal::from(from);
    let to_val = ScVal::from(to);
    let expected_data = ScVal::I128(Int128Parts {
        hi: (amount >> 64) as i64,
        lo: amount as u64,
    });

    let filtered = env.events().all().filter_by_contract(token);
    let found = filtered.events().iter().any(|event| {
        let ContractEventBody::V0(body) = &event.body;
        body.topics.first() == Some(&transfer_topic)
            && body.topics.get(1) == Some(&from_val)
            && body.topics.get(2) == Some(&to_val)
            && body.data == expected_data
    });
    assert!(
        found,
        "expected SAC transfer event on {token:?} from {from:?} to {to:?} for {amount}"
    );
}

fn invocation_contains(
    invocation: &AuthorizedInvocation,
    contract: &Address,
    target_fn: &Symbol,
) -> bool {
    let matches_self = matches!(
        &invocation.function,
        AuthorizedFunction::Contract((addr, fn_sym, _))
            if addr == contract && fn_sym == target_fn
    );
    if matches_self {
        return true;
    }
    invocation
        .sub_invocations
        .iter()
        .any(|sub| invocation_contains(sub, contract, target_fn))
}

/// Return `true` if the most recent invocation recorded an authorization by
/// `authorizer` covering `(contract, fn_name)` (either as the root authorized
/// function or as a nested sub-invocation).
#[must_use]
pub fn has_auth_invocation(
    env: &Env,
    authorizer: &Address,
    contract: &Address,
    fn_name: &str,
) -> bool {
    let target_fn = Symbol::new(env, fn_name);
    env.auths()
        .iter()
        .any(|(addr, inv)| addr == authorizer && invocation_contains(inv, contract, &target_fn))
}

/// Assert that the most recent invocation recorded an authorization by
/// `authorizer` covering `(contract, fn_name)`.
///
/// # Panics
///
/// Panics if no matching authorization was recorded in `env.auths()`.
pub fn assert_auth_invocation(env: &Env, authorizer: &Address, contract: &Address, fn_name: &str) {
    assert!(
        has_auth_invocation(env, authorizer, contract, fn_name),
        "expected {authorizer:?} to authorize {contract:?}::{fn_name}"
    );
}

#[cfg(test)]
mod tests {
    use super::{
        assert_auth_invocation, assert_sac_transfer_event, BalanceMirror, SeededRng,
        DEFAULT_COMPOSITION_SEEDS,
    };
    use crate::{new_env, TestAccounts, TokenFixture};

    #[test]
    fn seeded_rng_is_deterministic_across_runs() {
        for seed in DEFAULT_COMPOSITION_SEEDS {
            let mut rng_a = SeededRng::new(seed);
            let mut rng_b = SeededRng::new(seed);
            for _ in 0..16 {
                assert_eq!(rng_a.next_u64(), rng_b.next_u64());
                assert_eq!(rng_a.range_i128(10, 10_000), rng_b.range_i128(10, 10_000));
                assert_eq!(rng_a.range_u32(1, 50), rng_b.range_u32(1, 50));
                assert_eq!(rng_a.choose_bool(), rng_b.choose_bool());
            }
        }
    }

    #[test]
    fn balance_mirror_tracks_mints_transfers_and_snapshots() {
        let env = new_env();
        let accounts = TestAccounts::generate(&env);
        let token = TokenFixture::new(&env, &accounts.deployer);

        let participants = [
            accounts.user1.clone(),
            accounts.user2.clone(),
            accounts.user3.clone(),
        ];
        let mut mirror = BalanceMirror::new(&env, &token.address, &participants);
        mirror.assert_conserved(&env);
        assert_eq!(mirror.total_supply(), 0);

        token.mint(&env, &accounts.user1, 5_000);
        mirror.record_mint(&accounts.user1, 5_000);
        mirror.assert_conserved(&env);
        assert_eq!(mirror.total_supply(), 5_000);

        let snap = mirror.snapshot();
        mirror.assert_unchanged(&env, &snap);

        token.transfer(&env, &accounts.user1, &accounts.user2, 1_250);
        assert_sac_transfer_event(
            &env,
            &token.address,
            &accounts.user1,
            &accounts.user2,
            1_250,
        );
        assert_auth_invocation(&env, &accounts.user1, &token.address, "transfer");

        mirror.record_transfer(&accounts.user1, &accounts.user2, 1_250);
        mirror.assert_conserved(&env);
        assert_eq!(mirror.expected_balance(&accounts.user1), 3_750);
        assert_eq!(mirror.expected_balance(&accounts.user2), 1_250);
        assert_eq!(mirror.total_supply(), 5_000);
    }
}
