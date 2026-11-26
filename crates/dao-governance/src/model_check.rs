//! State-machine model checking for the DAO governance lifecycle.
//!
//! Validates the DAO governance contract against an independent state-machine model:
//! - Tracks proposal statuses: Uncreated -> Active -> Succeeded -> Executed / Defeated / Cancelled
//! - Enforces vote tally conservation: for_votes + against_votes == sum of distinct voter weights
//! - Enforces bond conservation: contract + proposer + treasury == initial_bond_mint
//! - Verifies voting window: voting is rejected once deadline has passed
//! - Verifies terminal states: Defeated, Executed, Cancelled are terminal and irreversible
//! - Guarantees non-mutation: invalid operations are rejected without modifying storage or balances

extern crate std;

use crate::{BondState, DaoGovernance, ProposalState, SorobanForgeDaoGovernanceClient};
use core::fmt;
use soroban_forge_shared_utils::ForgeError;
use soroban_forge_test_utils::model::{
    assert_model_check, LifecycleModel, ModelDriver, ModelRng, OpGenerator,
};
use soroban_forge_test_utils::{MockTarget, TestAccounts};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Bytes, Env, Vec as SdkVec};
use std::format;
use std::string::{String, ToString};
use std::vec::Vec;

const START_TIMESTAMP: u64 = 1_000_000;
const DURATION: u64 = 86_400;
const BOND_AMOUNT: i128 = 500;
const INITIAL_BOND_MINT: i128 = 10_000;
const VOTER_WEIGHT: i128 = 100;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DaoModelState {
    Uncreated,
    Active {
        for_votes: i128,
        against_votes: i128,
        voted_voters: Vec<usize>,
        voting_ends: u64,
        created_at: u64,
    },
    Succeeded {
        for_votes: i128,
        against_votes: i128,
    },
    Executed {
        for_votes: i128,
        against_votes: i128,
    },
    Defeated {
        for_votes: i128,
        against_votes: i128,
    },
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DaoModelOp {
    Propose { duration: u64 },
    Vote { voter_idx: usize, support: bool },
    Execute,
    CancelProposal,
    AdvanceTime { seconds: u64 },
    InvalidOp,
}

impl fmt::Display for DaoModelOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Propose { duration } => write!(f, "Propose(duration={duration})"),
            Self::Vote { voter_idx, support } => {
                write!(f, "Vote(voter={voter_idx}, support={support})")
            }
            Self::Execute => write!(f, "Execute"),
            Self::CancelProposal => write!(f, "CancelProposal"),
            Self::AdvanceTime { seconds } => write!(f, "AdvanceTime({seconds}s)"),
            Self::InvalidOp => write!(f, "InvalidOp"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaoSnapshot {
    pub proposal_state: Option<ProposalState>,
    pub bond_state: Option<BondState>,
    pub for_votes: i128,
    pub against_votes: i128,
    pub contract_bond: i128,
    pub proposer_bond: i128,
    pub treasury_bond: i128,
    pub current_time: u64,
}

pub struct DaoLifecycleModel {
    pub initial_time: u64,
    pub voter_weight: i128,
}

impl LifecycleModel for DaoLifecycleModel {
    type State = (DaoModelState, u64); // (state, current_time)
    type Op = DaoModelOp;
    type Snapshot = DaoSnapshot;

    fn initial_state(&self) -> Self::State {
        (DaoModelState::Uncreated, self.initial_time)
    }

    fn transition(&self, state: &Self::State, op: &Self::Op) -> Result<Self::State, String> {
        let (current_state, current_time) = state;

        match op {
            DaoModelOp::AdvanceTime { seconds } => {
                Ok((current_state.clone(), current_time.saturating_add(*seconds)))
            }
            DaoModelOp::Propose { duration } => match current_state {
                DaoModelState::Uncreated => {
                    if *duration == 0 {
                        Err("duration must be > 0".into())
                    } else {
                        Ok((
                            DaoModelState::Active {
                                for_votes: 0,
                                against_votes: 0,
                                voted_voters: Vec::new(),
                                voting_ends: current_time.saturating_add(*duration),
                                created_at: *current_time,
                            },
                            *current_time,
                        ))
                    }
                }
                _ => Err("proposal already created".into()),
            },
            DaoModelOp::Vote { voter_idx, support } => match current_state {
                DaoModelState::Active {
                    for_votes,
                    against_votes,
                    voted_voters,
                    voting_ends,
                    created_at,
                } => {
                    if *current_time >= *voting_ends {
                        Err("voting has ended".into())
                    } else if voted_voters.contains(voter_idx) {
                        Err("duplicate vote".into())
                    } else {
                        let mut next_voters = voted_voters.clone();
                        next_voters.push(*voter_idx);

                        let (new_for, new_against) = if *support {
                            (for_votes.saturating_add(self.voter_weight), *against_votes)
                        } else {
                            (*for_votes, against_votes.saturating_add(self.voter_weight))
                        };

                        Ok((
                            DaoModelState::Active {
                                for_votes: new_for,
                                against_votes: new_against,
                                voted_voters: next_voters,
                                voting_ends: *voting_ends,
                                created_at: *created_at,
                            },
                            *current_time,
                        ))
                    }
                }
                _ => Err("cannot vote unless proposal is Active".into()),
            },
            DaoModelOp::CancelProposal => match current_state {
                DaoModelState::Active {
                    for_votes,
                    against_votes,
                    created_at,
                    ..
                } => {
                    if *current_time > *created_at || *for_votes > 0 || *against_votes > 0 {
                        Err("cannot cancel after voting starts".into())
                    } else {
                        Ok((DaoModelState::Cancelled, *current_time))
                    }
                }
                _ => Err("cannot cancel non-active proposal".into()),
            },
            DaoModelOp::Execute => match current_state {
                DaoModelState::Active {
                    for_votes,
                    against_votes,
                    voting_ends,
                    ..
                } => {
                    if *current_time < *voting_ends {
                        Err("cannot execute before voting deadline".into())
                    } else if *for_votes > *against_votes {
                        Ok((
                            DaoModelState::Succeeded {
                                for_votes: *for_votes,
                                against_votes: *against_votes,
                            },
                            *current_time,
                        ))
                    } else {
                        Ok((
                            DaoModelState::Defeated {
                                for_votes: *for_votes,
                                against_votes: *against_votes,
                            },
                            *current_time,
                        ))
                    }
                }
                DaoModelState::Succeeded {
                    for_votes,
                    against_votes,
                } => {
                    // Second execute dispatches the target and transitions to Executed
                    Ok((
                        DaoModelState::Executed {
                            for_votes: *for_votes,
                            against_votes: *against_votes,
                        },
                        *current_time,
                    ))
                }
                _ => Err("cannot execute non-active / non-succeeded proposal".into()),
            },
            DaoModelOp::InvalidOp => Err("explicit invalid operation".into()),
        }
    }

    fn check_invariants(
        &self,
        state: &Self::State,
        snapshot: &Self::Snapshot,
    ) -> Result<(), String> {
        let (current_state, current_time) = state;

        if *current_time != snapshot.current_time {
            return Err(format!(
                "time mismatch: model={current_time}, on-chain={}",
                snapshot.current_time
            ));
        }

        // Bond token conservation: contract + proposer + treasury == INITIAL_BOND_MINT
        let total_bond = snapshot
            .contract_bond
            .saturating_add(snapshot.proposer_bond)
            .saturating_add(snapshot.treasury_bond);

        if total_bond != INITIAL_BOND_MINT {
            return Err(format!(
                "Bond conservation violated! Expected {INITIAL_BOND_MINT}, got {total_bond}"
            ));
        }

        match current_state {
            DaoModelState::Uncreated => {
                if snapshot.proposal_state.is_some() || snapshot.contract_bond != 0 {
                    return Err("Uncreated proposal has on-chain state or bond".into());
                }
            }
            DaoModelState::Active {
                for_votes,
                against_votes,
                ..
            } => {
                if snapshot.proposal_state != Some(ProposalState::Active) {
                    return Err(format!(
                        "Expected Active proposal, got {:?}",
                        snapshot.proposal_state
                    ));
                }
                if snapshot.for_votes != *for_votes || snapshot.against_votes != *against_votes {
                    return Err(format!(
                        "Vote mismatch: model=({for_votes}, {against_votes}), on-chain=({}, {})",
                        snapshot.for_votes, snapshot.against_votes
                    ));
                }
                if snapshot.contract_bond != BOND_AMOUNT {
                    return Err(format!(
                        "Active proposal contract bond mismatch: expected {BOND_AMOUNT}, got {}",
                        snapshot.contract_bond
                    ));
                }
                if snapshot.bond_state != Some(BondState::Posted) {
                    return Err(format!(
                        "Expected Posted bond, got {:?}",
                        snapshot.bond_state
                    ));
                }
            }
            DaoModelState::Succeeded {
                for_votes,
                against_votes,
            } => {
                if snapshot.proposal_state != Some(ProposalState::Succeeded) {
                    return Err(format!(
                        "Expected Succeeded proposal, got {:?}",
                        snapshot.proposal_state
                    ));
                }
                if snapshot.for_votes != *for_votes || snapshot.against_votes != *against_votes {
                    return Err("Vote mismatch in Succeeded".into());
                }
                if snapshot.contract_bond != BOND_AMOUNT {
                    return Err("Succeeded proposal still holds bond until executed".into());
                }
            }
            DaoModelState::Executed { .. } => {
                if snapshot.proposal_state != Some(ProposalState::Executed) {
                    return Err(format!(
                        "Expected Executed proposal, got {:?}",
                        snapshot.proposal_state
                    ));
                }
                if snapshot.contract_bond != 0 {
                    return Err("Executed proposal must refund bond to proposer".into());
                }
                if snapshot.bond_state != Some(BondState::Refunded) {
                    return Err(format!(
                        "Expected Refunded bond, got {:?}",
                        snapshot.bond_state
                    ));
                }
            }
            DaoModelState::Defeated { .. } => {
                if snapshot.proposal_state != Some(ProposalState::Defeated) {
                    return Err(format!(
                        "Expected Defeated proposal, got {:?}",
                        snapshot.proposal_state
                    ));
                }
                if snapshot.contract_bond != 0 {
                    return Err("Defeated proposal must forfeit bond to treasury".into());
                }
                if snapshot.treasury_bond != BOND_AMOUNT {
                    return Err(format!(
                        "Treasury balance mismatch on Defeated: expected {BOND_AMOUNT}, got {}",
                        snapshot.treasury_bond
                    ));
                }
                if snapshot.bond_state != Some(BondState::Forfeited) {
                    return Err(format!(
                        "Expected Forfeited bond, got {:?}",
                        snapshot.bond_state
                    ));
                }
            }
            DaoModelState::Cancelled => {
                if snapshot.proposal_state != Some(ProposalState::Cancelled) {
                    return Err(format!(
                        "Expected Cancelled proposal, got {:?}",
                        snapshot.proposal_state
                    ));
                }
                if snapshot.contract_bond != 0 {
                    return Err("Cancelled proposal must refund bond".into());
                }
                if snapshot.bond_state != Some(BondState::Refunded) {
                    return Err(format!(
                        "Expected Refunded bond on Cancelled, got {:?}",
                        snapshot.bond_state
                    ));
                }
            }
        }

        Ok(())
    }

    fn is_terminal(&self, state: &Self::State) -> bool {
        matches!(
            state.0,
            DaoModelState::Executed { .. }
                | DaoModelState::Defeated { .. }
                | DaoModelState::Cancelled
        )
    }
}

pub struct DaoModelDriver {
    pub contract_id: Address,
    pub _gov_token: Address,
    pub bond_token: Address,
    pub treasury: Address,
    pub accounts: TestAccounts,
    pub voters: Vec<Address>,
    pub target: Address,
    pub proposal_id: Option<u64>,
}

impl DaoModelDriver {
    pub fn new(env: &Env) -> Self {
        env.mock_all_auths();
        env.ledger().set_timestamp(START_TIMESTAMP);

        let admin = Address::generate(env);
        let sac_gov = env.register_stellar_asset_contract_v2(admin.clone());
        let gov_token = sac_gov.address();

        let sac_bond = env.register_stellar_asset_contract_v2(admin);
        let bond_token = sac_bond.address();

        let accounts = TestAccounts::generate(env);
        let treasury = Address::generate(env);

        // Mint bond tokens to proposer (user1)
        StellarAssetClient::new(env, &bond_token).mint(&accounts.user1, &INITIAL_BOND_MINT);

        // Voters pool with governance tokens
        let mut voters = Vec::new();
        let gov_client = StellarAssetClient::new(env, &gov_token);
        for _ in 0..4 {
            let v = Address::generate(env);
            gov_client.mint(&v, &VOTER_WEIGHT);
            voters.push(v);
        }

        let target = env.register(MockTarget, ());
        let contract_id = env.register(DaoGovernance, ());
        let client = SorobanForgeDaoGovernanceClient::new(env, &contract_id);

        client.initialize(&gov_token);
        client.configure_bond(&bond_token, &BOND_AMOUNT, &treasury);

        Self {
            contract_id,
            _gov_token: gov_token,
            bond_token,
            treasury,
            accounts,
            voters,
            target,
            proposal_id: None,
        }
    }

    fn client<'a>(&self, env: &'a Env) -> SorobanForgeDaoGovernanceClient<'a> {
        SorobanForgeDaoGovernanceClient::new(env, &self.contract_id)
    }

    fn bond_client<'a>(&self, env: &'a Env) -> TokenClient<'a> {
        TokenClient::new(env, &self.bond_token)
    }
}

