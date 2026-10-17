//! Randomized invariant suite (proptest).
//!
//! The hand-written suite in `tests` (lib.rs) pins behaviour on known values;
//! this module tries to *falsify* the governance contract's vote-counting and
//! lifecycle claims over generated inputs. Three properties are exercised:
//!
//! **P1 — Vote tally.** For a generated bounded sequence of vote actions over
//! a fixed voter pool:
//!
//! ```text
//! for_votes + against_votes == sum of balances for distinct voters who successfully voted
//! ```
//!
//! The generator can produce repeated pool indices, which the contract rejects
//! as duplicate votes with `ForgeError::InvalidInput` (not a host abort).
//! The test counts only the actions that actually landed and asserts the tally
//! matches exactly. This proves the contract never double-counts, drops a
//! valid vote, or miscategorises for/against.
//!
//! **P2 — Deadline / lifecycle.** A proposal that has not yet passed its
//! `voting_ends` timestamp must remain `Active` and must not be finalisable:
//! `execute` before the deadline returns `ForgeError::InvalidInput` and the
//! proposal stays `Active`. The deadline is exercised using the actual
//! `Ledger::set_timestamp` mechanism used by the existing tests.
//!
//! **Soroban host limitation note.** The Soroban test environment has no
//! automatic time advancement; only explicit `set_timestamp` calls move the
//! clock. There is no way to exercise a "voting window that closes mid-
//! sequence" without controlling time from test code. The invariant therefore
//! probes the exact boundary: at `voting_ends - 1` the proposal is still
//! `Active` and `execute` is rejected; the post-deadline path is covered
//! comprehensively by P3. This mirrors the approach used by the existing
//! governance tests in lib.rs.
//!
//! **P3 — Finalization outcome.** After voting ends, `execute` must produce:
//!
//! ```text
//! for_votes > against_votes → Succeeded
//! for_votes <= against_votes → Defeated   (includes ties and zero-vote proposals)
//! ```
//!
//! Specifically:
//! - 0 for / 0 against → `Defeated`
//! - equal for / against → `Defeated`
//! - strict for majority → `Succeeded`
//! - strict against majority → `Defeated`
//!
//! Four deterministic companion unit tests pin each boundary case explicitly
//! (see end of file), complementing the randomized P3 suite.
//!
//! All properties run against a real Stellar Asset Contract for bond custody.
//! Runs are deterministic (fixed strategy bounds, proptest's default seed);
//! a failure prints its case seed for replay. Override the case count with
//! `PROPTEST_CASES=n cargo test -p soroban-forge-dao-governance props`.

use crate::{DaoGovernance, ProposalState, SorobanForgeDaoGovernanceClient};
use proptest::prelude::*;
use soroban_forge_shared_utils::ForgeError;
use soroban_forge_test_utils::{MockTarget, TestAccounts};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Bytes, Env};

const START: u64 = 1_000_000;
const DURATION: u64 = 86_400;
const BOND: i128 = 100;
const FUNDS: i128 = 10_000;

// The fixed pool of voter slots available for property tests. A voter is
// selected by index, so the generator can produce both unique and repeated
// voter picks, proving the tally is exact regardless of how many duplicates
// occur.
const VOTER_POOL_SIZE: usize = 8;

// -----------------------------------------------------------------------
// World: one fresh env, SAC, DAO contract, target, and funded accounts
// -----------------------------------------------------------------------

struct World {
    env: Env,
    contract_id: Address,
    accounts: TestAccounts,
    target: Address,
    /// Fixed pool of extra voters beyond the TestAccounts set.
    voters: std::vec::Vec<Address>,
}

fn setup_world() -> World {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);

    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin);
    let token = sac.address();
    let token_admin = StellarAssetClient::new(&env, &token);

    let contract_id = env.register(DaoGovernance, ());
    let client = SorobanForgeDaoGovernanceClient::new(&env, &contract_id);

    let accounts = TestAccounts::generate(&env);
    client.configure_bond(&token, &BOND, &accounts.deployer);
    client.initialize(&token);
    token_admin.mint(&accounts.user1, &FUNDS);

    let mut voters = std::vec::Vec::with_capacity(VOTER_POOL_SIZE);
    for _ in 0..VOTER_POOL_SIZE {
        let voter = Address::generate(&env);
        token_admin.mint(&voter, &FUNDS);
        voters.push(voter);
    }

    let target = env.register(MockTarget, ());

    World {
        env,
        contract_id,
        accounts,
        target,
        voters,
    }
}

