//! Model-based proposal lifecycle state machine property tests (Issue #739).
//!
//! Validates the DAO governance contract against a randomized, model-driven state machine:
//! - Predicts proposal state and bond balances after every action.
//! - Verifies votes are counted once and tally matches model.
//! - Asserts finalized proposals (Executed, Defeated, Cancelled) are terminal.
//! - Asserts bond conservation across contract, proposer, and treasury.
//! - Runs at least 128 cases of up to 40 steps in under 90 seconds.
//! - Shrunk failures format human-readable governance action sequences.

extern crate std;

use crate::{BondState, DaoGovernance, ProposalState, SorobanForgeDaoGovernanceClient};
use core::fmt;
use proptest::prelude::*;
use proptest::test_runner::{RngSeed, TestCaseError, TestError, TestRunner};
use soroban_forge_test_utils::{MockTarget, TestAccounts};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Bytes, Env, Vec as SdkVec};
use std::collections::BTreeSet;
use std::format;
use std::string::String;

const START_TIMESTAMP: u64 = 1_000_000;
const BOND_AMOUNT: i128 = 100;
const INITIAL_PROPOSER_MINT: i128 = 10_000;
const VOTER_WEIGHT: i128 = 500;

/// Number of state-machine cases run (>= 128).
const LIFECYCLE_CASES: u32 = 128;
/// Maximum number of steps per generated sequence (up to 40).
const MAX_STEPS: usize = 40;
/// Fixed seed for deterministic property runs.
const LIFECYCLE_SEED: u64 = 0x4441_4f4c_4946_454d; // "DAOLIFEM"

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Propose { duration: u64, delay: u64 },
    Vote { voter_idx: usize, support: bool },
    AdvanceTime { seconds: u64 },
    CancelProposal { is_proposer: bool },
    Execute,
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Propose { duration, delay } => {
                write!(f, "Propose(duration={}, delay={})", duration, delay)
            }
            Self::Vote { voter_idx, support } => {
                write!(f, "Vote(voter={}, support={})", voter_idx, support)
            }
            Self::AdvanceTime { seconds } => write!(f, "AdvanceTime(+{}s)", seconds),
            Self::CancelProposal { is_proposer } => {
                write!(f, "CancelProposal(is_proposer={})", is_proposer)
            }
            Self::Execute => write!(f, "Execute"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelProposalState {
    Uncreated,
    Active {
        voting_start: u64,
        voting_ends: u64,
        for_votes: i128,
        against_votes: i128,
        voters: BTreeSet<usize>,
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

#[derive(Clone, Debug)]
pub struct Model {
    pub time: u64,
    pub state: ModelProposalState,
    pub proposer_balance: i128,
    pub contract_balance: i128,
    pub treasury_balance: i128,
    pub proposal_id: Option<u64>,
}

impl Model {
    pub fn new() -> Self {
        Self {
            time: START_TIMESTAMP,
            state: ModelProposalState::Uncreated,
            proposer_balance: INITIAL_PROPOSER_MINT,
            contract_balance: 0,
            treasury_balance: 0,
            proposal_id: None,
        }
    }

    pub fn apply(&mut self, action: Action) {
        match action {
            Action::Propose { duration, delay } => {
                if self.state == ModelProposalState::Uncreated
                    && duration > 0
                    && self.proposer_balance >= BOND_AMOUNT
                {
                    let voting_start = self.time + delay;
                    let voting_ends = voting_start + duration;
                    self.proposer_balance -= BOND_AMOUNT;
                    self.contract_balance += BOND_AMOUNT;
                    self.proposal_id = Some(1);
                    self.state = ModelProposalState::Active {
                        voting_start,
                        voting_ends,
                        for_votes: 0,
                        against_votes: 0,
                        voters: BTreeSet::new(),
                    };
                }
            }
            Action::Vote { voter_idx, support } => {
                if let ModelProposalState::Active {
                    voting_start,
                    voting_ends,
                    ref mut for_votes,
                    ref mut against_votes,
                    ref mut voters,
                } = self.state
                {
                    if self.time >= voting_start
                        && self.time < voting_ends
                        && !voters.contains(&voter_idx)
                    {
                        voters.insert(voter_idx);
                        if support {
                            *for_votes += VOTER_WEIGHT;
                        } else {
                            *against_votes += VOTER_WEIGHT;
                        }
                    }
                }
            }
            Action::AdvanceTime { seconds } => {
                self.time = self.time.saturating_add(seconds);
            }
            Action::CancelProposal { is_proposer } => {
                if is_proposer {
                    if let ModelProposalState::Active {
                        voting_start,
                        for_votes,
                        against_votes,
                        ..
                    } = self.state
                    {
                        let voting_started = if voting_start > 0 {
                            if self.time < voting_start {
                                false
                            } else {
                                self.time > voting_start || for_votes > 0 || against_votes > 0
                            }
                        } else {
                            for_votes > 0 || against_votes > 0
                        };
                        if !voting_started {
                            self.state = ModelProposalState::Cancelled;
                            self.contract_balance -= BOND_AMOUNT;
                            self.proposer_balance += BOND_AMOUNT;
                        }
                    }
                }
            }
            Action::Execute => match self.state {
                ModelProposalState::Active {
                    voting_ends,
                    for_votes,
                    against_votes,
                    ..
                } => {
                    if self.time >= voting_ends {
                        if for_votes > against_votes {
                            self.state = ModelProposalState::Succeeded {
                                for_votes,
                                against_votes,
                            };
                        } else {
                            self.state = ModelProposalState::Defeated {
                                for_votes,
                                against_votes,
                            };
                            self.contract_balance -= BOND_AMOUNT;
                            self.treasury_balance += BOND_AMOUNT;
                        }
                    }
                }
                ModelProposalState::Succeeded {
                    for_votes,
                    against_votes,
                } => {
                    self.state = ModelProposalState::Executed {
                        for_votes,
                        against_votes,
                    };
                    self.contract_balance -= BOND_AMOUNT;
                    self.proposer_balance += BOND_AMOUNT;
                }
                _ => {}
            },
        }
    }
}

pub struct EnvHarness<'a> {
    pub env: &'a Env,
    pub client: SorobanForgeDaoGovernanceClient<'a>,
    pub accounts: TestAccounts,
    pub token_client: TokenClient<'a>,
    pub target: Address,
}

impl<'a> EnvHarness<'a> {
    pub fn new(env: &'a Env) -> Self {
        env.mock_all_auths();
        env.ledger().set_timestamp(START_TIMESTAMP);

        let admin = Address::generate(env);
        let sac = env.register_stellar_asset_contract_v2(admin);
        let token = sac.address();
        let token_admin = StellarAssetClient::new(env, &token);
        let token_client = TokenClient::new(env, &token);

        let contract_id = env.register(DaoGovernance, ());
        let client = SorobanForgeDaoGovernanceClient::new(env, &contract_id);
        let accounts = TestAccounts::generate(env);

        client.initialize(&token);
        client.configure_bond(&token, &BOND_AMOUNT, &accounts.deployer);
        token_admin.mint(&accounts.user1, &INITIAL_PROPOSER_MINT);
        for who in [&accounts.user2, &accounts.user3, &accounts.validator] {
            token_admin.mint(who, &VOTER_WEIGHT);
        }

        let target = env.register(MockTarget, ());

        Self {
            env,
            client,
            accounts,
            token_client,
            target,
        }
    }

    pub fn voter_address(&self, idx: usize) -> Address {
        match idx % 3 {
            0 => self.accounts.user2.clone(),
            1 => self.accounts.user3.clone(),
            _ => self.accounts.validator.clone(),
        }
    }

    pub fn step(&mut self, action: Action, model: &Model) -> Result<(), String> {
        let payload = Bytes::from_array(self.env, &[0xDE, 0xAD]);
        match action {
            Action::Propose { duration, delay } => {
                if self.client.get_proposal_count() == 0 {
                    let res = self.client.try_propose_with_delay(
                        &self.accounts.user1,
                        &self.target,
                        &payload,
                        &delay,
                        &duration,
                        &SdkVec::new(self.env),
                        &None,
                    );
                    if duration == 0 && res.is_ok() {
                        return Err("propose with 0 duration must fail".into());
                    }
                }
            }
            Action::Vote { voter_idx, support } => {
                if let Some(id) = model.proposal_id {
                    let voter = self.voter_address(voter_idx);
                    let _ = self.client.try_vote(&id, &voter, &support);
                }
            }
            Action::AdvanceTime { seconds: _ } => {
                self.env.ledger().set_timestamp(model.time);
            }
            Action::CancelProposal { is_proposer } => {
                if let Some(id) = model.proposal_id {
                    let caller = if is_proposer {
                        self.accounts.user1.clone()
                    } else {
                        self.accounts.user2.clone()
                    };
                    let _ = self.client.try_cancel_proposal(&id, &caller);
                }
            }
            Action::Execute => {
                if let Some(id) = model.proposal_id {
                    let _ = self.client.try_execute(&id);
                }
            }
        }
        Ok(())
    }

    pub fn assert_matches_model(&self, model: &Model) -> Result<(), String> {
        // Assert bond balances match model exactly
        let proposer_bal = self.token_client.balance(&self.accounts.user1);
        let contract_bal = self.token_client.balance(&self.client.address);
        let treasury_bal = self.token_client.balance(&self.accounts.deployer);

        if proposer_bal != model.proposer_balance {
            return Err(format!(
                "proposer balance mismatch: expected {}, got {}",
                model.proposer_balance, proposer_bal
            ));
        }
        if contract_bal != model.contract_balance {
            return Err(format!(
                "contract balance mismatch: expected {}, got {}",
                model.contract_balance, contract_bal
            ));
        }
        if treasury_bal != model.treasury_balance {
            return Err(format!(
                "treasury balance mismatch: expected {}, got {}",
                model.treasury_balance, treasury_bal
            ));
        }
        // Conservation check
        if proposer_bal + contract_bal + treasury_bal != INITIAL_PROPOSER_MINT {
            return Err("bond conservation violated".into());
        }

        if let Some(id) = model.proposal_id {
            let proposal = self
                .client
                .try_get_proposal(&id)
                .map_err(|_| "failed to get proposal")?
                .map_err(|_| "proposal not found")?;

            match &model.state {
                ModelProposalState::Active {
                    for_votes,
                    against_votes,
                    ..
                } => {
                    if proposal.state != ProposalState::Active {
                        return Err(format!("expected Active, got {:?}", proposal.state));
                    }
                    if proposal.for_votes != *for_votes || proposal.against_votes != *against_votes
                    {
                        return Err(format!(
                            "votes mismatch: expected ({}, {}), got ({}, {})",
                            for_votes, against_votes, proposal.for_votes, proposal.against_votes
                        ));
                    }
                }
                ModelProposalState::Succeeded {
                    for_votes,
                    against_votes,
                } => {
                    if proposal.state != ProposalState::Succeeded {
                        return Err(format!("expected Succeeded, got {:?}", proposal.state));
                    }
                    if proposal.for_votes != *for_votes || proposal.against_votes != *against_votes
                    {
                        return Err("votes mismatch in Succeeded".into());
                    }
                }
                ModelProposalState::Executed {
                    for_votes,
                    against_votes,
                } => {
                    if proposal.state != ProposalState::Executed {
                        return Err(format!("expected Executed, got {:?}", proposal.state));
                    }
                    if proposal.bond_state != BondState::Refunded {
                        return Err("executed proposal must refund bond".into());
                    }
                    if proposal.for_votes != *for_votes || proposal.against_votes != *against_votes
                    {
                        return Err("votes mismatch in Executed".into());
                    }
                }
                ModelProposalState::Defeated {
                    for_votes,
                    against_votes,
                } => {
                    if proposal.state != ProposalState::Defeated {
                        return Err(format!("expected Defeated, got {:?}", proposal.state));
                    }
                    if proposal.bond_state != BondState::Forfeited {
                        return Err("defeated proposal must forfeit bond".into());
                    }
                    if proposal.for_votes != *for_votes || proposal.against_votes != *against_votes
                    {
                        return Err("votes mismatch in Defeated".into());
                    }
                }
                ModelProposalState::Cancelled => {
                    if proposal.state != ProposalState::Cancelled {
                        return Err(format!("expected Cancelled, got {:?}", proposal.state));
                    }
                    if proposal.bond_state != BondState::Refunded {
                        return Err("cancelled proposal must refund bond".into());
                    }
                }
                ModelProposalState::Uncreated => {}
            }
        }
        Ok(())
    }
}

fn arb_action() -> impl Strategy<Value = Action> {
    prop_oneof![
        3 => (1_000u64..100_000, 0u64..10_000).prop_map(|(duration, delay)| Action::Propose { duration, delay }),
        5 => (0usize..3, any::<bool>()).prop_map(|(voter_idx, support)| Action::Vote { voter_idx, support }),
        4 => (10u64..100_000).prop_map(|seconds| Action::AdvanceTime { seconds }),
        2 => any::<bool>().prop_map(|is_proposer| Action::CancelProposal { is_proposer }),
        3 => Just(Action::Execute),
    ]
}

fn format_trace(actions: &[Action]) -> String {
    let mut s = String::new();
    for (i, a) in actions.iter().enumerate() {
        s.push_str(&format!("{}. {}\n", i + 1, a));
    }
    s
}

#[test]
fn p5_proposal_lifecycle_state_machine_fuzz() {
    let mut runner = TestRunner::new(ProptestConfig {
        cases: LIFECYCLE_CASES,
        rng_seed: RngSeed::Fixed(LIFECYCLE_SEED),
        ..ProptestConfig::default()
    });

    let strat = prop::collection::vec(arb_action(), 1..=MAX_STEPS);

    runner
        .run(&strat, |actions| {
            let env = Env::default();
            let mut harness = EnvHarness::new(&env);
            let mut model = Model::new();

            for action in actions {
                model.apply(action);
                harness.step(action, &model).map_err(TestCaseError::fail)?;
                harness
                    .assert_matches_model(&model)
                    .map_err(TestCaseError::fail)?;
            }
            Ok(())
        })
        .expect("state machine model invariant check failed");
}

#[test]
fn failing_lifecycle_case_shrinks_to_minimal_readable_steps() {
    let mut runner = TestRunner::new(ProptestConfig {
        cases: LIFECYCLE_CASES,
        rng_seed: RngSeed::Fixed(LIFECYCLE_SEED),
        ..ProptestConfig::default()
    });

    let strat = prop::collection::vec(arb_action(), 1..=MAX_STEPS);

    // Synthetic failing property: "proposal never reaches Executed"
    let err = runner
        .run(&strat, |actions| {
            let mut model = Model::new();
            for action in actions {
                model.apply(action);
                if matches!(model.state, ModelProposalState::Executed { .. }) {
                    return Err(TestCaseError::fail("executed state reached"));
                }
            }
            Ok(())
        })
        .expect_err("synthetic property must fail and shrink");

    match err {
        TestError::Fail(_, shrunk_steps) => {
            assert!(
                shrunk_steps.len() <= 6,
                "expected minimal shrunk trace (<= 6 steps), got {} steps:\n{}",
                shrunk_steps.len(),
                format_trace(&shrunk_steps)
            );
            let rendered = format_trace(&shrunk_steps);
            assert!(rendered.contains("Propose"));
            assert!(rendered.contains("Execute"));
        }
        TestError::Abort(reason) => panic!("unexpected abort: {reason}"),
    }
}
