#![no_std]

use soroban_sdk::{contract, contractimpl, Address, Env, Symbol, Timestamp, Vec};

mod contract;
mod errors;
mod types;

pub use contract::SubscriptionPayments;
pub use errors::*;
pub use types::*;

#[contract]
pub struct SubscriptionPayments;

#[contractimpl]
impl SubscriptionPayments for SubscriptionPayments {
    fn create_plan(
        env: Env,
        creator: Address,
        name: Symbol,
        amount: i128,
        interval: Symbol,
        interval_count: u32,
    ) -> Result<u64, ForgeError> {
        todo!()
    }

    fn subscribe(env: Env, plan_id: u64, subscriber: Address) -> Result<u64, ForgeError> {
        todo!()
    }

    fn charge(env: Env, subscription_id: u64) -> Result<i128, ForgeError> {
        todo!()
    }

    fn cancel(env: Env, subscription_id: u64) -> Result<(), ForgeError> {
        todo!()
    }

    fn pause(env: Env, plan_id: u64) -> Result<(), ForgeError> {
        todo!()
    }

    fn resume(env: Env, plan_id: u64) -> Result<(), ForgeError> {
        todo!()
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanStatus {
    Active,
    Paused,
    Deprecated,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubscriptionStatus {
    Active,
    Paused,
    Canceled,
    Expired,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SubscriptionPlan {
    pub id: u64,
    pub creator: Address,
    pub name: Symbol,
    pub amount: i128,
    pub interval: Symbol,
    pub interval_count: u32,
    pub status: PlanStatus,
    pub created_at: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Subscription {
    pub id: u64,
    pub plan_id: u64,
    pub subscriber: Address,
    pub next_charge_at: Timestamp,
    pub status: SubscriptionStatus,
    pub created_at: u64,
}