impl ModelDriver<DaoLifecycleModel> for DaoModelDriver {
    fn execute_op(&mut self, env: &Env, op: &DaoModelOp) -> Result<(), String> {
        let client = self.client(env);

        match op {
            DaoModelOp::AdvanceTime { seconds } => {
                let current = env.ledger().timestamp();
                env.ledger().set_timestamp(current.saturating_add(*seconds));
                Ok(())
            }
            DaoModelOp::Propose { duration } => {
                if self.proposal_id.is_some() {
                    return Err("already proposed".into());
                }
                let payload = Bytes::new(env);
                let requires = SdkVec::new(env);

                let res = client.try_propose(
                    &self.accounts.user1,
                    &self.target,
                    &payload,
                    duration,
                    &requires,
                    &None,
                );
                match res {
                    Ok(Ok(id)) => {
                        self.proposal_id = Some(id);
                        Ok(())
                    }
                    Ok(Err(_)) => Err("conversion error".into()),
                    Err(Ok(forge_err)) => Err(format!("{forge_err:?}")),
                    Err(Err(_)) => Err("invoke error".into()),
                }
            }
            DaoModelOp::Vote { voter_idx, support } => {
                let id = self.proposal_id.ok_or_else(|| "no proposal".to_string())?;
                let voter = self
                    .voters
                    .get(*voter_idx)
                    .ok_or_else(|| "invalid voter idx".to_string())?;
                map_result(client.try_vote(&id, voter, support))
            }
            DaoModelOp::Execute => {
                let id = self.proposal_id.ok_or_else(|| "no proposal".to_string())?;
                map_result(client.try_execute(&id))
            }
            DaoModelOp::CancelProposal => {
                let id = self.proposal_id.ok_or_else(|| "no proposal".to_string())?;
                map_result(client.try_cancel_proposal(&id, &self.accounts.user1))
            }
            DaoModelOp::InvalidOp => {
                let invalid_id = 999_999_u64;
                map_result(client.try_execute(&invalid_id))
            }
        }
    }