impl World {
    fn client(&self) -> SorobanForgeDaoGovernanceClient<'_> {
        SorobanForgeDaoGovernanceClient::new(&self.env, &self.contract_id)
    }

    fn payload(&self) -> Bytes {
        Bytes::from_array(&self.env, &[0xC0, 0xDE, 0x00, 0xFF])
    }

    /// Create a fresh proposal from `user1` and return its id.
    fn propose(&self) -> u64 {
        self.client().propose(
            &self.accounts.user1,
            &self.target,
            &self.payload(),
            &DURATION,
            &soroban_sdk::Vec::new(&self.env),
            &None,
        )
    }

    /// Return the voter at `idx % VOTER_POOL_SIZE` from the pool.
    fn voter(&self, idx: usize) -> &Address {
        &self.voters[idx % VOTER_POOL_SIZE]
    }
}

// -----------------------------------------------------------------------
// Strategies
// -----------------------------------------------------------------------

/// A single vote action: (voter_pool_index, support).
fn vote_action() -> impl Strategy<Value = (usize, bool)> {
    (0usize..VOTER_POOL_SIZE, prop::bool::ANY)
}

/// A bounded sequence of up to 16 vote actions (unique and repeated picks).
fn vote_sequence() -> impl Strategy<Value = std::vec::Vec<(usize, bool)>> {
    prop::collection::vec(vote_action(), 0..=16)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Random lower-id edges form DAGs. The independent mirror tracks each
    /// proposal's Active/Succeeded/Executed phase and permits dispatch only
    /// after every required id executed, regardless of caller order.
    #[test]
    fn p4_random_dependency_dags_match_execution_mirror(
        masks in prop::collection::vec(any::<u8>(), 1..=5),
        order in prop::collection::vec(any::<u8>(), 0..=40),
    ) {
        let w = setup_world();
        let n = masks.len();
        let mut edges: std::vec::Vec<std::vec::Vec<usize>> = std::vec::Vec::new();
        let mut ids = std::vec::Vec::new();
        for (idx, mask) in masks.iter().enumerate() {
            let requires: std::vec::Vec<usize> = (0..idx).filter(|parent| mask & (1 << parent) != 0).collect();
            let mut sdk_requires = soroban_sdk::Vec::new(&w.env);
            for parent in &requires { sdk_requires.push_back(ids[*parent]); }
            let id = w.client().propose(&w.accounts.user1, &w.target, &w.payload(), &DURATION, &sdk_requires, &None);
            w.client().vote(&id, &w.accounts.user1, &true);
            ids.push(id);
            edges.push(requires);
        }
        w.env.ledger().set_timestamp(START + DURATION + 1);
        let mut mirror = std::vec![0u8; n]; // 0 Active, 1 Succeeded, 2 Executed
        let mut sequence: std::vec::Vec<usize> = order.iter().map(|pick| usize::from(*pick) % n).collect();
        sequence.extend(0..n);
        sequence.extend(0..n);
        for idx in sequence {
            let deps_ready = edges[idx].iter().all(|parent| mirror[*parent] == 2);
            match mirror[idx] {
                0 => {
                    w.client().execute(&ids[idx]);
                    mirror[idx] = 1;
                }
                1 if deps_ready => {
                    w.client().execute(&ids[idx]);
                    mirror[idx] = 2;
                }
                1 => {
                    prop_assert_eq!(w.client().try_execute(&ids[idx]).unwrap_err().unwrap(), ForgeError::DeadlineReached);
                }
                _ => {
                    prop_assert_eq!(w.client().try_execute(&ids[idx]).unwrap_err().unwrap(), ForgeError::InvalidInput);
                }
            }
            let actual = w.client().get_proposal(&ids[idx]).state;
            let expected = match mirror[idx] { 0 => ProposalState::Active, 1 => ProposalState::Succeeded, _ => ProposalState::Executed };
            prop_assert_eq!(actual, expected);
        }
        prop_assert!(mirror.iter().all(|state| *state == 2));
    }
}

