//! Property-based suite for milestone-scheduled escrows.
//!
//! Random schedules (1..=[`MAX_MILESTONES`] milestones) are driven through
//! random sequences of approvals (in and out of order), disputes, arbiter
//! resolutions, split resolutions, party refunds, expiry refunds and time
//! jumps. An independent model is checked against the contract and the real
//! Stellar Asset Contract balances after every step:
//!
//! * conservation: `paid + refunded + held == funded total`;
//! * custody: the contract holds exactly the escrow's `remaining()`;
//! * no milestone is paid twice, and milestones are paid strictly in order
//!   (each success emits `MilestoneReleased` for the next index, and the
//!   seller's schedule income equals the prefix sum of the paid milestones);
//! * a dispute freezes the unpaid remainder, and arbitration or expiry
//!   refunds settle only that remainder.
//!
//! The seed is fixed so the same sequences run on every machine and in CI.

extern crate std;

use crate::{
    events, Escrow, EscrowStatus, MilestoneStatus, SorobanForgeEscrowClient, MAX_MILESTONES,
};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::events::Event as _;
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env, Vec};

const START: u64 = 1_000_000;
const TIMEOUT: u64 = 10 * 24 * 60 * 60;
const MAX_AMOUNT: i128 = 1_000_000_000_000;
const CASES: u32 = 128;

#[derive(Clone, Debug)]
enum Op {
    /// Approve the next milestone (`None`) or an arbitrary index.
    Approve(Option<u32>),
    Dispute {
        by_buyer: bool,
    },
    Resolve {
        to_seller: bool,
    },
    ResolveSplit {
        seller_bps: u32,
    },
    Refund,
    RefundExpired,
    Advance(u64),
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => Just(Op::Approve(None)),
        3 => (0u32..MAX_MILESTONES + 2).prop_map(|index| Op::Approve(Some(index))),
        1 => any::<bool>().prop_map(|by_buyer| Op::Dispute { by_buyer }),
        1 => any::<bool>().prop_map(|to_seller| Op::Resolve { to_seller }),
        1 => (0u32..=10_000).prop_map(|seller_bps| Op::ResolveSplit { seller_bps }),
        1 => Just(Op::Refund),
        1 => Just(Op::RefundExpired),
        2 => (0u64..=TIMEOUT / 2).prop_map(Op::Advance),
    ]
}

fn plan_strategy() -> impl Strategy<Value = std::vec::Vec<i128>> {
    proptest::collection::vec(1..=MAX_AMOUNT, 1..=MAX_MILESTONES as usize)
}

/// Independent reference model of one milestone escrow.
struct Model {
    amounts: std::vec::Vec<i128>,
    paid: u32,
    status: EscrowStatus,
    /// Seller income from approved milestones only.
    schedule_paid: i128,
    /// Seller income from arbitration.
    awarded: i128,
    /// Returned to the buyer by refund, expiry or arbitration.
    refunded: i128,
    now: u64,
}

impl Model {
    fn total(&self) -> i128 {
        self.amounts.iter().sum()
    }

    fn held(&self) -> i128 {
        self.total() - self.schedule_paid - self.awarded - self.refunded
    }

    fn deadline(&self) -> u64 {
        START + TIMEOUT
    }

    /// Apply `op` and return what the contract must answer.
    fn step(&mut self, op: &Op) -> Result<(), ForgeError> {
        match *op {
            Op::Approve(index) => {
                let index = index.unwrap_or(self.paid);
                if self.status != EscrowStatus::Funded || index != self.paid {
                    return Err(ForgeError::InvalidInput);
                }
                self.schedule_paid += self.amounts[index as usize];
                self.paid += 1;
                if self.paid as usize == self.amounts.len() {
                    self.status = EscrowStatus::Completed;
                }
                Ok(())
            }
            Op::Dispute { .. } => {
                if self.status != EscrowStatus::Funded {
                    return Err(ForgeError::InvalidInput);
                }
                self.status = EscrowStatus::Disputed;
                Ok(())
            }
            Op::Resolve { to_seller } => {
                if self.status != EscrowStatus::Disputed {
                    return Err(ForgeError::InvalidInput);
                }
                let remainder = self.held();
                if to_seller {
                    self.awarded += remainder;
                    self.status = EscrowStatus::Completed;
                } else {
                    self.refunded += remainder;
                    self.status = EscrowStatus::Refunded;
                }
                Ok(())
            }
            Op::ResolveSplit { seller_bps } => {
                if self.status != EscrowStatus::Disputed {
                    return Err(ForgeError::InvalidInput);
                }
                let remainder = self.held();
                let seller_share = remainder * i128::from(seller_bps) / 10_000;
                self.awarded += seller_share;
                self.refunded += remainder - seller_share;
                self.status = if seller_share == 0 {
                    EscrowStatus::Refunded
                } else {
                    EscrowStatus::Completed
                };
                Ok(())
            }
            Op::Refund => {
                if self.status != EscrowStatus::Funded {
                    return Err(ForgeError::InvalidInput);
                }
                self.refunded += self.held();
                self.status = EscrowStatus::Refunded;
                Ok(())
            }
            Op::RefundExpired => {
                if self.status != EscrowStatus::Funded {
                    return Err(ForgeError::InvalidInput);
                }
                if self.now <= self.deadline() {
                    return Err(ForgeError::DeadlineReached);
                }
                self.refunded += self.held();
                self.status = EscrowStatus::Refunded;
                Ok(())
            }
            Op::Advance(seconds) => {
                self.now += seconds;
                Ok(())
            }
        }
    }

