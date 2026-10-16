//! The resource & budget regression gate.
//!
//! Runs every benchmark flow in [`resource_budget::flows`], compares the
//! measured instruction/memory/footprint/fee resources against the versioned
//! baseline in `budgets/resource-baseline.json`, and fails when any metric
//! regresses beyond the documented thresholds (see `docs/resource-budgets.md`).
//!
//! To intentionally refresh the baseline, run
//! `scripts/update-resource-baseline.sh` (or set `UPDATE_RESOURCE_BASELINE=1`
//! when running this test); CI never sets that variable.

use resource_budget::gate;

#[test]
fn contract_entrypoint_resources_within_baseline() {
    let baseline = gate::load_baseline().expect("versioned baseline must load");
    let report = gate::run_gate(&baseline);

    let markdown = report.to_markdown();
    println!("{markdown}");
    match gate::write_report(&markdown) {
        Ok(path) => println!("resource budget report written to {}", path.display()),
        Err(e) => eprintln!("warning: could not write resource budget report: {e}"),
    }

    if std::env::var(gate::UPDATE_ENV_VAR).is_ok() {
        let updated = gate::baseline_from_report(&report, &baseline);
        let path = gate::write_baseline(&updated).expect("baseline must be writable");
        println!(
            "regenerated baseline from fresh measurements: {}",
            path.display()
        );
    }

    assert!(report.passed(), "\n{}", report.failure_summary());
}

/// The gate's own plumbing: labels, status roll-up, and the failure summary
/// that names package + entrypoint — without running any contract flow.
#[test]
fn gate_plumbing_reports_package_and_entrypoint() {
    use resource_budget::{
        evaluate, FlowCheck, MetricCheck, MetricStatus, ResourceSample, Thresholds, BASELINE_SCHEMA,
    };

    let baseline = resource_budget::Baseline {
        schema: BASELINE_SCHEMA.to_string(),
        note: String::new(),
        thresholds: Thresholds {
            max_regression_percent: 10,
            min_absolute_delta: ResourceSample::default(),
        },
        flows: vec![],
    };
    // An empty flow list exercises the coverage checks without paying for
    // contract fixtures: no stale entries (baseline is empty), but every
    // contract package must be reported as missing a flow.
    let report = evaluate(&[], &baseline);
    assert!(!report.passed());
    assert_eq!(
        report.packages_without_flow,
        resource_budget::gate::CONTRACT_PACKAGES.to_vec()
    );
    assert!(report
        .failure_summary()
        .contains("has no representative benchmark flow"));

    // A synthetic regressed flow must be reported with its package and
    // entrypoint.
    let check = FlowCheck {
        package: "soroban-forge-escrow",
        entrypoint: "release",
        scenario: "synthetic",
        sample: ResourceSample::default(),
        metrics: vec![MetricCheck {
            metric: "instructions",
            baseline: 100,
            measured: 1_000,
            allowed: 110,
            status: MetricStatus::Regression,
        }],
    };
    let mut synthetic = report;
    synthetic.flows.push(check);
    let summary = synthetic.failure_summary();
    assert!(summary.contains("soroban-forge-escrow :: release (synthetic)"));
    assert!(summary.contains("`instructions`"));
}