// -----------------------------------------------------------------------
// P1 — Vote tally invariant
// -----------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// For any bounded sequence of vote actions over a fixed voter pool:
    ///
    /// ```text
    /// for_votes + against_votes == distinct voters who successfully voted
    /// ```
    ///
    /// Duplicate picks return `ForgeError::InvalidInput` without touching the
    /// tally. The test tracks which pool slots have voted and asserts the
    /// counts match exactly — proving the contract never double-counts, drops
    /// a valid vote, or miscategorises for/against.
    #[test]
    fn p1_vote_tally_equals_distinct_successful_voters(
        actions in vote_sequence(),
    ) {
        let w = setup_world();
        let proposal_id = w.propose();

        let mut voted: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
        let mut expected_for: i128 = 0;
        let mut expected_against: i128 = 0;

        for (idx, support) in &actions {
            let voter = w.voter(*idx);
            let res = w.client().try_vote(&proposal_id, voter, support);

            match res {
                Ok(Ok(())) => {
                    // First vote from this pool slot — must count.
                    prop_assert!(
                        voted.insert(*idx),
                        "contract accepted a second vote from voter slot {idx}"
                    );
                    if *support {
                        expected_for += FUNDS;
                    } else {
                        expected_against += FUNDS;
                    }
                }
                Err(Ok(ForgeError::InvalidInput)) => {
                    // Duplicate vote: the contract correctly rejected it.
                    // The voter must already be in the voted set.
                    prop_assert!(
                        voted.contains(idx),
                        "contract rejected a first-time vote from voter slot {idx} with InvalidInput"
                    );
                }
                other => {
                    prop_assert!(
                        false,
                        "unexpected vote result: {:?}", other
                    );
                }
            }
        }

        let proposal = w.client().get_proposal(&proposal_id);
        prop_assert_eq!(
            proposal.for_votes,
            expected_for,
            "for_votes mismatch: expected {}",
            expected_for
        );
        prop_assert_eq!(
            proposal.against_votes,
            expected_against,
            "against_votes mismatch: expected {}",
            expected_against
        );
        prop_assert_eq!(
            proposal.for_votes + proposal.against_votes,
            voted.len() as i128 * FUNDS,
            "total votes must equal the weights of distinct successful voters"
        );
    }
}

// -----------------------------------------------------------------------
// P2 — Deadline / lifecycle invariant
// -----------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// A proposal that has not yet reached its `voting_ends` timestamp must
    /// remain `Active` and must not be finalisable.
    ///
    /// The test sets the ledger timestamp to `voting_ends - 1` (one second
    /// before the deadline), confirms the state is still `Active`, and
    /// confirms that `execute` is rejected with `ForgeError::InvalidInput`.
    ///
    /// **Limitation (Soroban host):** the Soroban test environment has no
    /// automatic time advancement; only explicit `set_timestamp` calls move
    /// the clock. The test therefore probes the exact pre-deadline boundary
    /// rather than simulating mid-sequence expiry. The post-deadline path is
    /// covered comprehensively by P3.
    #[test]
    fn p2_proposal_stays_active_before_deadline(
        actions in vote_sequence(),
    ) {
        let w = setup_world();
        let proposal_id = w.propose();

        // Cast votes while the voting window is still open (timestamp is START,
        // well before START + DURATION).
        for (idx, support) in &actions {
            let voter = w.voter(*idx);
            // Duplicates return InvalidInput; that is expected and fine here.
            let _ = w.client().try_vote(&proposal_id, voter, support);
        }

        // Advance to one second before the deadline — still within the window.
        w.env.ledger().set_timestamp(START + DURATION - 1);
        let proposal = w.client().get_proposal(&proposal_id);
        prop_assert_eq!(
            proposal.state,
            ProposalState::Active,
            "proposal must remain Active before voting_ends"
        );

        // execute before the deadline must be rejected with InvalidInput.
        let res = w.client().try_execute(&proposal_id);
        prop_assert!(
            matches!(res, Err(Ok(ForgeError::InvalidInput))),
            "execute before deadline must return InvalidInput, got {:?}", res
        );

        // The state must be unchanged after the rejected execute.
        let proposal = w.client().get_proposal(&proposal_id);
        prop_assert_eq!(
            proposal.state,
            ProposalState::Active,
            "proposal must remain Active after a rejected pre-deadline execute"
        );
    }
}

