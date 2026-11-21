//! Model-based lifecycle suite (proptest) for subscription billing.
//!
//! Random sequences of `charge`, `charge_catchup`, `pause`, `resume`,
//! `cancel`, `cancel_by_provider`, `deposit` and `withdraw_balance` run against
//! the real contract and an independent Rust model, with time jumps between
//! steps and a random (possibly insufficient) subscriber token balance. After
//! every step the model's prediction of the call result, status, next due date,
//! failed attempts, prepaid balance, amount billed and refund is compared with
//! the contract, and these invariants are checked from the contract's side:
//!
//! 1. Nothing is billed while a subscription is paused or cancelled.
//! 2. No period is billed twice: every billed period advances `last_charged`
//!    by exactly one period.
//! 3. Tokens are conserved across subscriber, provider and contract custody,
//!    and custody always equals the prepaid balance.
//! 4. `resume` (and every other call) never moves the due date earlier.
//!
//! Both pull mode and prepaid mode are covered: a case may open with a deposit
//! (switching to prepaid), and deposits can also arrive mid-sequence. The seed
//! is fixed so every machine and CI run explores the same sequences.

use crate::{
    SorobanForgeSubscriptionPaymentsClient, SubscriptionPayments, SubscriptionStatus,
    MAX_CATCHUP_PERIODS, MAX_RETRIES,
};
use proptest::prelude::*;
use proptest::test_runner::{RngSeed, TestCaseError};
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env};

const START: u64 = 1_000_000;
const CASES: u32 = 128;
const MAX_STEPS: usize = 40;

#[derive(Clone, Debug)]
enum Op {
    Charge,
    Catchup(u32),
    Pause,
    Resume,
    Cancel,
    CancelByProvider,
    Deposit(i128),
    Withdraw(i128),
}

#[derive(Clone, Debug)]
struct Case {
    amount: i128,
    period: u64,
    mint: i128,
    /// Open with a deposit so the case exercises prepaid mode from the start.
    opening_deposit: Option<i128>,
    /// `(time jump in tenths of a period, operation)`.
    steps: std::vec::Vec<(u64, Op)>,
}

/// Independent mirror of the contract's billing rules.
#[derive(Clone, Debug)]
struct Model {
    amount: i128,
    period: u64,
    status: SubscriptionStatus,
    last_charged: u64,
    paused_at: Option<u64>,
    failed: u32,
    prepaid: Option<i128>,
    subscriber: i128,
    provider: i128,
}

struct Outcome {
    result: Result<i128, ForgeError>,
    billed: i128,
    refunded: i128,
}

impl Outcome {
    fn ok(value: i128) -> Self {
        Self {
            result: Ok(value),
            billed: 0,
            refunded: 0,
        }
    }

    fn err(error: ForgeError) -> Self {
        Self {
            result: Err(error),
            billed: 0,
            refunded: 0,
        }
    }
}

impl Model {
    /// Return the whole positive prepaid balance to the subscriber.
    fn refund(&mut self) -> i128 {
        match self.prepaid {
            Some(balance) if balance > 0 => {
                self.subscriber += balance;
                self.prepaid = Some(0);
                balance
            }
            _ => 0,
        }
    }

    fn apply(&mut self, op: &Op, now: u64) -> Outcome {
        match op {
            Op::Charge => self.charge(now),
            Op::Catchup(max_periods) => self.catchup(*max_periods, now),
            Op::Pause => {
                if self.status != SubscriptionStatus::Active {
                    return Outcome::err(ForgeError::InvalidInput);
                }
                self.status = SubscriptionStatus::Paused;
                self.paused_at = Some(now);
                Outcome::ok(0)
            }
            Op::Resume => {
                if self.status != SubscriptionStatus::Paused {
                    return Outcome::err(ForgeError::InvalidInput);
                }
                self.last_charged += now - self.paused_at.unwrap_or(now);
                self.status = SubscriptionStatus::Active;
                self.paused_at = None;
                Outcome::ok(0)
            }
            Op::Cancel | Op::CancelByProvider => {
                if self.status == SubscriptionStatus::Cancelled {
                    return Outcome::err(ForgeError::InvalidInput);
                }
                let refunded = self.refund();
                self.status = SubscriptionStatus::Cancelled;
                self.paused_at = None;
                Outcome {
                    result: Ok(0),
                    billed: 0,
                    refunded,
                }
            }
            Op::Deposit(amount) => {
                if *amount <= 0 || self.status == SubscriptionStatus::Cancelled {
                    return Outcome::err(ForgeError::InvalidInput);
                }
                if self.subscriber < *amount {
                    return Outcome::err(ForgeError::TokenTransferFailed);
                }
                self.subscriber -= amount;
                let updated = self.prepaid.unwrap_or(0) + amount;
                self.prepaid = Some(updated);
                Outcome::ok(updated)
            }
            Op::Withdraw(amount) => {
                if *amount <= 0 || self.status == SubscriptionStatus::Cancelled {
                    return Outcome::err(ForgeError::InvalidInput);
                }
                match self.prepaid {
                    Some(balance) if balance >= *amount => {
                        self.prepaid = Some(balance - amount);
                        self.subscriber += amount;
                        Outcome::ok(balance - amount)
                    }
                    _ => Outcome::err(ForgeError::InsufficientFunds),
                }
            }
        }
    }

