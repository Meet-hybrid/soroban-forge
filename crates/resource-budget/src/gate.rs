//! The regression gate: run every benchmark flow, compare against the
//! versioned baseline, and produce a report that identifies the affected
//! package and entrypoint.

use std::path::PathBuf;

use serde::Serialize;

use crate::baseline::{Baseline, FlowBaseline, BASELINE_RELPATH, BASELINE_SCHEMA};
use crate::flows::{self, Flow};
use crate::metrics::ResourceSample;

/// Environment variable that switches the gate test from "compare against the
/// baseline" to "rewrite the baseline from the fresh measurements".
///
/// This is the only supported way to change baselines and is intended for
/// maintainers (see `docs/resource-budgets.md` and
/// `scripts/update-resource-baseline.sh`). CI never sets it.
pub const UPDATE_ENV_VAR: &str = "UPDATE_RESOURCE_BASELINE";

/// Every contract package that must be covered by at least one benchmark
/// flow. Keep in sync with the contract crates in the workspace.
pub const CONTRACT_PACKAGES: [&str; 6] = [
    "soroban-forge-escrow",
    "soroban-forge-vesting",
    "soroban-forge-multi-sig-wallet",
    "soroban-forge-dao-governance",
    "soroban-forge-subscription-payments",
    "soroban-forge-marketplace-royalties",
];

/// Workspace root, derived from this crate's manifest directory.
#[must_use]
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Absolute path of the versioned baseline file.
#[must_use]
pub fn baseline_path() -> PathBuf {
    workspace_root().join(BASELINE_RELPATH)
}

/// Absolute path of the Markdown report written on every gate run.
#[must_use]
pub fn report_path() -> PathBuf {
    workspace_root()
        .join("target")
        .join("resource-budget-report.md")
}

/// Load and validate the versioned baseline.
pub fn load_baseline() -> Result<Baseline, String> {
    let path = baseline_path();
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("failed to read baseline {}: {e}", path.display()))?;
    let baseline: Baseline = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse baseline {}: {e}", path.display()))?;
    if baseline.schema != BASELINE_SCHEMA {
        return Err(format!(
            "baseline {} has schema `{}`, expected `{}`",
            path.display(),
            baseline.schema,
            BASELINE_SCHEMA
        ));
    }
    Ok(baseline)
}

/// Serialize a baseline to the on-disk JSON representation.
pub fn render_baseline(baseline: &Baseline) -> Result<String, String> {
    let mut json = serde_json::to_string_pretty(baseline)
        .map_err(|e| format!("failed to serialize baseline: {e}"))?;
    json.push('\n');
    Ok(json)
}

/// Write the baseline file (only called via the maintainer update path).
pub fn write_baseline(baseline: &Baseline) -> Result<PathBuf, String> {
    let path = baseline_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    std::fs::write(&path, render_baseline(baseline)?)
        .map_err(|e| format!("failed to write baseline {}: {e}", path.display()))?;
    Ok(path)
}

/// Write the Markdown report next to the build artifacts.
pub fn write_report(markdown: &str) -> Result<PathBuf, String> {
    let path = report_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    std::fs::write(&path, markdown)
        .map_err(|e| format!("failed to write report {}: {e}", path.display()))?;
    Ok(path)
}

/// Verdict for a single metric of a single flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricStatus {
    /// Measured within the allowed budget.
    WithinBudget,
    /// Measured below the stored baseline (an improvement; never fails).
    Improved,
    /// Measured beyond the documented thresholds — the gate must fail.
    Regression,
}

/// One metric's comparison against the baseline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetricCheck {
    /// Metric name (field name of [`ResourceSample`]).
    pub metric: &'static str,
    /// Stored baseline value.
    pub baseline: u64,
    /// Freshly measured value.
    pub measured: u64,
    /// Largest value allowed without failing.
    pub allowed: u64,
    /// Verdict.
    pub status: MetricStatus,
}

/// The full comparison of one benchmark flow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowCheck {
    /// Cargo package name of the contract crate.
    pub package: &'static str,
    /// Measured entrypoint.
    pub entrypoint: &'static str,
    /// Scenario name of the fixture around the entrypoint.
    pub scenario: &'static str,
    /// Freshly measured resources.
    pub sample: ResourceSample,
    /// Per-metric comparison; empty when the flow has no baseline entry.
    pub metrics: Vec<MetricCheck>,
}

impl FlowCheck {
    /// Human-readable `package :: entrypoint (scenario)` label used in report
    /// rows and failure messages.
    #[must_use]
    pub fn label(&self) -> String {
        format!(
            "{} :: {} ({})",
            self.package, self.entrypoint, self.scenario
        )
    }

    /// Whether any metric regressed beyond the documented thresholds.
    #[must_use]
    pub fn has_regression(&self) -> bool {
        self.metrics
            .iter()
            .any(|m| m.status == MetricStatus::Regression)
    }