// -----------------------------------------------------------------------
// P3 — Finalization outcome invariant
// -----------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// After voting ends, `execute` must transition the proposal to the
    /// correct terminal state:
    ///
    /// ```text
    /// for_votes > against_votes → Succeeded
    /// for_votes <= against_votes → Defeated   (includes ties and zero votes)
    /// ```
    ///
    /// The zero-vote case (0 for / 0 against → Defeated) and the tie case
    /// (equal for / equal against → Defeated) are both reachable by the
    /// generator and asserted explicitly in the deterministic supplement below.
    #[test]
    fn p3_finalization_outcome_matches_vote_tally(
        actions in vote_sequence(),
    ) {
        let w = setup_world();
        let proposal_id = w.propose();

        // Cast votes.
        for (idx, support) in &actions {
            let voter = w.voter(*idx);
            // Duplicates return InvalidInput; ignore them.
            let _ = w.client().try_vote(&proposal_id, voter, support);
        }

        // Snapshot the final tally before advancing time.
        let snapshot = w.client().get_proposal(&proposal_id);
        let for_votes = snapshot.for_votes;
        let against_votes = snapshot.against_votes;

        // Advance past the deadline so execute is allowed.
        w.env.ledger().set_timestamp(START + DURATION + 1);

        let res = w.client().try_execute(&proposal_id);
        prop_assert!(
            matches!(res, Ok(Ok(()))),
            "execute after deadline must succeed, got {:?}", res
        );

        let after = w.client().get_proposal(&proposal_id);
        let expected_state = if for_votes > against_votes {
            ProposalState::Succeeded
        } else {
            ProposalState::Defeated
        };
        prop_assert_eq!(
            after.state,
            expected_state,
            "finalization: for={} against={}",
            for_votes,
            against_votes
        );
    }
}

// -----------------------------------------------------------------------
// P3 supplement — exhaustive boundary cases (deterministic)
// -----------------------------------------------------------------------
// These four unit tests are deterministic complements to the P3 proptest
// suite, pinning each corner case the issue requires explicitly:
//
//   0 for / 0 against → Defeated
//   equal for / against → Defeated
//   strict for majority → Succeeded
//   strict against majority → Defeated

#[test]
fn p3_zero_votes_is_defeated() {
    let w = setup_world();
    let proposal_id = w.propose();
    // No votes cast.
    w.env.ledger().set_timestamp(START + DURATION + 1);
    w.client().execute(&proposal_id);
    assert_eq!(
        w.client().get_proposal(&proposal_id).state,
        ProposalState::Defeated,
        "0 for / 0 against must produce Defeated"
    );
}

#[test]
fn p3_tie_is_defeated() {
    let w = setup_world();
    let proposal_id = w.propose();
    w.client().vote(&proposal_id, w.voter(0), &true);
    w.client().vote(&proposal_id, w.voter(1), &false);
    w.env.ledger().set_timestamp(START + DURATION + 1);
    w.client().execute(&proposal_id);
    assert_eq!(
        w.client().get_proposal(&proposal_id).state,
        ProposalState::Defeated,
        "1 for / 1 against (tie) must produce Defeated"
    );
}

#[test]
fn p3_for_majority_is_succeeded() {
    let w = setup_world();
    let proposal_id = w.propose();
    w.client().vote(&proposal_id, w.voter(0), &true);
    w.client().vote(&proposal_id, w.voter(1), &true);
    w.client().vote(&proposal_id, w.voter(2), &false);
    w.env.ledger().set_timestamp(START + DURATION + 1);
    w.client().execute(&proposal_id);
    assert_eq!(
        w.client().get_proposal(&proposal_id).state,
        ProposalState::Succeeded,
        "2 for / 1 against must produce Succeeded"
    );
}

#[test]
fn p3_against_majority_is_defeated() {
    let w = setup_world();
    let proposal_id = w.propose();
    w.client().vote(&proposal_id, w.voter(0), &true);
    w.client().vote(&proposal_id, w.voter(1), &false);
    w.client().vote(&proposal_id, w.voter(2), &false);
    w.env.ledger().set_timestamp(START + DURATION + 1);
    w.client().execute(&proposal_id);
    assert_eq!(
        w.client().get_proposal(&proposal_id).state,
        ProposalState::Defeated,
        "1 for / 2 against must produce Defeated"
    );
}

// -----------------------------------------------------------------------
// P5 — Proposal expiry, quorum boundary, retryable dispatch, and terminal
// state invariants (#438)
// -----------------------------------------------------------------------

struct BoundaryEnv {
    env: Env,
    token: Address,
    contract_id: Address,
    accounts: TestAccounts,
    target: Address,
}

fn setup_boundary_env() -> BoundaryEnv {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);

    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin);
    let token = sac.address();
    let token_admin = StellarAssetClient::new(&env, &token);

    let contract_id = env.register(DaoGovernance, ());
    let client = SorobanForgeDaoGovernanceClient::new(&env, &contract_id);

    let accounts = TestAccounts::generate(&env);
    client.configure_bond(&token, &BOND, &accounts.deployer);
    client.initialize(&token);
    token_admin.mint(&accounts.user1, &FUNDS);

    let target = env.register(MockTarget, ());

    BoundaryEnv {
        env,
        token,
        contract_id,
        accounts,
        target,
    }
}

