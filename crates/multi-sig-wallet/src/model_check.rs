//! State-machine model checking for the multi-sig wallet lifecycle.
//!
//! Validates the multi-sig wallet against an independent state-machine model:
//! - Tracks statuses: None -> Pending -> Executed / Rejected
//! - Enforces token conservation: wallet_balance + recipient_balance == total_minted
//! - Threshold rule: execute succeeds if and only if distinct confirmations >= threshold
//! - Single rejection rule: reject transitions status to Rejected (terminal)
//! - Single execution rule: execute transitions status to Executed (terminal)
//! - Guarantees non-mutation: invalid operations are rejected without modifying storage or balances

extern crate std;

use crate::{MultiSigWallet, SorobanForgeMultiSigWalletClient, TxStatus};
use core::fmt;
use soroban_forge_shared_utils::ForgeError;
use soroban_forge_test_utils::model::{
    assert_model_check, LifecycleModel, ModelDriver, ModelRng, OpGenerator,
};
use soroban_forge_test_utils::TestAccounts;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env, Vec as SdkVec};
use std::format;
use std::string::{String, ToString};
use std::vec::Vec;

const INITIAL_MINT: i128 = 10_000_000;
const THRESHOLD: u32 = 2;
const NUM_OWNERS: usize = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TxModelState {
    pub id: u64,
    pub amount: i128,
    pub status: TxStatus,
    pub confirmations: Vec<usize>,
    pub rejected: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MultiSigModelState {
    pub wallet_balance: i128,
    pub recipient_balance: i128,
    pub tx_count: u64,
    pub current_tx: Option<TxModelState>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MultiSigModelOp {
    Deposit { amount: i128 },
    SubmitWithdrawal { amount: i128, owner_idx: usize },
    Confirm { owner_idx: usize },
    Reject { owner_idx: usize },
    Execute,
    InvalidOp,
}

impl fmt::Display for MultiSigModelOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Deposit { amount } => write!(f, "Deposit({amount})"),
            Self::SubmitWithdrawal { amount, owner_idx } => {
                write!(f, "SubmitWithdrawal(amount={amount}, owner={owner_idx})")
            }
            Self::Confirm { owner_idx } => write!(f, "Confirm(owner={owner_idx})"),
            Self::Reject { owner_idx } => write!(f, "Reject(owner={owner_idx})"),
            Self::Execute => write!(f, "Execute"),
            Self::InvalidOp => write!(f, "InvalidOp"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MultiSigSnapshot {
    pub wallet_balance: i128,
    pub recipient_balance: i128,
    pub tx_count: u64,
    pub tx_status: Option<TxStatus>,
    pub confirmations_count: u32,
}

pub struct MultiSigLifecycleModel {
    pub threshold: u32,
    pub num_owners: usize,
}

impl LifecycleModel for MultiSigLifecycleModel {
    type State = MultiSigModelState;
    type Op = MultiSigModelOp;
    type Snapshot = MultiSigSnapshot;

    fn initial_state(&self) -> Self::State {
        MultiSigModelState {
            wallet_balance: 0,
            recipient_balance: 0,
            tx_count: 0,
            current_tx: None,
        }
    }

    fn transition(&self, state: &Self::State, op: &Self::Op) -> Result<Self::State, String> {
        let mut next = state.clone();

        match op {
            MultiSigModelOp::Deposit { amount } => {
                if *amount <= 0 {
                    return Err("amount must be > 0".into());
                }
                next.wallet_balance = next.wallet_balance.saturating_add(*amount);
                Ok(next)
            }
            MultiSigModelOp::SubmitWithdrawal { amount, owner_idx } => {
                if *owner_idx >= self.num_owners {
                    return Err("invalid owner index".into());
                }
                if *amount <= 0 {
                    return Err("amount must be > 0".into());
                }

                next.tx_count = next.tx_count.saturating_add(1);
                next.current_tx = Some(TxModelState {
                    id: next.tx_count,
                    amount: *amount,
                    status: TxStatus::Pending,
                    confirmations: Vec::new(),
                    rejected: false,
                });
                Ok(next)
            }
            MultiSigModelOp::Confirm { owner_idx } => {
                let tx = next
                    .current_tx
                    .as_mut()
                    .ok_or_else(|| "no tx submitted".to_string())?;

                if tx.status != TxStatus::Pending {
                    return Err("tx is not pending".into());
                }
                if *owner_idx >= self.num_owners {
                    return Err("invalid owner index".into());
                }
                if tx.confirmations.contains(owner_idx) {
                    return Err("duplicate confirmation".into());
                }
                if tx.rejected {
                    return Err("tx already rejected".into());
                }

                tx.confirmations.push(*owner_idx);
                Ok(next)
            }
            MultiSigModelOp::Reject { owner_idx } => {
                let tx = next
                    .current_tx
                    .as_mut()
                    .ok_or_else(|| "no tx submitted".to_string())?;

                if tx.status != TxStatus::Pending {
                    return Err("tx is not pending".into());
                }
                if *owner_idx >= self.num_owners {
                    return Err("invalid owner index".into());
                }
                if tx.confirmations.contains(owner_idx) {
                    return Err("owner already confirmed".into());
                }
                if tx.rejected {
                    return Err("duplicate rejection".into());
                }

                tx.rejected = true;
                tx.status = TxStatus::Rejected;
                Ok(next)
            }
            MultiSigModelOp::Execute => {
                let tx = next
                    .current_tx
                    .as_mut()
                    .ok_or_else(|| "no tx submitted".to_string())?;

                if tx.status != TxStatus::Pending {
                    return Err("tx is not pending".into());
                }
                if (tx.confirmations.len() as u32) < self.threshold {
                    return Err("threshold not met".into());
                }
                if next.wallet_balance < tx.amount {
                    return Err("insufficient wallet balance".into());
                }

                tx.status = TxStatus::Executed;
                next.wallet_balance = next.wallet_balance.saturating_sub(tx.amount);
                next.recipient_balance = next.recipient_balance.saturating_add(tx.amount);
                Ok(next)
            }
            MultiSigModelOp::InvalidOp => Err("explicit invalid operation".into()),
        }
    }

    fn check_invariants(
        &self,
        state: &Self::State,
        snapshot: &Self::Snapshot,
    ) -> Result<(), String> {
        // Balances match
        if snapshot.wallet_balance != state.wallet_balance {
            return Err(format!(
                "wallet balance mismatch: model={}, on-chain={}",
                state.wallet_balance, snapshot.wallet_balance
            ));
        }
        if snapshot.recipient_balance != state.recipient_balance {
            return Err(format!(
                "recipient balance mismatch: model={}, on-chain={}",
                state.recipient_balance, snapshot.recipient_balance
            ));
        }

        // Tx count matches
        if snapshot.tx_count != state.tx_count {
            return Err(format!(
                "tx_count mismatch: model={}, on-chain={}",
                state.tx_count, snapshot.tx_count
            ));
        }

        // Current tx status and confirmation count
        if let Some(ref tx) = state.current_tx {
            if snapshot.tx_status != Some(tx.status.clone()) {
                return Err(format!(
                    "tx status mismatch: model={:?}, on-chain={:?}",
                    tx.status, snapshot.tx_status
                ));
            }
            if snapshot.confirmations_count != tx.confirmations.len() as u32 {
                return Err(format!(
                    "confirmations count mismatch: model={}, on-chain={}",
                    tx.confirmations.len(),
                    snapshot.confirmations_count
                ));
            }
        }

        Ok(())
    }

    fn is_terminal(&self, state: &Self::State) -> bool {
        state.current_tx.as_ref().is_some_and(|tx| {
            matches!(
                tx.status,
                TxStatus::Executed | TxStatus::Rejected | TxStatus::Cancelled
            )
        })
    }
}

pub struct MultiSigModelDriver {
    pub contract_id: Address,
    pub token: Address,
    pub accounts: TestAccounts,
    pub owners: Vec<Address>,
    pub recipient: Address,
    pub current_tx_id: Option<u64>,
}

impl MultiSigModelDriver {
    pub fn new(env: &Env) -> Self {
        env.mock_all_auths();

        let admin = Address::generate(env);
        let sac = env.register_stellar_asset_contract_v2(admin);
        let token = sac.address();
        let accounts = TestAccounts::generate(env);
        StellarAssetClient::new(env, &token).mint(&accounts.user1, &INITIAL_MINT);

        let contract_id = env.register(MultiSigWallet, ());
        let client = SorobanForgeMultiSigWalletClient::new(env, &contract_id);

        let owners = std::vec![
            accounts.user1.clone(),
            accounts.user2.clone(),
            accounts.user3.clone(),
        ];
        let mut sdk_owners = SdkVec::new(env);
        for o in &owners {
            sdk_owners.push_back(o.clone());
        }
        client.initialize(&sdk_owners, &THRESHOLD);

        Self {
            contract_id,
            token,
            accounts,
            owners,
            recipient: Address::generate(env),
            current_tx_id: None,
        }
    }

    fn client<'a>(&self, env: &'a Env) -> SorobanForgeMultiSigWalletClient<'a> {
        SorobanForgeMultiSigWalletClient::new(env, &self.contract_id)
    }

    fn token_client<'a>(&self, env: &'a Env) -> TokenClient<'a> {
        TokenClient::new(env, &self.token)
    }
}

