//! Randomized invariant suite (proptest).
//!
//! The hand-written unit suite pins behaviour on known, fixed values; this
//! module systematically tests the contract's fund-safety, arithmetic conservation,
//! and monotonicity invariants over large spaces of generated inputs.
//!
//! Eight property invariants are exercised:
//!
//! **P1 — Total Conservation & Residue Safety.** For random amounts, cliffs,
//! and durations: across arbitrary sequences of timestamps and interleaved claims,
//! `claimed + remaining_claimable <= total_amount` holds at every point in time.
//! Rounding residue from floor-division never leaks tokens or causes payouts to
//! exceed `total_amount`. Upon full duration elapsed and final claim, the contract
//! retains 0 tokens and the beneficiary holds `total_amount` exactly.
//!
//! **P2 — Monotonicity.** For arbitrary monotonically non-decreasing timestamp
//! progressions: `vested(t)` is strictly monotonic non-decreasing ($t_a \le t_b \implies \text{vested}(t_a) \le \text{vested}(t_b)$),
//! cumulative `claimed` never decreases, and `claimable` increases monotonically
//! over time until `duration` (in the absence of claims).
//!
//! **P3 — Arbitrary Action Sequences.** For random sequences of time advancements
//! and claim attempts: `claim()` returns exactly the newly vested amount or `0`,
//! zero-amount claims make no token transfers, and total pool tokens (`beneficiary + contract`)
//! are conserved identically.
//!
//! **P4 — Tamper-Resilient Conservation.** An attacker attempting to mutate stored
//! schedule parameters (amount, claimed, start, cliff, duration) between calls can
//! never move value out of the pool beyond the token contract's balance.
//!
//! **P5 — Tranche Conservation.** For random unlock tables (including TGE
//! and never-unlocking `u64::MAX` tranches) and random claim sequences:
//! `claimed` never exceeds the table total, the contract's custody balance
//! decreases by exactly what the beneficiary gains, repeated claims never
//! overpay, the final claim transitions the schedule to `Completed` when no
//! unreachable tranche remains, and a failed transfer leaves stored state
//! untouched.
//!
//! **P6 — Tranche Claimable Monotonic With Exact Sums.** For random unlock
//! tables sampled at increasing timestamps: `claimable` is non-decreasing
//! over time and equals the exact partial sum of unlocked-but-unclaimed
//! tranches — 0 before the first offset, exact step sums between offsets,
//! and the full unlockable total at/after the last reachable offset.
//!
//! **P7 — Revoke, Claim, and Reassignment Conservation.** For random linear
//! schedules: claims before and after revocation plus reassigned-beneficiary
//! payouts never exceed the original allocation; revoked value never becomes
//! claimable, and repeated terminal operations do not release more value.
//!
//! **P8 — Beneficiary Reassignment Chains.** For random linear schedules and
//! chains of up to 10 successive `reassign_beneficiary` calls at random,
//! non-decreasing timestamps (pre-cliff, mid-vesting, and past `duration`),
//! with random interleaved `claim_for` calls and occasional reassignment back
//! to a former beneficiary, checked against an independent tenure model:
//! - *Strict conservation:* `sum(former_beneficiary_claims) +
//!   current_beneficiary_claimable + remaining_unvested + claimed ==
//!   total_amount` after every step (`claimed` is the value already paid
//!   out, which the issue's statement leaves implicit), and custody plus
//!   beneficiary balances always sum to `total_amount`.
//! - *Double-claim immunity:* every address can claim exactly what vested
//!   during its own tenures, in any claim order, and a repeated claim pays 0.
//! - *Duration monotonicity:* `start`, `cliff`, `duration`, and
//!   `total_amount` never change; the schedule stays `Locked` before the
//!   original cliff and is fully claimable at the original `duration`.
//! - `reassignment_count` equals the number of successful reassignments, and
//!   a rejected reassignment leaves it unchanged.
//!
//! Runs are deterministic with reproducible seeds. Override the case count with
//! `PROPTEST_CASES=n cargo test -p soroban-forge-vesting props`.

use crate::{SorobanForgeVestingClient, Tranche, Vesting, VestingSchedule, VestingStatus};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env};
use std::vec;
use std::vec::Vec;

const START: u64 = 1_000_000;
const MAX_AMOUNT: i128 = 1_000_000_000_000;
const MAX_DURATION: u64 = 10 * 365 * 24 * 3600; // 10 years in seconds
const MAX_TRANCHES: u32 = 32;

// Clock reading beyond every reachable tranche offset. Deliberately not
// `u64::MAX` so `start + offset` arithmetic in product code cannot overflow;
// `u64::MAX - START` elapsed still excludes a `u64::MAX` tranche.
const FAR_FUTURE: u64 = u64::MAX / 2;

// -----------------------------------------------------------------------
// World: one fresh env, SAC, vesting contract, and accounts per case
// -----------------------------------------------------------------------

struct World {
    env: Env,
    token: Address,
    vesting: Address,
    funder: Address,
    beneficiary: Address,
}

fn setup_world() -> World {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);

    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin);
    let token = sac.address();

    let vesting = env.register(Vesting, ());

    World {
        token,
        vesting,
        funder: Address::generate(&env),
        beneficiary: Address::generate(&env),
        env,
    }
}

impl World {
    fn vesting_client(&self) -> SorobanForgeVestingClient<'_> {
        SorobanForgeVestingClient::new(&self.env, &self.vesting)
    }

    fn token_client(&self) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.token)
    }

    fn mint_to_contract(&self, amount: i128) {
        StellarAssetClient::new(&self.env, &self.token).mint(&self.vesting, &amount);
    }

    fn create(&self, amount: i128, cliff: u64, duration: u64) -> u64 {
        self.vesting_client().create_schedule(
            &self.funder,
            &self.beneficiary,
            &self.token,
            &amount,
            &cliff,
            &duration,
        )
    }

    /// Create a tranche schedule, funding custody with the table total first
    /// (same mint-then-create shape as the linear fixture helpers).
    fn create_tranche(&self, tranches: &soroban_sdk::Vec<Tranche>, total_amount: i128) -> u64 {
        self.mint_to_contract(total_amount);
        self.vesting_client().create_tranche_schedule(
            &self.funder,
            &self.beneficiary,
            &self.token,
            tranches,
        )
    }

    /// Balances of (beneficiary, contract) — the whole pool.
    fn pool(&self) -> (i128, i128) {
        let t = self.token_client();
        (t.balance(&self.beneficiary), t.balance(&self.vesting))
    }

    /// The conserved total: everything held by the contract and the beneficiary.
    fn pool_total(&self) -> i128 {
        let (b, c) = self.pool();
        b + c
    }

    fn balance(&self, account: &Address) -> i128 {
        self.token_client().balance(account)
    }
}

// -----------------------------------------------------------------------
// Strategies
// -----------------------------------------------------------------------

