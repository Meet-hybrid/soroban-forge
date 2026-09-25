#![no_std]

//! # Soroban Forge — DAO Governance contract
//!
//! A minimal on-chain governance primitive: members create proposals, cast one
//! vote each (`for`/`against`, tallied in governance-token units), and a
//! proposal is finalised once voting ends — passing when it has a strict
//! majority of `for` votes.
//!
//! Lifecycle:
//!
//! ```text
//! propose (voting_ends = now + duration)
//!   --> Active --vote × n--> voting ends
//!   --> execute: for > against ? Succeeded : Defeated
//!   --> execute (on Succeeded): target.execute(action) --> Executed (terminal)
//!   --> cancel (proposer only): Cancelled (terminal)
//! ```
//!
//! When voting ends, calling `execute` finalises the vote tally:
//! - If `for_votes > against_votes`, the proposal becomes `Succeeded`.
//! - Otherwise, the proposal becomes `Defeated`.
//!
//! Once a proposal is `Succeeded`, calling `execute` performs a real
//! cross-contract call (`target.execute(action)`) using `env.try_invoke_contract`.
//! - If the target invocation succeeds, the proposal transitions to `Executed`.
//! - If the target invocation reverts, `ForgeError::ContractInvocationFailed` is
//!   returned and the proposal remains in the `Succeeded` state (not `Executed`),
//!   leaving target state unchanged and allowing execution to be re-attempted.
//!
//! `Defeated`, `Executed`, `Cancelled`, and still-`Active` proposals cannot be
//! executed; a second `execute` on an `Executed` proposal is rejected.
//!
//! Authorization model:
//! - `propose` requires the proposer.
//! - `vote` requires the voter and is one-vote-per-voter per proposal.
//! - `execute` may be called by anyone (permissionless), but only once voting has ended.
//! - `cancel_proposal` requires the original proposer and freezes a
//!   not-yet-executed proposal.
//! - `get_proposal` is a read-only view.
//!
//! The `Queued` state is reserved for an optional timelock that lands in a
//! follow-up; it is not reachable through the current public interface.
//! Weighted voting by governance-token balance is intentionally out of
//! scope for this iteration: the contract tracks proposals, votes, and timing,
//! not balances.

#[cfg(test)]
extern crate std;

use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::{
    contract, contractclient, contractevent, contractimpl, contracttype, Address, Bytes, Env,
    IntoVal, Symbol, Val,
};

/// Public interface for the Soroban Forge DAO governance contract.
#[contractclient(name = "SorobanForgeDaoGovernanceClient")]
pub trait SorobanForgeDaoGovernance {
    /// Create a new proposal with a target contract and encoded action payload.
    ///
    /// `duration` (seconds) defines how long voting stays open. Returns the
    /// stable proposal id.
    fn propose(
        env: Env,
        proposer: Address,
        target: Address,
        action: Bytes,
        duration: u64,
    ) -> Result<u64, soroban_forge_shared_utils::ForgeError>;

    /// Cast `voter`'s vote (for/against) on `proposal_id`. One vote per voter.
    fn vote(
        env: Env,
        proposal_id: u64,
        voter: Address,
        support: bool,
    ) -> Result<(), soroban_forge_shared_utils::ForgeError>;

    /// Finalise a proposal once voting has ended, and execute passed proposals.
    fn execute(env: Env, proposal_id: u64) -> Result<(), soroban_forge_shared_utils::ForgeError>;

    /// Withdraw a proposal that has not yet been executed.
    ///
    /// Only the original proposer may cancel. Once cancelled the proposal is
    /// frozen against further votes and cannot be executed.
    fn cancel_proposal(
        env: Env,
        proposal_id: u64,
        proposer: Address,
    ) -> Result<(), soroban_forge_shared_utils::ForgeError>;

    /// Read a stored proposal by id (read-only view).
    fn get_proposal(
        env: Env,
        proposal_id: u64,
    ) -> Result<Proposal, soroban_forge_shared_utils::ForgeError>;

    /// Return the total number of proposals created (read-only view).
    fn get_proposal_count(env: Env) -> u64;

    /// Read a paginated slice of proposals ordered by proposal ID (read-only view).
    fn get_proposals(
        env: Env,
        offset: u32,
        limit: u32,
    ) -> Result<soroban_sdk::Vec<Proposal>, soroban_forge_shared_utils::ForgeError>;

    /// Check whether a voter has already cast a vote on a proposal (read-only view).
    fn has_voted(
        env: Env,
        proposal_id: u64,
        voter: Address,
    ) -> Result<bool, soroban_forge_shared_utils::ForgeError>;
}