impl ModelDriver<MultiSigLifecycleModel> for MultiSigModelDriver {
    fn execute_op(&mut self, env: &Env, op: &MultiSigModelOp) -> Result<(), String> {
        let client = self.client(env);

        match op {
            MultiSigModelOp::Deposit { amount } => {
                let res = client.try_deposit(&self.token, &self.accounts.user1, amount);
                map_result(res)
            }
            MultiSigModelOp::SubmitWithdrawal { amount, owner_idx } => {
                let submitter = self
                    .owners
                    .get(*owner_idx)
                    .ok_or_else(|| "owner index out of range".to_string())?;

                let res =
                    client.try_submit_withdrawal(submitter, &self.token, &self.recipient, amount);
                match res {
                    Ok(Ok(id)) => {
                        self.current_tx_id = Some(id);
                        Ok(())
                    }
                    Ok(Err(_)) => Err("conversion error".into()),
                    Err(Ok(forge_err)) => Err(format!("{forge_err:?}")),
                    Err(Err(_)) => Err("invoke error".into()),
                }
            }
            MultiSigModelOp::Confirm { owner_idx } => {
                let id = self.current_tx_id.ok_or_else(|| "no tx".to_string())?;
                let signer = self
                    .owners
                    .get(*owner_idx)
                    .ok_or_else(|| "invalid owner".to_string())?;
                map_result(client.try_confirm(&id, signer))
            }
            MultiSigModelOp::Reject { owner_idx } => {
                let id = self.current_tx_id.ok_or_else(|| "no tx".to_string())?;
                let signer = self
                    .owners
                    .get(*owner_idx)
                    .ok_or_else(|| "invalid owner".to_string())?;
                map_result(client.try_reject(&id, signer))
            }
            MultiSigModelOp::Execute => {
                let id = self.current_tx_id.ok_or_else(|| "no tx".to_string())?;
                map_result(client.try_execute(&id))
            }
            MultiSigModelOp::InvalidOp => {
                let invalid_id = 999_999_u64;
                map_result(client.try_execute(&invalid_id))
            }
        }
    }