    fn expected_status(&self, index: u32) -> MilestoneStatus {
        if index < self.paid {
            return MilestoneStatus::Paid;
        }
        match self.status {
            EscrowStatus::Pending | EscrowStatus::Funded => MilestoneStatus::Unpaid,
            EscrowStatus::Disputed => MilestoneStatus::Frozen,
            EscrowStatus::Cancelled => MilestoneStatus::Cancelled,
            EscrowStatus::Completed | EscrowStatus::Refunded => MilestoneStatus::Settled,
        }
    }
}

fn unwrap_host<T: core::fmt::Debug, C: core::fmt::Debug, I: core::fmt::Debug>(
    result: Result<Result<T, C>, Result<ForgeError, I>>,
) -> Result<(), ForgeError> {
    match result {
        Ok(Ok(_)) => Ok(()),
        Err(Ok(error)) => Err(error),
        other => panic!("unexpected host result: {other:?}"),
    }
}

fn run_case(plan: &[i128], ops: &[Op]) -> Result<(), TestCaseError> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);
    let admin = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(admin).address();
    let contract = env.register(Escrow, ());
    let client = SorobanForgeEscrowClient::new(&env, &contract);
    let tc = TokenClient::new(&env, &token);
    let (buyer, seller, arbiter) = (
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    );

    let total: i128 = plan.iter().sum();
    StellarAssetClient::new(&env, &token).mint(&buyer, &total);
    let mut amounts = Vec::new(&env);
    for amount in plan {
        amounts.push_back(*amount);
    }
    let id = client.create_milestone_escrow(&buyer, &seller, &arbiter, &token, &amounts, &TIMEOUT);
    client.deposit(&id);

    let mut model = Model {
        amounts: plan.to_vec(),
        paid: 0,
        status: EscrowStatus::Funded,
        schedule_paid: 0,
        awarded: 0,
        refunded: 0,
        now: START,
    };

    for op in ops {
        let paid_before = model.paid;
        let expected = model.step(op);
        let actual = match *op {
            Op::Approve(index) => {
                unwrap_host(client.try_approve_milestone(&id, &index.unwrap_or(paid_before)))
            }
            Op::Dispute { by_buyer } => {
                let claimant = if by_buyer { &buyer } else { &seller };
                unwrap_host(client.try_dispute(&id, claimant))
            }
            Op::Resolve { to_seller } => unwrap_host(client.try_resolve(&id, &to_seller)),
            Op::ResolveSplit { seller_bps } => {
                unwrap_host(client.try_resolve_split(&id, &seller_bps))
            }
            Op::Refund => unwrap_host(client.try_refund(&id)),
            Op::RefundExpired => unwrap_host(client.try_refund_expired(&id)),
            Op::Advance(_) => {
                env.ledger().set_timestamp(model.now);
                Ok(())
            }
        };
        prop_assert_eq!(&actual, &expected, "op {:?}", op);

        // In order and never twice: a successful approval announces exactly
        // the milestone the model expected next.
        if matches!(op, Op::Approve(_)) && actual.is_ok() {
            let emitted = env
                .events()
                .all()
                .filter_by_contract(&contract)
                .events()
                .to_vec();
            let announced = events::MilestoneReleased {
                escrow_id: id,
                index: paid_before,
                amount: model.amounts[paid_before as usize],
            }
            .to_xdr(&env, &contract);
            prop_assert_eq!(&emitted[0], &announced);
        }

        let escrow = client.get_escrow(&id);
        let held = tc.balance(&contract);
        let buyer_balance = tc.balance(&buyer);
        let seller_balance = tc.balance(&seller);

        prop_assert_eq!(escrow.status.clone(), model.status.clone());
        prop_assert_eq!(held, model.held());
        prop_assert_eq!(held, escrow.remaining());
        prop_assert_eq!(seller_balance, model.schedule_paid + model.awarded);
        prop_assert_eq!(buyer_balance, model.refunded);
        prop_assert_eq!(tc.balance(&arbiter), 0);
        // paid + refunded + held == funded total
        prop_assert_eq!(seller_balance + buyer_balance + held, total);

        let prefix: i128 = model.amounts[..model.paid as usize].iter().sum();
        prop_assert_eq!(model.schedule_paid, prefix);

        let milestones = client.get_milestones(&id);
        prop_assert_eq!(milestones.len() as usize, model.amounts.len());
        for milestone in milestones.iter() {
            prop_assert_eq!(milestone.amount, model.amounts[milestone.index as usize]);
            prop_assert_eq!(milestone.status, model.expected_status(milestone.index));
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: CASES,
        // Fixed seed: identical sequences locally and in CI.
        rng_seed: RngSeed::Fixed(0x4d49_4c45_5354_4f4e),
        ..ProptestConfig::default()
    })]

    /// Conservation, in-order payment and freeze-on-dispute hold for random
    /// schedules under random approve / dispute / resolve / expiry sequences.
    #[test]
    fn prop_milestone_schedule_conserves_and_pays_in_order(
        plan in plan_strategy(),
        ops in proptest::collection::vec(op_strategy(), 1..32),
    ) {
        run_case(&plan, &ops)?;
    }
}
