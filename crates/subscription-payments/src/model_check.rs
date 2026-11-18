//! State-machine model checking for the subscription payments lifecycle.
//!
//! Validates the subscription payments contract against an independent state-machine model:
//! - Tracks statuses: Uncreated -> Active <-> Paused -> Cancelled
//! - Enforces token conservation: subscriber + provider == total_minted
//! - Verifies billing schedule: charges only bill when timestamp >= last_charged + period
//! - Verifies pause drift prevention: resume advances last_charged by elapsed paused time
//! - Verifies terminal states: Cancelled is irreversible and rejects any charges, pauses, or resumes
//! - Guarantees non-mutation: invalid operations are rejected without modifying storage or balances

extern crate std;

use crate::{SorobanForgeSubscriptionPaymentsClient, SubscriptionPayments, SubscriptionStatus};
use core::fmt;
use soroban_forge_shared_utils::ForgeError;
use soroban_forge_test_utils::model::{
    assert_model_check, LifecycleModel, ModelDriver, ModelRng, OpGenerator,
};
use soroban_forge_test_utils::TestAccounts;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env};
use std::format;
use std::string::{String, ToString};

const START_TIMESTAMP: u64 = 1_000_000;
const INITIAL_MINT: i128 = 10_000_000;
const SUB_AMOUNT: i128 = 50_000;
const SUB_PERIOD: u64 = 10_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubModelState {
    Uncreated,
    Active {
        amount: i128,
        period: u64,
        last_charged: u64,
        charged_count: u32,
    },
    Paused {
        amount: i128,
        period: u64,
        last_charged: u64,
        paused_at: u64,
        charged_count: u32,
    },
    Cancelled {
        charged_count: u32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubModelOp {
    Subscribe { amount: i128, period: u64 },
    Charge,
    Pause,
    Resume,
    Cancel,
    AdvanceTime { seconds: u64 },
    InvalidOp,
}

impl fmt::Display for SubModelOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Subscribe { amount, period } => {
                write!(f, "Subscribe(amount={amount}, period={period})")
            }
            Self::Charge => write!(f, "Charge"),
            Self::Pause => write!(f, "Pause"),
            Self::Resume => write!(f, "Resume"),
            Self::Cancel => write!(f, "Cancel"),
            Self::AdvanceTime { seconds } => write!(f, "AdvanceTime({seconds}s)"),
            Self::InvalidOp => write!(f, "InvalidOp"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubSnapshot {
    pub status: Option<SubscriptionStatus>,
    pub last_charged: u64,
    pub subscriber_balance: i128,
    pub provider_balance: i128,
    pub current_time: u64,
}

pub struct SubLifecycleModel {
    pub initial_time: u64,
}

impl LifecycleModel for SubLifecycleModel {
    type State = (SubModelState, u64); // (state, current_time)
    type Op = SubModelOp;
    type Snapshot = SubSnapshot;

    fn initial_state(&self) -> Self::State {
        (SubModelState::Uncreated, self.initial_time)
    }

    fn transition(&self, state: &Self::State, op: &Self::Op) -> Result<Self::State, String> {
        let (current_state, current_time) = state;

        match op {
            SubModelOp::AdvanceTime { seconds } => {
                Ok((current_state.clone(), current_time.saturating_add(*seconds)))
            }
            SubModelOp::Subscribe { amount, period } => match current_state {
                SubModelState::Uncreated => {
                    if *amount <= 0 || *period == 0 {
                        Err("invalid subscribe parameters".into())
                    } else {
                        Ok((
                            SubModelState::Active {
                                amount: *amount,
                                period: *period,
                                last_charged: *current_time,
                                charged_count: 0,
                            },
                            *current_time,
                        ))
                    }
                }
                _ => Err("already subscribed".into()),
            },
            SubModelOp::Charge => match current_state {
                SubModelState::Active {
                    amount,
                    period,
                    last_charged,
                    charged_count,
                } => {
                    let next_due = last_charged.saturating_add(*period);
                    if *current_time < next_due {
                        // Returns 0, no state change
                        Ok((current_state.clone(), *current_time))
                    } else {
                        // One charge bills one period
                        Ok((
                            SubModelState::Active {
                                amount: *amount,
                                period: *period,
                                last_charged: next_due,
                                charged_count: charged_count.saturating_add(1),
                            },
                            *current_time,
                        ))
                    }
                }
                _ => Err("cannot charge non-active subscription".into()),
            },
            SubModelOp::Pause => match current_state {
                SubModelState::Active {
                    amount,
                    period,
                    last_charged,
                    charged_count,
                } => Ok((
                    SubModelState::Paused {
                        amount: *amount,
                        period: *period,
                        last_charged: *last_charged,
                        paused_at: *current_time,
                        charged_count: *charged_count,
                    },
                    *current_time,
                )),
                _ => Err("cannot pause non-active subscription".into()),
            },
            SubModelOp::Resume => match current_state {
                SubModelState::Paused {
                    amount,
                    period,
                    last_charged,
                    paused_at,
                    charged_count,
                } => {
                    let paused_elapsed = current_time.saturating_sub(*paused_at);
                    let new_last_charged = last_charged.saturating_add(paused_elapsed);

                    Ok((
                        SubModelState::Active {
                            amount: *amount,
                            period: *period,
                            last_charged: new_last_charged,
                            charged_count: *charged_count,
                        },
                        *current_time,
                    ))
                }
                _ => Err("cannot resume non-paused subscription".into()),
            },
            SubModelOp::Cancel => match current_state {
                SubModelState::Active { charged_count, .. }
                | SubModelState::Paused { charged_count, .. } => Ok((
                    SubModelState::Cancelled {
                        charged_count: *charged_count,
                    },
                    *current_time,
                )),
                _ => Err("cannot cancel uncreated or already cancelled subscription".into()),
            },
            SubModelOp::InvalidOp => Err("explicit invalid operation".into()),
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

        // Token conservation: subscriber + provider == INITIAL_MINT
        let total_pool = snapshot
            .subscriber_balance
            .saturating_add(snapshot.provider_balance);

        if total_pool != INITIAL_MINT {
            return Err(format!(
                "Token conservation violated! Expected {INITIAL_MINT}, got {total_pool}"
            ));
        }

        match current_state {
            SubModelState::Uncreated => {
                if snapshot.status.is_some() || snapshot.provider_balance != 0 {
                    return Err(
                        "Uncreated subscription has on-chain state or provider balance".into(),
                    );
                }
            }
            SubModelState::Active {
                last_charged,
                charged_count,
                amount,
                ..
            } => {
                if snapshot.status != Some(SubscriptionStatus::Active) {
                    return Err(format!("Expected Active status, got {:?}", snapshot.status));
                }
                if snapshot.last_charged != *last_charged {
                    return Err(format!(
                        "last_charged mismatch: model={last_charged}, on-chain={}",
                        snapshot.last_charged
                    ));
                }
                let expected_provider = i128::from(*charged_count) * amount;
                if snapshot.provider_balance != expected_provider {
                    return Err(format!(
                        "Provider balance mismatch: expected {expected_provider}, got {}",
                        snapshot.provider_balance
                    ));
                }
            }
            SubModelState::Paused {
                last_charged,
                charged_count,
                amount,
                ..
            } => {
                if snapshot.status != Some(SubscriptionStatus::Paused) {
                    return Err(format!("Expected Paused status, got {:?}", snapshot.status));
                }
                if snapshot.last_charged != *last_charged {
                    return Err("last_charged mismatch in Paused".into());
                }
                let expected_provider = i128::from(*charged_count) * amount;
                if snapshot.provider_balance != expected_provider {
                    return Err("Provider balance mismatch in Paused".into());
                }
            }
            SubModelState::Cancelled { charged_count } => {
                if snapshot.status != Some(SubscriptionStatus::Cancelled) {
                    return Err(format!(
                        "Expected Cancelled status, got {:?}",
                        snapshot.status
                    ));
                }
                let expected_provider = i128::from(*charged_count) * SUB_AMOUNT;
                if snapshot.provider_balance != expected_provider {
                    return Err("Provider balance mismatch in Cancelled".into());
                }
            }
        }

        Ok(())
    }

    fn is_terminal(&self, state: &Self::State) -> bool {
        matches!(state.0, SubModelState::Cancelled { .. })
    }
}

pub struct SubModelDriver {
    pub contract_id: Address,
    pub token: Address,
    pub accounts: TestAccounts,
    pub subscription_id: Option<u64>,
}

impl SubModelDriver {
    pub fn new(env: &Env) -> Self {
        env.mock_all_auths_allowing_non_root_auth();
        env.ledger().set_timestamp(START_TIMESTAMP);

        let admin = Address::generate(env);
        let sac = env.register_stellar_asset_contract_v2(admin);
        let token = sac.address();
        let accounts = TestAccounts::generate(env);
        StellarAssetClient::new(env, &token).mint(&accounts.user1, &INITIAL_MINT);

        let contract_id = env.register(SubscriptionPayments, ());

        Self {
            contract_id,
            token,
            accounts,
            subscription_id: None,
        }
    }

    fn client<'a>(&self, env: &'a Env) -> SorobanForgeSubscriptionPaymentsClient<'a> {
        SorobanForgeSubscriptionPaymentsClient::new(env, &self.contract_id)
    }

    fn token_client<'a>(&self, env: &'a Env) -> TokenClient<'a> {
        TokenClient::new(env, &self.token)
    }
}