/// Lifecycle state of a governance proposal.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalState {
    /// Open for voting.
    Active,
    /// Approved and ready for execution.
    Succeeded,
    /// Rejected or expired.
    Defeated,
    /// Successfully executed on-chain (terminal).
    Executed,
    /// Queued for delayed execution (optional timelock).
    Queued,
    /// Withdrawn by the proposer before execution; terminal and immutable.
    Cancelled,
}

/// A single governance proposal.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proposal {
    /// Stable identifier assigned at creation.
    pub proposal_id: u64,
    /// Address that created the proposal.
    pub proposer: Address,
    /// Target contract to invoke on successful execution.
    pub target: Address,
    /// Encoded action to execute on success.
    pub action: Bytes,
    /// Tally of "for" votes (in governance-token units).
    pub for_votes: i128,
    /// Tally of "against" votes (in governance-token units).
    pub against_votes: i128,
    /// Ledger timestamp at which voting closes.
    pub voting_ends: u64,
    /// Current state.
    pub state: ProposalState,
}

/// Instance-storage keys.
#[contracttype]
enum DataKey {
    /// The proposal record for `u64` id.
    Proposal(u64),
    /// Marks that `voter` has already voted on `proposal_id`.
    Vote(u64, Address),
    /// Monotonic proposal id counter.
    Count,
}

/// The deployable DAO governance contract.
#[contract]
pub struct DaoGovernance;

#[contractimpl]
impl DaoGovernance {
    /// Create a new proposal and return its stable id.
    ///
    /// Requires `duration > 0`. The proposer is authorized at creation time.
    pub fn propose(
        env: Env,
        proposer: Address,
        target: Address,
        action: Bytes,
        duration: u64,
    ) -> Result<u64, ForgeError> {
        if duration == 0 {
            return Err(ForgeError::InvalidInput);
        }
        proposer.require_auth();

        let proposal_id = Self::next_id(&env)?;
        let voting_ends = env
            .ledger()
            .timestamp()
            .checked_add(duration)
            .ok_or(ForgeError::ArithmeticOverflow)?;
        let proposal = Proposal {
            proposal_id,
            proposer,
            target,
            action,
            for_votes: 0,
            against_votes: 0,
            voting_ends,
            state: ProposalState::Active,
        };
        env.storage()
            .instance()
            .set(&DataKey::Proposal(proposal_id), &proposal);
        events::proposed(&env, &proposal);
        Ok(proposal_id)
    }

    /// Cast a vote on an active proposal.
    ///
    /// Requires the voter. Each voter may vote exactly once; voting is closed
    /// once the deadline (`voting_ends`) passes.
    pub fn vote(
        env: Env,
        proposal_id: u64,
        voter: Address,
        support: bool,
    ) -> Result<(), ForgeError> {
        let mut proposal = Self::get_proposal_impl(&env, proposal_id)?;
        if proposal.state != ProposalState::Active {
            return Err(ForgeError::InvalidInput);
        }
        if env.ledger().timestamp() >= proposal.voting_ends {
            return Err(ForgeError::DeadlineReached);
        }
        voter.require_auth();

        let vote_key = DataKey::Vote(proposal_id, voter.clone());
        if env.storage().instance().has(&vote_key) {
            return Err(ForgeError::InvalidInput);
        }

        if support {
            proposal.for_votes = proposal
                .for_votes
                .checked_add(1)
                .ok_or(ForgeError::ArithmeticOverflow)?;
        } else {
            proposal.against_votes = proposal
                .against_votes
                .checked_add(1)
                .ok_or(ForgeError::ArithmeticOverflow)?;
        }
        env.storage().instance().set(&vote_key, &true);
        env.storage()
            .instance()
            .set(&DataKey::Proposal(proposal_id), &proposal);
        events::vote_cast(&env, proposal_id, &voter, support);
        Ok(())
    }

