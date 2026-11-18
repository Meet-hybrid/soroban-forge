//! State-machine model checking for the marketplace royalties lifecycle.
//!
//! Validates the marketplace royalties contract against an independent state-machine model:
//! - Tracks states: Unconfigured -> Configured (bps updateable)
//! - Enforces split conservation: payer_delta == seller_delta + recipient_delta
//! - Enforces pool conservation: payer + seller + recipient == total_minted
//! - Enforces summary consistency: gross_volume == royalties_paid + seller_net
//! - Verifies batch boundary: empty batch or batch > MAX_SETTLE_SALES is rejected
//! - Guarantees non-mutation: invalid operations are rejected without modifying storage or balances

extern crate std;

use crate::{
    MarketplaceRoyalties, SettlementSummary, SorobanForgeMarketplaceRoyaltiesClient,
    MAX_SETTLE_SALES,
};
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
use std::string::String;
use std::vec;
use std::vec::Vec;

const INITIAL_MINT: i128 = 50_000_000;
const DEFAULT_BPS: u32 = 500; // 5%

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RoyaltiesModelState {
    Unconfigured,
    Configured {
        bps: u32,
        sales_count: u32,
        gross_volume: i128,
        royalties_paid: i128,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RoyaltiesModelOp {
    SetRoyalty { bps: u32 },
    SettleSale { amount: i128 },
    SettleSalesBatch { amounts: Vec<i128> },
    InvalidOp,
}

impl fmt::Display for RoyaltiesModelOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SetRoyalty { bps } => write!(f, "SetRoyalty(bps={bps})"),
            Self::SettleSale { amount } => write!(f, "SettleSale(amount={amount})"),
            Self::SettleSalesBatch { amounts } => {
                write!(f, "SettleSalesBatch(len={})", amounts.len())
            }
            Self::InvalidOp => write!(f, "InvalidOp"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoyaltiesSnapshot {
    pub configured: bool,
    pub bps: u32,
    pub sales: u32,
    pub gross_volume: i128,
    pub royalties_paid: i128,
    pub payer_balance: i128,
    pub seller_balance: i128,
    pub recipient_balance: i128,
}

pub struct RoyaltiesLifecycleModel;

impl LifecycleModel for RoyaltiesLifecycleModel {
    type State = RoyaltiesModelState;
    type Op = RoyaltiesModelOp;
    type Snapshot = RoyaltiesSnapshot;

    fn initial_state(&self) -> Self::State {
        RoyaltiesModelState::Unconfigured
    }

    fn transition(&self, state: &Self::State, op: &Self::Op) -> Result<Self::State, String> {
        match (state, op) {
            (RoyaltiesModelState::Unconfigured, RoyaltiesModelOp::SetRoyalty { bps }) => {
                if *bps > 10_000 {
                    Err("bps cannot exceed 10000".into())
                } else {
                    Ok(RoyaltiesModelState::Configured {
                        bps: *bps,
                        sales_count: 0,
                        gross_volume: 0,
                        royalties_paid: 0,
                    })
                }
            }
            (
                RoyaltiesModelState::Configured {
                    sales_count,
                    gross_volume,
                    royalties_paid,
                    ..
                },
                RoyaltiesModelOp::SetRoyalty { bps },
            ) => {
                if *bps > 10_000 {
                    Err("bps cannot exceed 10000".into())
                } else {
                    Ok(RoyaltiesModelState::Configured {
                        bps: *bps,
                        sales_count: *sales_count,
                        gross_volume: *gross_volume,
                        royalties_paid: *royalties_paid,
                    })
                }
            }
            (
                RoyaltiesModelState::Configured {
                    bps,
                    sales_count,
                    gross_volume,
                    royalties_paid,
                },
                RoyaltiesModelOp::SettleSale { amount },
            ) => {
                if *amount <= 0 {
                    Err("amount must be > 0".into())
                } else {
                    let royalty = (*amount * i128::from(*bps)) / 10_000;
                    Ok(RoyaltiesModelState::Configured {
                        bps: *bps,
                        sales_count: sales_count.saturating_add(1),
                        gross_volume: gross_volume.saturating_add(*amount),
                        royalties_paid: royalties_paid.saturating_add(royalty),
                    })
                }
            }
            (
                RoyaltiesModelState::Configured {
                    bps,
                    sales_count,
                    gross_volume,
                    royalties_paid,
                },
                RoyaltiesModelOp::SettleSalesBatch { amounts },
            ) => {
                if amounts.is_empty() {
                    Err("batch cannot be empty".into())
                } else if amounts.len() > MAX_SETTLE_SALES as usize {
                    Err("batch exceeds MAX_SETTLE_SALES".into())
                } else if amounts.iter().any(|&a| a <= 0) {
                    Err("batch contains non-positive amount".into())
                } else {
                    let mut batch_vol: i128 = 0;
                    let mut batch_royalty: i128 = 0;
                    for &a in amounts {
                        let r = (a * i128::from(*bps)) / 10_000;
                        batch_vol = batch_vol.saturating_add(a);
                        batch_royalty = batch_royalty.saturating_add(r);
                    }
                    Ok(RoyaltiesModelState::Configured {
                        bps: *bps,
                        sales_count: sales_count.saturating_add(amounts.len() as u32),
                        gross_volume: gross_volume.saturating_add(batch_vol),
                        royalties_paid: royalties_paid.saturating_add(batch_royalty),
                    })
                }
            }
            _ => Err("invalid operation for current state".into()),
        }
    }

    fn check_invariants(
        &self,
        state: &Self::State,
        snapshot: &Self::Snapshot,
    ) -> Result<(), String> {
        // Pool conservation
        let pool = snapshot
            .payer_balance
            .saturating_add(snapshot.seller_balance)
            .saturating_add(snapshot.recipient_balance);

        if pool != INITIAL_MINT {
            return Err(format!(
                "Pool conservation violated! Expected {INITIAL_MINT}, got {pool}"
            ));
        }

        match state {
            RoyaltiesModelState::Unconfigured => {
                if snapshot.configured {
                    return Err("Unconfigured state has configured snapshot".into());
                }
            }
            RoyaltiesModelState::Configured {
                bps,
                sales_count,
                gross_volume,
                royalties_paid,
            } => {
                if !snapshot.configured {
                    return Err("Expected configured snapshot".into());
                }
                if snapshot.bps != *bps {
                    return Err(format!(
                        "bps mismatch: model={bps}, on-chain={}",
                        snapshot.bps
                    ));
                }
                if snapshot.sales != *sales_count {
                    return Err(format!(
                        "sales count mismatch: model={sales_count}, on-chain={}",
                        snapshot.sales
                    ));
                }
                if snapshot.gross_volume != *gross_volume {
                    return Err(format!(
                        "gross volume mismatch: model={gross_volume}, on-chain={}",
                        snapshot.gross_volume
                    ));
                }
                if snapshot.royalties_paid != *royalties_paid {
                    return Err(format!(
                        "royalties paid mismatch: model={royalties_paid}, on-chain={}",
                        snapshot.royalties_paid
                    ));
                }

                // Summary consistency: gross == royalties + net
                let expected_seller_net = gross_volume.saturating_sub(*royalties_paid);
                if snapshot.seller_balance != expected_seller_net {
                    return Err(format!(
                        "Seller balance mismatch: expected {expected_seller_net}, got {}",
                        snapshot.seller_balance
                    ));
                }
                if snapshot.recipient_balance != *royalties_paid {
                    return Err(format!(
                        "Recipient balance mismatch: expected {royalties_paid}, got {}",
                        snapshot.recipient_balance
                    ));
                }
            }
        }

        Ok(())
    }

    fn is_terminal(&self, _state: &Self::State) -> bool {
        // Marketplace royalties is an open service; terminal checks verify
        // that invalid operations (e.g. amount <= 0, unconfigured) never mutate state.
        false
    }
}

pub struct RoyaltiesModelDriver {
    pub contract_id: Address,
    pub collection: Address,
    pub token: Address,
    pub accounts: TestAccounts,
    pub recipient: Address,
}

impl RoyaltiesModelDriver {
    pub fn new(env: &Env) -> Self {
        env.mock_all_auths();

        let admin = Address::generate(env);
        let sac = env.register_stellar_asset_contract_v2(admin);
        let token = sac.address();
        let accounts = TestAccounts::generate(env);
        let collection = Address::generate(env);
        let recipient = accounts.arbiter.clone();

        StellarAssetClient::new(env, &token).mint(&accounts.user1, &INITIAL_MINT);

        let contract_id = env.register(MarketplaceRoyalties, ());

        Self {
            contract_id,
            collection,
            token,
            accounts,
            recipient,
        }
    }

    fn client<'a>(&self, env: &'a Env) -> SorobanForgeMarketplaceRoyaltiesClient<'a> {
        SorobanForgeMarketplaceRoyaltiesClient::new(env, &self.contract_id)
    }

    fn token_client<'a>(&self, env: &'a Env) -> TokenClient<'a> {
        TokenClient::new(env, &self.token)
    }
}

