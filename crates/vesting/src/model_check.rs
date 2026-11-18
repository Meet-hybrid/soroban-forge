//! State-machine model checking for the vesting contract lifecycle.
//!
//! Validates the vesting contract against an independent state-machine model:
//! - Tracks statuses: Uncreated -> Locked -> Vesting -> Completed / Revoked
//! - Enforces token conservation: funder + beneficiary + contract == total_minted
//! - Monotonicity: claimed amount never decreases; claimed <= total_amount
//! - Verifies terminal states: contract balance drops to 0 on Completed
//! - Guarantees non-mutation: invalid operations are rejected without modifying storage or balances

extern crate std;

use crate::{SorobanForgeVestingClient, Vesting, VestingStatus};
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
const VESTING_TOTAL: i128 = 1_000_000;
const DEFAULT_CLIFF: u64 = 10_000;
const DEFAULT_DURATION: u64 = 50_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VestingModelState {
    Uncreated,
    Active {
        total_amount: i128,
        cliff: u64,
        duration: u64,
        start: u64,
        claimed: i128,
    },
    Revoked {
        total_amount: i128,
        frozen_claimable: i128,
        claimed: i128,
    },
    Completed {
        total_amount: i128,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VestingModelOp {
    Create {
        amount: i128,
        cliff: u64,
        duration: u64,
    },
    Claim,
    Revoke,
    AdvanceTime {
        seconds: u64,
    },
    InvalidOp,
}

impl fmt::Display for VestingModelOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Create {
                amount,
                cliff,
                duration,
            } => {
                write!(
                    f,
                    "Create(amount={amount}, cliff={cliff}, duration={duration})"
                )
            }
            Self::Claim => write!(f, "Claim"),
            Self::Revoke => write!(f, "Revoke"),
            Self::AdvanceTime { seconds } => write!(f, "AdvanceTime({seconds}s)"),
            Self::InvalidOp => write!(f, "InvalidOp"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VestingSnapshot {
    pub status: Option<VestingStatus>,
    pub claimed: i128,
    pub contract_balance: i128,
    pub funder_balance: i128,
    pub beneficiary_balance: i128,
    pub other_beneficiary_balance: i128,
    pub current_time: u64,
}

fn calculate_vested(total: i128, cliff: u64, duration: u64, start: u64, current_time: u64) -> i128 {
    let cliff_time = start.saturating_add(cliff);
    let end_time = start.saturating_add(duration);
    if current_time < cliff_time {
        0
    } else if current_time >= end_time {
        total
    } else {
        let period = duration.saturating_sub(cliff);
        if period == 0 {
            total
        } else {
            let elapsed = current_time.saturating_sub(cliff_time);
            (total * (elapsed as i128)) / (period as i128)
        }
    }
}

pub struct VestingLifecycleModel {
    pub initial_time: u64,
}

impl LifecycleModel for VestingLifecycleModel {
    type State = (VestingModelState, u64); // (state, current_time)
    type Op = VestingModelOp;
    type Snapshot = VestingSnapshot;

    fn initial_state(&self) -> Self::State {
        (VestingModelState::Uncreated, self.initial_time)
    }

    fn transition(&self, state: &Self::State, op: &Self::Op) -> Result<Self::State, String> {
        let (current_state, current_time) = state;

        match op {
            VestingModelOp::AdvanceTime { seconds } => {
                Ok((current_state.clone(), current_time.saturating_add(*seconds)))
            }
            VestingModelOp::Create {
                amount,
                cliff,
                duration,
            } => match current_state {
                VestingModelState::Uncreated => {
                    if *amount <= 0 || *duration == 0 || *cliff > *duration {
                        Err("invalid schedule parameters".into())
                    } else {
                        Ok((
                            VestingModelState::Active {
                                total_amount: *amount,
                                cliff: *cliff,
                                duration: *duration,
                                start: *current_time,
                                claimed: 0,
                            },
                            *current_time,
                        ))
                    }
                }
                _ => Err("schedule already created".into()),
            },
            VestingModelOp::Claim => match current_state {
                VestingModelState::Active {
                    total_amount,
                    cliff,
                    duration,
                    start,
                    claimed,
                } => {
                    let vested =
                        calculate_vested(*total_amount, *cliff, *duration, *start, *current_time);
                    let new_claimable = vested.saturating_sub(*claimed);
                    let new_claimed = claimed.saturating_add(new_claimable);

                    if new_claimed >= *total_amount {
                        Ok((
                            VestingModelState::Completed {
                                total_amount: *total_amount,
                            },
                            *current_time,
                        ))
                    } else {
                        Ok((
                            VestingModelState::Active {
                                total_amount: *total_amount,
                                cliff: *cliff,
                                duration: *duration,
                                start: *start,
                                claimed: new_claimed,
                            },
                            *current_time,
                        ))
                    }
                }
                VestingModelState::Revoked {
                    total_amount,
                    frozen_claimable,
                    claimed,
                } => {
                    let new_claimed =
                        claimed.saturating_add(frozen_claimable.saturating_sub(*claimed));
                    Ok((
                        VestingModelState::Revoked {
                            total_amount: *total_amount,
                            frozen_claimable: *frozen_claimable,
                            claimed: new_claimed,
                        },
                        *current_time,
                    ))
                }
                VestingModelState::Completed { total_amount } => {
                    // Claim on completed returns 0 and leaves state unchanged
                    Ok((
                        VestingModelState::Completed {
                            total_amount: *total_amount,
                        },
                        *current_time,
                    ))
                }
                VestingModelState::Uncreated => Err("cannot claim uncreated schedule".into()),
            },
            VestingModelOp::Revoke => match current_state {
                VestingModelState::Active {
                    total_amount,
                    cliff,
                    duration,
                    start,
                    claimed,
                } => {
                    let vested_at_revocation =
                        calculate_vested(*total_amount, *cliff, *duration, *start, *current_time);

                    Ok((
                        VestingModelState::Revoked {
                            total_amount: *total_amount,
                            frozen_claimable: vested_at_revocation,
                            claimed: *claimed,
                        },
                        *current_time,
                    ))
                }
                _ => Err("cannot revoke non-active schedule".into()),
            },
            VestingModelOp::InvalidOp => Err("explicit invalid operation".into()),
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

        // Token conservation
        let pool = snapshot
            .contract_balance
            .saturating_add(snapshot.funder_balance)
            .saturating_add(snapshot.beneficiary_balance)
            .saturating_add(snapshot.other_beneficiary_balance);

        if pool != INITIAL_MINT {
            return Err(format!(
                "Token conservation violated! Expected {INITIAL_MINT}, got {pool}"
            ));
        }

        match current_state {
            VestingModelState::Uncreated => {
                if snapshot.status.is_some() || snapshot.contract_balance != 0 {
                    return Err("Uncreated schedule has on-chain record or balance".into());
                }
            }
            VestingModelState::Active {
                total_amount,
                cliff,
                start,
                claimed,
                ..
            } => {
                let expected_status = if *current_time < start.saturating_add(*cliff) {
                    VestingStatus::Locked
                } else {
                    VestingStatus::Vesting
                };
                if snapshot.status.as_ref() != Some(&expected_status) {
                    return Err(format!(
                        "Expected {expected_status:?}, got {:?}",
                        snapshot.status
                    ));
                }
                if snapshot.claimed != *claimed {
                    return Err(format!(
                        "Claimed mismatch: model={claimed}, on-chain={}",
                        snapshot.claimed
                    ));
                }
                let expected_contract = total_amount.saturating_sub(*claimed);
                if snapshot.contract_balance != expected_contract {
                    return Err(format!(
                        "Contract custody mismatch: expected {expected_contract}, got {}",
                        snapshot.contract_balance
                    ));
                }
            }
            VestingModelState::Revoked {
                total_amount,
                claimed,
                ..
            } => {
                if snapshot.status != Some(VestingStatus::Revoked) {
                    return Err(format!("Expected Revoked, got {:?}", snapshot.status));
                }
                let expected_contract = total_amount.saturating_sub(*claimed);
                if snapshot.contract_balance != expected_contract {
                    return Err(format!(
                        "Revoked contract balance mismatch: expected {expected_contract}, got {}",
                        snapshot.contract_balance
                    ));
                }
            }
            VestingModelState::Completed { total_amount } => {
                if snapshot.status != Some(VestingStatus::Completed) {
                    return Err(format!("Expected Completed, got {:?}", snapshot.status));
                }
                if snapshot.contract_balance != 0 {
                    return Err("Completed schedule must hold 0 in contract custody".into());
                }
                if snapshot.claimed != *total_amount {
                    return Err("Completed schedule claimed must equal total_amount".into());
                }
            }
        }

        Ok(())
    }

    fn is_terminal(&self, state: &Self::State) -> bool {
        matches!(state.0, VestingModelState::Completed { .. })
    }
}

pub struct VestingModelDriver {
    pub contract_id: Address,
    pub token: Address,
    pub accounts: TestAccounts,
    pub schedule_id: Option<u64>,
}

impl VestingModelDriver {
    pub fn new(env: &Env) -> Self {
        env.mock_all_auths();
        env.ledger().set_timestamp(START_TIMESTAMP);

        let admin = Address::generate(env);
        let sac = env.register_stellar_asset_contract_v2(admin);
        let token = sac.address();
        let accounts = TestAccounts::generate(env);
        StellarAssetClient::new(env, &token).mint(&accounts.user1, &INITIAL_MINT);

        let contract_id = env.register(Vesting, ());

        Self {
            contract_id,
            token,
            accounts,
            schedule_id: None,
        }
    }

    fn client<'a>(&self, env: &'a Env) -> SorobanForgeVestingClient<'a> {
        SorobanForgeVestingClient::new(env, &self.contract_id)
    }

    fn token_client<'a>(&self, env: &'a Env) -> TokenClient<'a> {
        TokenClient::new(env, &self.token)
    }
}

