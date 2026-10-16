//! Representative benchmark flows, one per contract crate.
//!
//! Every flow is deterministic and network-independent:
//!
//! - `Env::default()` uses a fixed base PRNG seed and default ledger info, so
//!   generated addresses and host state are reproducible;
//! - fixtures pin the ledger timestamp and mint fixed token amounts;
//! - no wall-clock time, no network access, no randomness.
//!
//! A flow builds its fixture (earlier top-level invocations are setup, not
//! measurement), invokes the representative entrypoint as the **final**
//! top-level invocation, and captures the resources of that invocation
//! immediately afterwards via [`ResourceSample::capture`] — before any
//! post-hoc assertions, which are themselves top-level invocations and would
//! clobber a "last invocation" cost reading. Where a lifecycle needs two
//! calls of the same entrypoint (e.g. DAO governance finalises a proposal on
//! the first `execute` and dispatches on the second), the first call is
//! fixture setup and only the final call is measured.

use crate::metrics::ResourceSample;

pub mod dao_governance;
pub mod escrow;
pub mod marketplace_royalties;
pub mod multi_sig_wallet;
pub mod subscription_payments;
pub mod vesting;

/// A benchmark flow: the contract package and entrypoint it exercises, a
/// stable scenario name, and the deterministic measurement routine.
pub struct Flow {
    /// Cargo package name of the contract crate.
    pub package: &'static str,
    /// The measured contract entrypoint.
    pub entrypoint: &'static str,
    /// Stable fixture/scenario name stored alongside the baseline.
    pub scenario: &'static str,
    /// Builds the fixture, invokes the entrypoint once, and returns the
    /// measured resources.
    pub run: fn() -> ResourceSample,
}

/// All registered benchmark flows. The gate requires every contract package
/// in [`crate::gate::CONTRACT_PACKAGES`] to appear here at least once.
pub const FLOWS: &[Flow] = &[
    Flow {
        package: "soroban-forge-escrow",
        entrypoint: "release",
        scenario: "create_deposit_release",
        run: escrow::run,
    },
    Flow {
        package: "soroban-forge-vesting",
        entrypoint: "claim",
        scenario: "create_schedule_claim_mid_vesting",
        run: vesting::run,
    },
    Flow {
        package: "soroban-forge-multi-sig-wallet",
        entrypoint: "execute",
        scenario: "submit_confirm_execute",
        run: multi_sig_wallet::run,
    },
    Flow {
        package: "soroban-forge-dao-governance",
        entrypoint: "execute",
        scenario: "propose_vote_execute",
        run: dao_governance::run,
    },
    Flow {
        package: "soroban-forge-subscription-payments",
        entrypoint: "charge",
        scenario: "subscribe_then_charge",
        run: subscription_payments::run,
    },
    Flow {
        package: "soroban-forge-marketplace-royalties",
        entrypoint: "settle_sale",
        scenario: "set_royalty_settle_sale",
        run: marketplace_royalties::run,
    },
];