    fn snapshot(&self, env: &Env) -> DaoSnapshot {
        let bc = self.bond_client(env);
        let client = self.client(env);

        let (proposal_state, bond_state, for_votes, against_votes) =
            if let Some(id) = self.proposal_id {
                match client.try_get_proposal(&id) {
                    Ok(Ok(prop)) => (
                        Some(prop.state),
                        Some(prop.bond_state),
                        prop.for_votes,
                        prop.against_votes,
                    ),
                    _ => (None, None, 0, 0),
                }
            } else {
                (None, None, 0, 0)
            };

        DaoSnapshot {
            proposal_state,
            bond_state,
            for_votes,
            against_votes,
            contract_bond: bc.balance(&self.contract_id),
            proposer_bond: bc.balance(&self.accounts.user1),
            treasury_bond: bc.balance(&self.treasury),
            current_time: env.ledger().timestamp(),
        }
    }

    fn reset(&mut self, env: &Env) {
        *self = Self::new(env);
    }
}

fn map_result<T, E1: fmt::Debug>(
    res: Result<Result<T, E1>, Result<ForgeError, soroban_sdk::InvokeError>>,
) -> Result<(), String> {
    match res {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(err)) => Err(format!("{err:?}")),
        Err(Ok(forge_err)) => Err(format!("{forge_err:?}")),
        Err(Err(_)) => Err("Invoke error".into()),
    }
}

