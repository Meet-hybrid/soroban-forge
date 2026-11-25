//! Rolling withdrawal limit invariants: window pruning, per-token isolation,
//! exact boundary enforcement, and the execution-time re-check.
//!
//! Declared after `lib.rs`'s `#[macro_use]` `tests` module, so its
//! `custody!()` and `limited!()` harness macros are in scope here. Every invariant is
//! exercised with 1-hour, 1-day, and 7-day windows, and a model-based
//! property test interleaves submissions, executions, limit changes, and
//! timestamp jumps across two tokens.
//!
//! - **Window pruning.** An entry counts while `now - submitted_at < window`
//!   and leaves exactly at `submitted_at + window`. Every write stores only
//!   in-window entries, so the stored list never keeps expired history and
//!   its sum always equals the reported usage — no orphaned totals.
//! - **Independent asset isolation.** Consuming, expiring, or re-limiting
//!   token A never changes token B's limit, usage, window, or allowance.
//! - **Exact boundary enforcement.** A withdrawal equal to the remaining
//!   allowance is admitted; one stroop more is rejected, at submission and
//!   in `check_withdrawal`.
//! - **Execution timing.** Execution re-checks the limit in force, so a stale
//!   approval cannot violate a lowered limit, without counting a withdrawal
//!   that already occupies the window twice.

use super::*;
// Names the harness macros expand to.
use crate::tests::{owner_vec, DEPOSIT};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use soroban_forge_shared_utils::{
    jump_after, jump_before, jump_to, BalanceSnapshot, TEST_START_TIMESTAMP,
};
use soroban_forge_test_utils::TestAccounts;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use std::vec::Vec as StdVec;

const HOUR: u64 = 3_600;
const DAY: u64 = 86_400;
const WEEK: u64 = 7 * DAY;
const WINDOWS: [u64; 3] = [HOUR, DAY, WEEK];

/// Custody minted and deposited per token: large enough that funding never
/// interferes with the limit invariants unless a test wants it to.
const FUNDING: i128 = 1_000_000;

const T0: u64 = TEST_START_TIMESTAMP;

type Client<'a> = SorobanForgeMultiSigWalletClient<'a>;

/// Mint `FUNDING` of `token` to `user1` and deposit it into custody.
fn fund(env: &Env, client: &Client<'_>, accounts: &TestAccounts, token: &Address) {
    StellarAssetClient::new(env, token).mint(&accounts.user1, &FUNDING);
    client.deposit(token, &accounts.user1, &FUNDING);
}

/// Register a second SAC token and fund the wallet with it.
fn second_token(env: &Env, client: &Client<'_>, accounts: &TestAccounts) -> Address {
    let token = env
        .register_stellar_asset_contract_v2(Address::generate(env))
        .address();
    fund(env, client, accounts, &token);
    token
}

/// Submit a withdrawal and collect the threshold, without executing it.
fn approved(client: &Client<'_>, accounts: &TestAccounts, token: &Address, amount: i128) -> u64 {
    let tx = client.submit_withdrawal(&accounts.user1, token, &accounts.arbiter, &amount);
    client.confirm(&tx, &accounts.user2);
    client.confirm(&tx, &accounts.user3);
    tx
}

fn submit_err(
    client: &Client<'_>,
    accounts: &TestAccounts,
    token: &Address,
    amount: i128,
) -> ForgeError {
    client
        .try_submit_withdrawal(&accounts.user1, token, &accounts.arbiter, &amount)
        .unwrap_err()
        .unwrap()
}

/// The raw stored window list for `token`, bypassing the read-only views.
fn stored_entries(env: &Env, wallet: &Address, token: &Address) -> StdVec<WindowEntry> {
    env.as_contract(wallet, || {
        env.storage()
            .persistent()
            .get::<_, soroban_sdk::Vec<WindowEntry>>(&DataKey::WindowUsage(token.clone()))
            .map(|entries| entries.iter().collect())
            .unwrap_or_default()
    })
}

fn stored_total(env: &Env, wallet: &Address, token: &Address) -> i128 {
    stored_entries(env, wallet, token)
        .iter()
        .map(|e| e.amount)
        .sum()
}

