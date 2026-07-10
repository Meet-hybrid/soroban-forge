#![no_std]

use soroban_sdk::{contract, contractimpl, Address, Env, Symbol, Timestamp, Vec};

mod contract;
mod errors;
mod types;

pub use contract::VestingContract;
pub use errors::*;
pub use types::*;

#[contract]
pub struct Vesting;

#[contractimpl]
impl VestingContract for Vesting {
    fn create_schedule(
        env: Env,
        beneficiary: Address,
        start: Timestamp,
        cliff: Timestamp,
        end: Timestamp,
        total_amount: i128,
    ) -> Result<u64, ForgeError> {
        todo!()
    }

    fn release(env: Env, schedule_id: u64) -> Result<i128, ForgeError> {
        todo!()
    }

    fn revoke(env: Env, schedule_id: u64) -> Result<i128, ForgeError> {
        todo!()
    }

    fn get_schedule(env: Env, schedule_id: u64) -> Result<VestingSchedule, ForgeError> {
        todo!()
    }

    fn get_releasable_amount(env: Env, schedule_id: u64) -> Result<i128, ForgeError> {
        todo!()
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VestingStatus {
    Active,
    Revoked,
    Completed,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct VestingSchedule {
    pub id: u64,
    pub beneficiary: Address,
    pub start: Timestamp,
    pub cliff: Timestamp,
    pub end: Timestamp,
    pub total_amount: i128,
    pub released_amount: i128,
    pub status: VestingStatus,
    pub created_at: u64,
}
