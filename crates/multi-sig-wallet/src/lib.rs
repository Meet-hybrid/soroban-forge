#![no_std]

use soroban_sdk::{contract, contractimpl, Address, Env, Symbol, Vec};

mod contract;
mod errors;
mod types;

pub use contract::MultiSigWallet;
pub use errors::*;
pub use types::*;

#[contract]
pub struct MultiSigWallet;

#[contractimpl]
impl MultiSigWallet for MultiSigWallet {
    fn submit_transaction(
        env: Env,
        proposer: Address,
        destination: Address,
        function_name: Symbol,
        args: Vec<Symbol>,
    ) -> Result<u64, ForgeError> {
        todo!()
    }

    fn approve(env: Env, tx_id: u64, approver: Address) -> Result<(), ForgeError> {
        todo!()
    }

    fn revoke(env: Env, tx_id: u64, approver: Address) -> Result<(), ForgeError> {
        todo!()
    }

    fn execute(env: Env, tx_id: u64) -> Result<(), ForgeError> {
        todo!()
    }

    fn get_transaction(env: Env, tx_id: u64) -> Result<Transaction, ForgeError> {
        todo!()
    }

    fn add_owner(env: Env, owner: Address) -> Result<(), ForgeError> {
        todo!()
    }

    fn remove_owner(env: Env, owner: Address) -> Result<(), ForgeError> {
        todo!()
    }

    fn update_threshold(env: Env, new_threshold: u32) -> Result<(), ForgeError> {
        todo!()
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionStatus {
    Pending,
    Approved,
    Executed,
    Rejected,
    Expired,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Transaction {
    pub id: u64,
    pub proposer: Address,
    pub destination: Address,
    pub function_name: Symbol,
    pub args: Vec<Symbol>,
    pub approvals_needed: u32,
    pub approvals: Vec<Address>,
    pub status: TransactionStatus,
    pub created_at: u64,
    pub executed_at: Option<u64>,
}