impl ModelDriver<SubLifecycleModel> for SubModelDriver {
    fn execute_op(&mut self, env: &Env, op: &SubModelOp) -> Result<(), String> {
        let client = self.client(env);

        match op {
            SubModelOp::AdvanceTime { seconds } => {
                let current = env.ledger().timestamp();
                env.ledger().set_timestamp(current.saturating_add(*seconds));
                Ok(())
            }
            SubModelOp::Subscribe { amount, period } => {
                if self.subscription_id.is_some() {
                    return Err("already subscribed".into());
                }
                let res = client.try_subscribe(
                    &self.accounts.user1,
                    &self.accounts.user2,
                    &self.token,
                    amount,
                    period,
                );
                match res {
                    Ok(Ok(id)) => {
                        self.subscription_id = Some(id);
                        Ok(())
                    }
                    Ok(Err(_)) => Err("conversion error".into()),
                    Err(Ok(forge_err)) => Err(format!("{forge_err:?}")),
                    Err(Err(_)) => Err("invoke error".into()),
                }
            }
            SubModelOp::Charge => {
                let id = self.subscription_id.ok_or_else(|| "no sub".to_string())?;
                map_result(client.try_charge(&id))
            }
            SubModelOp::Pause => {
                let id = self.subscription_id.ok_or_else(|| "no sub".to_string())?;
                map_result(client.try_pause(&id))
            }
            SubModelOp::Resume => {
                let id = self.subscription_id.ok_or_else(|| "no sub".to_string())?;
                map_result(client.try_resume(&id))
            }
            SubModelOp::Cancel => {
                let id = self.subscription_id.ok_or_else(|| "no sub".to_string())?;
                map_result(client.try_cancel(&id))
            }
            SubModelOp::InvalidOp => {
                let invalid_id = 999_999_u64;
                map_result(client.try_charge(&invalid_id))
            }
        }
    }