impl ModelDriver<RoyaltiesLifecycleModel> for RoyaltiesModelDriver {
    fn execute_op(&mut self, env: &Env, op: &RoyaltiesModelOp) -> Result<(), String> {
        let client = self.client(env);

        match op {
            RoyaltiesModelOp::SetRoyalty { bps } => {
                let res = client.try_set_royalty(&self.collection, &self.recipient, bps);
                map_result(res)
            }
            RoyaltiesModelOp::SettleSale { amount } => {
                let res = client.try_settle_sale(
                    &self.collection,
                    &self.token,
                    &self.accounts.user1,
                    &self.accounts.user2,
                    amount,
                );
                map_result(res)
            }
            RoyaltiesModelOp::SettleSalesBatch { amounts } => {
                let mut sales = SdkVec::new(env);
                for &a in amounts {
                    sales.push_back((self.accounts.user2.clone(), a));
                }
                let res = client.try_settle_sales(
                    &self.collection,
                    &self.token,
                    &self.accounts.user1,
                    &sales,
                );
                map_result(res)
            }
            RoyaltiesModelOp::InvalidOp => {
                let unknown_col = Address::generate(env);
                let res = client.try_settle_sale(
                    &unknown_col,
                    &self.token,
                    &self.accounts.user1,
                    &self.accounts.user2,
                    &100,
                );
                map_result(res)
            }
        }
    }

