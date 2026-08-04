#![no_std]

//! # Soroban Forge — Escrow contract
//!
//! A three-party escrow: a `buyer`, a `seller`, and an `arbiter` agree on an
//! `amount` and a `timeout`. The buyer funds the escrow, and the contract
//! tracks the lifecycle:
//!
//! ```text
//! Pending --deposit--> Funded --release--> Completed
//!                     |        --refund--> Refunded
//!          --cancel--> Cancelled (before funding only)
//! ```
//!
//! Authorization model (current scope):
//! - `create_escrow` requires both the buyer and the seller.
//! - `deposit` and `release` require the buyer.
//! - `refund` requires the seller before the deadline; after the deadline the
//!   buyer may reclaim (timed-out refund).
//! - `cancel` requires the buyer while the escrow is still `Pending`.
//!
//! The `Disputed` status is reserved for an arbiter dispute method that lands
//! in a follow-up; it is not reachable through the current public interface.
//! Token settlement (SAC transfers) is intentionally out of scope for this
//! iteration: the contract tracks state and authorization, not balances.

#[cfg(test)]
extern crate std;

use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::{contract, contractclient, contractimpl, contracttype, Address, Env};

/// Public interface for the Soroban Forge escrow contract.
#[contractclient(name = "SorobanForgeEscrowClient")]
pub trait SorobanForgeEscrow {
    fn create_escrow(
        env: Env,
        buyer: Address,
        seller: Address,
        arbiter: Address,
        amount: i128,
        timeout: u64,
    ) -> Result<u64, soroban_forge_shared_utils::ForgeError>;