fn arb_schedule_params() -> impl Strategy<Value = (i128, u64, u64)> {
    (1i128..=MAX_AMOUNT, 1u64..=MAX_DURATION)
        .prop_flat_map(|(amount, duration)| (Just(amount), 0u64..=duration, Just(duration)))
}

/// Build a host `Vec<Tranche>` from a raw `(unlock_at, amount)` table.
///
/// Strategies produce plain tuples because proptest strategies run outside
/// any `Env`; host objects like `soroban_sdk::Vec` must be allocated in the
/// case's own environment, which only exists once the test body starts.
fn to_tranche_vec(env: &Env, table: &[(u64, i128)]) -> soroban_sdk::Vec<Tranche> {
    let mut tranches = soroban_sdk::Vec::new(env);
    for (unlock_at, amount) in table {
        tranches.push_back(Tranche {
            unlock_at: *unlock_at,
            amount: *amount,
        });
    }
    tranches
}

/// Sum of every tranche amount, including never-unlocking ones.
fn table_total(table: &[(u64, i128)]) -> i128 {
    table.iter().map(|(_, amount)| *amount).sum()
}

/// Sum of tranches that can actually unlock (everything but `u64::MAX`).
fn unlockable_total(table: &[(u64, i128)]) -> i128 {
    table
        .iter()
        .filter(|(unlock_at, _)| *unlock_at != u64::MAX)
        .map(|(_, amount)| *amount)
        .sum()
}

/// Exact mirror of `tranche_unlocked`: cumulative sum over tranches whose
/// offset has elapsed at `elapsed` seconds after `start`.
fn expected_unlocked_at(table: &[(u64, i128)], elapsed: u64) -> i128 {
    table
        .iter()
        .filter(|(unlock_at, _)| *unlock_at <= elapsed)
        .map(|(_, amount)| *amount)
        .sum()
}

/// Exact mirror of `tranche_claimable` (unlocked minus claimed). The contract
/// uses checked subtraction, so this is exact, not saturating.
fn expected_claimable_at(table: &[(u64, i128)], elapsed: u64, claimed: i128) -> i128 {
    expected_unlocked_at(table, elapsed) - claimed
}

/// Generate `n` strictly increasing offsets in `lo..=hi` with random gaps.
///
/// Sorted uniform draws already yield exponentially-distributed gaps; entries
/// that collide are bumped forward, which keeps the sequence strictly
/// increasing without collapsing the step function into uniform steps
/// (uniform gaps would make the exact-sum property trivially easy).
fn strictly_increasing_offsets(n: usize, lo: u64, hi: u64) -> impl Strategy<Value = Vec<u64>> {
    prop::collection::vec(lo..=hi, n).prop_map(|mut offsets| {
        offsets.sort_unstable();
        for i in 1..offsets.len() {
            if offsets[i] <= offsets[i - 1] {
                offsets[i] = offsets[i - 1].saturating_add(1);
            }
        }
        offsets
    })
}

/// Positive per-tranche amounts summing to at most `cap * n`.
fn positive_amounts(n: usize, per_tranche_cap: i128) -> impl Strategy<Value = Vec<i128>> {
    prop::collection::vec(1i128..=per_tranche_cap, n)
}

/// Random tranche table: 1..=MAX_TRANCHES entries, every amount > 0,
/// strictly increasing offsets, random gaps between offsets.
///
/// Weighted mix of interesting shapes:
/// - plain random tables,
/// - a TGE first tranche (`unlock_at == 0`),
/// - a never-unlocking tail tranche (`unlock_at == u64::MAX`),
/// - both a TGE head and a `u64::MAX` tail,
/// - near-`i128::MAX` cumulative totals.
fn arb_tranche_table() -> impl Strategy<Value = Vec<(u64, i128)>> {
    prop_oneof![
        4 => (1u32..=MAX_TRANCHES).prop_flat_map(|n| {
            let cap = MAX_AMOUNT / n as i128;
            strictly_increasing_offsets(n as usize, 0, MAX_DURATION).prop_flat_map(move |offsets| {
                positive_amounts(n as usize, cap)
                    .prop_map(move |amounts| offsets.clone().into_iter().zip(amounts).collect())
            })
        }),
        2 => (1u32..=MAX_TRANCHES).prop_flat_map(|n| {
            let cap = MAX_AMOUNT / n as i128;
            strictly_increasing_offsets(n as usize - 1, 1, MAX_DURATION).prop_flat_map(move |offsets| {
                positive_amounts(n as usize, cap).prop_map(move |amounts| {
                    let mut table = vec![(0u64, amounts[0])];
                    for (i, offset) in offsets.clone().into_iter().enumerate() {
                        table.push((offset, amounts[i + 1]));
                    }
                    table
                })
            })
        }),
        2 => (2u32..=MAX_TRANCHES).prop_flat_map(|n| {
            let cap = MAX_AMOUNT / n as i128;
            strictly_increasing_offsets(n as usize - 1, 0, MAX_DURATION).prop_flat_map(move |offsets| {
                positive_amounts(n as usize, cap).prop_map(move |amounts| {
                    let mut table: Vec<(u64, i128)> =
                        offsets.clone().into_iter().zip(amounts.iter().cloned()).collect();
                    table.push((u64::MAX, *amounts.last().unwrap()));
                    table
                })
            })
        }),
        1 => (3u32..=MAX_TRANCHES).prop_flat_map(|n| {
            let cap = MAX_AMOUNT / n as i128;
            strictly_increasing_offsets(n as usize - 2, 1, MAX_DURATION).prop_flat_map(move |offsets| {
                positive_amounts(n as usize, cap).prop_map(move |amounts| {
                    let mut table = vec![(0u64, amounts[0])];
                    for (i, offset) in offsets.clone().into_iter().enumerate() {
                        table.push((offset, amounts[i + 1]));
                    }
                    table.push((u64::MAX, amounts[amounts.len() - 1]));
                    table
                })
            })
        }),
        1 => (1u32..=MAX_TRANCHES).prop_flat_map(|n| {
            let cap = i128::MAX / n as i128;
            strictly_increasing_offsets(n as usize, 0, MAX_DURATION).prop_flat_map(move |offsets| {
                positive_amounts(n as usize, cap)
                    .prop_map(move |amounts| offsets.clone().into_iter().zip(amounts).collect())
            })
        }),
    ]
}