impl BoundaryEnv {
    fn client(&self) -> SorobanForgeDaoGovernanceClient<'_> {
        SorobanForgeDaoGovernanceClient::new(&self.env, &self.contract_id)
    }

    fn token_client(&self) -> soroban_sdk::token::Client<'_> {
        soroban_sdk::token::Client::new(&self.env, &self.token)
    }

    fn token_admin(&self) -> StellarAssetClient<'_> {
        StellarAssetClient::new(&self.env, &self.token)
    }

    fn target_client(&self) -> soroban_forge_test_utils::MockTargetClient<'_> {
        soroban_forge_test_utils::MockTargetClient::new(&self.env, &self.target)
    }

    fn payload(&self) -> Bytes {
        Bytes::from_array(&self.env, &[0xCA, 0xFE, 0xBA, 0xBE])
    }

    fn propose(&self) -> u64 {
        self.client().propose(
            &self.accounts.user1,
            &self.target,
            &self.payload(),
            &DURATION,
            &soroban_sdk::Vec::new(&self.env),
            &None,
        )
    }
}

#[test]
fn quorum_threshold_below_at_and_above_boundary_invariants() {
    let against_weight = 500_i128;

    // 1. Below required threshold (`for_votes == against_votes - 1`) -> Defeated.
    {
        let b = setup_boundary_env();
        let id = b.propose();
        let for_voter = Address::generate(&b.env);
        let against_voter = Address::generate(&b.env);
        b.token_admin().mint(&for_voter, &(against_weight - 1));
        b.token_admin().mint(&against_voter, &against_weight);

        b.client().vote(&id, &for_voter, &true);
        b.client().vote(&id, &against_voter, &false);

        b.env.ledger().set_timestamp(START + DURATION);
        b.client().execute(&id);

        let proposal = b.client().get_proposal(&id);
        assert_eq!(proposal.for_votes, against_weight - 1);
        assert_eq!(proposal.against_votes, against_weight);
        assert_eq!(proposal.state, ProposalState::Defeated);
        assert_eq!(proposal.bond_state, crate::BondState::Forfeited);
        assert_eq!(b.target_client().count(), 0);
        assert_eq!(b.token_client().balance(&b.accounts.deployer), BOND);
    }

    // 2. Exactly at threshold / tie (`for_votes == against_votes`) -> Defeated.
    {
        let b = setup_boundary_env();
        let id = b.propose();
        let for_voter = Address::generate(&b.env);
        let against_voter = Address::generate(&b.env);
        b.token_admin().mint(&for_voter, &against_weight);
        b.token_admin().mint(&against_voter, &against_weight);

        b.client().vote(&id, &for_voter, &true);
        b.client().vote(&id, &against_voter, &false);

        b.env.ledger().set_timestamp(START + DURATION);
        b.client().execute(&id);

        let proposal = b.client().get_proposal(&id);
        assert_eq!(proposal.for_votes, against_weight);
        assert_eq!(proposal.against_votes, against_weight);
        assert_eq!(proposal.state, ProposalState::Defeated);
        assert_eq!(proposal.bond_state, crate::BondState::Forfeited);
        assert_eq!(b.target_client().count(), 0);
        assert_eq!(b.token_client().balance(&b.accounts.deployer), BOND);
    }

    // 3. Strictly above threshold (`for_votes == against_votes + 1`) -> Succeeded -> Executed.
    {
        let b = setup_boundary_env();
        let id = b.propose();
        let for_voter = Address::generate(&b.env);
        let against_voter = Address::generate(&b.env);
        b.token_admin().mint(&for_voter, &(against_weight + 1));
        b.token_admin().mint(&against_voter, &against_weight);

        b.client().vote(&id, &for_voter, &true);
        b.client().vote(&id, &against_voter, &false);

        b.env.ledger().set_timestamp(START + DURATION);
        b.client().execute(&id);

        let succeeded = b.client().get_proposal(&id);
        assert_eq!(succeeded.for_votes, against_weight + 1);
        assert_eq!(succeeded.against_votes, against_weight);
        assert_eq!(succeeded.state, ProposalState::Succeeded);
        assert_eq!(succeeded.bond_state, crate::BondState::Posted);
        assert_eq!(b.target_client().count(), 0);

        b.client().execute(&id);
        let executed = b.client().get_proposal(&id);
        assert_eq!(executed.state, ProposalState::Executed);
        assert_eq!(executed.bond_state, crate::BondState::Refunded);
        assert_eq!(b.target_client().count(), 1);
        assert_eq!(b.token_client().balance(&b.accounts.user1), FUNDS);
    }
}