    fn deposit(env: Env, escrow_id: u64) -> Result<(), soroban_forge_shared_utils::ForgeError>;
    fn release(env: Env, escrow_id: u64) -> Result<(), soroban_forge_shared_utils::ForgeError>;
    fn refund(env: Env, escrow_id: u64) -> Result<(), soroban_forge_shared_utils::ForgeError>;
    fn get_status(
        env: Env,
        escrow_id: u64,
    ) -> Result<EscrowStatus, soroban_forge_shared_utils::ForgeError>;
    fn cancel(env: Env, escrow_id: u64) -> Result<(), soroban_forge_shared_utils::ForgeError>;
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EscrowStatus {
    Pending,
    Funded,
    Completed,
    Refunded,
    Disputed,
    Cancelled,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct EscrowData {
    pub escrow_id: u64,
    pub buyer: Address,
    pub seller: Address,
    pub arbiter: Address,
    pub amount: i128,
    pub timeout: u64,
    pub status: EscrowStatus,
    pub created_at: u64,
}

/// Instance-storage keys.
#[contracttype]
enum DataKey {
    /// The escrow record for `u64` id.
    Escrow(u64),
    /// Monotonic id counter.
    Count,
}

/// The deployable escrow contract.
#[contract]
pub struct Escrow;

#[contractimpl]
impl Escrow {
    /// Create a new escrow and return its stable id.
    ///
    /// Requires `amount > 0` and `timeout > 0`. Both the buyer and the seller
    /// are authorized at creation time.
    pub fn create_escrow(
        env: Env,
        buyer: Address,
        seller: Address,
        arbiter: Address,
        amount: i128,
        timeout: u64,
    ) -> Result<u64, ForgeError> {
        if amount <= 0 {
            return Err(ForgeError::InvalidInput);
        }
        if timeout == 0 {
            return Err(ForgeError::InvalidInput);
        }
        buyer.require_auth();
        seller.require_auth();

        let id = Self::next_id(&env)?;
        let escrow = EscrowData {
            escrow_id: id,
            buyer,
            seller,
            arbiter,
            amount,
            timeout,
            status: EscrowStatus::Pending,
            created_at: env.ledger().timestamp(),
        };
        env.storage().instance().set(&DataKey::Escrow(id), &escrow);
        Ok(id)
    }

    /// Fund the escrow. Requires the buyer; only valid while `Pending`.
    pub fn deposit(env: Env, escrow_id: u64) -> Result<(), ForgeError> {
        let mut escrow = Self::get_escrow(&env, escrow_id)?;
        escrow.buyer.require_auth();
        if escrow.status != EscrowStatus::Pending {
            return Err(ForgeError::InvalidInput);
        }
        escrow.status = EscrowStatus::Funded;
        env.storage()
            .instance()
            .set(&DataKey::Escrow(escrow_id), &escrow);
        Ok(())
    }

    /// Release funds to the seller. Requires the buyer (confirms receipt);
    /// only valid while `Funded`.
    pub fn release(env: Env, escrow_id: u64) -> Result<(), ForgeError> {
        let mut escrow = Self::get_escrow(&env, escrow_id)?;
        escrow.buyer.require_auth();
        if escrow.status != EscrowStatus::Funded {
            return Err(ForgeError::InvalidInput);
        }
        escrow.status = EscrowStatus::Completed;
        env.storage()
            .instance()
            .set(&DataKey::Escrow(escrow_id), &escrow);
        Ok(())
    }

    /// Refund the escrow.
    ///
    /// Before the deadline the seller may refund; after the deadline the buyer
    /// may reclaim. Only valid while `Funded`.
    pub fn refund(env: Env, escrow_id: u64) -> Result<(), ForgeError> {
        let mut escrow = Self::get_escrow(&env, escrow_id)?;
        if escrow.status != EscrowStatus::Funded {
            return Err(ForgeError::InvalidInput);
        }
        let now = env.ledger().timestamp();
        let deadline = escrow
            .created_at
            .checked_add(escrow.timeout)
            .ok_or(ForgeError::ArithmeticOverflow)?;

        if now >= deadline {
            // Timeout reached: the buyer can reclaim the funds.
            escrow.buyer.require_auth();
        } else {
            // Before the deadline the seller can issue the refund.
            escrow.seller.require_auth();
        }

        escrow.status = EscrowStatus::Refunded;
        env.storage()
            .instance()
            .set(&DataKey::Escrow(escrow_id), &escrow);
        Ok(())
    }

    /// Cancel a `Pending` escrow before it is funded. Requires the buyer.
    pub fn cancel(env: Env, escrow_id: u64) -> Result<(), ForgeError> {
        let mut escrow = Self::get_escrow(&env, escrow_id)?;
        if escrow.status != EscrowStatus::Pending {
            return Err(ForgeError::InvalidInput);
        }
        escrow.buyer.require_auth();

        escrow.status = EscrowStatus::Cancelled;
        env.storage()
            .instance()
            .set(&DataKey::Escrow(escrow_id), &escrow);
        Ok(())
    }

    /// Read the current lifecycle status.
    pub fn get_status(env: Env, escrow_id: u64) -> Result<EscrowStatus, ForgeError> {
        Ok(Self::get_escrow(&env, escrow_id)?.status)
    }

    /// Allocate the next monotonic escrow id.
    fn next_id(env: &Env) -> Result<u64, ForgeError> {
        let count: u64 = env.storage().instance().get(&DataKey::Count).unwrap_or(0);
        let id = count.checked_add(1).ok_or(ForgeError::ArithmeticOverflow)?;
        env.storage().instance().set(&DataKey::Count, &id);
        Ok(id)
    }

    fn get_escrow(env: &Env, escrow_id: u64) -> Result<EscrowData, ForgeError> {
        env.storage()
            .instance()
            .get(&DataKey::Escrow(escrow_id))
            .ok_or(ForgeError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_forge_test_utils::TestAccounts;
    use soroban_sdk::testutils::Ledger as _;
    use soroban_sdk::Env;

    const START: u64 = 1_000_000;
    const TIMEOUT: u64 = 86_400;

    /// Build a fresh env with mocked auths, a registered contract, and named
    /// accounts. The generated client borrows the env, so it cannot be
    /// returned from a helper.
    macro_rules! setup {
        () => {{
            let env = Env::default();
            env.mock_all_auths();
            env.ledger().set_timestamp(START);
            let contract_id = env.register_contract(None, Escrow);
            let client = SorobanForgeEscrowClient::new(&env, &contract_id);
            let accounts = TestAccounts::generate(&env);
            (env, client, accounts)
        }};
    }

    // NOTE: negative authorization tests (calling `require_auth` without a
    // matching signature) are not runnable in-process with soroban-sdk 21.5.1:
    // the host raises a non-unwinding panic that aborts the test binary. They
    // are tracked in the security-invariant test backlog (Issue 7).

    fn create(client: &SorobanForgeEscrowClient<'_>, accounts: &TestAccounts) -> u64 {
        client.create_escrow(
            &accounts.user1,
            &accounts.user2,
            &accounts.arbiter,
            &1_000_i128,
            &TIMEOUT,
        )
    }

    #[test]
    fn create_escrow_succeeds_and_is_pending() {
        let (_env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        assert_eq!(client.get_status(&id), EscrowStatus::Pending);
    }

    #[test]
    fn create_escrow_assigns_distinct_ids() {
        let (_env, client, accounts) = setup!();
        let id1 = create(&client, &accounts);
        let id2 = create(&client, &accounts);
        assert_ne!(id1, id2);
    }

    #[test]
    fn create_escrow_rejects_zero_amount() {
        let (_env, client, accounts) = setup!();
        let err = client
            .try_create_escrow(
                &accounts.user1,
                &accounts.user2,
                &accounts.arbiter,
                &0_i128,
                &TIMEOUT,
            )
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn create_escrow_rejects_zero_timeout() {
        let (_env, client, accounts) = setup!();
        let err = client
            .try_create_escrow(
                &accounts.user1,
                &accounts.user2,
                &accounts.arbiter,
                &1_000_i128,
                &0_u64,
            )
            .unwrap_err()
            .unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn deposit_marks_funded() {
        let (_env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        client.deposit(&id);
        assert_eq!(client.get_status(&id), EscrowStatus::Funded);
    }

    #[test]
    fn deposit_twice_is_invalid() {
        let (_env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        client.deposit(&id);
        let err = client.try_deposit(&id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn deposit_missing_escrow_is_not_found() {
        let (_env, client, _accounts) = setup!();
        let err = client.try_deposit(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn release_after_deposit_completes() {
        let (_env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        client.deposit(&id);
        client.release(&id);
        assert_eq!(client.get_status(&id), EscrowStatus::Completed);
    }

    #[test]
    fn release_before_deposit_is_invalid() {
        let (_env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        let err = client.try_release(&id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn refund_before_deadline_succeeds() {
        let (_env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        client.deposit(&id);
        client.refund(&id);
        assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
    }

    #[test]
    fn refund_after_deadline_succeeds() {
        let (env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        client.deposit(&id);
        env.ledger().set_timestamp(START + TIMEOUT + 1);
        client.refund(&id);
        assert_eq!(client.get_status(&id), EscrowStatus::Refunded);
    }

    #[test]
    fn refund_before_funding_is_invalid() {
        let (_env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        let err = client.try_refund(&id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn cancel_pending_is_allowed() {
        let (_env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        client.cancel(&id);
        assert_eq!(client.get_status(&id), EscrowStatus::Cancelled);
    }

    #[test]
    fn cancel_after_deposit_is_invalid() {
        let (_env, client, accounts) = setup!();
        let id = create(&client, &accounts);
        client.deposit(&id);
        let err = client.try_cancel(&id).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::InvalidInput);
    }

    #[test]
    fn cancel_missing_escrow_is_not_found() {
        let (_env, client, _accounts) = setup!();
        let err = client.try_cancel(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }

    #[test]
    fn get_status_missing_escrow_is_not_found() {
        let (_env, client, _accounts) = setup!();
        let err = client.try_get_status(&999).unwrap_err().unwrap();
        assert_eq!(err, ForgeError::NotFound);
    }
}