// -----------------------------------------------------------------------
// P1 — Total Conservation & Residue Safety
// -----------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn p1_total_conservation_and_residue_safety(
        (total_amount, cliff, duration) in arb_schedule_params(),
        mut time_offsets in prop::collection::vec(0u64..=(MAX_DURATION * 2), 1..=8),
    ) {
        time_offsets.sort_unstable();

        let w = setup_world();
        w.mint_to_contract(total_amount);
        let id = w.create(total_amount, cliff, duration);

        let initial_pool = w.pool_total();
        prop_assert_eq!(initial_pool, total_amount, "initial pool must equal total_amount");

        let mut cumulative_claimed = 0i128;

        for offset in time_offsets {
            let current_time = START.saturating_add(offset);
            w.env.ledger().set_timestamp(current_time);

            let claimable = w.vesting_client().claimable(&id);
            prop_assert!(claimable >= 0, "claimable amount must never be negative");
            prop_assert!(
                cumulative_claimed + claimable <= total_amount,
                "claimed ({}) + claimable ({}) must not exceed total_amount ({})",
                cumulative_claimed,
                claimable,
                total_amount
            );

            let claimed_now = w.vesting_client().claim(&id);
            prop_assert_eq!(claimed_now, claimable, "claim() must return the exact claimable amount");
            cumulative_claimed += claimed_now;

            let (beneficiary_balance, contract_balance) = w.pool();
            prop_assert_eq!(
                beneficiary_balance,
                cumulative_claimed,
                "beneficiary balance must match cumulative claimed exactly"
            );
            prop_assert_eq!(
                contract_balance,
                total_amount - cumulative_claimed,
                "contract balance must match remaining unvested/unclaimed tokens exactly"
            );
            prop_assert_eq!(
                w.pool_total(),
                total_amount,
                "pool total must be invariant across claims"
            );
        }

        // Advance to full duration and perform final claim to verify residue safety
        w.env.ledger().set_timestamp(START.saturating_add(duration).saturating_add(1));
        let final_claimable = w.vesting_client().claimable(&id);
        let final_claimed = w.vesting_client().claim(&id);
        prop_assert_eq!(final_claimed, final_claimable);
        cumulative_claimed += final_claimed;

        let (final_b, final_c) = w.pool();
        prop_assert_eq!(cumulative_claimed, total_amount, "all tokens must be claimable at completion");
        prop_assert_eq!(final_b, total_amount, "beneficiary must receive exact total_amount with 0 token loss");
        prop_assert_eq!(final_c, 0, "contract balance must be drained to exactly 0");
        prop_assert_eq!(
            w.vesting_client().get_status(&id),
            VestingStatus::Completed,
            "status must be Completed after full claim"
        );
    }
}

// -----------------------------------------------------------------------
// P7 — Revoke, Claim, and Reassignment Conservation
// -----------------------------------------------------------------------
//
// These scenarios intentionally assert value conservation, not vesting
// rounding behavior; floor-division residue remains covered by P1 and tests.rs.

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn p7_revoke_claim_and_reassignment_conservation(
        (total_amount, cliff, duration) in arb_schedule_params(),
        claim_sample in 0u64..=MAX_DURATION,
        revoke_sample in 0u64..=MAX_DURATION,
        reassign_sample in 0u64..=MAX_DURATION,
    ) {
        let revoke_elapsed = revoke_sample.min(duration.saturating_sub(1));
        let claim_elapsed = claim_sample.min(revoke_elapsed);
        let reassign_elapsed = reassign_sample.min(duration.saturating_sub(1));

        // Claim before revocation, then claim the frozen remainder later.
        let revoked_after_claim = setup_world();
        revoked_after_claim.mint_to_contract(total_amount);
        let revoked_after_claim_id =
            revoked_after_claim.create(total_amount, cliff, duration);

        revoked_after_claim.env.ledger().set_timestamp(START + claim_elapsed);
        let claimed_before_revoke = revoked_after_claim
            .vesting_client()
            .claim(&revoked_after_claim_id);
        prop_assert!(
            claimed_before_revoke <= total_amount,
            "a pre-revocation claim must not exceed the allocation"
        );

        revoked_after_claim.env.ledger().set_timestamp(START + revoke_elapsed);
        revoked_after_claim
            .vesting_client()
            .revoke(&revoked_after_claim_id);
        let frozen_vested = revoked_after_claim
            .vesting_client()
            .get_schedule(&revoked_after_claim_id)
            .revoked_vested
            .expect("successful revocation must freeze a vested amount");
        prop_assert!(
            frozen_vested >= claimed_before_revoke && frozen_vested <= total_amount,
            "frozen vested value must include prior claims and stay within the allocation"
        );

        revoked_after_claim
            .env
            .ledger()
            .set_timestamp(START + duration + 1);
        let claim_after_revoke = revoked_after_claim
            .vesting_client()
            .claim(&revoked_after_claim_id);
        prop_assert_eq!(
            claim_after_revoke,
            frozen_vested - claimed_before_revoke,
            "a later claim may release only the vested amount frozen at revocation"
        );
        let paid_after_revoke = claimed_before_revoke + claim_after_revoke;
        prop_assert!(
            paid_after_revoke <= total_amount,
            "total released value must never exceed the original allocation"
        );
        prop_assert_eq!(
            revoked_after_claim.balance(&revoked_after_claim.beneficiary)
                + revoked_after_claim.balance(&revoked_after_claim.vesting),
            total_amount,
            "beneficiary and contract balances must conserve the allocation"
        );

        let revoked_schedule = revoked_after_claim
            .vesting_client()
            .get_schedule(&revoked_after_claim_id);
        prop_assert_eq!(
            revoked_after_claim
                .vesting_client()
                .claim(&revoked_after_claim_id),
            0,
            "a repeated post-revocation claim must release nothing"
        );
        prop_assert_eq!(
            revoked_after_claim
                .vesting_client()
                .try_revoke(&revoked_after_claim_id)
                .unwrap_err()
                .unwrap(),
            ForgeError::InvalidInput,
            "a repeated revoke must be rejected"
        );
        let revoked_schedule_after_repeat = revoked_after_claim
            .vesting_client()
            .get_schedule(&revoked_after_claim_id);
        prop_assert_eq!(
            revoked_schedule_after_repeat.revoked_vested,
            revoked_schedule.revoked_vested,
            "a repeated revoke must not change the frozen amount"
        );
        prop_assert_eq!(revoked_schedule_after_repeat.claimed, paid_after_revoke);
        prop_assert_eq!(
            revoked_after_claim.balance(&revoked_after_claim.beneficiary)
                + revoked_after_claim.balance(&revoked_after_claim.vesting),
            total_amount,
            "repeated terminal operations must not change the conserved pool"
        );

        // Revoke first, then advance beyond the original schedule: only the
        // frozen value can ever be claimed.
        let revoked_before_claim = setup_world();
        revoked_before_claim.mint_to_contract(total_amount);
        let revoked_before_claim_id =
            revoked_before_claim.create(total_amount, cliff, duration);
        revoked_before_claim
            .env
            .ledger()
            .set_timestamp(START + revoke_elapsed);
        revoked_before_claim
            .vesting_client()
            .revoke(&revoked_before_claim_id);
        let frozen_without_prior_claims = revoked_before_claim
            .vesting_client()
            .get_schedule(&revoked_before_claim_id)
            .revoked_vested
            .expect("successful revocation must freeze a vested amount");
        revoked_before_claim
            .env
            .ledger()
            .set_timestamp(START + duration + 1);
        let first_post_revoke_claim = revoked_before_claim
            .vesting_client()
            .claim(&revoked_before_claim_id);
        prop_assert_eq!(first_post_revoke_claim, frozen_without_prior_claims);
        prop_assert_eq!(
            revoked_before_claim
                .vesting_client()
                .claim(&revoked_before_claim_id),
            0,
            "a repeated claim after draining revoked value must be a no-op"
        );
        prop_assert!(
            revoked_before_claim.balance(&revoked_before_claim.beneficiary)
                <= total_amount,
            "a post-revocation claim must not release revoked value"
        );
        prop_assert_eq!(
            revoked_before_claim.balance(&revoked_before_claim.beneficiary)
                + revoked_before_claim.balance(&revoked_before_claim.vesting),
            total_amount,
            "revoke-first flow must conserve the allocation"
        );

        // Reassignment preserves the old beneficiary's claimed and vested
        // amounts while leaving only the future remainder to the new one.
        let reassigned = setup_world();
        reassigned.mint_to_contract(total_amount);
        let reassigned_id = reassigned.create(total_amount, cliff, duration);
        let new_beneficiary = Address::generate(&reassigned.env);

        let prior_claim_elapsed = claim_elapsed.min(reassign_elapsed);
        reassigned
            .env
            .ledger()
            .set_timestamp(START + prior_claim_elapsed);
        let claimed_before_reassignment =
            reassigned.vesting_client().claim(&reassigned_id);

        reassigned
            .env
            .ledger()
            .set_timestamp(START + reassign_elapsed);
        let remaining_for_old_beneficiary =
            reassigned.vesting_client().claimable(&reassigned_id);
        let old_beneficiary_vested =
            claimed_before_reassignment + remaining_for_old_beneficiary;
        reassigned
            .vesting_client()
            .reassign_beneficiary(&reassigned_id, &new_beneficiary);
        prop_assert_eq!(
            reassigned.vesting_client().claimable(&reassigned_id),
            0,
            "reassignment must not give the new beneficiary the old beneficiary's accrued value"
        );

        let old_beneficiary_claim = reassigned
            .vesting_client()
            .claim_for(&reassigned_id, &reassigned.beneficiary);
        prop_assert_eq!(old_beneficiary_claim, remaining_for_old_beneficiary);
        prop_assert_eq!(
            reassigned.balance(&reassigned.beneficiary),
            old_beneficiary_vested,
            "the former beneficiary must retain all value vested before reassignment"
        );

        reassigned
            .env
            .ledger()
            .set_timestamp(START + duration + 1);
        let new_beneficiary_claim = reassigned
            .vesting_client()
            .claim(&reassigned_id);
        prop_assert_eq!(
            new_beneficiary_claim,
            total_amount - old_beneficiary_vested,
            "the new beneficiary receives only the allocation remaining after reassignment"
        );
        prop_assert_eq!(
            reassigned
                .vesting_client()
                .claim(&reassigned_id),
            0,
            "a repeated claim after reassignment must be a no-op"
        );
        prop_assert_eq!(
            reassigned.balance(&reassigned.beneficiary)
                + reassigned.balance(&new_beneficiary)
                + reassigned.balance(&reassigned.vesting),
            total_amount,
            "old, new, and contract balances must conserve the original allocation"
        );
        prop_assert_eq!(
            reassigned.balance(&reassigned.beneficiary)
                + reassigned.balance(&new_beneficiary),
            total_amount,
            "total released value across both beneficiaries must equal the allocation"
        );
    }
}