    fn charge(&mut self, now: u64) -> Outcome {
        if matches!(
            self.status,
            SubscriptionStatus::Cancelled | SubscriptionStatus::Paused
        ) {
            return Outcome::err(ForgeError::InvalidInput);
        }
        if now < self.last_charged + self.period {
            return Outcome::ok(0);
        }
        let funded = match self.prepaid {
            Some(balance) => balance >= self.amount,
            None => self.subscriber >= self.amount,
        };
        if funded {
            match self.prepaid {
                Some(balance) => self.prepaid = Some(balance - self.amount),
                None => self.subscriber -= self.amount,
            }
            self.provider += self.amount;
            self.last_charged += self.period;
            self.failed = 0;
            self.status = SubscriptionStatus::Active;
            self.paused_at = None;
            return Outcome {
                result: Ok(self.amount),
                billed: self.amount,
                refunded: 0,
            };
        }
        self.failed += 1;
        let mut refunded = 0;
        if self.failed >= MAX_RETRIES {
            self.status = SubscriptionStatus::Cancelled;
            refunded = self.refund();
        } else {
            self.status = SubscriptionStatus::PastDue;
        }
        Outcome {
            result: Ok(0),
            billed: 0,
            refunded,
        }
    }

    fn catchup(&mut self, max_periods: u32, now: u64) -> Outcome {
        if max_periods > MAX_CATCHUP_PERIODS {
            return Outcome::err(ForgeError::InvalidInput);
        }
        if self.status != SubscriptionStatus::Active || self.prepaid.is_some() {
            return Outcome::err(ForgeError::InvalidInput);
        }
        let due = (now - self.last_charged) / self.period;
        let periods = core::cmp::min(u64::from(max_periods), due);
        let total = self.amount * i128::from(periods as u32);
        if self.subscriber < total {
            // Atomic: the whole batch rolls back.
            return Outcome::err(ForgeError::TokenTransferFailed);
        }
        self.subscriber -= total;
        self.provider += total;
        self.last_charged += periods * self.period;
        if periods > 0 {
            self.failed = 0;
        }
        Outcome {
            result: Ok(total),
            billed: total,
            refunded: 0,
        }
    }
}

fn flatten<T, X, Y>(
    result: Result<Result<T, X>, Result<ForgeError, Y>>,
    map: impl FnOnce(T) -> i128,
) -> Result<i128, ForgeError> {
    match result {
        Ok(Ok(value)) => Ok(map(value)),
        Err(Ok(error)) => Err(error),
        _ => panic!("unexpected host error"),
    }
}

fn call(
    client: &SorobanForgeSubscriptionPaymentsClient<'_>,
    id: u64,
    op: &Op,
) -> Result<i128, ForgeError> {
    match op {
        Op::Charge => flatten(client.try_charge(&id), |v| v),
        Op::Catchup(n) => flatten(client.try_charge_catchup(&id, n), |v| v),
        Op::Pause => flatten(client.try_pause(&id), |_| 0),
        Op::Resume => flatten(client.try_resume(&id), |_| 0),
        Op::Cancel => flatten(client.try_cancel(&id), |_| 0),
        Op::CancelByProvider => flatten(client.try_cancel_by_provider(&id), |_| 0),
        Op::Deposit(a) => flatten(client.try_deposit(&id, a), |v| v),
        Op::Withdraw(a) => flatten(client.try_withdraw_balance(&id, a), |v| v),
    }
}

