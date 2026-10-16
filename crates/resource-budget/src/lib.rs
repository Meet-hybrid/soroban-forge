//! Resource and budget regression gates for Soroban Forge contract builds.
//!
//! CI enforces a WASM size budget but historically had no way to notice when a
//! contract entrypoint starts consuming substantially more execution
//! resources. This crate closes that gap:
//!
//! - [`flows`] defines one deterministic, network-independent benchmark flow
//!   per contract crate. Each flow sets up a fixed fixture (fixed ledger
//!   timestamp, deterministic mock accounts, Stellar Asset Contract token) and
//!   invokes exactly one representative entrypoint; the invocation's resources
//!   are then read back through `Env::cost_estimate()`.
//! - [`baseline`] stores the versioned baseline budgets in
//!   `budgets/resource-baseline.json` together with the regression thresholds.
//! - [`gate`] runs every flow, compares the measured [`ResourceSample`]s
//!   against the baseline, renders a Markdown report identifying the affected
//!   package and entrypoint, and fails when a regression exceeds the
//!   documented thresholds.
//!
//! The gate runs as part of `cargo test --workspace --all-targets --locked`
//! (see `tests/resource_gate.rs`) and as its own CI job
//! (`.github/workflows/ci.yml`, "Resource Budget Gate").
//!
//! Maintainers update the baseline intentionally via
//! `scripts/update-resource-baseline.sh`; see `docs/resource-budgets.md`.

pub mod baseline;
pub mod flows;
pub mod gate;
pub mod metrics;

pub use baseline::{Baseline, FlowBaseline, Thresholds, BASELINE_SCHEMA};
pub use gate::{evaluate, run_gate, FlowCheck, GateReport, MetricCheck, MetricStatus};
pub use metrics::ResourceSample;