// -----------------------------------------------------------------------
// P2 — Monotonicity Invariants
// -----------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn p2_monotonicity_over_time(
        (total_amount, cliff, duration) in arb_schedule_params(),
        mut offsets in prop::collection::vec(0u64..=(MAX_DURATION * 2), 2..=10),
    ) {
        offsets.sort_unstable();

        let w = setup_world();
        w.mint_to_contract(total_amount);
        let id = w.create(total_amount, cliff, duration);

        let mut prev_claimable = 0i128;
        let mut prev_time = 0u64;

        for offset in offsets {
            let current_time = START.saturating_add(offset);
            w.env.ledger().set_timestamp(current_time);

            let claimable = w.vesting_client().claimable(&id);

            if offset < cliff {
                prop_assert_eq!(claimable, 0, "before cliff, claimable must be 0");
                prop_assert_eq!(
                    w.vesting_client().get_status(&id),
                    VestingStatus::Locked,
                    "before cliff, status must be Locked"
                );
            } else {
                prop_assert!(
                    claimable >= prev_claimable,
                    "claimable ({}) at t={} must be >= previous claimable ({}) at t={}",
                    claimable,
                    current_time,
                    prev_claimable,
                    prev_time
                );
                if offset >= duration {
                    prop_assert_eq!(
                        claimable,
                        total_amount,
                        "at or after duration, claimable must equal total_amount"
                    );
                }
            }

            prev_claimable = claimable;
            prev_time = current_time;
        }
    }
}

// -----------------------------------------------------------------------
// P3 — Arbitrary Interleaved Action Sequences
// -----------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn p3_interleaved_action_sequences(
        (total_amount, cliff, duration) in arb_schedule_params(),
        mut actions in prop::collection::vec((0u64..=(MAX_DURATION * 2), prop::bool::ANY), 1..=12),
    ) {
        actions.sort_by_key(|(offset, _)| *offset);

        let w = setup_world();
        w.mint_to_contract(total_amount);
        let id = w.create(total_amount, cliff, duration);

        let mut cumulative_claimed = 0i128;

        for (offset, do_claim) in actions {
            let current_time = START.saturating_add(offset);
            w.env.ledger().set_timestamp(current_time);

            let pool_before = w.pool_total();
            let claimable = w.vesting_client().claimable(&id);

            if do_claim {
                let claimed = w.vesting_client().claim(&id);
                prop_assert_eq!(claimed, claimable, "claim() must pay exact claimable amount");
                cumulative_claimed += claimed;
            }

            let pool_after = w.pool_total();
            prop_assert_eq!(pool_before, pool_after, "pool total must be conserved across action");
            prop_assert_eq!(pool_after, total_amount, "pool total must equal total_amount");

            let (b_bal, c_bal) = w.pool();
            prop_assert_eq!(b_bal, cumulative_claimed);
            prop_assert_eq!(c_bal, total_amount - cumulative_claimed);
        }
    }
}

// -----------------------------------------------------------------------
// P4 — Tamper-Resilient Conservation
// -----------------------------------------------------------------------