    /// Finalise a proposal once voting has ended, and execute passed proposals.
    ///
    /// Callable by anyone after the deadline (permissionless execution).
    ///
    /// - An `Active` proposal past deadline transitions to `Succeeded` on a
    ///   strict majority of `for` votes, or `Defeated` otherwise.
    /// - A `Succeeded` proposal performs a real cross-contract call to `target`
    ///   with `action` (`target.execute(action)`). On successful invocation,
    ///   it transitions to the terminal `Executed` state and emits an `Executed` event.
    /// - If the target invocation reverts, `ForgeError::ContractInvocationFailed`
    ///   is returned and the proposal remains `Succeeded` (not `Executed`),
    ///   leaving target state unchanged.
    /// - `Defeated`, `Executed`, `Cancelled`, and still-`Active` proposals
    ///   cannot be executed.
    pub fn execute(env: Env, proposal_id: u64) -> Result<(), ForgeError> {
        let mut proposal = Self::get_proposal_impl(&env, proposal_id)?;
        if env.ledger().timestamp() < proposal.voting_ends {
            return Err(ForgeError::InvalidInput);
        }

        match proposal.state {
            ProposalState::Active => {
                if proposal.for_votes > proposal.against_votes {
                    proposal.state = ProposalState::Succeeded;
                    env.storage()
                        .instance()
                        .set(&DataKey::Proposal(proposal_id), &proposal);
                    events::finalised(
                        &env,
                        proposal_id,
                        proposal.state.clone(),
                        proposal.for_votes,
                        proposal.against_votes,
                    );
                    Ok(())
                } else {
                    proposal.state = ProposalState::Defeated;
                    env.storage()
                        .instance()
                        .set(&DataKey::Proposal(proposal_id), &proposal);
                    events::finalised(
                        &env,
                        proposal_id,
                        proposal.state.clone(),
                        proposal.for_votes,
                        proposal.against_votes,
                    );
                    Ok(())
                }
            }
            ProposalState::Succeeded => {
                let target = &proposal.target;
                let payload_val: Val = proposal.action.clone().into_val(&env);
                let args = soroban_sdk::vec![&env, payload_val];
                let result = env.try_invoke_contract::<(), ForgeError>(
                    target,
                    &Symbol::new(&env, "execute"),
                    args,
                );

                if let Err(_) | Ok(Err(_)) = result {
                    return Err(ForgeError::ContractInvocationFailed);
                }

                proposal.state = ProposalState::Executed;
                env.storage()
                    .instance()
                    .set(&DataKey::Proposal(proposal_id), &proposal);
                events::finalised(
                    &env,
                    proposal_id,
                    proposal.state.clone(),
                    proposal.for_votes,
                    proposal.against_votes,
                );
                Ok(())
            }
            ProposalState::Defeated
            | ProposalState::Executed
            | ProposalState::Cancelled
            | ProposalState::Queued => Err(ForgeError::InvalidInput),
        }
    }

    /// Withdraw an active proposal before it is executed.
    ///
    /// Requires the original proposer. A proposal may be cancelled even after
    /// voting ends and quorum is met, as long as it has not been executed (or
    /// already cancelled). A cancelled proposal is terminal: further votes and
    /// execution are rejected.
    ///
    /// * [`ForgeError::NotFound`] — no proposal with this id.
    /// * [`ForgeError::Unauthorized`] — `proposer` is not the original proposer.
    /// * [`ForgeError::InvalidInput`] — the proposal is no longer `Active`.
    pub fn cancel_proposal(
        env: Env,
        proposal_id: u64,
        proposer: Address,
    ) -> Result<(), ForgeError> {
        let mut proposal = Self::get_proposal_impl(&env, proposal_id)?;
        if proposal.state != ProposalState::Active {
            return Err(ForgeError::InvalidInput);
        }
        if proposer != proposal.proposer {
            return Err(ForgeError::Unauthorized);
        }
        proposer.require_auth();

        proposal.state = ProposalState::Cancelled;
        env.storage()
            .instance()
            .set(&DataKey::Proposal(proposal_id), &proposal);
        Ok(())
    }

    /// Read a stored proposal by id (read-only view).
    pub fn get_proposal(env: Env, proposal_id: u64) -> Result<Proposal, ForgeError> {
        Self::get_proposal_impl(&env, proposal_id)
    }

    /// Return the total number of proposals created (read-only view).
    pub fn get_proposal_count(env: Env) -> u64 {
        env.storage().instance().get(&DataKey::Count).unwrap_or(0)
    }

    /// Read a paginated slice of proposals ordered by proposal ID (read-only view).
    ///
    /// Bounds clamping:
    /// - `limit == 0` returns `ForgeError::InvalidInput`.
    /// - If `offset >= total`, returns an empty `Vec`.
    /// - Returns at most `limit` items without overflowing.
    pub fn get_proposals(
        env: Env,
        offset: u32,
        limit: u32,
    ) -> Result<soroban_sdk::Vec<Proposal>, ForgeError> {
        if limit == 0 {
            return Err(ForgeError::InvalidInput);
        }

        let total: u64 = env.storage().instance().get(&DataKey::Count).unwrap_or(0);
        let offset_u64 = u64::from(offset);
        if offset_u64 >= total {
            return Ok(soroban_sdk::Vec::new(&env));
        }

        let start = offset_u64
            .checked_add(1)
            .ok_or(ForgeError::ArithmeticOverflow)?;
        let limit_u64 = u64::from(limit);
        let end = total.min(offset_u64.saturating_add(limit_u64));

        let mut proposals = soroban_sdk::Vec::new(&env);
        for id in start..=end {
            let proposal = Self::get_proposal_impl(&env, id)?;
            proposals.push_back(proposal);
        }

        Ok(proposals)
    }