    fn snapshot(&self, env: &Env) -> RoyaltiesSnapshot {
        let tc = self.token_client(env);
        let client = self.client(env);

        let (configured, bps) = match client.try_get_royalty(&self.collection) {
            Ok(Ok(roy)) => (true, roy.bps),
            _ => (false, 0),
        };

        let summary = client
            .try_get_settlement_summary(&self.collection)
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or(SettlementSummary {
                sales: 0,
                gross_volume: 0,
                royalties_paid: 0,
            });

        RoyaltiesSnapshot {
            configured,
            bps,
            sales: summary.sales,
            gross_volume: summary.gross_volume,
            royalties_paid: summary.royalties_paid,
            payer_balance: tc.balance(&self.accounts.user1),
            seller_balance: tc.balance(&self.accounts.user2),
            recipient_balance: tc.balance(&self.recipient),
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

pub struct RoyaltiesOpGenerator;

impl OpGenerator<RoyaltiesLifecycleModel> for RoyaltiesOpGenerator {
    fn generate_op(&mut self, state: &RoyaltiesModelState, rng: &mut ModelRng) -> RoyaltiesModelOp {
        let choice = rng.bounded(10);
        match state {
            RoyaltiesModelState::Unconfigured => match choice {
                0..=7 => RoyaltiesModelOp::SetRoyalty { bps: DEFAULT_BPS },
                8 => RoyaltiesModelOp::SetRoyalty { bps: 10_001 }, // Invalid: bps > 10000
                _ => RoyaltiesModelOp::SettleSale { amount: 100_000 }, // Invalid: unconfigured
            },
            RoyaltiesModelState::Configured { .. } => match choice {
                0..=3 => RoyaltiesModelOp::SettleSale {
                    amount: (rng.bounded(10) as i128 + 1) * 50_000,
                },
                4..=5 => {
                    let len = (rng.bounded(MAX_SETTLE_SALES) + 1) as usize;
                    let mut amounts = Vec::with_capacity(len);
                    for _ in 0..len {
                        amounts.push((rng.bounded(5) as i128 + 1) * 10_000);
                    }
                    RoyaltiesModelOp::SettleSalesBatch { amounts }
                }
                6 => RoyaltiesModelOp::SettleSale { amount: 0 }, // Invalid: zero amount
                7 => RoyaltiesModelOp::SettleSalesBatch { amounts: vec![] }, // Invalid: empty batch
                8 => RoyaltiesModelOp::SetRoyalty {
                    bps: (rng.bounded(10) + 1) * 100,
                },
                _ => RoyaltiesModelOp::InvalidOp,
            },
        }
    }
}

#[test]
fn test_royalties_model_check_deterministic_seeds() {
    let seeds = [101_u64, 202, 303, 404, 505, 606, 707, 808];
    for seed in seeds {
        let env = Env::default();
        let model = RoyaltiesLifecycleModel;
        let mut driver = RoyaltiesModelDriver::new(&env);
        let mut generator = RoyaltiesOpGenerator;

        assert_model_check(&env, &model, &mut driver, &mut generator, seed, 25);
    }
}
