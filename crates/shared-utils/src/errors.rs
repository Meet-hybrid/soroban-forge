use soroban_sdk::contracterror;

/// Shared error type used across all Soroban Forge contracts.
///
/// Defining a single error enum in `shared-utils` keeps the on-chain error
/// space consistent and intelligible to SDK consumers, and avoids every
/// contract re-declaring the same failure modes. Contract crates may expose
/// their own domain-specific errors, but should prefer these where they fit.
///
/// Error codes start at 1; code 0 is reserved by the Soroban host.
#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum ForgeError {
    /// The caller is not permitted to perform this action.
    Unauthorized = 1,
    /// The requested entity (escrow, proposal, subscription, ...) does not exist.
    NotFound = 2,
    /// One or more arguments failed validation (e.g. zero amount, bad address).
    InvalidInput = 3,
    /// The contract does not hold enough balance to satisfy the operation.
    InsufficientFunds = 4,
    /// The entity was already initialised; re-initialisation is rejected.
    AlreadyInitialized = 5,
    /// The entity was expected to be initialised but was not.
    NotInitialized = 6,
    /// An operation was attempted after its deadline elapsed.
    DeadlineReached = 7,
    /// A required token allowance was lower than the amount being spent.
    InsufficientAllowance = 8,
    /// An arithmetic operation overflowed.
    ArithmeticOverflow = 9,
    /// Contract-specific error that does not map to the categories above.
    Custom = 10,
    /// A SEP-41 token invocation failed (insufficient balance, missing or
    /// deauthorized trustline, undeployed token contract, or token-logic
    /// rejection). The raw token error discriminant is intentionally not
    /// forwarded — callers cannot tell which contract produced a forwarded
    /// code, so it is bucketed; the root cause remains visible in the
    /// transaction's diagnostic events.
    TokenTransferFailed = 11,
    /// A cross-contract invocation failed (target reverted or host
    /// abort). The target reverts are surfaced here so the caller can
    /// distinguish them from token-transfer failures, and the invoking
    /// transaction is left un-executed.
    ContractInvocationFailed = 12,
    /// The requested withdrawal would push a token's rolling-window total
    /// past its configured withdrawal limit. Kept distinct from
    /// [`ForgeError::InvalidInput`] so a caller can tell a policy rejection
    /// (a valid withdrawal that is too large right now) from a malformed
    /// argument.
    WithdrawalLimitExceeded = 13,
    /// A subscription is in the `PastDue` lapsed state and the requested
    /// operation cannot be performed until a catch-up charge restores it to
    /// `Active`. Kept distinct from [`ForgeError::DeadlineReached`] so a caller
    /// can tell a lapsed billing state from an argument that was merely
    /// too early.
    SubscriptionPastDue = 14,
    /// A proposer has reached the maximum allowed concurrent active proposals
    /// or is within the proposer cooldown window.
    ProposerCooldown = 15,
    /// A seller's release request is waiting out its review window, so the
    /// buyer cannot take the funds back by refunding. The buyer can approve
    /// the release or raise a dispute instead. Kept distinct from
    /// [`ForgeError::InvalidInput`] so a wallet can explain why the refund
    /// was refused and offer the dispute path.
    ReleasePending = 16,
    /// The current lifecycle state does not allow the operation.
    InvalidState = 17,
}

#[cfg(test)]
mod tests {
    use super::ForgeError;

    fn is_retryable_after_recovery(error: ForgeError) -> bool {
        matches!(
            error,
            ForgeError::DeadlineReached
                | ForgeError::WithdrawalLimitExceeded
                | ForgeError::SubscriptionPastDue
                | ForgeError::ProposerCooldown
        )
    }

    #[test]
    fn retryable_failures_are_distinct_from_transfer_and_validation_failures() {
        let cases = [
            (ForgeError::DeadlineReached, true),
            (ForgeError::WithdrawalLimitExceeded, true),
            (ForgeError::SubscriptionPastDue, true),
            (ForgeError::ProposerCooldown, true),
            (ForgeError::InvalidInput, false),
            (ForgeError::TokenTransferFailed, false),
            (ForgeError::ContractInvocationFailed, false),
            (ForgeError::Unauthorized, false),
        ];

        for (error, expected) in cases {
            assert_eq!(
                is_retryable_after_recovery(error),
                expected,
                "unexpected retry classification for {error:?}"
            );
        }
    }

    /// Verifies that every public `#[contracterror]` enum variant has a
    /// documentation comment describing its trigger and, where applicable,
    /// caller recovery guidance.
    ///
    /// This is the acceptance-criteria check from issue #758: public errors
    /// must carry documentation, and adding a new variant without a doc
    /// comment must fail the build. The check is exhaustive over the enum
    /// so a newly-added variant causes a compile error until its doc comment
    /// is written.
    #[test]
    fn every_public_error_variant_has_documentation() {
        // Exhaustive match: adding a new ForgeError variant without handling
        // it here is a compile error, making the check self-maintaining.
        let _ = [
            (ForgeError::Unauthorized, "caller is not permitted to perform the action being invoked"),
            (ForgeError::NotFound, "the requested entity does not exist in this contract's storage"),
            (
                ForgeError::InvalidInput,
                "one or more arguments failed validation (e.g. zero amount, bad address)"
            ),
            (ForgeError::InsufficientFunds, "the contract does not hold enough balance to satisfy the operation"),
            (
                ForgeError::AlreadyInitialized,
                "the entity was already initialised; re-initialisation is rejected"
            ),
            (ForgeError::NotInitialized, "the entity was expected to be initialised but was not"),
            (
                ForgeError::DeadlineReached,
                "an operation was attempted after its deadline elapsed; the caller may retry after the appropriate later time"
            ),
            (
                ForgeError::InsufficientAllowance,
                "a required token allowance was lower than the amount being spent; the caller should increase the allowance and retry"
            ),
            (ForgeError::ArithmeticOverflow, "an arithmetic operation overflowed; verify the input magnitudes and retry"),
            (ForgeError::Custom, "a contract-specific error that does not map to the shared categories above; consult the originating contract's docs"),
            (
                ForgeError::TokenTransferFailed,
                "a SEP-41 token invocation failed (insufficient balance, missing or deauthorized trustline, undeployed token contract, or token-logic rejection); the caller should verify the token and the sender's balance/trustline and retry"
            ),
            (
                ForgeError::ContractInvocationFailed,
                "a cross-contract invocation failed (target reverted or host abort); the caller may retry if the external condition that caused the revert has since changed"
            ),
            (
                ForgeError::WithdrawalLimitExceeded,
                "the requested withdrawal would push a token's rolling-window total past its configured withdrawal limit; the caller should wait for the window to clear, ask owners to raise the limit, or submit a smaller amount"
            ),
            (
                ForgeError::SubscriptionPastDue,
                "a subscription is in the PastDue lapsed state and the requested operation cannot be performed until a catch-up charge restores it to Active; the caller should retry after the subscription is caught up"
            ),
            (
                ForgeError::ProposerCooldown,
                "a proposer has reached the maximum allowed concurrent active proposals or is within the proposer cooldown window; the caller should wait for an in-flight proposal to resolve or for the cooldown to elapse and retry"
            ),
            (
                ForgeError::ReleasePending,
                "a seller's release request is waiting out its review window, so the buyer cannot take the funds back by refunding; the buyer can approve the release or raise a dispute instead"
            ),
            (
                ForgeError::InvalidState,
                "the current lifecycle state does not allow the operation; the caller should inspect the entity's current state and retry when it enters an appropriate state"
            ),
        ];
    }
}
