#![no_std]

//! # Soroban Forge — Vesting contract
//!
//! A token-vesting contract that releases a beneficiary's tokens linearly
//! over time, optionally behind a cliff. The public interface is declared by
//! [`SorobanForgeVesting`]; the concrete [`Vesting`] struct is the deployable
//! contract. The implementation is filled in by later commits — this crate
//! currently provides the interface surface and storage types.

use soroban_sdk::{contract, contractclient, contractimpl, contracttype, Address, Env};

/// Public interface for the Soroban Forge vesting contract.
///
/// Declared as a `contractclient` trait so SDK consumers (and the TypeScript
/// SDK generator) get a strongly-typed client without coupling to the
/// implementation crate.
#[contractclient(name = "SorobanForgeVestingClient")]
pub trait SorobanForgeVesting {
    /// Create a new vesting schedule for `beneficiary`.
    fn create_schedule(
        env: Env,
        beneficiary: Address,
        token: Address,
        total_amount: i128,
        cliff: u64,
        duration: u64,
    ) -> Result<u64, soroban_forge_shared_utils::ForgeError>;

    /// Claim tokens that have vested as of the current ledger time.
    fn claim(env: Env, schedule_id: u64) -> Result<i128, soroban_forge_shared_utils::ForgeError>;

    /// Return the amount currently claimable by `schedule_id`.
    fn claimable(
        env: Env,
        schedule_id: u64,
    ) -> Result<i128, soroban_forge_shared_utils::ForgeError>;
}

/// Lifecycle state of a vesting schedule.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VestingStatus {
    /// Schedule created, before the cliff has been reached.
    Locked,
    /// Past the cliff; tokens are vesting linearly.
    Vesting,
    /// Fully vested and claimed.
    Completed,
    /// Schedule was terminated before completion.
    Revoked,
}

/// A single token-vesting schedule.
#[contracttype]
#[derive(Clone, Debug)]
pub struct VestingSchedule {
    /// Recipient of the vested tokens.
    pub beneficiary: Address,
    /// Token contract whose balance is drawn down.
    pub token: Address,
    /// Total amount to vest over `duration` after `cliff`.
    pub total_amount: i128,
    /// Ledger timestamp at which vesting begins.
    pub start: u64,
    /// Ledger timestamp at which claims become possible.
    pub cliff: u64,
    /// Seconds over which the remaining amount vests linearly.
    pub duration: u64,
    /// Amount already claimed by the beneficiary.
    pub claimed: i128,
    /// Current lifecycle state.
    pub status: VestingStatus,
}

/// The deployable vesting contract.
///
/// The `#[contractimpl]` block is intentionally empty at this stage; the
/// schedule-creation and claim logic is added in a subsequent commit.
#[contract]
pub struct Vesting;

#[contractimpl]
impl Vesting {}
