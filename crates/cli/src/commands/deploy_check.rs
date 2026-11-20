use crate::cli::DeployCheckArgs;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

const SUMMARY_SCHEMA: &str = "soroban-forge/deployment-smoke@1";
const MANIFEST_SCHEMA_PREFIX: &str = "soroban-forge/testnet-deployment@";
const UNCONFIGURED_CONTRACT_ID: &str = "configured-at-runtime";

/// Deployment manifest emitted by `scripts/deploy-testnet.sh`.
///
/// Only the fields the smoke check needs are required; unknown fields are
/// ignored so the receipt can keep growing without breaking this command.
#[derive(Debug, Deserialize)]
struct DeploymentManifest {
    #[serde(default)]
    schema: Option<String>,
    #[serde(default)]
    network: Option<String>,
    contracts: Vec<ManifestContract>,
}

#[derive(Debug, Deserialize)]
struct ManifestContract {
    key: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    contract_id: String,
    /// Explicit read-only invocation configured in the manifest.
    #[serde(default)]
    read_only: Option<ReadOnlyInvocation>,
}

/// A credential-free, read-only method call declared in the manifest.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct ReadOnlyInvocation {
    method: String,
    /// Raw CLI arguments passed after the method name, for example
    /// `["--escrow_id", "1"]`.
    #[serde(default)]
    args: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CheckStatus {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Serialize)]
struct ContractReport {
    key: String,
    name: String,
    contract_id: String,
    code: CheckStatus,
    read_only: CheckStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct CheckSummary {
    schema: &'static str,
    network: String,
    contracts: Vec<ContractReport>,
    passed: usize,
    failed: usize,
}

/// Executes the two deployment probes for one contract.
///
/// Production talks to the Stellar CLI; tests inject a fake so the check
/// logic runs without a network or a real Stellar installation.
trait DeploymentProbe {
    fn check_code(&self, contract_id: &str) -> Result<()>;
    fn check_read_only(&self, contract_id: &str, invocation: &ReadOnlyInvocation) -> Result<()>;
}

struct StellarCliProbe {
    network: String,
    source: String,
}

impl StellarCliProbe {
    fn run(&self, mut command: Command, action: &str) -> Result<()> {
        let output = command
            .output()
            .with_context(|| format!("failed to run {action}"))?;
        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        if detail.is_empty() {
            bail!("{action} failed: stellar exited with a non-zero status");
        }
        bail!("{action} failed: {detail}");
    }
}

impl DeploymentProbe for StellarCliProbe {
    fn check_code(&self, contract_id: &str) -> Result<()> {
        let temp_dir = tempfile::Builder::new()
            .prefix("soroban-forge-deploy-check-")
            .tempdir()
            .context("failed to create temporary directory for contract code fetch")?;
        let out_file = temp_dir.path().join("contract.wasm");

        let mut command = Command::new("stellar");
        command
            .arg("contract")
            .arg("fetch")
            .arg("--id")
            .arg(contract_id)
            .arg("--network")
            .arg(&self.network)
            .arg("--out-file")
            .arg(&out_file);
        self.run(command, "contract code fetch")
    }

    fn check_read_only(&self, contract_id: &str, invocation: &ReadOnlyInvocation) -> Result<()> {
        let mut command = Command::new("stellar");
        command
            .arg("contract")
            .arg("invoke")
            .arg("--id")
            .arg(contract_id)
            .arg("--network")
            .arg(&self.network);
        if !self.source.trim().is_empty() {
            // Simulation only: never submit or sign a transaction.
            command.arg("--source-account").arg(&self.source);
        }
        command
            .arg("--send")
            .arg("no")
            .arg("--")
            .arg(&invocation.method)
            .args(&invocation.args);
        self.run(command, "read-only invocation")
    }
}

/// Default credential-free probes for manifests written before the
/// `read_only` field existed. Contracts without a safe no-argument read (and
/// unknown contract keys) must configure `read_only` explicitly.
fn default_read_only(key: &str) -> Option<ReadOnlyInvocation> {
    let method = match key {
        "multi-sig-wallet" => "get_threshold",
        "dao-governance" => "get_bond_config",
        "subscription-payments" => "get_subscription_count",
        _ => return None,
    };
    Some(ReadOnlyInvocation {
        method: method.to_owned(),
        args: Vec::new(),
    })
}

pub fn run(args: DeployCheckArgs) -> Result<()> {
    let manifest = parse_manifest(&args.manifest)?;
    let network = args
        .network
        .or_else(|| manifest.network.clone())
        .unwrap_or_else(|| "testnet".to_owned());
    let probe = StellarCliProbe {
        network: network.clone(),
        source: args.source,
    };

    let summary = check_manifest(&manifest, &probe, &args.contract, &network)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        print!("{}", render_text(&summary));
    }