// -----------------------------------------------------------------------
// Invariant 1 — window pruning
// -----------------------------------------------------------------------

#[test]
fn window_pruning_drops_entries_exactly_at_the_horizon() {
    for window in WINDOWS {
        let (env, client, accounts, token, _token_client) = custody!();
        fund(&env, &client, &accounts, &token);
        jump_to(&env, T0);
        limited!(client, accounts, token, 1_000_i128, window);

        for (offset, amount) in [(0, 100_i128), (10, 200), (20, 300)] {
            jump_to(&env, T0 + offset);
            approved(&client, &accounts, &token, amount);
        }

        // One second before the first entry's horizon everything counts.
        jump_before(&env, T0 + window, 1);
        assert_eq!(client.get_window_usage(&token), 600, "window {window}");
        assert_eq!(
            client.get_withdrawal_window(&token),
            WindowState {
                total: 600,
                window_start: Some(T0),
                reset_at: Some(T0 + window),
            }
        );

        // Exactly at the horizon the first entry leaves the window. Views
        // report it without writing; the next write prunes storage.
        jump_to(&env, T0 + window);
        assert_eq!(client.get_window_usage(&token), 500);
        assert_eq!(stored_entries(&env, &client.address, &token).len(), 3);
        approved(&client, &accounts, &token, 50);
        let stored = stored_entries(&env, &client.address, &token);
        assert_eq!(
            stored.iter().map(|e| e.submitted_at).collect::<StdVec<_>>(),
            std::vec![T0 + 10, T0 + 20, T0 + window],
            "the expired entry is pruned on write, the rest are kept"
        );
        assert_eq!(stored_total(&env, &client.address, &token), 550);
        assert_eq!(client.get_window_usage(&token), 550);
        assert_eq!(
            client.get_withdrawal_window(&token),
            WindowState {
                total: 550,
                window_start: Some(T0 + 10),
                reset_at: Some(T0 + 10 + window),
            }
        );

        // Far past every horizon the window is empty, and the next write
        // leaves exactly one entry: no stale history and no orphaned total.
        jump_to(&env, T0 + 5 * window);
        assert_eq!(client.get_window_usage(&token), 0);
        assert_eq!(
            client.get_withdrawal_window(&token),
            WindowState {
                total: 0,
                window_start: None,
                reset_at: None,
            }
        );
        approved(&client, &accounts, &token, 1_000);
        let stored = stored_entries(&env, &client.address, &token);
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].submitted_at, T0 + 5 * window);
        assert_eq!(client.get_window_usage(&token), 1_000);
        assert_eq!(stored_total(&env, &client.address, &token), 1_000);
    }
}

#[test]
fn executing_withdrawals_never_adds_usage_or_stale_history() {
    for window in WINDOWS {
        let (env, client, accounts, token, _token_client) = custody!();
        fund(&env, &client, &accounts, &token);
        jump_to(&env, T0);
        limited!(client, accounts, token, 900_i128, window);

        let txs: StdVec<u64> = (0..3)
            .map(|_| approved(&client, &accounts, &token, 300))
            .collect();
        let before = stored_entries(&env, &client.address, &token);
        for tx in &txs {
            client.execute(tx);
        }
        // Counted withdrawals settle without being recorded a second time.
        assert_eq!(stored_entries(&env, &client.address, &token), before);
        assert_eq!(client.get_window_usage(&token), 900);

        jump_to(&env, T0 + window);
        assert_eq!(client.get_window_usage(&token), 0);
        approved(&client, &accounts, &token, 900);
        assert_eq!(stored_entries(&env, &client.address, &token).len(), 1);
    }
}

// -----------------------------------------------------------------------
// Invariant 2 — independent asset isolation
// -----------------------------------------------------------------------