#[test]
fn voting_window_immediately_before_at_and_after_deadline_invariants() {
    let b = setup_boundary_env();
    let id = b.propose();
    let voting_ends = b.client().get_proposal(&id).voting_ends;
    assert_eq!(voting_ends, START + DURATION);

    let voter_before = Address::generate(&b.env);
    let voter_at = Address::generate(&b.env);
    let voter_after = Address::generate(&b.env);
    b.token_admin().mint(&voter_before, &300);
    b.token_admin().mint(&voter_at, &400);
    b.token_admin().mint(&voter_after, &500);

    // 1. Immediately before deadline (`voting_ends - 1`): vote is accepted, execute is rejected.
    b.env.ledger().set_timestamp(voting_ends - 1);
    b.client().vote(&id, &voter_before, &true);
    let snapshot_before = b.client().get_proposal(&id);
    assert_eq!(snapshot_before.for_votes, 300);
    assert_eq!(snapshot_before.state, ProposalState::Active);
    assert!(b.client().has_voted(&id, &voter_before));

    let early_exec_err = b.client().try_execute(&id).unwrap_err().unwrap();
    assert_eq!(early_exec_err, ForgeError::InvalidInput);
    assert_eq!(b.client().get_proposal(&id), snapshot_before);

    // 2. Exactly at deadline (`voting_ends`): vote is rejected with DeadlineReached and state is unchanged.
    b.env.ledger().set_timestamp(voting_ends);
    let at_err = b
        .client()
        .try_vote(&id, &voter_at, &true)
        .unwrap_err()
        .unwrap();
    assert_eq!(at_err, ForgeError::DeadlineReached);
    assert_eq!(b.client().get_proposal(&id), snapshot_before);
    assert!(!b.client().has_voted(&id, &voter_at));

    // 3. Immediately after deadline (`voting_ends + 1`): vote is rejected with DeadlineReached and state is unchanged.
    b.env.ledger().set_timestamp(voting_ends + 1);
    let after_err = b
        .client()
        .try_vote(&id, &voter_after, &false)
        .unwrap_err()
        .unwrap();
    assert_eq!(after_err, ForgeError::DeadlineReached);
    assert_eq!(b.client().get_proposal(&id), snapshot_before);
    assert!(!b.client().has_voted(&id, &voter_after));

    // Finalizing after the deadline succeeds using only the pre-deadline vote.
    b.client().execute(&id);
    assert_eq!(b.client().get_proposal(&id).state, ProposalState::Succeeded);
}

#[test]
fn expired_and_rejected_proposals_never_execute_target_or_reexecute() {
    // Case A: Expired with zero votes.
    let b = setup_boundary_env();
    let expired_id = b.propose();
    b.env.ledger().set_timestamp(START + DURATION);
    b.client().execute(&expired_id);

    let defeated_expired = b.client().get_proposal(&expired_id);
    assert_eq!(defeated_expired.state, ProposalState::Defeated);
    assert_eq!(defeated_expired.bond_state, crate::BondState::Forfeited);
    assert_eq!(b.target_client().count(), 0);

    // Re-executing an expired/defeated proposal is rejected and leaves state unchanged.
    let reexec_expired = b.client().try_execute(&expired_id).unwrap_err().unwrap();
    assert_eq!(reexec_expired, ForgeError::InvalidInput);
    assert_eq!(b.client().get_proposal(&expired_id), defeated_expired);
    assert_eq!(b.target_client().count(), 0);

    // Case B: Rejected by majority against votes.
    let rejected_id = b.propose();
    let voter_for = Address::generate(&b.env);
    let voter_against = Address::generate(&b.env);
    b.token_admin().mint(&voter_for, &200);
    b.token_admin().mint(&voter_against, &600);
    b.client().vote(&rejected_id, &voter_for, &true);
    b.client().vote(&rejected_id, &voter_against, &false);

    b.env.ledger().set_timestamp(START + DURATION * 2);
    b.client().execute(&rejected_id);

    let defeated_rejected = b.client().get_proposal(&rejected_id);
    assert_eq!(defeated_rejected.state, ProposalState::Defeated);
    assert_eq!(defeated_rejected.bond_state, crate::BondState::Forfeited);
    assert_eq!(b.target_client().count(), 0);

    let reexec_rejected = b.client().try_execute(&rejected_id).unwrap_err().unwrap();
    assert_eq!(reexec_rejected, ForgeError::InvalidInput);
    assert_eq!(b.client().get_proposal(&rejected_id), defeated_rejected);
    assert_eq!(b.target_client().count(), 0);
}