    ensure_all_passed(&summary)
}

fn parse_manifest(path: &Path) -> Result<DeploymentManifest> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read deployment manifest {}", path.display()))?;
    parse_manifest_str(&raw)
        .with_context(|| format!("failed to parse deployment manifest {}", path.display()))
}

fn parse_manifest_str(raw: &str) -> Result<DeploymentManifest> {
    let manifest: DeploymentManifest = serde_json::from_str(raw)?;
    if let Some(schema) = &manifest.schema {
        if !schema.starts_with(MANIFEST_SCHEMA_PREFIX) {
            bail!("unsupported deployment manifest schema `{schema}`");
        }
    }
    if manifest.contracts.is_empty() {
        bail!("deployment manifest contains no contracts");
    }
    Ok(manifest)
}

fn check_manifest(
    manifest: &DeploymentManifest,
    probe: &impl DeploymentProbe,
    selected: &[String],
    network: &str,
) -> Result<CheckSummary> {
    let filter: BTreeSet<&str> = selected.iter().map(String::as_str).collect();
    if !filter.is_empty() {
        let known: BTreeSet<&str> = manifest
            .contracts
            .iter()
            .map(|contract| contract.key.as_str())
            .collect();
        let unknown: Vec<&str> = filter.difference(&known).copied().collect();
        if !unknown.is_empty() {
            bail!(
                "unknown contract key(s) in --contract: {}",
                unknown.join(", ")
            );
        }
    }

    let mut contracts: Vec<ContractReport> = manifest
        .contracts
        .iter()
        .filter(|contract| filter.is_empty() || filter.contains(contract.key.as_str()))
        .map(|contract| check_contract(contract, probe))
        .collect();

    // Stable output for CI logs regardless of manifest ordering.
    contracts.sort_by(|left, right| left.key.cmp(&right.key));

    let passed = contracts
        .iter()
        .filter(|contract| {
            contract.code == CheckStatus::Passed && contract.read_only == CheckStatus::Passed
        })
        .count();
    let failed = contracts.len() - passed;

    Ok(CheckSummary {
        schema: SUMMARY_SCHEMA,
        network: network.to_owned(),
        contracts,
        passed,
        failed,
    })
}

fn check_contract(contract: &ManifestContract, probe: &impl DeploymentProbe) -> ContractReport {
    let name = contract
        .name
        .clone()
        .unwrap_or_else(|| contract.key.clone());
    let mut report = ContractReport {
        key: contract.key.clone(),
        name,
        contract_id: contract.contract_id.clone(),
        code: CheckStatus::Skipped,
        read_only: CheckStatus::Skipped,
        method: None,
        error: None,
    };

    if contract.contract_id.trim().is_empty() || contract.contract_id == UNCONFIGURED_CONTRACT_ID {
        report.error = Some("contract id is not configured in the manifest".to_owned());
        return report;
    }

    let invocation = contract
        .read_only
        .clone()
        .or_else(|| default_read_only(&contract.key));
    let Some(invocation) = invocation else {
        report.error = Some("no read-only invocation configured in the manifest".to_owned());
        return report;
    };
    if invocation.method.trim().is_empty() {
        report.error = Some("read-only invocation has an empty method".to_owned());
        return report;
    }
    if invocation
        .args
        .iter()
        .any(|arg| arg == UNCONFIGURED_CONTRACT_ID)
    {
        report.error =
            Some("read-only invocation argument is not configured in the manifest".to_owned());
        return report;
    }
    report.method = Some(invocation.method.clone());

    if let Err(error) = probe.check_code(&contract.contract_id) {
        report.code = CheckStatus::Failed;
        report.error = Some(format!("code presence: {error:#}"));
        return report;
    }
    report.code = CheckStatus::Passed;

    if let Err(error) = probe.check_read_only(&contract.contract_id, &invocation) {
        report.read_only = CheckStatus::Failed;
        report.error = Some(format!("read-only `{}`: {error:#}", invocation.method));
        return report;
    }
    report.read_only = CheckStatus::Passed;
    report
}