#[test]
fn consuming_one_tokens_limit_never_touches_another_token() {
    for (window_a, window_b) in [(HOUR, WEEK), (DAY, HOUR), (WEEK, DAY)] {
        let (env, client, accounts, token_a, _token_client) = custody!();
        fund(&env, &client, &accounts, &token_a);
        let token_b = second_token(&env, &client, &accounts);
        jump_to(&env, T0);
        limited!(client, accounts, token_a, 1_000_i128, window_a);
        limited!(client, accounts, token_b, 500_i128, window_b);
        let b_limit = client.get_withdrawal_limit(&token_b);

        // Interleave: A to its cap, B untouched, then B to its cap.
        let a1 = approved(&client, &accounts, &token_a, 600);
        assert_eq!(client.get_window_usage(&token_b), 0);
        let a2 = approved(&client, &accounts, &token_a, 400);
        assert_eq!(
            submit_err(&client, &accounts, &token_a, 1),
            ForgeError::WithdrawalLimitExceeded
        );
        assert_eq!(
            client.check_withdrawal(&token_b, &500),
            CheckResult::Allowed
        );
        assert_eq!(
            client.get_withdrawal_window(&token_b),
            WindowState {
                total: 0,
                window_start: None,
                reset_at: None,
            }
        );
        let b1 = approved(&client, &accounts, &token_b, 500);
        assert_eq!(client.get_window_usage(&token_a), 1_000);
        assert_eq!(client.get_window_usage(&token_b), 500);

        // Executing A's withdrawals moves only A's custody.
        client.execute(&a1);
        client.execute(&a2);
        assert_eq!(client.balance(&token_a), FUNDING - 1_000);
        assert_eq!(client.balance(&token_b), FUNDING);
        assert_eq!(client.get_window_usage(&token_b), 500);

        // Re-limiting A leaves B's policy as it was.
        limited!(client, accounts, token_a, 2_000_i128, window_a);
        assert_eq!(client.get_withdrawal_limit(&token_b), b_limit);

        // Each window expires on its own clock.
        jump_to(&env, T0 + window_a.min(window_b));
        let (expired, kept) = if window_a < window_b {
            (&token_a, &token_b)
        } else {
            (&token_b, &token_a)
        };
        assert_eq!(client.get_window_usage(expired), 0);
        assert!(client.get_window_usage(kept) > 0);

        client.execute(&b1);
        assert_eq!(client.balance(&token_b), FUNDING - 500);
        assert_eq!(client.balance(&token_a), FUNDING - 1_000);
    }
}

// -----------------------------------------------------------------------
// Invariant 3 — exact boundary enforcement
// -----------------------------------------------------------------------

#[test]
fn exactly_the_remaining_allowance_passes_and_one_stroop_more_fails() {
    for window in WINDOWS {
        let (env, client, accounts, token, _token_client) = custody!();
        fund(&env, &client, &accounts, &token);
        jump_to(&env, T0);
        limited!(client, accounts, token, 1_000_i128, window);

        // Empty window: the whole limit is the remaining allowance.
        assert_eq!(
            client.check_withdrawal(&token, &1_001),
            CheckResult::LimitExceeded
        );
        assert_eq!(
            submit_err(&client, &accounts, &token, 1_001),
            ForgeError::WithdrawalLimitExceeded
        );

        approved(&client, &accounts, &token, 400);
        jump_after(&env, T0, 1);
        assert_eq!(
            client.check_withdrawal(&token, &601),
            CheckResult::LimitExceeded
        );
        assert_eq!(
            submit_err(&client, &accounts, &token, 601),
            ForgeError::WithdrawalLimitExceeded
        );
        assert_eq!(client.check_withdrawal(&token, &600), CheckResult::Allowed);
        approved(&client, &accounts, &token, 600);
        assert_eq!(client.get_window_usage(&token), 1_000);
        assert_eq!(
            submit_err(&client, &accounts, &token, 1),
            ForgeError::WithdrawalLimitExceeded
        );

        // When the first entry expires, exactly its 400 is free again.
        jump_to(&env, T0 + window);
        assert_eq!(
            submit_err(&client, &accounts, &token, 401),
            ForgeError::WithdrawalLimitExceeded
        );
        approved(&client, &accounts, &token, 400);
        assert_eq!(client.get_window_usage(&token), 1_000);
    }
}

// -----------------------------------------------------------------------
// Invariant 4 — execution-time re-check
// -----------------------------------------------------------------------