#[test]
fn retryable_dispatch_failures_preserve_succeeded_state_and_allow_retry() {
    let b = setup_boundary_env();
    let id = b.propose();
    let voter = Address::generate(&b.env);
    b.token_admin().mint(&voter, &500);
    b.client().vote(&id, &voter, &true);

    b.env.ledger().set_timestamp(START + DURATION + 1);
    b.client().execute(&id);

    let succeeded_snapshot = b.client().get_proposal(&id);
    assert_eq!(succeeded_snapshot.state, ProposalState::Succeeded);
    assert_eq!(succeeded_snapshot.bond_state, crate::BondState::Posted);

    // Force target revert and verify repeated dispatch attempts fail closed without state drift.
    b.target_client().set_revert(&true);
    for _ in 0..2 {
        let err = b.client().try_execute(&id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::ContractInvocationFailed);
        assert_eq!(b.client().get_proposal(&id), succeeded_snapshot);
        assert_eq!(b.target_client().count(), 0);
        assert_eq!(b.target_client().last_payload(), None);
        assert_eq!(b.client().get_active_proposal_count(&b.accounts.user1), 1);
        assert_eq!(b.token_client().balance(&b.contract_id), BOND);
        assert_eq!(b.token_client().balance(&b.accounts.user1), FUNDS - BOND);
    }

    // Clear target revert and verify retry succeeds cleanly.
    b.target_client().set_revert(&false);
    b.client().execute(&id);

    let executed = b.client().get_proposal(&id);
    assert_eq!(executed.state, ProposalState::Executed);
    assert_eq!(executed.bond_state, crate::BondState::Refunded);
    assert_eq!(b.target_client().count(), 1);
    assert_eq!(b.target_client().last_payload(), Some(b.payload()));
    assert_eq!(b.client().get_active_proposal_count(&b.accounts.user1), 0);
    assert_eq!(b.token_client().balance(&b.contract_id), 0);
    assert_eq!(b.token_client().balance(&b.accounts.user1), FUNDS);
}

