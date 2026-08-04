#![no_std]

//! # Soroban Forge — Multi-signature Wallet contract
//!
//! A wallet that requires `threshold` approvals from a set of owners before a
//! transaction executes. The public interface is declared by
//! [`SorobanForgeMultiSigWallet`]; [`MultiSigWallet`] is the deployable
//! contract. Implementation arrives in a later commit.

use soroban_sdk::{contract, contractclient, contractimpl, contracttype, Address, Bytes, Env};

/// Public interface for the Soroban Forge multi-signature wallet contract.
#[contractclient(name = "SorobanForgeMultiSigWalletClient")]
pub trait SorobanForgeMultiSigWallet {
    /// Add a pending transaction and open it for owner approvals.
    fn submit(env: Env, tx: Bytes) -> Result<u64, soroban_forge_shared_utils::ForgeError>;

    /// Record `signer`'s approval of `tx_id`.
    fn confirm(
        env: Env,
        tx_id: u64,
        signer: Address,
    ) -> Result<(), soroban_forge_shared_utils::ForgeError>;

    /// Execute `tx_id` once approvals meet the configured threshold.
    fn execute(env: Env, tx_id: u64) -> Result<(), soroban_forge_shared_utils::ForgeError>;
}

/// Lifecycle state of a submitted transaction.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TxStatus {
    /// Open for confirmations; threshold not yet met.
    Pending,
    /// Threshold met and executed successfully.
    Executed,
    /// Rejected by owners (reached a rejection threshold or manually revoked).
    Rejected,
}

/// A transaction awaiting multi-signature approval.
#[contracttype]
#[derive(Clone, Debug)]
pub struct WalletTx {
    /// Stable identifier assigned at submission time.
    pub tx_id: u64,
    /// Address that submitted the transaction.
    pub submitter: Address,
    /// The encoded transaction payload to execute.
    pub payload: Bytes,
    /// Owners that have confirmed so far.
    pub confirmations: soroban_sdk::Vec<Address>,
    /// Current state.
    pub status: TxStatus,
}

/// The deployable multi-signature wallet contract.
///
/// The `#[contractimpl]` block is intentionally empty at this stage; the
/// submission/confirmation/execution logic is added in a subsequent commit.
#[contract]
pub struct MultiSigWallet;

#[contractimpl]
impl MultiSigWallet {}