#[test]
fn stale_approval_is_rechecked_against_the_limit_in_force() {
    for window in WINDOWS {
        let (env, client, accounts, token, _token_client) = custody!();
        fund(&env, &client, &accounts, &token);
        jump_to(&env, T0);
        limited!(client, accounts, token, 1_000_i128, window);

        let stale = approved(&client, &accounts, &token, 700);
        let other = approved(&client, &accounts, &token, 200);

        // Lowered below the in-window total: the stale approval fails with
        // nothing changed, and stays pending.
        jump_after(&env, T0, 1);
        limited!(client, accounts, token, 899_i128, window);
        assert_eq!(
            client.try_execute(&stale).unwrap_err().unwrap(),
            ForgeError::WithdrawalLimitExceeded
        );
        assert_eq!(client.get_tx(&stale).status, TxStatus::Pending);
        let holders = [accounts.arbiter.clone(), client.address.clone()];
        let mid = BalanceSnapshot::new(&env, &token, &holders);
        mid.assert_balances(&[0, FUNDING]);
        assert_eq!(client.get_window_usage(&token), 900);

        // Lowered to exactly the in-window total: both settle, counted once.
        limited!(client, accounts, token, 900_i128, window);
        client.execute(&stale);
        client.execute(&other);
        let after = BalanceSnapshot::new(&env, &token, &holders);
        mid.assert_delta(&after, &[900, -900]);
        assert_eq!(mid.total(), after.total());
        assert_eq!(client.get_window_usage(&token), 900);
        assert_eq!(stored_total(&env, &client.address, &token), 900);
    }
}

#[test]
fn withdrawal_submitted_before_any_limit_is_admitted_at_execution() {
    for window in WINDOWS {
        let (env, client, accounts, token, token_client) = custody!();
        fund(&env, &client, &accounts, &token);
        jump_to(&env, T0);

        // No limit at submission: nothing is recorded.
        let big = approved(&client, &accounts, &token, 800);
        let small = approved(&client, &accounts, &token, 300);
        assert!(stored_entries(&env, &client.address, &token).is_empty());

        jump_after(&env, T0, 1);
        limited!(client, accounts, token, 500_i128, window);

        // 800 alone breaks the new limit.
        assert_eq!(
            client.try_execute(&big).unwrap_err().unwrap(),
            ForgeError::WithdrawalLimitExceeded
        );
        // 300 fits and is recorded at execution, so the window sees it.
        client.execute(&small);
        assert_eq!(token_client.balance(&accounts.arbiter), 300);
        assert_eq!(client.get_window_usage(&token), 300);
        assert_eq!(
            client.get_withdrawal_window(&token).window_start,
            Some(T0 + 1)
        );
        assert_eq!(
            client.check_withdrawal(&token, &201),
            CheckResult::LimitExceeded
        );
        assert_eq!(client.check_withdrawal(&token, &200), CheckResult::Allowed);

        // Once that window clears and the limit is raised, the big one fits.
        jump_to(&env, T0 + 1 + window);
        limited!(client, accounts, token, 800_i128, window);
        client.execute(&big);
        assert_eq!(token_client.balance(&accounts.arbiter), 1_100);
        assert_eq!(client.get_window_usage(&token), 800);
    }
}

#[test]
fn approval_older_than_its_window_must_fit_the_current_window_again() {
    for window in WINDOWS {
        let (env, client, accounts, token, token_client) = custody!();
        fund(&env, &client, &accounts, &token);
        jump_to(&env, T0);
        limited!(client, accounts, token, 1_000_i128, window);
        let old = approved(&client, &accounts, &token, 600);

        // The old entry expires; newer withdrawals fill the window.
        jump_to(&env, T0 + window);
        approved(&client, &accounts, &token, 500);
        assert_eq!(client.get_window_usage(&token), 500);
        assert_eq!(
            client.try_execute(&old).unwrap_err().unwrap(),
            ForgeError::WithdrawalLimitExceeded,
            "500 in window + 600 re-admitted would exceed 1,000"
        );

        // When the newer entries clear, it is recorded again at execution.
        jump_to(&env, T0 + 2 * window);
        client.execute(&old);
        assert_eq!(token_client.balance(&accounts.arbiter), 600);
        assert_eq!(client.get_window_usage(&token), 600);
        assert_eq!(
            stored_entries(&env, &client.address, &token),
            std::vec![WindowEntry {
                submitted_at: T0 + 2 * window,
                amount: 600,
            }]
        );
    }
}