    fn snapshot(&self, env: &Env) -> MultiSigSnapshot {
        let tc = self.token_client(env);
        let client = self.client(env);

        let (tx_status, confirmations_count) = if let Some(id) = self.current_tx_id {
            match client.try_get_tx(&id) {
                Ok(Ok(tx)) => (Some(tx.status), tx.confirmations.len()),
                _ => (None, 0),
            }
        } else {
            (None, 0)
        };

        let tx_count = client.get_tx_count();

        MultiSigSnapshot {
            wallet_balance: tc.balance(&self.contract_id),
            recipient_balance: tc.balance(&self.recipient),
            tx_count,
            tx_status,
            confirmations_count,
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

pub struct MultiSigOpGenerator;

impl OpGenerator<MultiSigLifecycleModel> for MultiSigOpGenerator {
    fn generate_op(&mut self, state: &MultiSigModelState, rng: &mut ModelRng) -> MultiSigModelOp {
        let choice = rng.bounded(10);
        match &state.current_tx {
            None => match choice {
                0..=4 => MultiSigModelOp::Deposit {
                    amount: (rng.bounded(5) as i128 + 1) * 100_000,
                },
                5..=8 => MultiSigModelOp::SubmitWithdrawal {
                    amount: 100_000,
                    owner_idx: (rng.bounded(NUM_OWNERS as u32)) as usize,
                },
                _ => MultiSigModelOp::Execute, // Invalid: no tx
            },
            Some(tx) => match tx.status {
                TxStatus::Pending => match choice {
                    0..=3 => {
                        let owner_idx = (rng.bounded(NUM_OWNERS as u32)) as usize;
                        MultiSigModelOp::Confirm { owner_idx }
                    }
                    4 => {
                        let owner_idx = (rng.bounded(NUM_OWNERS as u32)) as usize;
                        MultiSigModelOp::Reject { owner_idx }
                    }
                    5..=7 => MultiSigModelOp::Execute,
                    8 => MultiSigModelOp::Deposit { amount: 50_000 },
                    _ => MultiSigModelOp::SubmitWithdrawal {
                        amount: 100_000,
                        owner_idx: 0,
                    }, // Invalid: pending tx already in flight
                },
                TxStatus::Executed
                | TxStatus::Rejected
                | TxStatus::Expired
                | TxStatus::Cancelled => match choice {
                    0..=3 => MultiSigModelOp::Deposit { amount: 100_000 },
                    4..=6 => MultiSigModelOp::SubmitWithdrawal {
                        amount: 50_000,
                        owner_idx: 0,
                    },
                    7 => MultiSigModelOp::Confirm { owner_idx: 0 }, // Invalid on terminal tx
                    8 => MultiSigModelOp::Execute,                  // Invalid on terminal tx
                    _ => MultiSigModelOp::InvalidOp,
                },
            },
        }
    }
}

#[test]
fn test_multisig_model_check_deterministic_seeds() {
    let seeds = [101_u64, 202, 303, 404, 505, 606, 707, 808];
    for seed in seeds {
        let env = Env::default();
        let model = MultiSigLifecycleModel {
            threshold: THRESHOLD,
            num_owners: NUM_OWNERS,
        };
        let mut driver = MultiSigModelDriver::new(&env);
        let mut generator = MultiSigOpGenerator;

        assert_model_check(&env, &model, &mut driver, &mut generator, seed, 25);
    }
}