fn run_case(case: Case) -> Result<(), TestCaseError> {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    env.ledger().set_timestamp(START);

    let admin = Address::generate(&env);
    let token_address = env.register_stellar_asset_contract_v2(admin).address();
    let subscriber = Address::generate(&env);
    let provider = Address::generate(&env);
    StellarAssetClient::new(&env, &token_address).mint(&subscriber, &case.mint);
    let contract_id = env.register(SubscriptionPayments, ());
    let client = SorobanForgeSubscriptionPaymentsClient::new(&env, &contract_id);
    let token = TokenClient::new(&env, &token_address);
    let id = client.subscribe(
        &subscriber,
        &provider,
        &token_address,
        &case.amount,
        &case.period,
    );

    let mut model = Model {
        amount: case.amount,
        period: case.period,
        status: SubscriptionStatus::Active,
        last_charged: START,
        paused_at: None,
        failed: 0,
        prepaid: None,
        subscriber: case.mint,
        provider: 0,
    };

    let mut steps: std::vec::Vec<(u64, Op)> = std::vec::Vec::new();
    if let Some(deposit) = case.opening_deposit {
        steps.push((0, Op::Deposit(deposit)));
    }
    steps.extend(case.steps.iter().cloned());

    let mut now = START;
    for (tenths, op) in steps {
        now += case.period * tenths / 10;
        env.ledger().set_timestamp(now);

        let before = client.get_subscription(&id);
        let provider_before = token.balance(&provider);
        let expected = model.apply(&op, now);
        let actual = call(&client, id, &op);

        // Model vs contract: call result and full observable state.
        prop_assert_eq!(&actual, &expected.result);
        let after = client.get_subscription(&id);
        prop_assert_eq!(&after.status, &model.status);
        prop_assert_eq!(after.last_charged, model.last_charged);
        prop_assert_eq!(after.paused_at, model.paused_at);
        prop_assert_eq!(after.failed_attempts, model.failed);
        prop_assert_eq!(after.prepaid_balance, model.prepaid);
        prop_assert_eq!(token.balance(&subscriber), model.subscriber);
        prop_assert_eq!(token.balance(&provider), model.provider);
        prop_assert_eq!(token.balance(&contract_id), model.prepaid.unwrap_or(0));

        // Amounts billed and refunded.
        let billed = token.balance(&provider) - provider_before;
        prop_assert_eq!(billed, expected.billed);
        if expected.refunded > 0 {
            prop_assert_eq!(before.prepaid_balance, Some(expected.refunded));
            prop_assert_eq!(after.prepaid_balance, Some(0));
        }

        // Nothing is billed while paused or cancelled.
        if matches!(
            before.status,
            SubscriptionStatus::Paused | SubscriptionStatus::Cancelled
        ) {
            prop_assert_eq!(billed, 0);
        }

        // No period is billed twice: each billed period moves the due date by
        // exactly one period, and nothing else moves it except `resume`.
        prop_assert_eq!(billed % case.amount, 0);
        let periods_billed = (billed / case.amount) as u64;
        if matches!(op, Op::Resume) && actual.is_ok() {
            prop_assert!(after.last_charged >= before.last_charged);
        } else {
            prop_assert_eq!(
                after.last_charged,
                before.last_charged + periods_billed * case.period
            );
        }
        prop_assert!(after.last_charged >= before.last_charged);

        // Tokens are conserved across subscriber, provider and custody.
        prop_assert_eq!(
            token.balance(&subscriber) + token.balance(&provider) + token.balance(&contract_id),
            case.mint
        );
    }
    Ok(())
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        5 => Just(Op::Charge),
        2 => (0u32..=MAX_CATCHUP_PERIODS + 2).prop_map(Op::Catchup),
        2 => Just(Op::Pause),
        2 => Just(Op::Resume),
        1 => Just(Op::Cancel),
        1 => Just(Op::CancelByProvider),
        3 => (0i128..=3_000).prop_map(Op::Deposit),
        2 => (0i128..=3_000).prop_map(Op::Withdraw),
    ]
}

fn jump_strategy() -> impl Strategy<Value = u64> {
    prop_oneof![
        3 => Just(0u64),
        4 => 1u64..=12,
        2 => 13u64..=45,
    ]
}

fn case_strategy() -> impl Strategy<Value = Case> {
    (
        1i128..=500,
        10u64..=1_000,
        0i128..=2_500,
        prop::option::of(1i128..=3_000),
        prop::collection::vec((jump_strategy(), op_strategy()), 1..=MAX_STEPS),
    )
        .prop_map(|(amount, period, mint_pct, opening_deposit, steps)| Case {
            amount,
            period,
            // 0 to 25 periods of funding: includes unfunded subscribers.
            mint: amount * mint_pct / 100,
            opening_deposit,
            steps,
        })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: CASES,
        // Fixed seed: the same lifecycles run on every machine and in CI.
        rng_seed: RngSeed::Fixed(0x4c49_4645_4359_434c),
        ..ProptestConfig::default()
    })]

    #[test]
    fn prop_billing_lifecycle_matches_model(case in case_strategy()) {
        run_case(case)?;
    }
}
