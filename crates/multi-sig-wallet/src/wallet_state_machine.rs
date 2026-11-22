//! Fixed-seed, shrinking traces across governance and monetary operations.
extern crate std;

use crate::{MultiSigWallet, SorobanForgeMultiSigWalletClient, TxStatus};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use soroban_forge_test_utils::MockTarget;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Bytes, Env};
use std::vec::Vec;

#[derive(Clone, Debug)]
enum Proposal {
    Call,
    Withdrawal(i128),
    Add(usize),
    Remove(usize),
    Threshold(u32),
    Limit(i128),
}

#[derive(Clone, Debug)]
enum Action {
    Propose(Proposal, usize),
    Confirm(usize),
    Reject(usize),
    Cancel,
    Execute,
    Advance(u64),
}

fn actions() -> impl Strategy<Value = Vec<Action>> {
    let proposal = prop_oneof![
        Just(Proposal::Call),
        (1_i128..250).prop_map(Proposal::Withdrawal),
        (0_usize..4).prop_map(Proposal::Add),
        (0_usize..4).prop_map(Proposal::Remove),
        (0_u32..5).prop_map(Proposal::Threshold),
        (1_i128..300).prop_map(Proposal::Limit),
    ];
    prop::collection::vec(
        prop_oneof![
            4 => (proposal, 0_usize..4).prop_map(|(kind, owner)| Action::Propose(kind, owner)),
            5 => (0_usize..4).prop_map(Action::Confirm),
            1 => (0_usize..4).prop_map(Action::Reject),
            1 => Just(Action::Cancel),
            5 => Just(Action::Execute),
            1 => (1_u64..30).prop_map(Action::Advance),
        ],
        1..=40,
    )
}

#[derive(Clone, Debug)]
struct Transaction {
    kind: Proposal,
    submitter: usize,
    approvals: Vec<usize>,
    rejection: Option<usize>,
    status: TxStatus,
    expiry: Option<u64>,
    admitted: Option<u64>,
}

#[derive(Debug)]
struct Model {
    owners: Vec<usize>,
    threshold: u32,
    transactions: Vec<Transaction>,
    balance: i128,
    received: i128,
    limit: Option<i128>,
    window: Vec<(u64, i128)>,
    now: u64,
}

impl Model {
    fn status(&self, tx: &Transaction) -> TxStatus {
        if tx.status == TxStatus::Pending
            && tx.expiry.is_some_and(|end| self.now >= end)
            && tx.approvals.len() < self.threshold as usize
        {
            TxStatus::Expired
        } else {
            tx.status.clone()
        }
    }

    fn usage(&self) -> i128 {
        self.window
            .iter()
            .filter(|(at, _)| self.now - at < 20)
            .map(|(_, amount)| amount)
            .sum()
    }

    fn can_execute(&self, tx: &Transaction) -> bool {
        if self.status(tx) != TxStatus::Pending
            || tx.approvals.len() < self.threshold as usize
            || tx.rejection.is_some()
        {
            return false;
        }
        match tx.kind {
            Proposal::Call | Proposal::Limit(_) => true,
            Proposal::Add(owner) => !self.owners.contains(&owner),
            Proposal::Remove(owner) => {
                self.owners.contains(&owner) && self.owners.len() > self.threshold as usize
            }
            Proposal::Threshold(value) => value > 0 && value <= self.owners.len() as u32,
            Proposal::Withdrawal(amount) => {
                let extra = if tx.admitted.is_some_and(|at| self.now - at < 20) {
                    0
                } else {
                    amount
                };
                self.balance >= amount
                    && self.limit.is_none_or(|limit| self.usage() + extra <= limit)
            }
        }
    }

