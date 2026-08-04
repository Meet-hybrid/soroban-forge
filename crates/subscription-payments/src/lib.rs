#![no_std]

//! # Soroban Forge — Subscription Payments contract
//!
//! Recurring, on-chain subscription billing: a subscriber authorises a
//! provider to pull a fixed amount per period from a token balance. The public
//! interface is declared by [`SorobanForgeSubscriptionPayments`]; [`SubscriptionPayments`]
//! is the deployable contract. Implementation arrives in a later commit.

use soroban_sdk::{contract, contractclient, contractimpl, contracttype, Address, Env};

/// Public interface for the Soroban Forge subscription payments contract.
#[contractclient(name = "SorobanForgeSubscriptionPaymentsClient")]
pub trait SorobanForgeSubscriptionPayments {
    /// Subscribe `subscriber` to `provider`'s service at `amount` per `period`.
    fn subscribe(
        env: Env,
        subscriber: Address,
        provider: Address,
        token: Address,
        amount: i128,
        period: u64,
    ) -> Result<u64, soroban_forge_shared_utils::ForgeError>;

    /// Charge the next due payment for `subscription_id`.
    fn charge(
        env: Env,
        subscription_id: u64,
    ) -> Result<i128, soroban_forge_shared_utils::ForgeError>;

    /// Cancel `subscription_id`, preventing further charges.
    fn cancel(env: Env, subscription_id: u64)
        -> Result<(), soroban_forge_shared_utils::ForgeError>;
}

/// Lifecycle state of a subscription.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubscriptionStatus {
    /// Active and chargeable.
    Active,
    /// Cancelled; no further charges.
    Cancelled,
    /// Payment failed and the subscription is in arrears.
    PastDue,
}

/// A recurring payment agreement.
#[contracttype]
#[derive(Clone, Debug)]
pub struct Subscription {
    /// Stable identifier assigned at creation.
    pub subscription_id: u64,
    /// Account being charged.
    pub subscriber: Address,
    /// Account receiving payments.
    pub provider: Address,
    /// Token contract used for settlement.
    pub token: Address,
    /// Amount charged per period.
    pub amount: i128,
    /// Length of one billing period, in seconds.
    pub period: u64,
    /// Ledger timestamp of the last successful charge.
    pub last_charged: u64,
    /// Current state.
    pub status: SubscriptionStatus,
}

/// The deployable subscription payments contract.
///
/// The `#[contractimpl]` block is intentionally empty at this stage; the
/// subscribe/charge/cancel logic is added in a subsequent commit.
#[contract]
pub struct SubscriptionPayments;

#[contractimpl]
impl SubscriptionPayments {}