impl ModelDriver<VestingLifecycleModel> for VestingModelDriver {
    fn execute_op(&mut self, env: &Env, op: &VestingModelOp) -> Result<(), String> {
        let client = self.client(env);

        match op {
            VestingModelOp::AdvanceTime { seconds } => {
                let current = env.ledger().timestamp();
                env.ledger().set_timestamp(current.saturating_add(*seconds));
                Ok(())
            }
            VestingModelOp::Create {
                amount,
                cliff,
                duration,
            } => {
                if self.schedule_id.is_some() {
                    return Err("already created".into());
                }
                let res = client.try_create_schedule(
                    &self.accounts.user1,
                    &self.accounts.user2,
                    &self.token,
                    amount,
                    cliff,
                    duration,
                );
                match res {
                    Ok(Ok(id)) => {
                        self.schedule_id = Some(id);
                        // Funder deposits grant amount into contract custody
                        let tc = self.token_client(env);
                        tc.transfer(&self.accounts.user1, &self.contract_id, amount);
                        Ok(())
                    }
                    Ok(Err(_)) => Err("conversion error".into()),
                    Err(Ok(forge_err)) => Err(format!("{forge_err:?}")),
                    Err(Err(_)) => Err("invoke error".into()),
                }
            }
            VestingModelOp::Claim => {
                let id = self.schedule_id.ok_or_else(|| "no schedule".to_string())?;
                map_result(client.try_claim(&id))
            }
            VestingModelOp::Revoke => {
                let id = self.schedule_id.ok_or_else(|| "no schedule".to_string())?;
                map_result(client.try_revoke(&id))
            }
            VestingModelOp::InvalidOp => {
                let invalid_id = 999_999_u64;
                map_result(client.try_claim(&invalid_id))
            }
        }
    }