fn render_text(summary: &CheckSummary) -> String {
    use std::fmt::Write as _;

    let mut output = String::new();
    let _ = writeln!(
        output,
        "deployment smoke check: network={} contracts={} passed={} failed={}",
        summary.network,
        summary.contracts.len(),
        summary.passed,
        summary.failed
    );
    for contract in &summary.contracts {
        let status =
            if contract.code == CheckStatus::Passed && contract.read_only == CheckStatus::Passed {
                "PASS"
            } else {
                "FAIL"
            };
        let method = contract.method.as_deref().unwrap_or("-");
        let _ = write!(
            output,
            "{status} {} ({}) code={} read={}",
            contract.key,
            contract.contract_id,
            status_name(contract.code),
            method
        );
        if contract.read_only != CheckStatus::Skipped {
            let _ = write!(output, "={}", status_name(contract.read_only));
        }
        if let Some(error) = &contract.error {
            let _ = write!(output, " error={error}");
        }
        output.push('\n');
    }
    output
}

fn status_name(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Passed => "passed",
        CheckStatus::Failed => "failed",
        CheckStatus::Skipped => "skipped",
    }
}

fn ensure_all_passed(summary: &CheckSummary) -> Result<()> {
    if summary.failed == 0 {
        return Ok(());
    }
    let failed: Vec<String> = summary
        .contracts
        .iter()
        .filter(|contract| {
            contract.code != CheckStatus::Passed || contract.read_only != CheckStatus::Passed
        })
        .map(|contract| format!("{} ({})", contract.key, contract.contract_id))
        .collect();
    bail!(
        "deployment smoke check failed for {} contract(s): {}",
        summary.failed,
        failed.join(", ")
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeProbe {
        fail_code: Vec<String>,
        fail_read: Vec<String>,
        calls: RefCell<Vec<String>>,
    }

    impl DeploymentProbe for FakeProbe {
        fn check_code(&self, contract_id: &str) -> Result<()> {
            self.calls.borrow_mut().push(format!("code:{contract_id}"));
            if self.fail_code.iter().any(|id| id == contract_id) {
                bail!("contract not found");
            }
            Ok(())
        }

        fn check_read_only(
            &self,
            contract_id: &str,
            invocation: &ReadOnlyInvocation,
        ) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("read:{contract_id}:{}", invocation.method));
            if self.fail_read.iter().any(|id| id == contract_id) {
                bail!("method not found");
            }
            Ok(())
        }
    }

    fn fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/deploy")
            .join(name);
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn all_success_manifest_passes_both_probes_deterministically() {
        let manifest = parse_manifest_str(&fixture("all-success.json")).unwrap();
        let probe = FakeProbe::default();
        let summary = check_manifest(&manifest, &probe, &[], "testnet").unwrap();

        assert_eq!(summary.contracts.len(), 2);
        assert_eq!(summary.passed, 2);
        assert_eq!(summary.failed, 0);
        assert!(summary
            .contracts
            .iter()
            .all(|contract| contract.code == CheckStatus::Passed
                && contract.read_only == CheckStatus::Passed));

        let first = render_text(&summary);
        let second = render_text(&summary);
        assert_eq!(first, second);
        assert!(first
            .starts_with("deployment smoke check: network=testnet contracts=2 passed=2 failed=0"));
        assert!(first.contains("PASS escrow"));
        assert!(first.contains("PASS multi-sig-wallet"));
        assert!(ensure_all_passed(&summary).is_ok());
    }

    #[test]
    fn partial_failure_manifest_identifies_each_failed_contract() {
        let manifest = parse_manifest_str(&fixture("partial-failure.json")).unwrap();
        let probe = FakeProbe {
            fail_code: vec!["CCODE_MISSING".to_owned()],
            fail_read: vec!["CREAD_MISSING".to_owned()],
            ..FakeProbe::default()
        };
        let summary = check_manifest(&manifest, &probe, &[], "testnet").unwrap();

        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 2);

        let text = render_text(&summary);
        assert!(text.contains("FAIL escrow"));
        assert!(text.contains("code presence: contract not found"));
        assert!(text.contains("FAIL vesting"));
        assert!(text.contains("read-only `get_schedule`"));
        assert!(text.contains("PASS multi-sig-wallet"));

        let error = ensure_all_passed(&summary).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("escrow (CCODE_MISSING)"));
        assert!(message.contains("vesting (CREAD_MISSING)"));
    }

    #[test]
    fn missing_read_only_configuration_fails_without_probing() {
        let manifest = parse_manifest_str(
            r#"{
                "schema": "soroban-forge/testnet-deployment@1",
                "network": "testnet",
                "contracts": [
                    {"key": "custom", "contract_id": "CCUSTOM"}
                ]
            }"#,
        )
        .unwrap();
        let probe = FakeProbe::default();
        let summary = check_manifest(&manifest, &probe, &[], "testnet").unwrap();

        assert_eq!(summary.failed, 1);
        assert!(probe.calls.borrow().is_empty());
        assert_eq!(
            summary.contracts[0].error.as_deref(),
            Some("no read-only invocation configured in the manifest")
        );
    }

    #[test]
    fn known_contracts_fall_back_to_default_read_only_checks() {
        let manifest = parse_manifest_str(
            r#"{
                "contracts": [
                    {"key": "multi-sig-wallet", "contract_id": "CWALLET"}
                ]
            }"#,
        )
        .unwrap();
        let probe = FakeProbe::default();
        let summary = check_manifest(&manifest, &probe, &[], "testnet").unwrap();

        assert_eq!(summary.passed, 1);
        assert_eq!(
            summary.contracts[0].method.as_deref(),
            Some("get_threshold")
        );
        assert_eq!(
            probe.calls.borrow().as_slice(),
            [
                "code:CWALLET".to_owned(),
                "read:CWALLET:get_threshold".to_owned()
            ]
        );
    }

    #[test]
    fn unconfigured_contract_id_fails_without_probing() {
        let manifest = parse_manifest_str(
            r#"{
                "contracts": [
                    {
                        "key": "escrow",
                        "contract_id": "configured-at-runtime",
                        "read_only": {"method": "get_escrow", "args": ["--escrow_id", "1"]}
                    }
                ]
            }"#,
        )
        .unwrap();
        let probe = FakeProbe::default();
        let summary = check_manifest(&manifest, &probe, &[], "testnet").unwrap();

        assert_eq!(summary.failed, 1);
        assert!(probe.calls.borrow().is_empty());
        assert_eq!(
            summary.contracts[0].error.as_deref(),
            Some("contract id is not configured in the manifest")
        );
    }

    #[test]
    fn unconfigured_read_only_argument_fails_without_probing() {
        let manifest = parse_manifest_str(
            r#"{
                "contracts": [
                    {
                        "key": "marketplace-royalties",
                        "contract_id": "CMARKETPLACE",
                        "read_only": {
                            "method": "get_royalty",
                            "args": ["--collection", "configured-at-runtime"]
                        }
                    }
                ]
            }"#,
        )
        .unwrap();
        let probe = FakeProbe::default();
        let summary = check_manifest(&manifest, &probe, &[], "testnet").unwrap();

        assert_eq!(summary.failed, 1);
        assert!(probe.calls.borrow().is_empty());
        assert_eq!(
            summary.contracts[0].error.as_deref(),
            Some("read-only invocation argument is not configured in the manifest")
        );
    }

    #[test]
    fn subscription_payments_falls_back_to_get_subscription_count() {
        let manifest = parse_manifest_str(
            r#"{
                "contracts": [
                    {"key": "subscription-payments", "contract_id": "CSUBSCRIPTION"}
                ]
            }"#,
        )
        .unwrap();
        let probe = FakeProbe::default();
        let summary = check_manifest(&manifest, &probe, &[], "testnet").unwrap();

        assert_eq!(summary.passed, 1);
        assert_eq!(
            summary.contracts[0].method.as_deref(),
            Some("get_subscription_count")
        );
        assert_eq!(
            probe.calls.borrow().as_slice(),
            [
                "code:CSUBSCRIPTION".to_owned(),
                "read:CSUBSCRIPTION:get_subscription_count".to_owned()
            ]
        );
    }

    #[test]
    fn dry_run_manifest_fails_smoke_check_with_unconfigured_placeholders() {
        let manifest = parse_manifest_str(
            r#"{
                "schema": "soroban-forge/testnet-deployment@1",
                "status": "dry_run",
                "network": "testnet",
                "contracts": [
                    {
                        "key": "marketplace-royalties",
                        "contract_id": "configured-at-runtime",
                        "read_only": {
                            "method": "get_royalty",
                            "args": ["--collection", "configured-at-runtime"]
                        }
                    }
                ]
            }"#,
        )
        .unwrap();
        let probe = FakeProbe::default();
        let summary = check_manifest(&manifest, &probe, &[], "testnet").unwrap();

        assert_eq!(summary.passed, 0);
        assert_eq!(summary.failed, 1);
        assert!(probe.calls.borrow().is_empty());
        assert_eq!(
            summary.contracts[0].error.as_deref(),
            Some("contract id is not configured in the manifest")
        );
        assert!(ensure_all_passed(&summary).is_err());
    }

    #[test]
    fn manifest_preservation_retains_contract_level_metadata() {
        let raw = r#"{
            "schema": "soroban-forge/testnet-deployment@1",
            "status": "passed",
            "network": "testnet",
            "contracts": [
                {
                    "key": "subscription-payments",
                    "package": "soroban-forge-subscription-payments",
                    "name": "SubscriptionPayments",
                    "contract_id": "CSUBSCRIPTION",
                    "verification_status": "verified",
                    "read_only": {
                        "method": "get_subscription_count",
                        "args": []
                    },
                    "smoke": {
                        "status": "passed",
                        "transaction": "tx123",
                        "events": ["subscribed"],
                        "state": "subscription active"
                    }
                }
            ]
        }"#;
        let manifest = parse_manifest_str(raw).unwrap();
        let probe = FakeProbe::default();
        let summary = check_manifest(&manifest, &probe, &[], "testnet").unwrap();

        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 0);
        assert_eq!(
            summary.contracts[0].method.as_deref(),
            Some("get_subscription_count")
        );
        assert!(ensure_all_passed(&summary).is_ok());
    }

    #[test]
    fn contract_filter_limits_the_checked_entries() {
        let manifest = parse_manifest_str(&fixture("all-success.json")).unwrap();
        let probe = FakeProbe::default();
        let summary = check_manifest(
            &manifest,
            &probe,
            &["multi-sig-wallet".to_owned()],
            "testnet",
        )
        .unwrap();

        assert_eq!(summary.contracts.len(), 1);
        assert_eq!(summary.contracts[0].key, "multi-sig-wallet");
    }

    #[test]
    fn unknown_contract_filter_is_rejected() {
        let manifest = parse_manifest_str(&fixture("all-success.json")).unwrap();
        let probe = FakeProbe::default();
        let error =
            check_manifest(&manifest, &probe, &["unknown".to_owned()], "testnet").unwrap_err();
        assert!(format!("{error:#}").contains("unknown contract key(s)"));
    }

    #[test]
    fn invalid_or_empty_manifests_are_rejected() {
        assert!(parse_manifest_str("not json").is_err());
        assert!(parse_manifest_str(r#"{"contracts": []}"#).is_err());
        assert!(
            parse_manifest_str(r#"{"schema": "other/schema", "contracts": [{"key": "x"}]}"#)
                .is_err()
        );
    }

    #[test]
    fn json_summary_is_machine_readable_and_deterministic() {
        let manifest = parse_manifest_str(&fixture("all-success.json")).unwrap();
        let probe = FakeProbe::default();
        let summary = check_manifest(&manifest, &probe, &[], "testnet").unwrap();
        let first = serde_json::to_string_pretty(&summary).unwrap();
        let second = serde_json::to_string_pretty(&summary).unwrap();

        assert_eq!(first, second);
        assert!(first.contains("\"schema\": \"soroban-forge/deployment-smoke@1\""));
        assert!(first.contains("\"passed\": 2"));
    }
}
