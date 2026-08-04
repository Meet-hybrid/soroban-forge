#![no_std]

//! # Soroban Forge — DAO Governance contract
//!
//! A minimal on-chain governance primitive: members create proposals, vote
//! with governance tokens, and approved proposals become executable. The
//! public interface is declared by [`SorobanForgeDaoGovernance`]; [`DaoGovernance`]
//! is the deployable contract. Implementation arrives in a later commit.

use soroban_sdk::{contract, contractclient, contractimpl, contracttype, Address, Env};

/// Public interface for the Soroban Forge DAO governance contract.
#[contractclient(name = "SorobanForgeDaoGovernanceClient")]
pub trait SorobanForgeDaoGovernance {
    /// Create a new proposal with an encoded action payload.
    fn propose(
        env: Env,
        proposer: Address,
        action: soroban_sdk::Val,
    ) -> Result<u64, soroban_forge_shared_utils::ForgeError>;

    /// Cast `voter`'s vote (for/against) on `proposal_id`.
    fn vote(
        env: Env,
        proposal_id: u64,
        voter: Address,
        support: bool,
    ) -> Result<(), soroban_forge_shared_utils::ForgeError>;

    /// Finalise a proposal once voting has ended.
    fn execute(env: Env, proposal_id: u64) -> Result<(), soroban_forge_shared_utils::ForgeError>;
}

/// Lifecycle state of a governance proposal.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalState {
    /// Open for voting.
    Active,
    /// Approved and executed.
    Succeeded,
    /// Rejected or expired.
    Defeated,
    /// Queued for delayed execution (optional timelock).
    Queued,
}

/// A single governance proposal.
#[contracttype]
#[derive(Clone, Debug)]
pub struct Proposal {
    /// Stable identifier assigned at creation.
    pub proposal_id: u64,
    /// Address that created the proposal.
    pub proposer: Address,
    /// Encoded action to execute on success.
    pub action: soroban_sdk::Val,
    /// Tally of "for" votes (in governance-token units).
    pub for_votes: i128,
    /// Tally of "against" votes (in governance-token units).
    pub against_votes: i128,
    /// Ledger timestamp at which voting closes.
    pub voting_ends: u64,
    /// Current state.
    pub state: ProposalState,
}

/// The deployable DAO governance contract.
///
/// The `#[contractimpl]` block is intentionally empty at this stage; the
/// proposal/voting/execution logic is added in a subsequent commit.
#[contract]
pub struct DaoGovernance;

#[contractimpl]
impl DaoGovernance {}