fn mutate_vesting_record(w: &World, id: u64, kind: u8, delta: i128) {
    let key = crate::DataKey::Schedule(id);
    w.env.as_contract(&w.vesting, || {
        if let Some(mut rec) = w.env.storage().instance().get::<_, VestingSchedule>(&key) {
            match kind {
                // Fuzz total_amount
                0 => {
                    rec.total_amount = rec.total_amount.wrapping_add(delta).max(1);
                }
                // Fuzz claimed
                1 => {
                    rec.claimed = rec.claimed.wrapping_add(delta).max(0);
                }
                // Fuzz cliff
                2 => {
                    rec.cliff = (rec.cliff as i128).wrapping_add(delta).max(0) as u64;
                }
                // Fuzz duration
                3 => {
                    rec.duration = (rec.duration as i128).wrapping_add(delta).max(1) as u64;
                }
                _ => {}
            }
            w.env.storage().instance().set(&key, &rec);
        }
    });
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn p4_tampered_storage_never_violates_pool_conservation(
        (total_amount, cliff, duration) in arb_schedule_params(),
        mutations in prop::collection::vec((0u8..=3, -10_000i128..=10_000i128), 0..=4),
    ) {
        let w = setup_world();
        w.mint_to_contract(total_amount);
        let id = w.create(total_amount, cliff, duration);

        for (kind, delta) in mutations {
            mutate_vesting_record(&w, id, kind, delta);
        }

        w.env.ledger().set_timestamp(START.saturating_add(duration).saturating_add(10));

        let client = w.vesting_client();
        let pool_before = w.pool_total();

        match client.try_claim(&id) {
            Ok(Ok(_paid)) => {
                let (b, c) = w.pool();
                prop_assert!(c >= 0, "contract balance must not be negative");
                prop_assert!(b >= 0, "beneficiary balance must not be negative");
                prop_assert_eq!(b + c, pool_before, "pool total must be conserved on successful claim");
            }
            Err(Ok(ForgeError::TokenTransferFailed)) => {
                // Over-payment prevented by token balance check; pool unchanged
                prop_assert_eq!(w.pool_total(), pool_before);
            }
            Err(Ok(ForgeError::ArithmeticOverflow)) => {
                // Arithmetic overflow prevented; pool unchanged
                prop_assert_eq!(w.pool_total(), pool_before);
            }
            Err(Ok(ForgeError::NotFound)) => {}
            other => panic!("unexpected outcome under tampering: {:?}", other),
        }
    }
}

// -----------------------------------------------------------------------
// P5 — Tranche Conservation
// -----------------------------------------------------------------------
//
// Over random tranche tables and random claim sequences:
// - claimed never exceeds total_amount
// - contract custody balance decreases by exactly what the beneficiary gains
// - repeated claims never overpay
// - final claim transitions the schedule to Completed (when no u64::MAX
//   tranche withholds part of the total)
// - failed transfers leave state untouched

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn p5_tranche_conservation(
        table in arb_tranche_table(),
        // Claim offsets share the unlock offsets' range so claims land
        // between unlocks (partial claims) instead of clumping after the
        // full unlock; the final drain below covers the everything-at-once
        // pattern.
        mut claim_offsets in prop::collection::vec(0u64..=MAX_DURATION, 1..=8),
    ) {
        claim_offsets.sort_unstable();

        let w = setup_world();
        let total_amount = table_total(&table);
        let id = w.create_tranche(&to_tranche_vec(&w.env, &table), total_amount);

        let initial_pool = w.pool_total();
        prop_assert_eq!(initial_pool, total_amount, "initial pool must equal total_amount");

        let mut cumulative_claimed = 0i128;

        for offset in claim_offsets {
            let current_time = START.saturating_add(offset);
            w.env.ledger().set_timestamp(current_time);

            let claimable = w.vesting_client().claimable(&id);
            prop_assert!(claimable >= 0, "claimable amount must never be negative");

            // Exact partial sum: unlocked-but-unclaimed at this instant.
            let elapsed = current_time - START;
            prop_assert_eq!(
                claimable,
                expected_claimable_at(&table, elapsed, cumulative_claimed),
                "claimable must equal the exact unlocked-minus-claimed sum at elapsed={}",
                elapsed
            );

            prop_assert!(
                cumulative_claimed + claimable <= total_amount,
                "claimed ({}) + claimable ({}) must not exceed total_amount ({})",
                cumulative_claimed,
                claimable,
                total_amount
            );

            let claimed_now = w.vesting_client().claim(&id);
            prop_assert_eq!(claimed_now, claimable, "claim() must return the exact claimable amount");
            cumulative_claimed += claimed_now;

            let (beneficiary_balance, contract_balance) = w.pool();
            prop_assert_eq!(
                beneficiary_balance,
                cumulative_claimed,
                "beneficiary balance must match cumulative claimed exactly"
            );
            prop_assert_eq!(
                contract_balance,
                total_amount - cumulative_claimed,
                "contract custody must have decreased by exactly what the beneficiary gained"
            );
            prop_assert_eq!(
                w.pool_total(),
                total_amount,
                "pool total must be invariant across claims"
            );

            // A repeated claim at the same instant must never overpay.
            let repeat = w.vesting_client().claim(&id);
            prop_assert_eq!(repeat, 0, "repeated claim at the same instant must pay 0");
            let (b_repeat, c_repeat) = w.pool();
            prop_assert_eq!(b_repeat, cumulative_claimed, "repeat claim must not move beneficiary balance");
            prop_assert_eq!(c_repeat, total_amount - cumulative_claimed, "repeat claim must not move contract balance");
        }

        // At the last reachable offset (inclusive boundary), everything
        // unlockable is claimable.
        let last_reachable = table
            .iter()
            .rev()
            .find(|(unlock_at, _)| *unlock_at != u64::MAX)
            .map(|(unlock_at, _)| *unlock_at)
            .expect("strategy never produces an all-u64::MAX table");
        w.env
            .ledger()
            .set_timestamp(START.saturating_add(last_reachable));
        prop_assert_eq!(
            w.vesting_client().claimable(&id),
            unlockable_total(&table) - cumulative_claimed,
            "at the last reachable offset, the full unlockable remainder must be claimable"
        );

        // One final claim of everything at a far-future instant: reachable
        // tranches drain; a u64::MAX tranche never unlocks and stays in
        // custody.
        w.env.ledger().set_timestamp(FAR_FUTURE);
        let final_claimable = w.vesting_client().claimable(&id);
        prop_assert_eq!(
            final_claimable,
            unlockable_total(&table) - cumulative_claimed,
            "far-future claimable must be exactly the unlockable remainder"
        );
        let final_claimed = w.vesting_client().claim(&id);
        prop_assert_eq!(final_claimed, final_claimable);
        cumulative_claimed += final_claimed;

        let (final_b, final_c) = w.pool();
        prop_assert_eq!(
            final_b,
            unlockable_total(&table),
            "beneficiary must hold exactly the unlockable total"
        );
        prop_assert_eq!(
            final_c,
            total_amount - unlockable_total(&table),
            "contract must retain exactly the never-unlocking tranche amounts"
        );
        prop_assert_eq!(w.pool_total(), total_amount, "pool total must be conserved at the end");

        // Lifecycle: Completed only when claimed == total_amount; a schedule
        // with an unclaimed u64::MAX tranche stays Vesting forever.
        if unlockable_total(&table) == total_amount {
            prop_assert_eq!(
                w.vesting_client().get_status(&id),
                VestingStatus::Completed,
                "fully-claimed schedule must transition to Completed on the final claim"
            );
        } else {
            prop_assert_eq!(
                w.vesting_client().get_status(&id),
                VestingStatus::Vesting,
                "a schedule with an unclaimed u64::MAX tranche must remain Vesting"
            );
        }

        // The stored record agrees with the tokens that actually moved.
        let stored = w.vesting_client().get_tranche_schedule(&id);
        prop_assert_eq!(stored.claimed, cumulative_claimed, "stored claimed must equal cumulative paid");
        prop_assert_eq!(stored.total_amount, total_amount, "stored total must equal the table sum");

        // Failed transfers leave state untouched: custody now holds exactly
        // the never-unlocking remainder, so a TGE schedule asking for one
        // token more than the contract owns must fail the SEP-41 transfer
        // (same bucketing as P4) without touching either record.
        let pool_before_probe = w.pool_total();
        let probe_amount = total_amount - unlockable_total(&table) + 1;
        let probe_table = [(0u64, probe_amount)];
        let probe_id = w.vesting_client().create_tranche_schedule(
            &w.funder,
            &w.beneficiary,
            &w.token,
            &to_tranche_vec(&w.env, &probe_table),
        );
        let probe_before = w.vesting_client().get_tranche_schedule(&probe_id);

        let probe_result = w.vesting_client().try_claim(&probe_id);
        prop_assert!(
            matches!(probe_result, Err(Ok(ForgeError::TokenTransferFailed))),
            "over-custody claim must fail with TokenTransferFailed"
        );
        prop_assert_eq!(w.pool_total(), pool_before_probe, "failed transfer must not move the pool");
        let probe_after = w.vesting_client().get_tranche_schedule(&probe_id);
        prop_assert_eq!(probe_after.claimed, probe_before.claimed, "failed transfer must not touch claimed");
        prop_assert_eq!(probe_after.status, probe_before.status, "failed transfer must not touch status");
    }
}

