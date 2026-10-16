//! Versioned baseline budgets and the regression thresholds they carry.
//!
//! The baseline lives in `budgets/resource-baseline.json` and is the only
//! source of truth the gate compares against. It is updated exclusively via
//! `scripts/update-resource-baseline.sh` (maintainer-controlled, see
//! `docs/resource-budgets.md`); CI never rewrites it.

use serde::{Deserialize, Serialize};

use crate::metrics::ResourceSample;

/// Schema identifier stamped into the baseline file. Bump when the file
/// format changes incompatibly.
pub const BASELINE_SCHEMA: &str = "soroban-forge/resource-baseline@1";

/// Versioned baseline file, relative to the workspace root.
pub const BASELINE_RELPATH: &str = "budgets/resource-baseline.json";

/// The stored baseline: schema version, maintainer note, thresholds, and one
/// entry per benchmark flow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Baseline {
    /// Must equal [`BASELINE_SCHEMA`].
    pub schema: String,
    /// Free-form maintainer note (e.g. the reason for the last intentional
    /// baseline refresh).
    #[serde(default)]
    pub note: String,
    /// Regression thresholds applied to every flow.
    pub thresholds: Thresholds,
    /// One measured budget per benchmark flow, keyed by
    /// `(package, entrypoint, scenario)`.
    pub flows: Vec<FlowBaseline>,
}

/// Regression thresholds.
///
/// A metric regresses when the measured value exceeds
/// `baseline + max(baseline * max_regression_percent / 100, min_absolute_delta)`.
/// In other words a regression must clear *both* its percentage budget and the
/// per-metric absolute floor, so tiny absolute wiggles on small numbers never
/// fail the gate while substantial execution-cost jumps always do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thresholds {
    /// Percentage budget per metric, e.g. `10` = allow up to +10 % over the
    /// baseline value.
    pub max_regression_percent: u64,
    /// Per-metric absolute floors (same field names as [`ResourceSample`]);
    /// a regression must also exceed the floor to fail.
    pub min_absolute_delta: ResourceSample,
}

/// The stored budget of one benchmark flow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowBaseline {
    /// Cargo package name of the contract crate, e.g. `soroban-forge-escrow`.
    pub package: String,
    /// The measured contract entrypoint, e.g. `release`.
    pub entrypoint: String,
    /// Stable name of the fixture/scenario around the entrypoint.
    pub scenario: String,
    /// The resources measured for the flow when the baseline was written.
    pub measured: ResourceSample,
}

impl Baseline {
    /// Find the stored budget for a `(package, entrypoint, scenario)` key.
    #[must_use]
    pub fn find_flow(
        &self,
        package: &str,
        entrypoint: &str,
        scenario: &str,
    ) -> Option<&FlowBaseline> {
        self.flows
            .iter()
            .find(|f| f.package == package && f.entrypoint == entrypoint && f.scenario == scenario)
    }
}

impl Thresholds {
    /// The largest value one metric may reach without failing the gate, given
    /// its baseline value and its per-metric absolute floor.
    #[must_use]
    pub fn allowed(&self, baseline_value: u64, absolute_floor: u64) -> u64 {
        let percent_budget =
            u128::from(baseline_value) * u128::from(self.max_regression_percent) / 100;
        let headroom = percent_budget.max(u128::from(absolute_floor));
        u64::try_from(u128::from(baseline_value).saturating_add(headroom)).unwrap_or(u64::MAX)
    }
}