    /// The worst regression, as `(metric, percent over baseline)`; `None` when
    /// the flow is within budget. Percentages are `None` when the baseline is
    /// zero.
    #[must_use]
    pub fn worst_regression(&self) -> Option<(&'static str, Option<u64>)> {
        self.metrics
            .iter()
            .filter(|m| m.status == MetricStatus::Regression)
            .max_by_key(|m| m.measured.saturating_sub(m.allowed))
            .map(|m| {
                // A regression implies `measured > allowed >= baseline`, so the
                // subtraction cannot underflow. `checked_div` also yields
                // `None` for a zero baseline — reported as a new cost.
                let percent = (m.measured - m.baseline)
                    .checked_mul(100)
                    .and_then(|over| over.checked_div(m.baseline));
                (m.metric, percent)
            })
    }
}

/// The result of running every flow against a baseline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateReport {
    /// One entry per benchmark flow, in registry order.
    pub flows: Vec<FlowCheck>,
    /// Flows that are missing from the baseline (`package`, `entrypoint`,
    /// `scenario`).
    pub missing_baseline: Vec<(String, String, String)>,
    /// Baseline entries without a matching flow (stale budgets).
    pub stale_baseline: Vec<(String, String, String)>,
    /// Contract packages without any benchmark flow.
    pub packages_without_flow: Vec<String>,
}

impl GateReport {
    /// The gate passes only when every flow is covered by the baseline, every
    /// baseline entry is still exercised, every contract package has at least
    /// one flow, and no metric regressed beyond the thresholds.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.missing_baseline.is_empty()
            && self.stale_baseline.is_empty()
            && self.packages_without_flow.is_empty()
            && self.flows.iter().all(|f| !f.has_regression())
    }

    /// Renders the Markdown report (summary table, regression details,
    /// coverage errors, and the fresh measurements as JSON).
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# Soroban Forge resource budget report\n\n");
        out.push_str(&format!(
            "Flows measured: {}. Gate: **{}**.\n\n",
            self.flows.len(),
            if self.passed() { "PASS" } else { "FAIL" }
        ));

        out.push_str("## Flow summary\n\n");
        out.push_str("| status | package | entrypoint | scenario | worst regression |\n");
        out.push_str("| --- | --- | --- | --- | --- |\n");
        for flow in &self.flows {
            let (status, worst) = if flow.has_regression() {
                ("REGRESSION", flow.worst_regression())
            } else if flow.metrics.is_empty() {
                ("NO BASELINE", None)
            } else {
                ("ok", None)
            };
            let worst_text = match worst {
                Some((metric, Some(percent))) => format!("+{percent}% on `{metric}`"),
                Some((metric, None)) => format!("`{metric}` (new cost, baseline 0)"),
                None => String::new(),
            };
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                status, flow.package, flow.entrypoint, flow.scenario, worst_text
            ));
        }
        out.push('\n');

        out.push_str("## Regressions\n\n");
        let any_regression = self.flows.iter().any(FlowCheck::has_regression);
        if any_regression {
            out.push_str(
                "| package | entrypoint | scenario | metric | baseline | measured | allowed |\n",
            );
            out.push_str("| --- | --- | --- | --- | --- | --- | --- |\n");
            for flow in &self.flows {
                for metric in &flow.metrics {
                    if metric.status == MetricStatus::Regression {
                        out.push_str(&format!(
                            "| {} | {} | {} | {} | {} | {} | {} |\n",
                            flow.package,
                            flow.entrypoint,
                            flow.scenario,
                            metric.metric,
                            metric.baseline,
                            metric.measured,
                            metric.allowed
                        ));
                    }
                }
            }
        } else {
            out.push_str("None.\n");
        }
        out.push('\n');

        if !self.missing_baseline.is_empty()
            || !self.stale_baseline.is_empty()
            || !self.packages_without_flow.is_empty()
        {
            out.push_str("## Coverage errors\n\n");
            for (package, entrypoint, scenario) in &self.missing_baseline {
                out.push_str(&format!(
                    "- flow `{package} :: {entrypoint} ({scenario})` has no baseline entry; run `scripts/update-resource-baseline.sh`\n"
                ));
            }
            for (package, entrypoint, scenario) in &self.stale_baseline {
                out.push_str(&format!(
                    "- baseline entry `{package} :: {entrypoint} ({scenario})` has no matching flow; run `scripts/update-resource-baseline.sh`\n"
                ));
            }
            for package in &self.packages_without_flow {
                out.push_str(&format!(
                    "- contract package `{package}` has no representative benchmark flow\n"
                ));
            }
            out.push('\n');
        }

        out.push_str("## Fresh measurements\n\n");
        out.push_str("```json\n");
        out.push_str(&samples_json(&self.flows));
        out.push_str("```\n");
        out
    }

    /// A failure message that names every affected package and entrypoint.
    #[must_use]
    pub fn failure_summary(&self) -> String {
        let mut out = String::from("resource budget gate failed:\n");
        for flow in &self.flows {
            for metric in &flow.metrics {
                if metric.status == MetricStatus::Regression {
                    out.push_str(&format!(
                        "- REGRESSION in {} metric `{}`: measured {} > allowed {} (baseline {})\n",
                        flow.label(),
                        metric.metric,
                        metric.measured,
                        metric.allowed,
                        metric.baseline
                    ));
                }
            }
        }
        for (package, entrypoint, scenario) in &self.missing_baseline {
            out.push_str(&format!(
                "- flow `{package} :: {entrypoint} ({scenario})` is missing from the baseline; run scripts/update-resource-baseline.sh\n"
            ));
        }
        for (package, entrypoint, scenario) in &self.stale_baseline {
            out.push_str(&format!(
                "- baseline entry `{package} :: {entrypoint} ({scenario})` no longer has a matching flow; run scripts/update-resource-baseline.sh\n"
            ));
        }
        for package in &self.packages_without_flow {
            out.push_str(&format!(
                "- contract package `{package}` has no representative benchmark flow\n"
            ));
        }
        out.push_str("See docs/resource-budgets.md for the baseline update process.\n");
        out
    }
}