// -----------------------------------------------------------------------
// P6 — Tranche Claimable Monotonic With Exact Sums
// -----------------------------------------------------------------------
//
// For random tranche tables:
// - claimable is non-decreasing over randomly increasing timestamps
// - claimable equals the exact partial sum of unlocked-but-unclaimed tranches
//   at sampled points:
//   * 0 before the first offset
//   * exact step sums between offsets
//   * the full unlockable total at/after the last reachable offset

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn p6_tranche_claimable_monotonic_with_exact_sums(
        table in arb_tranche_table(),
        // Sample timestamps share the unlock offsets' range so samples hit
        // the step function's plateaus and edges, not just its final state.
        mut sample_offsets in prop::collection::vec(0u64..=MAX_DURATION, 3..=10),
    ) {
        sample_offsets.sort_unstable();

        let first_reachable = table
            .first()
            .filter(|(unlock_at, _)| *unlock_at != u64::MAX)
            .map(|(unlock_at, _)| *unlock_at)
            .expect("strategy never produces an all-u64::MAX table");

        let w = setup_world();
        let total_amount = table_total(&table);
        let id = w.create_tranche(&to_tranche_vec(&w.env, &table), total_amount);

        let mut baseline = 0i128;
        let mut cumulative_claimed = 0i128;
        let mut prev_elapsed = 0u64;

        for (index, offset) in sample_offsets.iter().enumerate() {
            let current_time = START.saturating_add(*offset);
            w.env.ledger().set_timestamp(current_time);
            let elapsed = current_time - START;

            let claimable = w.vesting_client().claimable(&id);

            // Exact partial sum of unlocked-but-unclaimed tranches.
            prop_assert_eq!(
                claimable,
                expected_claimable_at(&table, elapsed, cumulative_claimed),
                "claimable must equal the exact partial sum at elapsed={}",
                elapsed
            );

            // Monotonicity holds between samples (no claims in between);
            // after a claim the baseline resets to the post-claim value.
            prop_assert!(
                claimable >= baseline,
                "claimable ({}) at elapsed={} must be >= previous baseline ({}) at elapsed={}",
                claimable,
                elapsed,
                baseline,
                prev_elapsed
            );

            // Strictly before the first offset nothing is claimable.
            if elapsed < first_reachable {
                prop_assert_eq!(claimable, 0, "claimable must be 0 before the first offset");
            }

            baseline = claimable;

            // Interleave one claim every other sample so the "minus claimed"
            // term is exercised; claim() must pay the exact view.
            if index % 2 == 1 {
                let claimed_now = w.vesting_client().claim(&id);
                prop_assert_eq!(claimed_now, claimable, "claim() must pay the exact claimable view");
                cumulative_claimed += claimed_now;
                prop_assert_eq!(
                    w.vesting_client().claimable(&id),
                    0,
                    "after claiming everything unlocked so far, claimable must be 0"
                );
                baseline = 0;
            }

            prev_elapsed = elapsed;
        }

        // The three canonical sample points, observed on a fresh schedule so
        // `claimed == 0` and the sums are the raw step function.
        let w2 = setup_world();
        let id2 = w2.create_tranche(&to_tranche_vec(&w2.env, &table), total_amount);

        // Point 1: before the first offset (or at creation time for a TGE
        // head, where elapsed 0 IS the unlock instant).
        if first_reachable == 0 {
            prop_assert_eq!(
                w2.vesting_client().claimable(&id2),
                table[0].1,
                "at a TGE first offset, claimable at the creation instant must equal the TGE amount"
            );
        } else {
            w2.env.ledger().set_timestamp(START);
            prop_assert_eq!(
                w2.vesting_client().claimable(&id2),
                0,
                "claimable must be 0 strictly before the first offset"
            );
        }

        // Point 2: strictly between two offsets — the exact step sum of every
        // tranche unlocked so far (only meaningful when a gap exists).
        let reachable: Vec<u64> = table
            .iter()
            .filter(|(unlock_at, _)| *unlock_at != u64::MAX)
            .map(|(unlock_at, _)| *unlock_at)
            .collect();
        if let Some(pair) = reachable.windows(2).find(|pair| pair[1] - pair[0] >= 2) {
            let mid = pair[0] + (pair[1] - pair[0]) / 2;
            w2.env.ledger().set_timestamp(START.saturating_add(mid));
            prop_assert_eq!(
                w2.vesting_client().claimable(&id2),
                expected_unlocked_at(&table, mid),
                "claimable strictly between offsets must equal the exact step sum at elapsed={}",
                mid
            );
            assert!(pair[0] < mid && mid < pair[1], "sample point must lie strictly between offsets");
        }

        // Point 3: at the last reachable offset (inclusive boundary) — the
        // full unlockable total, u64::MAX tail excluded.
        let last_reachable = reachable[reachable.len() - 1];
        w2.env
            .ledger()
            .set_timestamp(START.saturating_add(last_reachable));
        prop_assert_eq!(
            w2.vesting_client().claimable(&id2),
            unlockable_total(&table),
            "claimable at the last reachable offset must be the full unlockable total"
        );
    }
}

