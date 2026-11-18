use std::process::{Command, Output};

const VALID_PLAN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fixtures/plans/valid-create-escrow.json"
);
const INVALID_PLAN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fixtures/plans/invalid-multi-error.json"
);

fn run_validate_plan(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
        .args(args)
        .output()
        .expect("soroban-forge binary must run")
}

#[test]
fn valid_plan_exits_zero_with_actionable_text() {
    let output = run_validate_plan(&["validate-plan", "--plan", VALID_PLAN]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(0));

    let text = String::from_utf8(output.stdout).expect("output must be UTF-8");
    assert!(text.contains("Transaction Plan Validation: VALID (exit_code=0)"));
    assert!(text.contains("Network : testnet"));
    assert!(text.contains("Method  : create_escrow"));
    assert!(text.contains("all transaction-plan checks passed"));
}

#[test]
fn invalid_plan_exits_one_with_machine_readable_report() {
    let output = run_validate_plan(&[
        "validate-plan",
        "--plan",
        INVALID_PLAN,
        "--expected-network",
        "futurenet",
        "--json",
    ]);

    assert_eq!(output.status.code(), Some(1));

    let text = String::from_utf8(output.stdout).expect("output must be UTF-8");
    let report: serde_json::Value = serde_json::from_str(&text).expect("report must be JSON");

    assert_eq!(
        report["schema"],
        "soroban-forge/tx-plan-validation-report@1"
    );
    assert_eq!(report["valid"], false);
    assert_eq!(report["exit_code"], 1);
    assert_eq!(
        report["error_count"],
        report["errors"].as_array().unwrap().len()
    );

    let categories: Vec<&str> = report["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|issue| issue["category"].as_str().unwrap())
        .collect();
    for category in [
        "network_mismatch",
        "invalid_contract_id",
        "unsupported_method",
        "malformed_argument",
    ] {
        assert!(categories.contains(&category), "missing {category}");
    }
}
