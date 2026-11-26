use soroban_sdk::contracterror;

/// Shared error type used across all Soroban Forge contracts.
///
/// Contract crates may expose their own domain-specific errors, but should
/// prefer these where they fit.
///
/// Error codes start at 1; code 0 is reserved by the Soroban host.
#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum ForgeError {
    /// One or more arguments failed validation (e.g. zero amount, bad address).
    InvalidInput = 1,
    /// The requested entity (escrow, proposal, subscription, ...) does not exist.
    NotFound = 2,
    /// The caller is not permitted to perform this action.
    Unauthorized = 3,
}
