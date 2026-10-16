#![no_std]

//! Shared testing utilities for Soroban Forge contracts.
//!
//! Provides lightweight helpers used by contract integration and composition
//! tests:
//! - [`new_env`] to construct a configured [`Env`]
//! - [`TestAccounts`] to obtain deterministic, distinct mock addresses
//! - [`TokenFixture`] to deploy and interact with SEP-41 Stellar Asset Contracts
//! - [`composition`] for independent balance-conservation mirroring
//!   ([`BalanceMirror`]), deterministic seed scheduling ([`SeededRng`]), and
//!   event/authorization assertions
//! - [`ttl`] for ledger-sequence manipulation, entry-liveness introspection,
//!   and the deterministic TTL chaos harness ([`chaos_drive`])
//!
//! This crate is intended to be used as a `dev-dependency` by contract crates.

use soroban_sdk::Env;

pub mod composition;
pub mod mocks;
pub mod ttl;

pub use composition::{
    assert_auth_invocation, assert_contract_event, assert_no_contract_events,
    assert_sac_transfer_event, contract_event_count, has_auth_invocation, BalanceMirror, SeededRng,
    DEFAULT_COMPOSITION_SEEDS,
};
pub use mocks::{
    MockTarget, MockTargetClient, RevertingTarget, RevertingTargetClient, TestAccounts,
    TokenFixture,
};
pub use ttl::{
    advance_ledger, assert_no_violations, chaos_drive, entry_is_live, entry_ttl, ChaosStep,
    ChaosTarget, ChaosTrace, StepOutcome, BUMP_AMOUNT, BUMP_THRESHOLD, DAY_IN_LEDGERS,
};

/// Create a [`Env`] configured for contract testing.
///
/// Enables [`Env::mock_all_auths`], which lets tests invoke authorised
/// contract functions without manually signing every call. Tests that need to
/// assert authorisation failures should call [`Env::set_auths`] / disable
/// mocking explicitly.
#[must_use]
pub fn new_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env
}