    /// Check whether `voter` has voted on `proposal_id` (read-only view).
    ///
    /// Returns `ForgeError::NotFound` if `proposal_id` does not exist.
    pub fn has_voted(env: Env, proposal_id: u64, voter: Address) -> Result<bool, ForgeError> {
        // Verify proposal existence
        Self::get_proposal_impl(&env, proposal_id)?;

        let vote_key = DataKey::Vote(proposal_id, voter);
        Ok(env.storage().instance().has(&vote_key))
    }

    /// Allocate the next monotonic proposal id.
    fn next_id(env: &Env) -> Result<u64, ForgeError> {
        let count: u64 = env.storage().instance().get(&DataKey::Count).unwrap_or(0);
        let id = count.checked_add(1).ok_or(ForgeError::ArithmeticOverflow)?;
        env.storage().instance().set(&DataKey::Count, &id);
        Ok(id)
    }

    fn get_proposal_impl(env: &Env, proposal_id: u64) -> Result<Proposal, ForgeError> {
        env.storage()
            .instance()
            .get(&DataKey::Proposal(proposal_id))
            .ok_or(ForgeError::NotFound)
    }
}

/// Lifecycle events emitted by the DAO governance contract.
mod events {
    use super::*;

    #[contractevent]
    pub struct Proposed {
        #[topic]
        pub proposal_id: u64,
        pub data: Proposal,
    }

    #[contractevent]
    pub struct VoteCast {
        #[topic]
        pub proposal_id: u64,
        pub voter: Address,
        pub support: bool,
    }

    #[contractevent]
    pub struct Finalised {
        #[topic]
        pub proposal_id: u64,
        pub state: ProposalState,
        pub for_votes: i128,
        pub against_votes: i128,
    }

    pub fn proposed(env: &Env, proposal: &Proposal) {
        Proposed {
            proposal_id: proposal.proposal_id,
            data: proposal.clone(),
        }
        .publish(env);
    }

    pub fn vote_cast(env: &Env, proposal_id: u64, voter: &Address, support: bool) {
        VoteCast {
            proposal_id,
            voter: voter.clone(),
            support,
        }
        .publish(env);
    }