pub struct DaoOpGenerator;

impl OpGenerator<DaoLifecycleModel> for DaoOpGenerator {
    fn generate_op(&mut self, state: &(DaoModelState, u64), rng: &mut ModelRng) -> DaoModelOp {
        let (current_state, _time) = state;

        let choice = rng.bounded(10);
        match current_state {
            DaoModelState::Uncreated => match choice {
                0..=7 => DaoModelOp::Propose { duration: DURATION },
                8 => DaoModelOp::Propose { duration: 0 }, // Invalid: zero duration
                _ => DaoModelOp::Execute,                 // Invalid: uncreated
            },
            DaoModelState::Active { .. } => match choice {
                0..=4 => {
                    let voter_idx = (rng.bounded(4)) as usize;
                    DaoModelOp::Vote {
                        voter_idx,
                        support: rng.gen_bool(),
                    }
                }
                5..=6 => {
                    // Decide whether to advance before or past deadline
                    if rng.gen_bool() {
                        DaoModelOp::AdvanceTime {
                            seconds: rng.gen_range_u64(1_000, 20_000),
                        }
                    } else {
                        DaoModelOp::AdvanceTime {
                            seconds: DURATION.saturating_add(10),
                        }
                    }
                }
                7 => DaoModelOp::Execute,
                8 => DaoModelOp::CancelProposal,
                _ => DaoModelOp::Propose { duration: DURATION }, // Invalid: already created
            },
            DaoModelState::Succeeded { .. } => match choice {
                0..=5 => DaoModelOp::Execute, // Finalize execution to Executed
                6 => DaoModelOp::Vote {
                    voter_idx: 0,
                    support: true,
                }, // Invalid
                7 => DaoModelOp::CancelProposal, // Invalid
                _ => DaoModelOp::AdvanceTime { seconds: 1_000 },
            },
            DaoModelState::Executed { .. }
            | DaoModelState::Defeated { .. }
            | DaoModelState::Cancelled => match choice {
                0..=2 => DaoModelOp::Execute, // Invalid
                3..=4 => DaoModelOp::Vote {
                    voter_idx: 0,
                    support: true,
                }, // Invalid
                5..=6 => DaoModelOp::CancelProposal, // Invalid
                7 => DaoModelOp::InvalidOp,
                _ => DaoModelOp::AdvanceTime { seconds: 1_000 },
            },
        }
    }
}

#[test]
fn test_dao_model_check_deterministic_seeds() {
    let seeds = [101_u64, 202, 303, 404, 505, 606, 707, 808];
    for seed in seeds {
        let env = Env::default();
        let model = DaoLifecycleModel {
            initial_time: START_TIMESTAMP,
            voter_weight: VOTER_WEIGHT,
        };
        let mut driver = DaoModelDriver::new(&env);
        let mut generator = DaoOpGenerator;

        assert_model_check(&env, &model, &mut driver, &mut generator, seed, 25);
    }
}
