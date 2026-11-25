//! Deterministic, test-only fixtures shared by contract test suites.
//!
//! These helpers live behind the crate's `testutils` feature (or `cfg(test)`
//! inside this crate) so they never become part of a production contract API.
//! Every helper avoids randomness: ledger jumps advance the sequence by a
//! fixed step, accounts come from fixed strkeys, and snapshots preserve the
//! caller's ordering.

use soroban_sdk::testutils::Ledger as _;
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env, Vec};

/// Fixed starting timestamp used by boundary fixtures.
pub const TEST_START_TIMESTAMP: u64 = 1_000_000;

/// Fixed contract-address strkeys, in a stable order. Each encodes a 32-byte
/// payload of the repeated index byte (1..=6), so no RNG or live network is
/// needed to regenerate them. Contract addresses are used deliberately: SAC
/// mints to classic (`G`) accounts would require a trustline per holder.
const ACCOUNT_STRKEYS: [&str; 6] = [
    "CAAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQC526",
    "CABAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAFNSZ",
    "CABQGAYDAMBQGAYDAMBQGAYDAMBQGAYDAMBQGAYDAMBQGAYDAMBQGCK3",
    "CACAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAINCW",
    "CACQKBIFAUCQKBIFAUCQKBIFAUCQKBIFAUCQKBIFAUCQKBIFAUCQLC2U",
    "CADAMBQGAYDAMBQGAYDAMBQGAYDAMBQGAYDAMBQGAYDAMBQGAYDAMSST",
];

/// Return the fixed account used for fixture index `index`.
///
/// # Panics
///
/// Panics when `index` is outside the fixture account list.
#[must_use]
pub fn deterministic_account(env: &Env, index: usize) -> Address {
    let strkey = ACCOUNT_STRKEYS
        .get(index)
        .unwrap_or_else(|| panic!("fixture account index {index} is out of range"));
    Address::from_str(env, strkey)
}

/// Return all fixture accounts in their stable order.
#[must_use]
pub fn deterministic_accounts(env: &Env) -> Vec<Address> {
    let mut accounts = Vec::new(env);
    for strkey in ACCOUNT_STRKEYS {
        accounts.push_back(Address::from_str(env, strkey));
    }
    accounts
}

/// Place the test ledger at an absolute timestamp and advance its sequence by
/// exactly one ledger.
///
/// Unlike the monotonic ledger helpers in `soroban-forge-test-utils`, this is
/// meant for boundary placement and may move the timestamp backwards. Other
/// ledger fields are preserved.
pub fn jump_to_timestamp(env: &Env, timestamp: u64) -> u64 {
    let ledger = env.ledger();
    let sequence = ledger.sequence().checked_add(1).expect("ledger overflow");
    ledger.with_mut(|info| {
        info.sequence_number = sequence;
        info.timestamp = timestamp;
    });
    timestamp
}

/// Move the test ledger `seconds` past `boundary`.
pub fn jump_after(env: &Env, boundary: u64, seconds: u64) -> u64 {
    jump_to_timestamp(env, boundary.saturating_add(seconds))
}

/// Move the test ledger `seconds` before `boundary`.
pub fn jump_before(env: &Env, boundary: u64, seconds: u64) -> u64 {
    jump_to_timestamp(env, boundary.saturating_sub(seconds))
}

/// Place the test ledger exactly on `boundary`.
pub fn jump_to(env: &Env, boundary: u64) -> u64 {
    jump_to_timestamp(env, boundary)
}

/// A registered Stellar Asset Contract with a deterministic admin.
pub struct TestSac {
    env: Env,
    /// Address of the registered asset contract.
    pub token: Address,
    /// Address holding the SAC admin role.
    pub admin: Address,
}

impl TestSac {
    /// Register a SAC using the first deterministic fixture account as admin.
    #[must_use]
    pub fn new(env: &Env) -> Self {
        Self::with_admin(env, deterministic_account(env, 0))
    }

    /// Register a SAC with an explicitly chosen admin.
    #[must_use]
    pub fn with_admin(env: &Env, admin: Address) -> Self {
        let token = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        Self {
            env: env.clone(),
            token,
            admin,
        }
    }

    /// Mint `amount` to `to` through the SAC admin client.
    pub fn mint(&self, to: &Address, amount: i128) {
        StellarAssetClient::new(&self.env, &self.token).mint(to, &amount);
    }

    /// Return the SEP-41 client for the registered asset.
    #[must_use]
    pub fn client(&self) -> TokenClient<'static> {
        TokenClient::new(&self.env, &self.token)
    }

    /// Return one account's balance.
    #[must_use]
    pub fn balance(&self, holder: &Address) -> i128 {
        self.client().balance(holder)
    }

    /// Capture balances for `holders` in the supplied order.
    #[must_use]
    pub fn snapshot(&self, holders: &[Address]) -> BalanceSnapshot {
        BalanceSnapshot::new(&self.env, &self.token, holders)
    }
}

/// Balances captured for a fixed set of addresses at one point in a test.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BalanceSnapshot {
    holders: Vec<Address>,
    balances: Vec<i128>,
}

impl BalanceSnapshot {
    /// Capture `token` balances for `holders` in the supplied order.
    #[must_use]
    pub fn new(env: &Env, token: &Address, holders: &[Address]) -> Self {
        let mut snapshot_holders = Vec::new(env);
        let mut balances = Vec::new(env);
        let token_client = TokenClient::new(env, token);
        for holder in holders {
            snapshot_holders.push_back(holder.clone());
            balances.push_back(token_client.balance(holder));
        }
        Self {
            holders: snapshot_holders,
            balances,
        }
    }

