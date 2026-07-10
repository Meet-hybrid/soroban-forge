#![no_std]

use soroban_sdk::{contract, contractimpl, Address, Env, Symbol, Vec};

mod contract;
mod errors;
mod types;

pub use contract::MarketplaceRoyalties;
pub use errors::*;
pub use types::*;

#[contract]
pub struct MarketplaceRoyalties;

#[contractimpl]
impl MarketplaceRoyalties for MarketplaceRoyalties {
    fn register_asset(
        env: Env,
        creator: Address,
        asset_id: Symbol,
        url: Symbol,
    ) -> Result<(), ForgeError> {
        todo!()
    }

    fn create_sale(
        env: Env,
        asset_id: Symbol,
        seller: Address,
        price: i128,
    ) -> Result<(), ForgeError> {
        todo!()
    }

    fn purchase(env: Env, asset_id: Symbol, buyer: Address, price: i128) -> Result<(), ForgeError> {
        todo!()
    }

    fn update_royalties(
        env: Env,
        asset_id: Symbol,
        recipients: Vec<(Address, i128)>,
    ) -> Result<(), ForgeError> {
        todo!()
    }

    fn get_royalty_info(env: Env, asset_id: Symbol) -> Result<Vec<(Address, i128)>, ForgeError> {
        todo!()
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SaleStatus {
    Active,
    Sold,
    Cancelled,
    Expired,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Sale {
    pub asset_id: Symbol,
    pub seller: Address,
    pub price: i128,
    pub status: SaleStatus,
    pub created_at: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct AssetRoyalties {
    pub asset_id: Symbol,
    pub creator: Address,
    pub url: Symbol,
    pub recipients: Vec<(Address, i128)>,
    pub created_at: u64,
}
