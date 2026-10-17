#![no_std]

//! Shared testing utilities for Soroban Forge contracts.
//!
//! Provides lightweight helpers used by contract integration tests:
//! - [`new_env`] to construct a configured [`Env`]
//! - [`TestAccounts`] to obtain deterministic, distinct mock addresses
//! - [`LedgerProgressionFixture`] for deterministic monotonic ledger sequence,
//!   timestamp, and TTL boundary progression
//! - [`ttl`] for ledger-sequence manipulation, entry-liveness introspection,
//!   and the deterministic TTL chaos harness ([`chaos_drive`])
//!
//! This crate is intended to be used as a `dev-dependency` by contract crates.

use soroban_sdk::Env;

pub mod mocks;
pub mod ttl;

pub use mocks::{
    MockTarget, MockTargetClient, RevertingTarget, RevertingTargetClient, TestAccounts,
};
pub use ttl::{
    advance_ledger, advance_ledger_and_timestamp, advance_timestamp, assert_no_violations,
    chaos_drive, entry_is_live, entry_ttl, set_ledger_monotonic, ChaosStep, ChaosTarget,
    ChaosTrace, LedgerProgressionFixture, LedgerSnapshot, ProgressionError, StepOutcome,
    BUMP_AMOUNT, BUMP_THRESHOLD, DAY_IN_LEDGERS, DEFAULT_INITIAL_SEQUENCE,
    DEFAULT_INITIAL_TIMESTAMP, DEFAULT_SECONDS_PER_LEDGER,
};

/// Create a [`Env`] configured for contract testing.
///
/// Enables [`Env::mock_all_auths`], which lets tests invoke authorised
/// contract functions without manually signing every call. Tests that need to
/// assert authorisation failures should call [`Env::set_auths`] / disable
/// mocking explicitly.
pub fn new_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env
}