    fn snapshot(&self, env: &Env) -> SubSnapshot {
        let tc = self.token_client(env);
        let client = self.client(env);

        let (status, last_charged) = if let Some(id) = self.subscription_id {
            match client.try_get_subscription(&id) {
                Ok(Ok(sub)) => (Some(sub.status), sub.last_charged),
                _ => (None, 0),
            }
        } else {
            (None, 0)
        };

        SubSnapshot {
            status,
            last_charged,
            subscriber_balance: tc.balance(&self.accounts.user1),
            provider_balance: tc.balance(&self.accounts.user2),
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

pub struct SubOpGenerator;

impl OpGenerator<SubLifecycleModel> for SubOpGenerator {
    fn generate_op(&mut self, state: &(SubModelState, u64), rng: &mut ModelRng) -> SubModelOp {
        let (current_state, _time) = state;

        let choice = rng.bounded(10);
        match current_state {
            SubModelState::Uncreated => match choice {
                0..=7 => SubModelOp::Subscribe {
                    amount: SUB_AMOUNT,
                    period: SUB_PERIOD,
                },
                8 => SubModelOp::Subscribe {
                    amount: 0, // Invalid: zero amount
                    period: SUB_PERIOD,
                },
                _ => SubModelOp::Charge, // Invalid: uncreated
            },
            SubModelState::Active { period, .. } => match choice {
                0..=2 => SubModelOp::Charge,
                3..=4 => {
                    // Decide whether to advance small or full period
                    if rng.gen_bool() {
                        SubModelOp::AdvanceTime {
                            seconds: *period / 2,
                        }
                    } else {
                        SubModelOp::AdvanceTime {
                            seconds: period.saturating_add(100),
                        }
                    }
                }
                5..=6 => SubModelOp::Pause,
                7 => SubModelOp::Cancel,
                8 => SubModelOp::Resume, // Invalid while Active
                _ => SubModelOp::Subscribe {
                    amount: SUB_AMOUNT,
                    period: SUB_PERIOD,
                }, // Invalid: already subscribed
            },
            SubModelState::Paused { .. } => match choice {
                0..=4 => SubModelOp::Resume,
                5 => SubModelOp::Charge, // Invalid while Paused
                6 => SubModelOp::Pause,  // Invalid: double pause
                7 => SubModelOp::Cancel,
                _ => SubModelOp::AdvanceTime { seconds: 1_000 },
            },
            SubModelState::Cancelled { .. } => match choice {
                0..=2 => SubModelOp::Charge, // Invalid
                3 => SubModelOp::Pause,      // Invalid
                4 => SubModelOp::Resume,     // Invalid
                5 => SubModelOp::Cancel,     // Invalid
                6 => SubModelOp::InvalidOp,
                _ => SubModelOp::AdvanceTime { seconds: 1_000 },
            },
        }
    }
}

#[test]
fn test_subscription_model_check_deterministic_seeds() {
    let seeds = [101_u64, 202, 303, 404, 505, 606, 707, 808];
    for seed in seeds {
        let env = Env::default();
        let model = SubLifecycleModel {
            initial_time: START_TIMESTAMP,
        };
        let mut driver = SubModelDriver::new(&env);
        let mut generator = SubOpGenerator;

        assert_model_check(&env, &model, &mut driver, &mut generator, seed, 25);
    }
}