    fn execute(&mut self, index: usize) {
        match self.transactions[index].kind {
            Proposal::Call => {}
            Proposal::Add(owner) => self.owners.push(owner),
            Proposal::Remove(owner) => {
                self.owners.retain(|existing| *existing != owner);
                for (at, tx) in self.transactions.iter_mut().enumerate() {
                    if at != index && tx.status == TxStatus::Pending {
                        tx.approvals.retain(|signer| *signer != owner);
                    }
                }
            }
            Proposal::Threshold(value) => self.threshold = value,
            Proposal::Limit(value) => self.limit = Some(value),
            Proposal::Withdrawal(amount) => {
                if self.limit.is_some()
                    && self.transactions[index]
                        .admitted
                        .is_none_or(|at| self.now - at >= 20)
                {
                    self.window.retain(|(at, _)| self.now - at < 20);
                    self.window.push((self.now, amount));
                }
                self.balance -= amount;
                self.received += amount;
            }
        }
        self.transactions[index].status = TxStatus::Executed;
    }
}

fn run_trace(trace: &[Action]) -> Result<(), TestCaseError> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1000);
    // The model covers contract state; CPU metering is measured by wall time.
    env.cost_estimate().budget().reset_unlimited();
    let owners: Vec<Address> = (0..4).map(|_| Address::generate(&env)).collect();
    let recipient = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    let contract = env.register(MultiSigWallet, ());
    let target = env.register(MockTarget, ());
    let client = SorobanForgeMultiSigWalletClient::new(&env, &contract);
    client.initialize(
        &soroban_sdk::vec![
            &env,
            owners[0].clone(),
            owners[1].clone(),
            owners[2].clone()
        ],
        &2,
    );
    StellarAssetClient::new(&env, &token).mint(&owners[0], &1000);
    client.deposit(&token, &owners[0], &1000);
    let mut model = Model {
        owners: std::vec![0, 1, 2],
        threshold: 2,
        transactions: Vec::new(),
        balance: 1000,
        received: 0,
        limit: None,
        window: Vec::new(),
        now: 1000,
    };

    for (step, action) in trace.iter().enumerate() {
        let context =
            || std::format!("step {step}: {action:?}\ntrace: {trace:#?}\nmodel: {model:#?}");
        match action {
            Action::Advance(seconds) => {
                model.now += seconds;
                env.ledger().set_timestamp(model.now);
            }
            Action::Propose(kind, owner) => {
                let valid_owner = model.owners.contains(owner);
                let valid = valid_owner
                    && match kind {
                        Proposal::Add(new) => !model.owners.contains(new),
                        // Submission permits a pending removal that execution must recheck.
                        Proposal::Remove(old) => {
                            model.owners.contains(old) && model.owners.len() > 1
                        }
                        Proposal::Threshold(value) => {
                            *value > 0 && *value <= model.owners.len() as u32
                        }
                        Proposal::Withdrawal(amount) => model
                            .limit
                            .is_none_or(|limit| model.usage() + amount <= limit),
                        _ => true,
                    };
                let result = match kind {
                    Proposal::Call => client.try_submit(
                        &owners[*owner],
                        &target,
                        &Bytes::from_slice(&env, &[1]),
                        &Some(model.now + 10),
                    ),
                    Proposal::Withdrawal(amount) => {
                        client.try_submit_withdrawal(&owners[*owner], &token, &recipient, amount)
                    }
                    Proposal::Add(new) => client.try_add_owner(&owners[*owner], &owners[*new]),
                    Proposal::Remove(old) => {
                        client.try_remove_owner(&owners[*owner], &owners[*old])
                    }
                    Proposal::Threshold(value) => client.try_set_threshold(&owners[*owner], value),
                    Proposal::Limit(value) => {
                        client.try_set_withdrawal_limit(&owners[*owner], &token, value, &20)
                    }
                };
                prop_assert_eq!(matches!(result, Ok(Ok(_))), valid, "{}", context());
                if valid {
                    let admitted =
                        if matches!(kind, Proposal::Withdrawal(_)) && model.limit.is_some() {
                            Some(model.now)
                        } else {
                            None
                        };
                    if let (Some(at), Proposal::Withdrawal(amount)) = (admitted, kind) {
                        model.window.retain(|(at, _)| model.now - at < 20);
                        model.window.push((at, *amount));
                    }
                    model.transactions.push(Transaction {
                        kind: kind.clone(),
                        submitter: *owner,
                        approvals: Vec::new(),
                        rejection: None,
                        status: TxStatus::Pending,
                        expiry: matches!(kind, Proposal::Call).then_some(model.now + 10),
                        admitted,
                    });
                }
            }
            _ if model.transactions.is_empty() => {}
            _ => {
                let index = model.transactions.len() - 1;
                let tx = &model.transactions[index];
                let id = index as u64 + 1;
                match action {
                    Action::Confirm(owner) | Action::Reject(owner) => {
                        let valid = model.status(tx) == TxStatus::Pending
                            && model.owners.contains(owner)
                            && !tx.approvals.contains(owner)
                            && tx.rejection.is_none()
                            && (!matches!(action, Action::Confirm(_))
                                || tx.expiry.is_none_or(|end| model.now < end));
                        let result = if matches!(action, Action::Confirm(_)) {
                            client.try_confirm(&id, &owners[*owner])
                        } else {
                            client.try_reject(&id, &owners[*owner])
                        };
                        prop_assert_eq!(matches!(result, Ok(Ok(()))), valid, "{}", context());
                        if valid {
                            if matches!(action, Action::Confirm(_)) {
                                model.transactions[index].approvals.push(*owner);
                            } else {
                                model.transactions[index].rejection = Some(*owner);
                                model.transactions[index].status = TxStatus::Rejected;
                            }
                        }
                    }
                    Action::Cancel => {
                        let valid = model.status(tx) == TxStatus::Pending;
                        let result = client.try_cancel_tx(&id, &owners[tx.submitter]);
                        prop_assert_eq!(matches!(result, Ok(Ok(()))), valid, "{}", context());
                        if valid {
                            model.transactions[index].status = TxStatus::Cancelled;
                        }
                    }
                    Action::Execute => {
                        let valid = model.can_execute(tx);
                        let result = client.try_execute(&id);
                        prop_assert_eq!(matches!(result, Ok(Ok(()))), valid, "{}", context());
                        if valid {
                            model.execute(index);
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
        let actual_owners: Vec<Address> = client.get_owners().iter().collect();
        let expected_owners: Vec<Address> = model
            .owners
            .iter()
            .map(|index| owners[*index].clone())
            .collect();
        prop_assert_eq!(
            actual_owners,
            expected_owners,
            "step {}: {:#?}",
            step,
            trace
        );
        prop_assert_eq!(client.get_threshold(), model.threshold);
        prop_assert!(model.threshold > 0 && model.threshold as usize <= model.owners.len());
        prop_assert_eq!(client.get_tx_count(), model.transactions.len() as u64);
        prop_assert_eq!(
            TokenClient::new(&env, &token).balance(&contract),
            model.balance
        );
        prop_assert_eq!(
            TokenClient::new(&env, &token).balance(&recipient),
            model.received
        );
        prop_assert_eq!(client.balance(&token), model.balance);
        prop_assert_eq!(model.balance + model.received, 1000);
        for (index, expected) in model.transactions.iter().enumerate() {
            let actual = client.get_tx(&(index as u64 + 1));
            prop_assert_eq!(
                actual.status,
                model.status(expected),
                "step {}: {:#?}",
                step,
                trace
            );
            prop_assert_eq!(actual.submitter, owners[expected.submitter].clone());
            prop_assert_eq!(actual.expiry, expected.expiry);
            let rejections: Vec<Address> = expected
                .rejection
                .iter()
                .map(|index| owners[*index].clone())
                .collect();
            prop_assert_eq!(actual.rejections.iter().collect::<Vec<_>>(), rejections);
            let approvals: Vec<Address> = expected
                .approvals
                .iter()
                .map(|index| owners[*index].clone())
                .collect();
            prop_assert_eq!(actual.confirmations.iter().collect::<Vec<_>>(), approvals);
        }
        if let Some(limit) = model.limit {
            prop_assert_eq!(client.get_withdrawal_limit(&token).unwrap().amount, limit);
            prop_assert_eq!(client.get_withdrawal_window(&token).total, model.usage());
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        rng_seed: RngSeed::Fixed(0x743_2026),
        max_shrink_iters: 4096,
        ..ProptestConfig::default()
    })]
    #[test]
    fn fixed_seed_wallet_state_machine(trace in actions()) {
        run_trace(&trace)?;
    }
}