    /// Return the captured holder addresses.
    #[must_use]
    pub fn holders(&self) -> &Vec<Address> {
        &self.holders
    }

    /// Return one holder's captured balance, if the holder was snapshotted.
    #[must_use]
    pub fn balance_of(&self, holder: &Address) -> Option<i128> {
        for index in 0..self.holders.len() {
            if self.holders.get(index) == Some(holder.clone()) {
                return Some(self.balances.get(index).unwrap());
            }
        }
        None
    }

    /// Return the sum of all captured balances.
    #[must_use]
    pub fn total(&self) -> i128 {
        let mut total = 0_i128;
        for index in 0..self.balances.len() {
            total += self.balances.get(index).unwrap();
        }
        total
    }

    /// Assert the snapshot matches `expected` in the same order.
    ///
    /// # Panics
    ///
    /// Panics when the lengths differ or any balance differs.
    pub fn assert_balances(&self, expected: &[i128]) {
        assert_eq!(
            self.balances.len() as usize,
            expected.len(),
            "snapshot holder count must match expected balance count"
        );
        for (index, amount) in expected.iter().enumerate() {
            assert_eq!(
                self.balances.get(index as u32).unwrap(),
                *amount,
                "balance mismatch at holder index {index}"
            );
        }
    }

    /// Assert every holder changed by the corresponding amount in `deltas`.
    ///
    /// # Panics
    ///
    /// Panics when the snapshots cover different holders or any delta differs.
    pub fn assert_delta(&self, after: &Self, deltas: &[i128]) {
        assert_eq!(
            self.holders, after.holders,
            "snapshots must cover the same holders"
        );
        assert_eq!(
            self.balances.len() as usize,
            deltas.len(),
            "snapshot holder count must match delta count"
        );
        for (index, delta) in deltas.iter().enumerate() {
            let before = self.balances.get(index as u32).unwrap();
            let actual = after.balances.get(index as u32).unwrap();
            assert_eq!(
                actual,
                before + *delta,
                "balance delta mismatch at holder index {index}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    #[test]
    fn accounts_are_fixed_and_distinct() {
        let env = Env::default();
        let accounts = deterministic_accounts(&env);
        assert_eq!(accounts.len(), ACCOUNT_STRKEYS.len() as u32);
        assert_eq!(deterministic_account(&env, 0), accounts.get(0).unwrap());

        for first in 0..accounts.len() {
            for second in (first + 1)..accounts.len() {
                assert_ne!(accounts.get(first).unwrap(), accounts.get(second).unwrap());
            }
        }
    }

    #[test]
    fn timestamp_jumps_are_exact_and_advance_one_ledger() {
        let env = Env::default();
        let sequence = env.ledger().sequence();
        let before = env.ledger().get();

        assert_eq!(
            jump_to_timestamp(&env, TEST_START_TIMESTAMP),
            TEST_START_TIMESTAMP
        );
        assert_eq!(env.ledger().timestamp(), TEST_START_TIMESTAMP);
        assert_eq!(env.ledger().sequence(), sequence + 1);
        assert_eq!(env.ledger().get().protocol_version, before.protocol_version);

        assert_eq!(
            jump_before(&env, TEST_START_TIMESTAMP, 1),
            TEST_START_TIMESTAMP - 1
        );
        assert_eq!(env.ledger().timestamp(), TEST_START_TIMESTAMP - 1);
        assert_eq!(jump_to(&env, TEST_START_TIMESTAMP), TEST_START_TIMESTAMP);
        assert_eq!(
            jump_after(&env, TEST_START_TIMESTAMP, 1),
            TEST_START_TIMESTAMP + 1
        );
        assert_eq!(env.ledger().sequence(), sequence + 4);
    }

    #[test]
    fn sac_and_balance_snapshot_fixtures_are_repeatable() {
        let env = Env::default();
        env.mock_all_auths();
        let sac = TestSac::new(&env);
        let holders = [
            deterministic_account(&env, 1),
            deterministic_account(&env, 2),
        ];
        sac.mint(&holders[0], 100);

        let before = sac.snapshot(&holders);
        before.assert_balances(&[100, 0]);
        assert_eq!(before.balance_of(&holders[0]), Some(100));
        assert_eq!(before.balance_of(&deterministic_account(&env, 3)), None);

        sac.client().transfer(&holders[0], &holders[1], &40);
        let after = sac.snapshot(&holders);
        before.assert_delta(&after, &[-40, 40]);
        assert_eq!(before.total(), after.total());
    }

    #[test]
    fn replaying_the_same_fixture_sequence_is_identical() {
        fn run() -> (BalanceSnapshot, BalanceSnapshot) {
            let env = Env::default();
            env.mock_all_auths();
            jump_to_timestamp(&env, TEST_START_TIMESTAMP);
            let sac = TestSac::new(&env);
            let holders = [
                deterministic_account(&env, 1),
                deterministic_account(&env, 2),
            ];
            sac.mint(&holders[0], 50);
            let before = sac.snapshot(&holders);
            jump_after(&env, TEST_START_TIMESTAMP, 10);
            sac.client().transfer(&holders[0], &holders[1], &25);
            let after = sac.snapshot(&holders);
            (before, after)
        }

        assert_eq!(run(), run());
    }
}