#[test]
fn removed_limit_lets_stale_approvals_settle() {
    let (env, client, accounts, token, token_client) = custody!();
    fund(&env, &client, &accounts, &token);
    jump_to(&env, T0);
    limited!(client, accounts, token, 1_000_i128, HOUR);
    let pending = approved(&client, &accounts, &token, 1_000);
    limited!(client, accounts, token, 1_i128, HOUR);
    assert_eq!(
        client.try_execute(&pending).unwrap_err().unwrap(),
        ForgeError::WithdrawalLimitExceeded
    );

    let remove = client.remove_withdrawal_limit(&accounts.user1, &token);
    client.confirm(&remove, &accounts.user2);
    client.confirm(&remove, &accounts.user3);
    client.execute(&remove);
    client.execute(&pending);
    assert_eq!(token_client.balance(&accounts.arbiter), 1_000);
}

// -----------------------------------------------------------------------
// Model-based interleaving across two tokens and timestamp jumps
// -----------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Op {
    /// Submit (and approve) `amount` of token `token`.
    Submit { token: usize, amount: i128 },
    /// Submit the token's remaining allowance plus `delta` (-1, 0, or 1).
    SubmitRemaining { token: usize, delta: i128 },
    /// Execute the pending withdrawal at `pick % pending.len()`.
    Execute { pick: usize },
    /// Advance the ledger clock.
    Advance { seconds: u64 },
    /// Change a token's limit amount (same window).
    SetLimit { token: usize, amount: i128 },
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0usize..2, 1i128..=1_200).prop_map(|(token, amount)| Op::Submit { token, amount }),
        2 => (0usize..2, -1i128..=1).prop_map(|(token, delta)| Op::SubmitRemaining { token, delta }),
        4 => any::<usize>().prop_map(|pick| Op::Execute { pick }),
        3 => prop_oneof![
            Just(1u64),
            Just(HOUR - 1),
            Just(HOUR),
            Just(DAY),
            Just(WEEK),
            1u64..=2 * DAY,
        ]
        .prop_map(|seconds| Op::Advance { seconds }),
        1 => (0usize..2, 1i128..=2_000).prop_map(|(token, amount)| Op::SetLimit { token, amount }),
    ]
}

struct TokenModel {
    limit: i128,
    window: u64,
    /// `(submitted_at, amount)` of every recorded window entry.
    entries: StdVec<(u64, i128)>,
}

impl TokenModel {
    fn usage(&self, now: u64) -> i128 {
        self.entries
            .iter()
            .filter(|(at, _)| now.saturating_sub(*at) < self.window)
            .map(|(_, amount)| amount)
            .sum()
    }
}

