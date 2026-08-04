#![no_std]

//! # Soroban Forge — Marketplace Royalties contract
//!
//! Enforces creator royalty splits on secondary sales: when an NFT changes
//! hands, the sale proceeds are split between the seller and one or more
//! royalty recipients according to configured percentages. The public
//! interface is declared by [`SorobanForgeMarketplaceRoyalties`]; [`MarketplaceRoyalties`]
//! is the deployable contract. Implementation arrives in a later commit.

use soroban_sdk::{contract, contractclient, contractimpl, contracttype, Address, Env};

/// Public interface for the Soroban Forge marketplace royalties contract.
#[contractclient(name = "SorobanForgeMarketplaceRoyaltiesClient")]
pub trait SorobanForgeMarketplaceRoyalties {
    /// Register a royalty recipient and basis-point rate for `collection`.
    fn set_royalty(
        env: Env,
        collection: Address,
        recipient: Address,
        bps: u32,
    ) -> Result<(), soroban_forge_shared_utils::ForgeError>;

    /// Distribute `amount` from a sale of `collection`, returning the net to
    /// the seller after royalties.
    fn distribute(
        env: Env,
        collection: Address,
        seller: Address,
        amount: i128,
    ) -> Result<i128, soroban_forge_shared_utils::ForgeError>;
}

/// Lifecycle state of a registered royalty configuration.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RoyaltyStatus {
    /// Active and applied to sales.
    Active,
    /// Disabled; sales settle to the seller in full.
    Disabled,
}

/// A royalty configuration for a single collection.
#[contracttype]
#[derive(Clone, Debug)]
pub struct Royalty {
    /// Collection (NFT contract) this configuration applies to.
    pub collection: Address,
    /// Address entitled to royalty payments.
    pub recipient: Address,
    /// Royalty rate in basis points (100 bps = 1%).
    pub bps: u32,
    /// Whether the configuration is currently enforced.
    pub status: RoyaltyStatus,
}

/// The deployable marketplace royalties contract.
///
/// The `#[contractimpl]` block is intentionally empty at this stage; the
/// registration/distribution logic is added in a subsequent commit.
#[contract]
pub struct MarketplaceRoyalties;

#[contractimpl]
impl MarketplaceRoyalties {}