/// Run every registered benchmark flow against `baseline`.
#[must_use]
pub fn run_gate(baseline: &Baseline) -> GateReport {
    evaluate(flows::FLOWS, baseline)
}

/// Run `flows_to_run` against `baseline` (parameterized for tests).
#[must_use]
pub fn evaluate(flows_to_run: &[Flow], baseline: &Baseline) -> GateReport {
    let mut report = GateReport {
        flows: Vec::new(),
        missing_baseline: Vec::new(),
        stale_baseline: Vec::new(),
        packages_without_flow: Vec::new(),
    };

    for flow in flows_to_run {
        let sample = (flow.run)();
        match baseline.find_flow(flow.package, flow.entrypoint, flow.scenario) {
            None => {
                report.missing_baseline.push((
                    flow.package.to_string(),
                    flow.entrypoint.to_string(),
                    flow.scenario.to_string(),
                ));
                report.flows.push(FlowCheck {
                    package: flow.package,
                    entrypoint: flow.entrypoint,
                    scenario: flow.scenario,
                    sample,
                    metrics: Vec::new(),
                });
            }
            Some(entry) => {
                report.flows.push(compare(flow, sample, entry, baseline));
            }
        }
    }

    for entry in &baseline.flows {
        let matched = flows_to_run.iter().any(|f| {
            f.package == entry.package
                && f.entrypoint == entry.entrypoint
                && f.scenario == entry.scenario
        });
        if !matched {
            report.stale_baseline.push((
                entry.package.clone(),
                entry.entrypoint.clone(),
                entry.scenario.clone(),
            ));
        }
    }

    for package in CONTRACT_PACKAGES {
        if !flows_to_run.iter().any(|f| f.package == package) {
            report.packages_without_flow.push(package.to_string());
        }
    }

    report
}

/// Rebuild the baseline's flow list from freshly measured samples, keeping
/// schema, note, and thresholds (the maintainer update path).
#[must_use]
pub fn baseline_from_report(report: &GateReport, previous: &Baseline) -> Baseline {
    Baseline {
        schema: previous.schema.clone(),
        note: previous.note.clone(),
        thresholds: previous.thresholds,
        flows: report
            .flows
            .iter()
            .map(|f| FlowBaseline {
                package: f.package.to_string(),
                entrypoint: f.entrypoint.to_string(),
                scenario: f.scenario.to_string(),
                measured: f.sample,
            })
            .collect(),
    }
}

fn compare(
    flow: &Flow,
    sample: ResourceSample,
    entry: &FlowBaseline,
    baseline: &Baseline,
) -> FlowCheck {
    let thresholds = baseline.thresholds;
    let sample_entries = sample.entries();
    let baseline_entries = entry.measured.entries();
    let floor_entries = thresholds.min_absolute_delta.entries();
    let mut metrics = Vec::with_capacity(sample_entries.len());
    for (((metric, measured), (_, stored)), (_, floor)) in sample_entries
        .into_iter()
        .zip(baseline_entries)
        .zip(floor_entries)
    {
        let allowed = thresholds.allowed(stored, floor);
        let status = if measured > allowed {
            MetricStatus::Regression
        } else if measured < stored {
            MetricStatus::Improved
        } else {
            MetricStatus::WithinBudget
        };
        metrics.push(MetricCheck {
            metric,
            baseline: stored,
            measured,
            allowed,
            status,
        });
    }
    FlowCheck {
        package: flow.package,
        entrypoint: flow.entrypoint,
        scenario: flow.scenario,
        sample,
        metrics,
    }
}

fn samples_json(flows: &[FlowCheck]) -> String {
    #[derive(Serialize)]
    struct SampleRow<'a> {
        package: &'a str,
        entrypoint: &'a str,
        scenario: &'a str,
        measured: ResourceSample,
    }
    let rows: Vec<SampleRow<'_>> = flows
        .iter()
        .map(|f| SampleRow {
            package: f.package,
            entrypoint: f.entrypoint,
            scenario: f.scenario,
            measured: f.sample,
        })
        .collect();
    let mut json = serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string());
    json.push('\n');
    json
}