// -----------------------------------------------------------------------
// P8 — Beneficiary Reassignment Chains
// -----------------------------------------------------------------------

/// Most reassignments a generated chain performs.
const MAX_REASSIGNMENTS: usize = 10;

/// One link of a generated reassignment chain.
#[derive(Clone, Debug)]
struct ChainStep {
    /// Elapsed seconds since `START` at which this link runs.
    elapsed: u64,
    /// `Some(k)` reassigns back to former beneficiary `k % formers`; `None`
    /// (or no eligible former) reassigns to a fresh address.
    revisit: Option<u8>,
    /// Bit `i` set: address `i` calls `claim_for` just before this link.
    claim_mask: u16,
}

/// A linear schedule plus a reassignment chain over its lifetime.
#[derive(Clone, Debug)]
struct ChainCase {
    total_amount: i128,
    cliff: u64,
    duration: u64,
    steps: Vec<ChainStep>,
    /// Sort keys giving the order in which every address claims at the end.
    final_claim_keys: Vec<u16>,
}

/// Independent model of who owns which vested value.
///
/// Each address is entitled to whatever vested while it was the current
/// beneficiary (its tenures), minus what it has already been paid. This is
/// computed from the original curve only, not from the contract's
/// `reassignment_vested`/`beneficiary_claimed` bookkeeping.
struct ChainModel {
    total_amount: i128,
    cliff: u64,
    duration: u64,
    addresses: Vec<Address>,
    current: usize,
    tenure_start_vested: i128,
    accrued: Vec<i128>,
    paid: Vec<i128>,
    successful_reassignments: u32,
}

impl ChainModel {
    /// Vested amount on the original curve at `elapsed` seconds after start.
    fn vested(&self, elapsed: u64) -> i128 {
        if elapsed < self.cliff {
            0
        } else if elapsed >= self.duration {
            self.total_amount
        } else {
            self.total_amount * i128::from(elapsed - self.cliff)
                / i128::from(self.duration - self.cliff)
        }
    }

    fn entitled(&self, index: usize, elapsed: u64) -> i128 {
        let tenure = if index == self.current {
            self.vested(elapsed) - self.tenure_start_vested
        } else {
            0
        };
        self.accrued[index] + tenure - self.paid[index]
    }

    fn total_paid(&self) -> i128 {
        self.paid.iter().sum()
    }

    /// Everything vested during `index`'s tenures up to `elapsed`.
    fn earned(&self, index: usize, elapsed: u64) -> i128 {
        self.entitled(index, elapsed) + self.paid[index]
    }
}

fn arb_chain_case() -> impl Strategy<Value = ChainCase> {
    // `cliff >= 1` keeps a pre-cliff window and `cliff < duration` keeps a
    // mid-vesting window, so both phases are always reachable.
    (1i128..=MAX_AMOUNT, 2u64..=MAX_DURATION)
        .prop_flat_map(|(amount, duration)| (Just(amount), 1u64..duration, Just(duration)))
        .prop_flat_map(|(total_amount, cliff, duration)| {
            let raw_step = (
                any::<u64>(),
                proptest::option::of(any::<u8>()),
                any::<u16>(),
            );
            (
                Just(total_amount),
                Just(cliff),
                Just(duration),
                proptest::collection::vec(raw_step, 1..=MAX_REASSIGNMENTS),
                0usize..=MAX_REASSIGNMENTS,
                proptest::collection::vec(any::<u16>(), MAX_REASSIGNMENTS + 1),
            )
        })
        .prop_map(
            |(total_amount, cliff, duration, raw_steps, pre_cliff, final_claim_keys)| {
                let pre_cliff = pre_cliff.min(raw_steps.len());
                // The first `pre_cliff` links fall before the cliff; the rest
                // fall between the cliff and slightly past `duration`.
                let tail = duration - cliff + duration / 8 + 1;
                let mut pre: Vec<u64> = raw_steps[..pre_cliff]
                    .iter()
                    .map(|(sample, _, _)| sample % cliff)
                    .collect();
                let mut post: Vec<u64> = raw_steps[pre_cliff..]
                    .iter()
                    .map(|(sample, _, _)| cliff + sample % tail)
                    .collect();
                pre.sort_unstable();
                post.sort_unstable();
                let steps = pre
                    .into_iter()
                    .chain(post)
                    .zip(raw_steps)
                    .map(|(elapsed, (_, revisit, claim_mask))| ChainStep {
                        elapsed,
                        revisit,
                        claim_mask,
                    })
                    .collect();
                ChainCase {
                    total_amount,
                    cliff,
                    duration,
                    steps,
                    final_claim_keys,
                }
            },
        )
}

/// Check every chain invariant against the contract at `elapsed`.
fn check_chain_invariants(
    w: &World,
    id: u64,
    model: &ChainModel,
    elapsed: u64,
) -> Result<(), TestCaseError> {
    let client = w.vesting_client();
    let schedule = client.get_schedule(&id);

    // Duration monotonicity: the curve's boundaries are never touched.
    prop_assert_eq!(schedule.start, START);
    prop_assert_eq!(schedule.cliff, model.cliff, "cliff must never move");
    prop_assert_eq!(
        schedule.duration,
        model.duration,
        "duration must never move"
    );
    prop_assert_eq!(schedule.total_amount, model.total_amount);
    prop_assert_eq!(
        schedule.reassignment_count,
        model.successful_reassignments,
        "reassignment_count must equal the successful reassignments"
    );
    prop_assert_eq!(&schedule.beneficiary, &model.addresses[model.current]);
    if elapsed < model.cliff {
        prop_assert_eq!(
            client.get_status(&id),
            VestingStatus::Locked,
            "a reassignment must not move the schedule past its original cliff"
        );
    }

    // Per-address entitlement matches the tenure model exactly.
    let mut former_claims = 0i128;
    for (index, address) in model.addresses.iter().enumerate() {
        let claimable = client.claimable_for(&id, address);
        prop_assert_eq!(
            claimable,
            model.entitled(index, elapsed),
            "address {} claimable must equal what vested during its tenures minus payouts",
            index
        );
        if index != model.current {
            former_claims += claimable;
        }
    }
    let current_claimable = client.claimable(&id);
    prop_assert_eq!(
        current_claimable,
        client.claimable_for(&id, &model.addresses[model.current])
    );

    // Strict conservation.
    let remaining_unvested = model.total_amount - model.vested(elapsed);
    prop_assert_eq!(schedule.claimed, model.total_paid());
    prop_assert_eq!(
        former_claims + current_claimable + remaining_unvested + schedule.claimed,
        model.total_amount,
        "former claims + current claimable + unvested + paid must equal total_amount"
    );
    let paid_out: i128 = model.addresses.iter().map(|a| w.balance(a)).sum();
    prop_assert_eq!(paid_out, schedule.claimed);
    prop_assert_eq!(
        w.balance(&w.vesting),
        model.total_amount - schedule.claimed,
        "custody must hold exactly the unpaid allocation"
    );
    Ok(())
}

