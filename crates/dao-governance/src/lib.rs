#![no_std]

use soroban_sdk::{contract, contractimpl, Address, Env, Symbol, Timestamp, Vec};

mod contract;
mod errors;
mod types;

pub use contract::DaoGovernance;
pub use errors::*;
pub use types::*;

#[contract]
pub struct DaoGovernance;

#[contractimpl]
impl DaoGovernance for DaoGovernance {
    fn create_proposal(
        env: Env,
        proposer: Address,
        title: Symbol,
        description: Symbol,
        start_time: Timestamp,
        end_time: Timestamp,
    ) -> Result<u64, ForgeError> {
        todo!()
    }

    fn vote(
        env: Env,
        proposal_id: u64,
        voter: Address,
        support: bool,
        weight: i128,
    ) -> Result<(), ForgeError> {
        todo!()
    }

    fn execute(env: Env, proposal_id: u64) -> Result<bool, ForgeError> {
        todo!()
    }

    fn cancel(env: Env, proposal_id: u64) -> Result<bool, ForgeError> {
        todo!()
    }

    fn get_proposal(env: Env, proposal_id: u64) -> Result<Proposal, ForgeError> {
        todo!()
    }

    fn get_vote(env: Env, proposal_id: u64, voter: Address) -> Result<Vote, ForgeError> {
        todo!()
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalStatus {
    Active,
    Succeeded,
    Defeated,
    Executed,
    Cancelled,
    Expired,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Proposal {
    pub id: u64,
    pub proposer: Address,
    pub title: Symbol,
    pub description: Symbol,
    pub start_time: Timestamp,
    pub end_time: Timestamp,
    pub for_votes: i128,
    pub against_votes: i128,
    pub quorum: i128,
    pub status: ProposalStatus,
    pub created_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Vote {
    pub proposal_id: u64,
    pub voter: Address,
    pub support: bool,
    pub weight: i128,
    pub voted_at: u64,
}