    pub fn finalised(
        env: &Env,
        proposal_id: u64,
        state: ProposalState,
        for_votes: i128,
        against_votes: i128,
    ) {
        Finalised {
            proposal_id,
            state,
            for_votes,
            against_votes,
        }
        .publish(env);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_forge_test_utils::{MockTarget, MockTargetClient, RevertingTarget, TestAccounts};
    use soroban_sdk::testutils::{Events as _, Ledger as _};
    use soroban_sdk::{Bytes, Env};

    const START: u64 = 1_000_000;
    const DURATION: u64 = 86_400;

    /// Build a fresh env with mocked auths, a registered contract, a registered
    /// mock target, a pending proposal, and named accounts.
    macro_rules! setup {
        () => {{
            let env = Env::default();
            env.mock_all_auths();
            env.ledger().set_timestamp(START);
            let contract_id = env.register(DaoGovernance, ());
            let client = SorobanForgeDaoGovernanceClient::new(&env, &contract_id);
            let accounts = TestAccounts::generate(&env);
            let target_id = env.register(MockTarget, ());
            let proposal_id =
                client.propose(&accounts.user1, &target_id, &payload(&env), &DURATION);
            (env, client, accounts, proposal_id, target_id)
        }};
    }

    fn payload(env: &Env) -> Bytes {
        Bytes::from_array(env, &[0xC0, 0xDE, 0x00, 0xFF])
    }

    #[contract]
    pub struct AuthCheckingTarget;

    #[contractimpl]
    impl AuthCheckingTarget {
        pub fn execute(env: Env, _payload: Bytes) {
            // A target's own authorization requirement is not implicitly
            // satisfied by the DAO's cross-contract invocation.
            let caller = env.current_contract_address();
            caller.require_auth();
        }
    }

    #[test]
    fn propose_succeeds_and_is_active() {
        let (env, client, accounts, proposal_id, target_id) = setup!();
        let proposal = client.get_proposal(&proposal_id);
        assert_eq!(proposal.proposer, accounts.user1);
        assert_eq!(proposal.target, target_id);
        assert_eq!(proposal.action, payload(&env));
        assert_eq!(proposal.state, ProposalState::Active);
        assert_eq!(proposal.for_votes, 0);
        assert_eq!(proposal.against_votes, 0);
    }

    #[test]
    fn propose_assigns_distinct_ids() {
        let (env, client, accounts, _id, target_id) = setup!();
        let id2 = client.propose(&accounts.user2, &target_id, &payload(&env), &DURATION);
        let id3 = client.propose(&accounts.user3, &target_id, &payload(&env), &DURATION);
        assert_ne!(id2, id3);
    }

    #[test]
    fn propose_rejects_zero_duration() {
        let (env, client, accounts, _id, target_id) = setup!();
        let err = client
            .try_propose(&accounts.user1, &target_id, &payload(&env), &0_u64)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn vote_records_support() {
        let (_env, client, accounts, proposal_id, _target_id) = setup!();
        client.vote(&proposal_id, &accounts.user2, &true);
        let proposal = client.get_proposal(&proposal_id);
        assert_eq!(proposal.for_votes, 1);
        assert_eq!(proposal.against_votes, 0);
    }

    #[test]
    fn vote_records_against() {
        let (_env, client, accounts, proposal_id, _target_id) = setup!();
        client.vote(&proposal_id, &accounts.user2, &false);
        let proposal = client.get_proposal(&proposal_id);
        assert_eq!(proposal.for_votes, 0);
        assert_eq!(proposal.against_votes, 1);
    }

    #[test]
    fn vote_twice_is_invalid() {
        let (_env, client, accounts, proposal_id, _target_id) = setup!();
        client.vote(&proposal_id, &accounts.user2, &true);
        let err = client
            .try_vote(&proposal_id, &accounts.user2, &true)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn vote_after_deadline_is_rejected() {
        let (env, client, accounts, proposal_id, _target_id) = setup!();
        env.ledger().set_timestamp(START + DURATION);
        let err = client
            .try_vote(&proposal_id, &accounts.user2, &true)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::DeadlineReached);
    }

    #[test]
    fn vote_missing_proposal_is_not_found() {
        let (_env, client, accounts, _id, _target_id) = setup!();
        let err = client
            .try_vote(&999, &accounts.user2, &true)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn execute_before_deadline_is_invalid() {
        let (_env, client, _accounts, proposal_id, _target_id) = setup!();
        let err = client.try_execute(&proposal_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn execute_after_deadline_passes_majority() {
        let (env, client, accounts, proposal_id, _target_id) = setup!();
        client.vote(&proposal_id, &accounts.user2, &true);
        env.ledger().set_timestamp(START + DURATION + 1);
        client.execute(&proposal_id);
        assert_eq!(
            client.get_proposal(&proposal_id).state,
            ProposalState::Succeeded
        );
    }

    #[test]
    fn execute_after_deadline_defeats_minority() {
        let (env, client, accounts, proposal_id, _target_id) = setup!();
        client.vote(&proposal_id, &accounts.user2, &false);
        client.vote(&proposal_id, &accounts.user3, &true);
        client.vote(&proposal_id, &accounts.validator, &false);
        env.ledger().set_timestamp(START + DURATION + 1);
        client.execute(&proposal_id);
        assert_eq!(
            client.get_proposal(&proposal_id).state,
            ProposalState::Defeated
        );
    }

    #[test]
    fn execute_tie_is_defeated() {
        let (env, client, accounts, proposal_id, _target_id) = setup!();
        client.vote(&proposal_id, &accounts.user2, &true);
        client.vote(&proposal_id, &accounts.user3, &false);
        env.ledger().set_timestamp(START + DURATION + 1);
        client.execute(&proposal_id);
        assert_eq!(
            client.get_proposal(&proposal_id).state,
            ProposalState::Defeated
        );
    }

    #[test]
    fn execute_no_votes_is_defeated() {
        let (env, client, _accounts, proposal_id, _target_id) = setup!();
        env.ledger().set_timestamp(START + DURATION + 1);
        client.execute(&proposal_id);
        assert_eq!(
            client.get_proposal(&proposal_id).state,
            ProposalState::Defeated
        );
    }

    #[test]
    fn execute_twice_is_invalid() {
        let (env, client, accounts, proposal_id, _target_id) = setup!();
        client.vote(&proposal_id, &accounts.user2, &true);
        env.ledger().set_timestamp(START + DURATION + 1);
        client.execute(&proposal_id); // Active -> Succeeded
        client.execute(&proposal_id); // Succeeded -> Executed
        let err = client.try_execute(&proposal_id).unwrap_err().unwrap(); // Executed -> rejected
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn execute_missing_proposal_is_not_found() {
        let (_env, client, _accounts, _id, _target_id) = setup!();
        let err = client.try_execute(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn get_proposal_missing_is_not_found() {
        let (_env, client, _accounts, _id, _target_id) = setup!();
        let err = client.try_get_proposal(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn cancel_active_proposal_succeeds() {
        let (_env, client, accounts, proposal_id, _target_id) = setup!();
        client.cancel_proposal(&proposal_id, &accounts.user1);
        assert_eq!(
            client.get_proposal(&proposal_id).state,
            ProposalState::Cancelled
        );
    }

    #[test]
    fn cancel_immediately_after_propose_succeeds() {
        let (env, client, accounts, _id, target_id) = setup!();
        let fresh_id = client.propose(&accounts.user3, &target_id, &payload(&env), &DURATION);
        client.cancel_proposal(&fresh_id, &accounts.user3);
        assert_eq!(
            client.get_proposal(&fresh_id).state,
            ProposalState::Cancelled
        );
    }

    #[test]
    fn cancel_after_quorum_before_execute_succeeds() {
        let (env, client, accounts, proposal_id, _target_id) = setup!();
        client.vote(&proposal_id, &accounts.user2, &true);
        client.vote(&proposal_id, &accounts.user3, &true);
        env.ledger().set_timestamp(START + DURATION + 1);
        client.cancel_proposal(&proposal_id, &accounts.user1);
        assert_eq!(
            client.get_proposal(&proposal_id).state,
            ProposalState::Cancelled
        );
        assert_eq!(client.get_proposal(&proposal_id).for_votes, 2);
    }

    #[test]
    fn cancel_by_non_proposer_is_unauthorized() {
        let (_env, client, accounts, proposal_id, _target_id) = setup!();
        let err = client
            .try_cancel_proposal(&proposal_id, &accounts.user2)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::Unauthorized);
    }

    #[test]
    fn cancel_missing_proposal_is_not_found() {
        let (_env, client, accounts, _id, _target_id) = setup!();
        let err = client
            .try_cancel_proposal(&999, &accounts.user1)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn double_cancel_is_invalid() {
        let (_env, client, accounts, proposal_id, _target_id) = setup!();
        client.cancel_proposal(&proposal_id, &accounts.user1);
        let err = client
            .try_cancel_proposal(&proposal_id, &accounts.user1)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn cancel_then_vote_is_invalid() {
        let (_env, client, accounts, proposal_id, _target_id) = setup!();
        client.cancel_proposal(&proposal_id, &accounts.user1);
        let err = client
            .try_vote(&proposal_id, &accounts.user2, &true)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn cancel_then_execute_is_invalid() {
        let (_env, client, accounts, proposal_id, _target_id) = setup!();
        client.cancel_proposal(&proposal_id, &accounts.user1);
        let err = client.try_execute(&proposal_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn cancel_after_execute_is_invalid() {
        let (env, client, accounts, proposal_id, _target_id) = setup!();
        env.ledger().set_timestamp(START + DURATION + 1);
        client.execute(&proposal_id);
        let err = client
            .try_cancel_proposal(&proposal_id, &accounts.user1)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    // -------------------------------------------------------------------
    // Cross-Contract Invocation & State Verification (Issue 58)
    // -------------------------------------------------------------------

    #[test]
    fn execute_dispatches_to_mock_target_and_changes_state_once() {
        let (env, client, accounts, proposal_id, target_id) = setup!();
        let mock_target = MockTargetClient::new(&env, &target_id);
        assert_eq!(mock_target.count(), 0);
        assert_eq!(mock_target.last_payload(), None);

        // Pass proposal with strict majority:
        client.vote(&proposal_id, &accounts.user2, &true);
        env.ledger().set_timestamp(START + DURATION + 1);

        // First execute: finalises Active -> Succeeded (target invocation not yet run)
        client.execute(&proposal_id);
        assert_eq!(
            client.get_proposal(&proposal_id).state,
            ProposalState::Succeeded
        );
        assert_eq!(mock_target.count(), 0);

        // Second execute: performs cross-contract call, transitions Succeeded -> Executed
        client.execute(&proposal_id);
        assert_eq!(
            client.get_proposal(&proposal_id).state,
            ProposalState::Executed
        );
        assert_eq!(mock_target.count(), 1);
        assert_eq!(mock_target.last_payload(), Some(payload(&env)));

        // Third execute on Executed proposal: rejected, target state remains 1
        let err = client.try_execute(&proposal_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
        assert_eq!(mock_target.count(), 1);
    }

    #[test]
    fn execute_reverting_target_leaves_proposal_succeeded_and_target_unchanged() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(START);
        let contract_id = env.register(DaoGovernance, ());
        let client = SorobanForgeDaoGovernanceClient::new(&env, &contract_id);
        let accounts = TestAccounts::generate(&env);
        let reverting_target_id = env.register(RevertingTarget, ());

        let proposal_id = client.propose(
            &accounts.user1,
            &reverting_target_id,
            &payload(&env),
            &DURATION,
        );

        client.vote(&proposal_id, &accounts.user2, &true);
        env.ledger().set_timestamp(START + DURATION + 1);

        // Finalise Active -> Succeeded
        client.execute(&proposal_id);
        assert_eq!(
            client.get_proposal(&proposal_id).state,
            ProposalState::Succeeded
        );

        // Execution attempt against reverting target fails with ContractInvocationFailed
        let err = client.try_execute(&proposal_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::ContractInvocationFailed);

        // Failure ordering guarantee: proposal remains Succeeded (NOT Executed)
        assert_eq!(
            client.get_proposal(&proposal_id).state,
            ProposalState::Succeeded
        );
    }

    #[test]
    fn defeated_and_still_active_proposals_cannot_be_executed() {
        let (env, client, accounts, proposal_id, target_id) = setup!();
        let mock_target = MockTargetClient::new(&env, &target_id);

        // 1. Still-Active proposal before deadline cannot be executed
        let err_active = client.try_execute(&proposal_id).unwrap_err().unwrap();
        assert_eq!(err_active, ForgeError::InvalidInput);
        assert_eq!(mock_target.count(), 0);

        // 2. Defeated proposal cannot be executed
        client.vote(&proposal_id, &accounts.user2, &false);
        env.ledger().set_timestamp(START + DURATION + 1);
        client.execute(&proposal_id); // Transitions Active -> Defeated
        assert_eq!(
            client.get_proposal(&proposal_id).state,
            ProposalState::Defeated
        );

        let err_defeated = client.try_execute(&proposal_id).unwrap_err().unwrap();
        assert_eq!(err_defeated, ForgeError::InvalidInput);
        assert_eq!(mock_target.count(), 0);
    }

    #[test]
    fn target_auth_requirement_is_not_satisfied_implicitly() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(START);
        let contract_id = env.register(DaoGovernance, ());
        let client = SorobanForgeDaoGovernanceClient::new(&env, &contract_id);
        let accounts = TestAccounts::generate(&env);
        let auth_target_id = env.register(AuthCheckingTarget, ());

        let proposal_id =
            client.propose(&accounts.user1, &auth_target_id, &payload(&env), &DURATION);
        client.vote(&proposal_id, &accounts.user2, &true);
        env.ledger().set_timestamp(START + DURATION + 1);

        client.execute(&proposal_id); // Succeeded
        let err = client.try_execute(&proposal_id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::ContractInvocationFailed);
        assert_eq!(
            client.get_proposal(&proposal_id).state,
            ProposalState::Succeeded
        );
    }

    #[test]
    fn execute_is_permissionless_callable_by_unrelated_account() {
        let (env, client, accounts, proposal_id, target_id) = setup!();
        let mock_target = MockTargetClient::new(&env, &target_id);

        client.vote(&proposal_id, &accounts.user2, &true);
        env.ledger().set_timestamp(START + DURATION + 1);

        // An account that did not propose or vote triggers finalisation and execution
        client.execute(&proposal_id); // user3 or any caller
        client.execute(&proposal_id);
        assert_eq!(
            client.get_proposal(&proposal_id).state,
            ProposalState::Executed
        );
        assert_eq!(mock_target.count(), 1);
    }

    #[test]
    fn events_emitted_during_proposal_lifecycle() {
        use soroban_sdk::xdr::{self, ScVal};

        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(START);
        let contract_id = env.register(DaoGovernance, ());
        let client = SorobanForgeDaoGovernanceClient::new(&env, &contract_id);
        let accounts = TestAccounts::generate(&env);
        let target_id = env.register(MockTarget, ());

        // 1. propose emits Proposed with topic proposal_id
        let proposal_id = client.propose(&accounts.user1, &target_id, &payload(&env), &DURATION);
        let all = env.events().all();
        let events = all.events();
        assert_eq!(events.len(), 1);
        let event = &events[0];
        let xdr::ContractEventBody::V0(body) = &event.body;
        assert_eq!(
            body.topics.first().unwrap(),
            &ScVal::Symbol("proposed".try_into().unwrap())
        );
        assert_eq!(body.topics.get(1).unwrap(), &ScVal::U64(proposal_id));

        // 2. vote emits VoteCast with topic proposal_id
        client.vote(&proposal_id, &accounts.user2, &true);
        let all = env.events().all();
        let events = all.events();
        assert_eq!(events.len(), 1);
        let event = &events[0];
        let xdr::ContractEventBody::V0(body) = &event.body;
        assert_eq!(
            body.topics.first().unwrap(),
            &ScVal::Symbol("vote_cast".try_into().unwrap())
        );
        assert_eq!(body.topics.get(1).unwrap(), &ScVal::U64(proposal_id));

        // 3. Negative assertion: failed duplicate vote emits no events
        let err = client
            .try_vote(&proposal_id, &accounts.user2, &true)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
        assert_eq!(env.events().all().events().len(), 0);

        // 4. Negative assertion: read-only get_proposal emits no events
        let _ = client.get_proposal(&proposal_id);
        assert_eq!(env.events().all().events().len(), 0);

        // 5. execute finalisation (Active -> Succeeded) emits Finalised
        env.ledger().set_timestamp(START + DURATION + 1);
        client.execute(&proposal_id);
        let all = env.events().all();
        let events = all.events();
        assert_eq!(events.len(), 1);
        let event = &events[0];
        let xdr::ContractEventBody::V0(body) = &event.body;
        assert_eq!(
            body.topics.first().unwrap(),
            &ScVal::Symbol("finalised".try_into().unwrap())
        );
        assert_eq!(body.topics.get(1).unwrap(), &ScVal::U64(proposal_id));

        // 6. execute invocation (Succeeded -> Executed) emits Finalised
        client.execute(&proposal_id);
        let all = env.events().all();
        let events = all.events();
        assert_eq!(events.len(), 1);
        let event = &events[0];
        let xdr::ContractEventBody::V0(body) = &event.body;
        assert_eq!(
            body.topics.first().unwrap(),
            &ScVal::Symbol("finalised".try_into().unwrap())
        );
        assert_eq!(body.topics.get(1).unwrap(), &ScVal::U64(proposal_id));
    }

    #[test]
    fn events_emitted_on_defeated_proposal() {
        use soroban_sdk::xdr::{self, ScVal};

        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(START);
        let contract_id = env.register(DaoGovernance, ());
        let client = SorobanForgeDaoGovernanceClient::new(&env, &contract_id);
        let accounts = TestAccounts::generate(&env);
        let target_id = env.register(MockTarget, ());

        let proposal_id = client.propose(&accounts.user1, &target_id, &payload(&env), &DURATION);
        client.vote(&proposal_id, &accounts.user2, &false);

        env.ledger().set_timestamp(START + DURATION + 1);
        client.execute(&proposal_id);
        let all = env.events().all();
        let events = all.events();
        assert_eq!(events.len(), 1);
        let event = &events[0];
        let xdr::ContractEventBody::V0(body) = &event.body;
        assert_eq!(
            body.topics.first().unwrap(),
            &ScVal::Symbol("finalised".try_into().unwrap())
        );
        assert_eq!(body.topics.get(1).unwrap(), &ScVal::U64(proposal_id));
    }

    #[test]
    fn test_introspection_count_pagination_and_has_voted() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(START);
        let contract_id = env.register(DaoGovernance, ());
        let client = SorobanForgeDaoGovernanceClient::new(&env, &contract_id);
        let accounts = TestAccounts::generate(&env);
        let target_id = env.register(MockTarget, ());

        // 1. Initial uninitialized state
        assert_eq!(client.get_proposal_count(), 0);
        assert_eq!(client.get_proposals(&0, &10).len(), 0);

        // 2. Create 5 proposals
        for _ in 0..5 {
            client.propose(&accounts.user1, &target_id, &payload(&env), &DURATION);
        }
        assert_eq!(client.get_proposal_count(), 5);

        // 3. Test pagination bounds
        // Page 1: offset 0, limit 2 -> [p1, p2]
        let page1 = client.get_proposals(&0, &2);
        assert_eq!(page1.len(), 2);
        assert_eq!(page1.get(0).unwrap().proposal_id, 1);
        assert_eq!(page1.get(1).unwrap().proposal_id, 2);

        // Page 2: offset 2, limit 2 -> [p3, p4]
        let page2 = client.get_proposals(&2, &2);
        assert_eq!(page2.len(), 2);
        assert_eq!(page2.get(0).unwrap().proposal_id, 3);
        assert_eq!(page2.get(1).unwrap().proposal_id, 4);

        // Page 3: offset 4, limit 2 -> [p5]
        let page3 = client.get_proposals(&4, &2);
        assert_eq!(page3.len(), 1);
        assert_eq!(page3.get(0).unwrap().proposal_id, 5);

        // Past total count: offset 5, limit 2 -> []
        let past = client.get_proposals(&5, &2);
        assert_eq!(past.len(), 0);

        // Zero limit returns InvalidInput
        assert_eq!(
            client.try_get_proposals(&0, &0).unwrap_err().unwrap(),
            ForgeError::InvalidInput
        );

        // 4. Test has_voted
        // Before voting
        assert!(!client.has_voted(&1, &accounts.user2));

        // Vote on proposal 1
        client.vote(&1, &accounts.user2, &true);

        // After voting
        assert!(client.has_voted(&1, &accounts.user2));
        assert!(!client.has_voted(&1, &accounts.user1));

        // Query voting status on non-existent proposal ID
        assert_eq!(
            client
                .try_has_voted(&999, &accounts.user2)
                .unwrap_err()
                .unwrap(),
            ForgeError::NotFound
        );
    }
}