/// Claim for `index` at `elapsed`, then prove a repeat claim pays nothing.
fn claim_and_repeat(
    w: &World,
    id: u64,
    model: &mut ChainModel,
    index: usize,
    elapsed: u64,
) -> Result<(), TestCaseError> {
    let client = w.vesting_client();
    let address = model.addresses[index].clone();
    let expected = model.entitled(index, elapsed);
    let before = w.balance(&address);
    prop_assert_eq!(client.claim_for(&id, &address), expected);
    prop_assert_eq!(w.balance(&address), before + expected);
    model.paid[index] += expected;
    prop_assert_eq!(
        client.claim_for(&id, &address),
        0,
        "a repeated claim must never pay twice"
    );
    prop_assert!(
        w.balance(&address) <= model.earned(index, elapsed),
        "an address can never hold more than what vested during its tenures"
    );
    Ok(())
}

/// Drive one reassignment chain end to end against the model.
fn run_reassignment_chain(case: &ChainCase) -> Result<(), TestCaseError> {
    let w = setup_world();
    w.mint_to_contract(case.total_amount);
    let id = w.create(case.total_amount, case.cliff, case.duration);
    let client = w.vesting_client();

    let mut model = ChainModel {
        total_amount: case.total_amount,
        cliff: case.cliff,
        duration: case.duration,
        addresses: vec![w.beneficiary.clone()],
        current: 0,
        tenure_start_vested: 0,
        accrued: vec![0],
        paid: vec![0],
        successful_reassignments: 0,
    };
    check_chain_invariants(&w, id, &model, 0)?;

    for step in &case.steps {
        w.env.ledger().set_timestamp(START + step.elapsed);

        for index in 0..model.addresses.len() {
            if step.claim_mask & (1 << index) != 0 {
                claim_and_repeat(&w, id, &mut model, index, step.elapsed)?;
                check_chain_invariants(&w, id, &model, step.elapsed)?;
            }
        }

        let formers: Vec<usize> = (0..model.addresses.len())
            .filter(|&index| index != model.current)
            .collect();
        let next = match step.revisit {
            Some(k) if !formers.is_empty() => formers[usize::from(k) % formers.len()],
            _ => {
                model.addresses.push(Address::generate(&w.env));
                model.accrued.push(0);
                model.paid.push(0);
                model.addresses.len() - 1
            }
        };
        let next_address = model.addresses[next].clone();
        let result = client.try_reassign_beneficiary(&id, &next_address);

        if model.total_paid() >= model.total_amount {
            // Everything is paid out: the schedule is complete and further
            // reassignment is rejected without touching the count.
            prop_assert_eq!(result.unwrap_err().unwrap(), ForgeError::InvalidInput);
        } else {
            prop_assert!(result.is_ok(), "reassignment must succeed: {:?}", result);
            let vested_now = model.vested(step.elapsed);
            model.accrued[model.current] += vested_now - model.tenure_start_vested;
            model.tenure_start_vested = vested_now;
            model.current = next;
            model.successful_reassignments += 1;
            prop_assert_eq!(
                client.claimable(&id),
                model.entitled(next, step.elapsed),
                "a new beneficiary inherits none of the previous holder's accrual"
            );
        }
        check_chain_invariants(&w, id, &model, step.elapsed)?;
    }

    // Past the original duration everything has vested; every address
    // claims in a random order and receives exactly its tenure earnings.
    let last = case.steps.last().map_or(0, |step| step.elapsed);
    let final_elapsed = last.max(case.duration) + 1;
    w.env.ledger().set_timestamp(START + final_elapsed);
    check_chain_invariants(&w, id, &model, final_elapsed)?;

    let mut order: Vec<usize> = (0..model.addresses.len()).collect();
    order.sort_by_key(|&index| (case.final_claim_keys[index], index));
    for index in order {
        claim_and_repeat(&w, id, &mut model, index, final_elapsed)?;
        check_chain_invariants(&w, id, &model, final_elapsed)?;
    }

    for (index, address) in model.addresses.iter().enumerate() {
        prop_assert_eq!(
            w.balance(address),
            model.earned(index, final_elapsed),
            "address {} must end with exactly what vested during its tenures",
            index
        );
    }
    prop_assert_eq!(w.balance(&w.vesting), 0, "custody must be fully drained");
    prop_assert_eq!(model.total_paid(), case.total_amount);
    prop_assert_eq!(client.get_status(&id), VestingStatus::Completed);

    let count = client.get_schedule(&id).reassignment_count;
    let late = Address::generate(&w.env);
    prop_assert_eq!(
        client
            .try_reassign_beneficiary(&id, &late)
            .unwrap_err()
            .unwrap(),
        ForgeError::InvalidInput,
        "a completed schedule cannot be reassigned"
    );
    prop_assert_eq!(client.get_schedule(&id).reassignment_count, count);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        // Each case runs up to ten reassignments with full model checks after
        // every call, so fewer cases than P1–P7 keep the suite fast.
        cases: 64,
        // Fixed seed: the same chains run on every machine and in CI.
        rng_seed: RngSeed::Fixed(0x5052_4f50_5f50_3800),
        ..ProptestConfig::default()
    })]

    #[test]
    fn p8_reassignment_chain_conservation(case in arb_chain_case()) {
        run_reassignment_chain(&case)?;
    }
}

fn fixed_chain(cliff: u64, duration: u64, elapsed: &[u64], revisit: Option<u8>) -> ChainCase {
    ChainCase {
        total_amount: 1_000_003,
        cliff,
        duration,
        steps: elapsed
            .iter()
            .enumerate()
            .map(|(i, &elapsed)| ChainStep {
                elapsed,
                // Every other link goes back to a former beneficiary.
                revisit: if i % 2 == 1 { revisit } else { None },
                claim_mask: if i % 3 == 0 { u16::MAX } else { 0 },
            })
            .collect(),
        final_claim_keys: (0..=MAX_REASSIGNMENTS as u16).rev().collect(),
    }
}

#[test]
fn p8_ten_pre_cliff_reassignments_leave_the_whole_grant_to_the_last_beneficiary() {
    let case = fixed_chain(
        1_000,
        10_000,
        &[0, 1, 2, 50, 100, 400, 500, 700, 998, 999],
        None,
    );
    run_reassignment_chain(&case).unwrap();
}

#[test]
fn p8_ten_mid_vesting_reassignments_with_revisits_conserve_value() {
    let case = fixed_chain(
        1_000,
        10_007,
        &[
            1_000, 1_001, 2_333, 2_333, 4_999, 6_000, 7_777, 9_000, 10_006, 10_006,
        ],
        Some(0),
    );
    run_reassignment_chain(&case).unwrap();
}

#[test]
fn p8_mixed_chain_crossing_the_cliff_and_duration() {
    let case = fixed_chain(
        500,
        5_000,
        &[0, 499, 500, 1_250, 2_500, 4_999, 5_000, 6_000],
        Some(3),
    );
    run_reassignment_chain(&case).unwrap();
}
