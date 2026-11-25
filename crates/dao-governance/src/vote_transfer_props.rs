use crate::{DaoGovernance, ProposalState, SorobanForgeDaoGovernanceClient};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use soroban_forge_shared_utils::ForgeError;
use soroban_forge_test_utils::MockTarget;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{StellarAssetClient, TokenClient};
use soroban_sdk::{Address, Bytes, Env, Vec};

const VOTERS: usize = 4;
const PROPOSALS: usize = 3;
const DURATION: u64 = 86_400;
const INITIAL_BALANCE: i128 = 20;

#[derive(Clone, Debug)]
enum Action {
    Transfer(usize, usize, i128),
    Vote(usize, usize, bool),
}

fn actions() -> impl Strategy<Value = std::vec::Vec<Action>> {
    prop::collection::vec(
        prop_oneof![
            (0..VOTERS, 1..VOTERS, 1i128..=INITIAL_BALANCE).prop_map(|(from, distance, amount)| {
                Action::Transfer(from, (from + distance) % VOTERS, amount)
            }),
            (0..PROPOSALS, 0..VOTERS, any::<bool>())
                .prop_map(|(proposal, voter, support)| Action::Vote(proposal, voter, support)),
        ],
        8..=24,
    )
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 16,
        rng_seed: RngSeed::Fixed(710),
        ..ProptestConfig::default()
    })]

    #[test]
    fn tallies_match_balance_snapshots_under_transfers(steps in actions()) {
        let env = Env::default();
        env.mock_all_auths();
        let proposer = Address::generate(&env);
        let treasury = Address::generate(&env);
        let token = env.register_stellar_asset_contract_v2(proposer.clone()).address();
        let mint = StellarAssetClient::new(&env, &token);
        let token_client = TokenClient::new(&env, &token);
        let voters: [Address; VOTERS] = core::array::from_fn(|_| Address::generate(&env));
        mint.mint(&proposer, &(PROPOSALS as i128));
        for voter in &voters {
            mint.mint(voter, &INITIAL_BALANCE);
        }
        let dao = env.register(DaoGovernance, ());
        let client = SorobanForgeDaoGovernanceClient::new(&env, &dao);
        client.configure_bond(&token, &1, &treasury);
        client.initialize(&token);
        let target = env.register(MockTarget, ());
        let proposals: [u64; PROPOSALS] = core::array::from_fn(|_| {
            client.propose(
                &proposer,
                &target,
                &Bytes::from_array(&env, &[1]),
                &DURATION,
                &Vec::new(&env),
                &None,
            )
        });

        let mut balances = [INITIAL_BALANCE; VOTERS];
        let mut receipts = [[None; VOTERS]; PROPOSALS];
        let mut tallies = [[0i128; 2]; PROPOSALS];
        for action in steps {
            match action {
                Action::Transfer(from, to, amount) => {
                    let amount = amount.min(balances[from]);
                    if amount > 0 {
                        token_client.transfer(&voters[from], &voters[to], &amount);
                        balances[from] -= amount;
                        balances[to] += amount;
                    }
                }
                Action::Vote(proposal, voter, support) => {
                    let result = client.try_vote(&proposals[proposal], &voters[voter], &support);
                    let weight = balances[voter];
                    if receipts[proposal][voter].is_some() || weight == 0 {
                        prop_assert_eq!(result, Err(Ok(ForgeError::InvalidInput)));
                    } else {
                        prop_assert_eq!(result, Ok(Ok(())));
                        receipts[proposal][voter] = Some((support, weight));
                    }
                }
            }
            prop_assert_eq!(balances.iter().sum::<i128>(), INITIAL_BALANCE * VOTERS as i128);
            for (voter, balance) in voters.iter().zip(balances) {
                prop_assert_eq!(token_client.balance(voter), balance);
            }
            for (index, proposal) in proposals.iter().enumerate() {
                let mut tally = [0; 2];
                for (voter, expected) in voters.iter().zip(receipts[index]) {
                    let receipt = client.get_vote_receipt(proposal, voter);
                    prop_assert_eq!(receipt.has_voted, expected.is_some());
                    prop_assert_eq!(receipt.support, expected.map(|(support, _)| support));
                    prop_assert_eq!(receipt.weight, expected.map_or(0, |(_, weight)| weight));
                    if let Some(support) = receipt.support {
                        tally[usize::from(support)] += receipt.weight;
                    }
                }
                let stored = client.get_proposal(proposal);
                prop_assert_eq!([stored.against_votes, stored.for_votes], tally);
                tallies[index] = tally;
            }
        }

        env.ledger().set_timestamp(DURATION);
        for (proposal, [against, support]) in proposals.iter().zip(tallies) {
            client.execute(proposal);
            if support > against {
                prop_assert_eq!(client.get_proposal(proposal).state, ProposalState::Succeeded);
                client.execute(proposal);
                prop_assert_eq!(client.get_proposal(proposal).state, ProposalState::Executed);
            } else {
                prop_assert_eq!(client.get_proposal(proposal).state, ProposalState::Defeated);
                prop_assert!(client.try_execute(proposal).is_err());
            }
        }
    }
}
