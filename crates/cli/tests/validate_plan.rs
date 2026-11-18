use std::process::Command;

const VALID_PLAN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fixtures/plans/valid-create-escrow.json"
);
const INVALID_PLAN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fixtures/plans/invalid-multi-error.json"
);

#[test]
fn valid_plan_exits_zero_with_actionable_text() {
    let output = Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
        .args([
            "validate-plan",
            "--plan",
            VALID_PLAN,
            "--expected-network",
            "testnet",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Transaction plan: VALID"));
    assert!(stdout.contains("exit_code=0"));
    assert!(stdout.contains("all transaction-plan checks passed"));
}

#[test]
fn invalid_plan_exits_one_with_machine_readable_report() {
    let output = Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
        .args(["validate-plan", "--plan", INVALID_PLAN, "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        report["schema"],
        "soroban-forge/tx-plan-validation-report@1"
    );
    assert_eq!(report["exit_code"], 1);
    assert_eq!(report["valid"], false);

    let categories: Vec<&str> = report["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|error| error["category"].as_str().unwrap())
        .collect();
    for category in [
        "network_mismatch",
        "invalid_contract_id",
        "unsupported_method",
        "malformed_argument",
        "sensitive_input_rejected",
    ] {
        assert!(
            categories.contains(&category),
            "missing category: {category}"
        );
    }
    assert!(!stdout.contains("should-never-be-echoed"));
}