struct PendingModel {
    tx: u64,
    token: usize,
    amount: i128,
    admitted_at: u64,
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        // Fixed seed: the same interleavings run on every machine and in CI.
        rng_seed: RngSeed::Fixed(0x4c49_4d49_5453_2900),
        ..ProptestConfig::default()
    })]

    #[test]
    fn interleaved_multi_token_withdrawals_match_the_window_model(
        windows in (prop::sample::select(WINDOWS.to_vec()), prop::sample::select(WINDOWS.to_vec())),
        limits in (1i128..=2_000, 1i128..=2_000),
        ops in prop::collection::vec(op_strategy(), 1..40),
    ) {
        let (env, client, accounts, token_a, _token_client) = custody!();
        fund(&env, &client, &accounts, &token_a);
        let token_b = second_token(&env, &client, &accounts);
        let tokens = [token_a, token_b];
        let mut now = T0;
        jump_to(&env, now);

        let mut models = [
            TokenModel { limit: limits.0, window: windows.0, entries: StdVec::new() },
            TokenModel { limit: limits.1, window: windows.1, entries: StdVec::new() },
        ];
        for (token, model) in tokens.iter().zip(&models) {
            limited!(client, accounts, token, model.limit, model.window);
        }
        let mut pending: StdVec<PendingModel> = StdVec::new();

        for op in ops {
            let mut written: Option<usize> = None;
            match op {
                Op::Submit { .. } | Op::SubmitRemaining { .. } => {
                    let (token, amount) = match op {
                        Op::Submit { token, amount } => (token, amount),
                        Op::SubmitRemaining { token, delta } => {
                            (token, models[token].limit - models[token].usage(now) + delta)
                        }
                        _ => unreachable!(),
                    };
                    if amount <= 0 {
                        continue;
                    }
                    let fits = models[token].usage(now) + amount <= models[token].limit;
                    prop_assert_eq!(
                        client.check_withdrawal(&tokens[token], &amount) == CheckResult::Allowed,
                        fits
                    );
                    let result = client.try_submit_withdrawal(
                        &accounts.user1,
                        &tokens[token],
                        &accounts.arbiter,
                        &amount,
                    );
                    if fits {
                        let tx = result.unwrap().unwrap();
                        client.confirm(&tx, &accounts.user2);
                        client.confirm(&tx, &accounts.user3);
                        models[token].entries.push((now, amount));
                        pending.push(PendingModel { tx, token, amount, admitted_at: now });
                        written = Some(token);
                    } else {
                        prop_assert_eq!(
                            result.unwrap_err().unwrap(),
                            ForgeError::WithdrawalLimitExceeded
                        );
                    }
                }
                Op::Execute { pick } => {
                    if pending.is_empty() {
                        continue;
                    }
                    let index = pick % pending.len();
                    let p = &pending[index];
                    let model = &models[p.token];
                    let counted = now.saturating_sub(p.admitted_at) < model.window;
                    let usage = model.usage(now);
                    let allowed = if counted {
                        usage <= model.limit
                    } else {
                        usage + p.amount <= model.limit
                    };
                    let custody_before = client.balance(&tokens[p.token]);
                    let result = client.try_execute(&p.tx);
                    if allowed {
                        prop_assert!(result.is_ok(), "execute must succeed: {:?}", result);
                        prop_assert_eq!(
                            client.balance(&tokens[p.token]),
                            custody_before - p.amount
                        );
                        if !counted {
                            models[p.token].entries.push((now, p.amount));
                            written = Some(p.token);
                        }
                        pending.remove(index);
                    } else {
                        prop_assert_eq!(
                            result.unwrap_err().unwrap(),
                            ForgeError::WithdrawalLimitExceeded
                        );
                        prop_assert_eq!(client.get_tx(&p.tx).status, TxStatus::Pending);
                        prop_assert_eq!(client.balance(&tokens[p.token]), custody_before);
                    }
                }
                Op::Advance { seconds } => {
                    now += seconds;
                    jump_to(&env, now);
                }
                Op::SetLimit { token, amount } => {
                    let window = models[token].window;
                    let limited_token = tokens[token].clone();
                    limited!(client, accounts, limited_token, amount, window);
                    models[token].limit = amount;
                }
            }

            for (index, token) in tokens.iter().enumerate() {
                let model = &models[index];
                // Usage, window views, and the policy match the model for
                // both tokens after every operation (isolation included).
                prop_assert_eq!(client.get_window_usage(token), model.usage(now));
                let state = client.get_withdrawal_window(token);
                prop_assert_eq!(state.total, model.usage(now));
                let oldest = model
                    .entries
                    .iter()
                    .filter(|(at, _)| now.saturating_sub(*at) < model.window)
                    .map(|(at, _)| *at)
                    .min();
                prop_assert_eq!(state.window_start, oldest);
                prop_assert_eq!(state.reset_at, oldest.map(|at| at + model.window));
                prop_assert_eq!(
                    client.get_withdrawal_limit(token).map(|l| (l.amount, l.window_seconds)),
                    Some((model.limit, model.window))
                );
            }
            if let Some(index) = written {
                // A write leaves no expired entry and no orphaned total.
                let stored = stored_entries(&env, &client.address, &tokens[index]);
                let window = models[index].window;
                prop_assert!(stored.iter().all(|e| now.saturating_sub(e.submitted_at) < window));
                prop_assert_eq!(
                    stored.iter().map(|e| e.amount).sum::<i128>(),
                    models[index].usage(now)
                );
            }
        }
    }
}
