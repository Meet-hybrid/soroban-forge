#![no_std]

//! Shared testing utilities for Soroban Forge contracts.
//!
//! Provides lightweight helpers used by contract integration tests:
//! - [`new_env`] to construct a configured [`Env`]
//! - [`TestAccounts`] to obtain deterministic, distinct mock addresses
//!
//! This crate is intended to be used as a `dev-dependency` by contract crates.

extern crate std;

use soroban_sdk::Env;

pub mod mocks;

pub use mocks::{
    MockTarget, MockTargetClient, RevertingTarget, RevertingTargetClient, TestAccounts,
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

/// Emit one machine-readable resource record for the most recent invocation.
///
/// Entry counts are the SDK host's invocation-resource estimates; they are
/// intended for regression comparison, not as an RPC transaction footprint.
pub fn emit_resource_report(env: &Env, scenario: &str) {
    let estimate = env.cost_estimate();
    let resources = estimate.resources();
    let read_entries = resources.disk_read_entries + resources.memory_read_entries;
    let footprint_entries = read_entries + resources.write_entries;
    let budget = estimate.budget();
    let instructions = budget.cpu_instruction_cost();
    let memory_bytes = budget.memory_bytes_cost();
    let transaction_fee_stroops = estimate.fee().total;

    std::println!(
        "CONTRACT_RESOURCE_REPORT:{{\"scenario\":\"{}\",\"metrics\":{{\"footprint_entries\":{},\"read_entries\":{},\"write_entries\":{},\"instructions\":{},\"memory_bytes\":{},\"transaction_fee_stroops\":{}}}}}",
        scenario,
        footprint_entries,
        read_entries,
        resources.write_entries,
        instructions,
        memory_bytes,
        transaction_fee_stroops,
    );
}