#[test]
fn terminal_proposal_states_reject_reexecution_and_mutations_without_state_drift() {
    let b = setup_boundary_env();
    let voter = Address::generate(&b.env);
    let late_voter = Address::generate(&b.env);
    b.token_admin().mint(&voter, &500);
    b.token_admin().mint(&late_voter, &500);

    // 1. Cancelled (terminal)
    let cancelled_id = b.propose();
    b.client().cancel_proposal(&cancelled_id, &b.accounts.user1);
    let cancelled_snapshot = b.client().get_proposal(&cancelled_id);
    assert_eq!(cancelled_snapshot.state, ProposalState::Cancelled);

    // 2. Defeated (terminal)
    let defeated_id = b.propose();
    b.client().vote(&defeated_id, &voter, &false);

    // 3. Executed (terminal)
    let executed_id = b.propose();
    b.client().vote(&executed_id, &voter, &true);

    b.env.ledger().set_timestamp(START + DURATION + 1);
    b.client().execute(&defeated_id);
    b.client().execute(&executed_id); // Active -> Succeeded
    b.client().execute(&executed_id); // Succeeded -> Executed

    let defeated_snapshot = b.client().get_proposal(&defeated_id);
    let executed_snapshot = b.client().get_proposal(&executed_id);
    assert_eq!(defeated_snapshot.state, ProposalState::Defeated);
    assert_eq!(executed_snapshot.state, ProposalState::Executed);

    let target_count_before = b.target_client().count();
    let proposer_bal_before = b.token_client().balance(&b.accounts.user1);
    let treasury_bal_before = b.token_client().balance(&b.accounts.deployer);
    let custody_bal_before = b.token_client().balance(&b.contract_id);

    for (id, snapshot) in [
        (cancelled_id, &cancelled_snapshot),
        (defeated_id, &defeated_snapshot),
        (executed_id, &executed_snapshot),
    ] {
        assert_eq!(
            b.client().try_execute(&id).unwrap_err().unwrap(),
            ForgeError::InvalidInput
        );
        assert_eq!(
            b.client()
                .try_vote(&id, &late_voter, &true)
                .unwrap_err()
                .unwrap(),
            ForgeError::InvalidInput
        );
        assert_eq!(
            b.client()
                .try_cancel_proposal(&id, &b.accounts.user1)
                .unwrap_err()
                .unwrap(),
            ForgeError::InvalidInput
        );
        assert_eq!(&b.client().get_proposal(&id), snapshot);
        assert!(!b.client().has_voted(&id, &late_voter));
    }

    assert_eq!(b.target_client().count(), target_count_before);
    assert_eq!(
        b.token_client().balance(&b.accounts.user1),
        proposer_bal_before
    );
    assert_eq!(
        b.token_client().balance(&b.accounts.deployer),
        treasury_bal_before
    );
    assert_eq!(b.token_client().balance(&b.contract_id), custody_bal_before);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn p5_quorum_expiry_and_retryable_execution_boundary_invariants(
        for_weight in 0i128..=2_000_i128,
        against_weight in 0i128..=2_000_i128,
        revert_attempts in 0u8..=3u8,
    ) {
        let b = setup_boundary_env();
        let id = b.propose();
        let voting_ends = START + DURATION;

        if for_weight > 0 {
            let v_for = Address::generate(&b.env);
            b.token_admin().mint(&v_for, &for_weight);
            b.env.ledger().set_timestamp(voting_ends - 1);
            b.client().vote(&id, &v_for, &true);
        }
        if against_weight > 0 {
            let v_against = Address::generate(&b.env);
            b.token_admin().mint(&v_against, &against_weight);
            b.env.ledger().set_timestamp(voting_ends - 1);
            b.client().vote(&id, &v_against, &false);
        }

        // Pre-deadline execute at `voting_ends - 1` must fail without mutating proposal state.
        b.env.ledger().set_timestamp(voting_ends - 1);
        let pre_deadline = b.client().get_proposal(&id);
        prop_assert_eq!(
            b.client().try_execute(&id).unwrap_err().unwrap(),
            ForgeError::InvalidInput
        );
        prop_assert_eq!(b.client().get_proposal(&id), pre_deadline.clone());

        // Voting at exact `voting_ends` must fail with DeadlineReached and preserve tallies.
        b.env.ledger().set_timestamp(voting_ends);
        let late_voter = Address::generate(&b.env);
        b.token_admin().mint(&late_voter, &100);
        prop_assert_eq!(
            b.client().try_vote(&id, &late_voter, &true).unwrap_err().unwrap(),
            ForgeError::DeadlineReached
        );
        prop_assert_eq!(b.client().get_proposal(&id), pre_deadline);

        // Finalize at exact `voting_ends`.
        b.client().execute(&id);
        let finalized = b.client().get_proposal(&id);

        if for_weight > against_weight {
            prop_assert_eq!(finalized.state.clone(), ProposalState::Succeeded);
            prop_assert_eq!(finalized.bond_state.clone(), crate::BondState::Posted);

            b.target_client().set_revert(&true);
            for _ in 0..revert_attempts {
                prop_assert_eq!(
                    b.client().try_execute(&id).unwrap_err().unwrap(),
                    ForgeError::ContractInvocationFailed
                );
                prop_assert_eq!(b.client().get_proposal(&id), finalized.clone());
                prop_assert_eq!(b.target_client().count(), 0);
            }

            b.target_client().set_revert(&false);
            b.client().execute(&id);
            let executed = b.client().get_proposal(&id);
            prop_assert_eq!(executed.state.clone(), ProposalState::Executed);
            prop_assert_eq!(executed.bond_state.clone(), crate::BondState::Refunded);
            prop_assert_eq!(b.target_client().count(), 1);

            // Terminal Executed state rejects re-execution and mutation.
            prop_assert_eq!(
                b.client().try_execute(&id).unwrap_err().unwrap(),
                ForgeError::InvalidInput
            );
            prop_assert_eq!(b.client().get_proposal(&id), executed);
        } else {
            prop_assert_eq!(finalized.state.clone(), ProposalState::Defeated);
            prop_assert_eq!(finalized.bond_state.clone(), crate::BondState::Forfeited);
            prop_assert_eq!(b.target_client().count(), 0);

            // Terminal Defeated state rejects re-execution and mutation.
            prop_assert_eq!(
                b.client().try_execute(&id).unwrap_err().unwrap(),
                ForgeError::InvalidInput
            );
            prop_assert_eq!(b.client().get_proposal(&id), finalized);
        }
    }
}