    fn snapshot(&self, env: &Env) -> VestingSnapshot {
        let tc = self.token_client(env);
        let client = self.client(env);

        let (status, claimed) = if let Some(id) = self.schedule_id {
            let st = client.try_get_status(&id).ok().and_then(|r| r.ok());
            let cl = client
                .try_get_schedule(&id)
                .ok()
                .and_then(|r| r.ok())
                .map(|s| s.claimed)
                .unwrap_or(0);
            (st, cl)
        } else {
            (None, 0)
        };

        VestingSnapshot {
            status,
            claimed,
            contract_balance: tc.balance(&self.contract_id),
            funder_balance: tc.balance(&self.accounts.user1),
            beneficiary_balance: tc.balance(&self.accounts.user2),
            other_beneficiary_balance: tc.balance(&self.accounts.user3),
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

pub struct VestingOpGenerator;

impl OpGenerator<VestingLifecycleModel> for VestingOpGenerator {
    fn generate_op(
        &mut self,
        state: &(VestingModelState, u64),
        rng: &mut ModelRng,
    ) -> VestingModelOp {
        let (current_state, _time) = state;

        let choice = rng.bounded(10);
        match current_state {
            VestingModelState::Uncreated => match choice {
                0..=7 => VestingModelOp::Create {
                    amount: VESTING_TOTAL,
                    cliff: DEFAULT_CLIFF,
                    duration: DEFAULT_DURATION,
                },
                8 => VestingModelOp::Create {
                    amount: VESTING_TOTAL,
                    cliff: 50_000,
                    duration: 10_000, // Invalid: cliff > duration
                },
                _ => VestingModelOp::Claim, // Invalid: uncreated
            },
            VestingModelState::Active { .. } => match choice {
                0..=4 => VestingModelOp::AdvanceTime {
                    seconds: rng.gen_range_u64(1_000, 20_000),
                },
                5..=7 => VestingModelOp::Claim,
                8 => VestingModelOp::Revoke,
                _ => VestingModelOp::Create {
                    amount: VESTING_TOTAL,
                    cliff: DEFAULT_CLIFF,
                    duration: DEFAULT_DURATION,
                }, // Invalid: already created
            },
            VestingModelState::Revoked { .. } => match choice {
                0..=4 => VestingModelOp::Claim,
                5..=6 => VestingModelOp::Revoke, // Invalid: double revoke
                _ => VestingModelOp::AdvanceTime { seconds: 1_000 },
            },
            VestingModelState::Completed { .. } => match choice {
                0..=4 => VestingModelOp::Claim,  // Claim returns 0 on completed
                5..=6 => VestingModelOp::Revoke, // Invalid: cannot revoke completed
                7 => VestingModelOp::InvalidOp,
                _ => VestingModelOp::AdvanceTime { seconds: 1_000 },
            },
        }
    }
}

#[test]
fn test_vesting_model_check_deterministic_seeds() {
    let seeds = [101_u64, 202, 303, 404, 505, 606, 707, 808];
    for seed in seeds {
        let env = Env::default();
        let model = VestingLifecycleModel {
            initial_time: START_TIMESTAMP,
        };
        let mut driver = VestingModelDriver::new(&env);
        let mut generator = VestingOpGenerator;

        assert_model_check(&env, &model, &mut driver, &mut generator, seed, 25);
    }
}
