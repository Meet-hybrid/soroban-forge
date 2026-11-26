//! TTL audit command for Soroban contract storage entries.
//!
//! Calls the Soroban RPC `getLedgerEntries` and `getLatestLedger` methods to
//! inspect how many ledgers remain on the contract instance entry and any
//! additional persistent keys supplied through a JSON key-spec file.  Entries
//! below the threshold are flagged, and an extend-TTL plan compatible with the
//! existing `plan` command is optionally written to disk.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use stellar_strkey::Contract;
use stellar_xdr::curr::{
    ContractDataDurability, Hash, LedgerKey, LedgerKeyContractData, Limits, ScAddress, ScVal,
    WriteXdr,
};

use crate::cli::TtlAuditArgs;
use crate::commands::plan::Plan;

// ── Network profiles ─────────────────────────────────────────────────────────

/// Return the Soroban RPC URL for a named network profile.
///
/// Users can pass any HTTP or HTTPS URL directly as `--network`.  The
/// well-known profile names (`testnet`, `mainnet`, `futurenet`) are expanded
/// here; everything else is passed through unchanged as a URL.
pub fn rpc_url_for_network(network: &str) -> String {
    match network {
        "testnet" => "https://soroban-testnet.stellar.org".to_owned(),
        "mainnet" => "https://mainnet.stellar.validationcloud.io/v1/stellar/rpc".to_owned(),
        "futurenet" => "https://rpc-futurenet.stellar.org".to_owned(),
        other => other.to_owned(),
    }
}

// ── Key-spec file ─────────────────────────────────────────────────────────────

/// One entry in the JSON key-spec file supplied through `--keys`.
///
/// ```json
/// [
///   { "xdr": "<base64-encoded LedgerKey XDR>" },
///   { "xdr": "..." }
/// ]
/// ```
#[derive(Debug, Deserialize)]
pub struct KeySpec {
    /// Base64-encoded XDR of a `LedgerKey`.
    pub xdr: String,
    /// Human-readable label shown in reports.  Defaults to the XDR string.
    #[serde(default)]
    pub label: Option<String>,
}

fn load_key_specs(path: &Path) -> Result<Vec<KeySpec>> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read key-spec file at {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse key-spec file at {}", path.display()))
}

// ── XDR helpers ───────────────────────────────────────────────────────────────

fn contract_instance_key_xdr(contract_id: &str) -> Result<String> {
    let key_bytes = Contract::from_string(contract_id)
        .with_context(|| format!("invalid contract address: {contract_id}"))?
        .0;
    let hash = Hash(key_bytes);
    let key = LedgerKey::ContractData(LedgerKeyContractData {
        contract: ScAddress::Contract(hash),
        key: ScVal::LedgerKeyContractInstance,
        durability: ContractDataDurability::Persistent,
    });
    key.to_xdr_base64(Limits::none())
        .context("failed to encode contract instance LedgerKey to XDR")
}

// ── RPC client ────────────────────────────────────────────────────────────────

/// Abstraction over the two RPC calls we need.  Extracted as a trait so tests
/// can substitute a mock without touching the network.
pub trait RpcClient {
    fn get_latest_ledger(&self) -> Result<u32>;
    fn get_ledger_entries(&self, keys: &[String]) -> Result<Vec<LedgerEntryResult>>;
}

/// Result for one ledger entry as returned by `getLedgerEntries`.
#[derive(Debug, Clone)]
pub struct LedgerEntryResult {
    /// The base64-encoded LedgerKey XDR that was queried.
    pub key_xdr: String,
    /// Status of this particular entry.
    pub status: EntryStatus,
}

#[derive(Debug, Clone)]
pub enum EntryStatus {
    /// The entry exists and the TTL was returned.
    Found { live_until_ledger: u32 },
    /// The entry has been archived (past the persistent TTL limit).
    Archived,
    /// The entry does not exist on-chain.
    Missing,
}

// ── HTTP RPC client ───────────────────────────────────────────────────────────

/// Minimal Soroban RPC client over HTTP/HTTPS using `ureq`.
pub struct HttpRpcClient {
    url: String,
}

impl HttpRpcClient {
    pub fn new(url: String) -> Self {
        Self { url }
    }

    fn call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });

        let response = ureq::post(&self.url)
            .set("Content-Type", "application/json")
            .send_string(&serde_json::to_string(&body)?)
            .map_err(|e| anyhow::anyhow!("RPC request to {} failed: {}", self.url, e))?;

        let text = response
            .into_string()
            .context("failed to read RPC response body")?;

        let json: serde_json::Value =
            serde_json::from_str(&text).context("RPC response is not valid JSON")?;

        if let Some(err) = json.get("error") {
            bail!("RPC error: {err}");
        }

        json.get("result")
            .cloned()
            .context("RPC response missing 'result' field")
    }
}

impl RpcClient for HttpRpcClient {
    fn get_latest_ledger(&self) -> Result<u32> {
        let result = self.call("getLatestLedger", serde_json::json!({}))?;
        result
            .get("sequence")
            .and_then(serde_json::Value::as_u64)
            .map(|n| n as u32)
            .context("getLatestLedger response missing 'sequence' field")
    }

    fn get_ledger_entries(&self, keys: &[String]) -> Result<Vec<LedgerEntryResult>> {
        let result = self.call("getLedgerEntries", serde_json::json!({ "keys": keys }))?;

        // The RPC returns `{ entries: [...], latestLedger: N }`.
        // If a key is not found, it is simply absent from `entries`.
        let found_entries: Vec<RpcLedgerEntry> = result
            .get("entries")
            .and_then(serde_json::Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value::<RpcLedgerEntry>(v.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();

        // Build a map from key XDR → entry so we can join against the
        // requested key list.
        let mut by_key: BTreeMap<String, RpcLedgerEntry> = BTreeMap::new();
        for entry in found_entries {
            by_key.insert(entry.key.clone(), entry);
        }

        let mut out = Vec::with_capacity(keys.len());
        for key_xdr in keys {
            let status = if let Some(entry) = by_key.get(key_xdr.as_str()) {
                match entry.live_until_ledger_seq {
                    Some(lu) => EntryStatus::Found {
                        live_until_ledger: lu,
                    },
                    // Entry returned but no TTL — treat as archived.
                    None => EntryStatus::Archived,
                }
            } else {
                EntryStatus::Missing
            };
            out.push(LedgerEntryResult {
                key_xdr: key_xdr.clone(),
                status,
            });
        }
        Ok(out)
    }
}

#[derive(Debug, Deserialize)]
struct RpcLedgerEntry {
    key: String,
    #[serde(rename = "liveUntilLedgerSeq")]
    live_until_ledger_seq: Option<u32>,
}

// ── Audit logic ───────────────────────────────────────────────────────────────

/// A single audited entry in the report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditEntry {
    /// Human-readable key label.
    pub key: String,
    /// `persistent` or `temporary` (always `persistent` for the instance key).
    pub durability: String,
    /// Ledger at which the entry expires, or `null` for missing/archived.
    pub live_until_ledger: Option<u32>,
    /// Ledgers remaining until expiry, or `null`.
    pub remaining_ledgers: Option<i64>,
    /// Whether this entry is below the threshold.
    pub below_threshold: bool,
    /// `healthy`, `expiring`, `archived`, or `missing`.
    pub status: String,
}

impl AuditEntry {
    fn new(
        key: String,
        durability: String,
        entry_status: &EntryStatus,
        current_ledger: u32,
        threshold: u32,
    ) -> Self {
        match entry_status {
            EntryStatus::Found { live_until_ledger } => {
                let live_until = *live_until_ledger;
                let remaining = i64::from(live_until).saturating_sub(i64::from(current_ledger));
                let below = remaining < i64::from(threshold);
                let status = if below {
                    "expiring".to_owned()
                } else {
                    "healthy".to_owned()
                };
                AuditEntry {
                    key,
                    durability,
                    live_until_ledger: Some(live_until),
                    remaining_ledgers: Some(remaining),
                    below_threshold: below,
                    status,
                }
            }
            EntryStatus::Archived => AuditEntry {
                key,
                durability,
                live_until_ledger: None,
                remaining_ledgers: None,
                below_threshold: true,
                status: "archived".to_owned(),
            },
            EntryStatus::Missing => AuditEntry {
                key,
                durability,
                live_until_ledger: None,
                remaining_ledgers: None,
                below_threshold: false,
                status: "missing".to_owned(),
            },
        }
    }
}

/// Full audit report returned by `run_audit`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditReport {
    pub contract: String,
    pub network: String,
    pub current_ledger: u32,
    pub threshold: u32,
    /// Sorted by `key` for deterministic output.
    pub entries: Vec<AuditEntry>,
    pub below_threshold_count: usize,
    pub healthy_count: usize,
    pub missing_count: usize,
    pub archived_count: usize,
}

impl AuditReport {
    fn build(
        contract: String,
        network: String,
        current_ledger: u32,
        threshold: u32,
        mut entries: Vec<AuditEntry>,
    ) -> Self {
        // Deterministic order: sort by key label.
        entries.sort_by(|a, b| a.key.cmp(&b.key));

        let below_threshold_count = entries.iter().filter(|e| e.below_threshold).count();
        let healthy_count = entries.iter().filter(|e| e.status == "healthy").count();
        let missing_count = entries.iter().filter(|e| e.status == "missing").count();
        let archived_count = entries.iter().filter(|e| e.status == "archived").count();

        AuditReport {
            contract,
            network,
            current_ledger,
            threshold,
            entries,
            below_threshold_count,
            healthy_count,
            missing_count,
            archived_count,
        }
    }
}

/// Exit codes returned by `run`.
///
/// - `0` – nothing below threshold
/// - `1` – one or more entries are below threshold (or archived)
/// - `2` – usage or RPC error
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditExitCode {
    Ok = 0,
    BelowThreshold = 1,
    /// Returned as `process::exit(2)` from `main` when `run` returns `Err`.
    #[allow(dead_code)]
    Error = 2,
}

/// Core audit logic, decoupled from I/O for testing.
pub fn run_audit(args: &TtlAuditArgs, rpc: &dyn RpcClient) -> Result<(AuditReport, AuditExitCode)> {
    // Build the list of LedgerKey XDRs to query.
    let instance_xdr = contract_instance_key_xdr(&args.contract)
        .map_err(|e| anyhow::anyhow!("usage error: {e}"))?;

    let mut keys_xdr: Vec<String> = vec![instance_xdr.clone()];
    // Map from XDR → human label.
    let mut labels: BTreeMap<String, String> = BTreeMap::new();
    labels.insert(instance_xdr.clone(), "ContractInstance".to_owned());

    if let Some(keys_path) = &args.keys {
        let specs = load_key_specs(keys_path).map_err(|e| anyhow::anyhow!("usage error: {e}"))?;
        for spec in specs {
            let label = spec
                .label
                .unwrap_or_else(|| spec.xdr[..spec.xdr.len().min(24)].to_owned() + "…");
            labels.insert(spec.xdr.clone(), label);
            if !keys_xdr.contains(&spec.xdr) {
                keys_xdr.push(spec.xdr);
            }
        }
    }

    let threshold = args.threshold.unwrap_or(DEFAULT_THRESHOLD);

    // Query the RPC node.
    let current_ledger = rpc
        .get_latest_ledger()
        .map_err(|e| anyhow::anyhow!("RPC error: {e}"))?;

    let entry_results = rpc
        .get_ledger_entries(&keys_xdr)
        .map_err(|e| anyhow::anyhow!("RPC error: {e}"))?;

    // Build audit entries.
    let audit_entries: Vec<AuditEntry> = entry_results
        .iter()
        .map(|r| {
            let label = labels
                .get(&r.key_xdr)
                .cloned()
                .unwrap_or_else(|| r.key_xdr.clone());
            AuditEntry::new(
                label,
                "persistent".to_owned(),
                &r.status,
                current_ledger,
                threshold,
            )
        })
        .collect();

    let report = AuditReport::build(
        args.contract.clone(),
        args.network.clone(),
        current_ledger,
        threshold,
        audit_entries,
    );

    let exit_code = if report.below_threshold_count > 0 {
        AuditExitCode::BelowThreshold
    } else {
        AuditExitCode::Ok
    };

    Ok((report, exit_code))
}

/// Default threshold in ledgers (~1 week at ~5 s/ledger).
pub const DEFAULT_THRESHOLD: u32 = 120_960;

// ── Plan generation ───────────────────────────────────────────────────────────

/// Build an extend-TTL plan for all entries that are below the threshold.
/// The plan uses the existing `plan` command format (version 1).
pub fn build_extend_plan(report: &AuditReport, source: &str, extend_to: u32) -> Plan {
    use crate::commands::plan::Call;

    let calls = report
        .entries
        .iter()
        .filter(|e| e.below_threshold && e.status != "missing")
        .map(|e| Call {
            kind: Some("invoke".to_owned()),
            contract: report.contract.clone(),
            function: "extend_ttl".to_owned(),
            args: vec![format!("key={}", e.key), format!("extend_to={extend_to}")],
            assertion: None,
        })
        .collect();

    Plan {
        version: "1".to_owned(),
        network: report.network.clone(),
        source: source.to_owned(),
        calls,
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────────

/// Render the audit report as a human-readable table.
pub fn render_table(report: &AuditReport, out: &mut impl io::Write) -> io::Result<()> {
    let key_w = report
        .entries
        .iter()
        .map(|e| e.key.len())
        .max()
        .unwrap_or(3)
        .max(3);

    writeln!(out, "Contract : {}", report.contract)?;
    writeln!(out, "Network  : {}", report.network)?;
    writeln!(out, "Ledger   : {}", report.current_ledger)?;
    writeln!(out, "Threshold: {}", report.threshold)?;
    writeln!(out)?;

    writeln!(
        out,
        "{:<key_w$}  {:<11}  {:<14}  {:<17}  {:<9}  STATUS",
        "KEY", "DURABILITY", "LIVE_UNTIL", "REMAINING_LEDGERS", "THRESHOLD"
    )?;
    writeln!(
        out,
        "{:-<key_w$}  {:-<11}  {:-<14}  {:-<17}  {:-<9}  {:-<8}",
        "", "", "", "", "", ""
    )?;

    for e in &report.entries {
        let lu = e
            .live_until_ledger
            .map_or_else(|| "—".to_owned(), |n| n.to_string());
        let rem = e
            .remaining_ledgers
            .map_or_else(|| "—".to_owned(), |n| n.to_string());
        let thr = if e.below_threshold { "⚠ BELOW" } else { "OK" };
        writeln!(
            out,
            "{:<key_w$}  {:<11}  {:<14}  {:<17}  {:<9}  {}",
            e.key, e.durability, lu, rem, thr, e.status
        )?;
    }

    writeln!(out)?;
    writeln!(
        out,
        "Summary: {} healthy, {} expiring/archived, {} missing",
        report.healthy_count, report.below_threshold_count, report.missing_count
    )?;

    Ok(())
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Main entry point called from `main.rs`.
///
/// Returns the desired process exit code.  Callers should propagate it without
/// using `process::exit` inside this function.
pub fn run(args: TtlAuditArgs) -> Result<AuditExitCode> {
    let rpc_url = rpc_url_for_network(&args.network);
    let client = HttpRpcClient::new(rpc_url);
    run_with_client(args, &client)
}

pub fn run_with_client(args: TtlAuditArgs, rpc: &dyn RpcClient) -> Result<AuditExitCode> {
    let (report, exit_code) = run_audit(&args, rpc)?;

    match args.format.as_deref().unwrap_or("table") {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        _ => {
            render_table(&report, &mut io::stdout())?;
        }
    }

    // Optionally write extend-TTL plan.
    if let Some(plan_path) = &args.plan {
        let source = args.source.as_deref().unwrap_or("__source__");
        let extend_to = report.current_ledger + report.threshold;
        let plan = build_extend_plan(&report, source, extend_to);
        let json = serde_json::to_string_pretty(&plan)?;
        fs::write(plan_path, &json)
            .with_context(|| format!("failed to write plan to {}", plan_path.display()))?;
        eprintln!("extend-TTL plan written to {}", plan_path.display());
    }

    Ok(exit_code)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    // ── Mock RPC ──────────────────────────────────────────────────────────────

    struct MockRpc {
        latest_ledger: u32,
        /// key_xdr → (live_until_ledger, exists)
        entries: HashMap<String, Option<u32>>,
        /// If set, `get_ledger_entries` returns this error.
        ledger_error: Option<String>,
        /// If set, `get_latest_ledger` returns this error.
        latest_error: Option<String>,
        call_log: Arc<Mutex<Vec<String>>>,
    }

    impl MockRpc {
        fn new(latest_ledger: u32) -> Self {
            Self {
                latest_ledger,
                entries: HashMap::new(),
                ledger_error: None,
                latest_error: None,
                call_log: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn with_entry(mut self, key: &str, live_until: Option<u32>) -> Self {
            self.entries.insert(key.to_owned(), live_until);
            self
        }

        fn with_ledger_error(mut self, msg: &str) -> Self {
            self.ledger_error = Some(msg.to_owned());
            self
        }

        fn with_latest_error(mut self, msg: &str) -> Self {
            self.latest_error = Some(msg.to_owned());
            self
        }
    }

    impl RpcClient for MockRpc {
        fn get_latest_ledger(&self) -> Result<u32> {
            self.call_log
                .lock()
                .unwrap()
                .push("get_latest_ledger".to_owned());
            if let Some(ref e) = self.latest_error {
                bail!("{e}");
            }
            Ok(self.latest_ledger)
        }

        fn get_ledger_entries(&self, keys: &[String]) -> Result<Vec<LedgerEntryResult>> {
            self.call_log
                .lock()
                .unwrap()
                .push(format!("get_ledger_entries({})", keys.len()));
            if let Some(ref e) = self.ledger_error {
                bail!("{e}");
            }
            let mut results = Vec::new();
            for key in keys {
                let status = match self.entries.get(key.as_str()) {
                    Some(Some(lu)) => EntryStatus::Found {
                        live_until_ledger: *lu,
                    },
                    Some(None) => EntryStatus::Archived,
                    None => EntryStatus::Missing,
                };
                results.push(LedgerEntryResult {
                    key_xdr: key.clone(),
                    status,
                });
            }
            Ok(results)
        }
    }

    // ── Test contract ID (valid strkey) ──────────────────────────────────────

    const CONTRACT_ID: &str = "CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB";

    fn instance_xdr() -> String {
        contract_instance_key_xdr(CONTRACT_ID).unwrap()
    }

    fn base_args() -> TtlAuditArgs {
        TtlAuditArgs {
            contract: CONTRACT_ID.to_owned(),
            network: "testnet".to_owned(),
            keys: None,
            threshold: None,
            format: None,
            plan: None,
            source: None,
        }
    }

    // ── 1. Healthy entry ─────────────────────────────────────────────────────

    #[test]
    fn healthy_entry_returns_exit_ok() {
        let current = 1_000_000_u32;
        let live_until = current + DEFAULT_THRESHOLD + 100;
        let xdr = instance_xdr();
        let rpc = MockRpc::new(current).with_entry(&xdr, Some(live_until));

        let (report, code) = run_audit(&base_args(), &rpc).unwrap();
        assert_eq!(code, AuditExitCode::Ok);
        assert_eq!(report.below_threshold_count, 0);
        assert_eq!(report.healthy_count, 1);
        let e = &report.entries[0];
        assert_eq!(e.status, "healthy");
        assert_eq!(e.live_until_ledger, Some(live_until));
    }

    // ── 2. Expiring entry (below threshold) ──────────────────────────────────

    #[test]
    fn expiring_entry_returns_below_threshold() {
        let current = 1_000_000_u32;
        let live_until = current + 100; // well below default threshold
        let xdr = instance_xdr();
        let rpc = MockRpc::new(current).with_entry(&xdr, Some(live_until));

        let (report, code) = run_audit(&base_args(), &rpc).unwrap();
        assert_eq!(code, AuditExitCode::BelowThreshold);
        assert_eq!(report.below_threshold_count, 1);
        let e = &report.entries[0];
        assert_eq!(e.status, "expiring");
        assert_eq!(e.remaining_ledgers, Some(100));
    }

    // ── 3. Missing entry ─────────────────────────────────────────────────────

    #[test]
    fn missing_entry_does_not_count_as_below_threshold() {
        let rpc = MockRpc::new(500_000);
        // No entries registered → all Missing.
        let (report, code) = run_audit(&base_args(), &rpc).unwrap();
        assert_eq!(code, AuditExitCode::Ok);
        assert_eq!(report.missing_count, 1);
        assert_eq!(report.below_threshold_count, 0);
        assert_eq!(report.entries[0].status, "missing");
    }

    // ── 4. Archived entry ────────────────────────────────────────────────────

    #[test]
    fn archived_entry_counts_as_below_threshold() {
        let xdr = instance_xdr();
        let rpc = MockRpc::new(900_000).with_entry(&xdr, None);
        // None live_until → Archived
        let (report, code) = run_audit(&base_args(), &rpc).unwrap();
        assert_eq!(code, AuditExitCode::BelowThreshold);
        assert_eq!(report.archived_count, 1);
        assert_eq!(report.entries[0].status, "archived");
    }

    // ── 5. Custom threshold ──────────────────────────────────────────────────

    #[test]
    fn custom_threshold_respected() {
        let current = 1_000_000_u32;
        let live_until = current + 500;
        let xdr = instance_xdr();
        let rpc = MockRpc::new(current).with_entry(&xdr, Some(live_until));

        let mut args = base_args();
        args.threshold = Some(1_000); // 1000 > 500 remaining → expiring
        let (_report, code) = run_audit(&args, &rpc).unwrap();
        assert_eq!(code, AuditExitCode::BelowThreshold);

        args.threshold = Some(499); // 499 < 500 remaining → healthy
        let (report2, code2) = run_audit(&args, &rpc).unwrap();
        assert_eq!(code2, AuditExitCode::Ok);
        assert_eq!(report2.entries[0].status, "healthy");
    }

    // ── 6. Key-spec file ─────────────────────────────────────────────────────

    #[test]
    fn key_spec_file_entries_audited() {
        let dir = tempdir().unwrap();
        let spec_path = dir.path().join("keys.json");

        // Create a fake XDR key (the mock doesn't decode it).
        let fake_xdr = "AAAA1234deadbeef==";
        let spec_json = serde_json::json!([
            { "xdr": fake_xdr, "label": "MyKey" }
        ]);
        fs::write(&spec_path, serde_json::to_string(&spec_json).unwrap()).unwrap();

        let current = 1_000_000_u32;
        let live_until = current + DEFAULT_THRESHOLD + 1;
        let instance = instance_xdr();
        let rpc = MockRpc::new(current)
            .with_entry(&instance, Some(live_until))
            .with_entry(fake_xdr, Some(live_until));

        let mut args = base_args();
        args.keys = Some(spec_path);
        let (report, code) = run_audit(&args, &rpc).unwrap();
        assert_eq!(code, AuditExitCode::Ok);
        assert_eq!(report.entries.len(), 2);
        // Both should be healthy.
        assert!(report.entries.iter().all(|e| e.status == "healthy"));
        // "MyKey" label should appear.
        assert!(report.entries.iter().any(|e| e.key == "MyKey"));
    }

    // ── 7. Malformed key-spec file ───────────────────────────────────────────

    #[test]
    fn malformed_key_spec_returns_usage_error() {
        let dir = tempdir().unwrap();
        let spec_path = dir.path().join("bad.json");
        fs::write(&spec_path, b"not valid json at all!!!").unwrap();

        let mut args = base_args();
        args.keys = Some(spec_path);
        let rpc = MockRpc::new(500_000);
        let err = run_audit(&args, &rpc).unwrap_err();
        assert!(err.to_string().contains("usage error"), "got: {err}");
    }

    // ── 8. RPC failure on getLedgerEntries ───────────────────────────────────

    #[test]
    fn rpc_failure_on_entries_returns_rpc_error() {
        let rpc = MockRpc::new(500_000).with_ledger_error("connection refused");
        let err = run_audit(&base_args(), &rpc).unwrap_err();
        assert!(err.to_string().contains("RPC error"), "got: {err}");
    }

    // ── 9. RPC failure on getLatestLedger ────────────────────────────────────

    #[test]
    fn rpc_failure_on_latest_ledger_returns_rpc_error() {
        let rpc = MockRpc::new(0).with_latest_error("timeout");
        let err = run_audit(&base_args(), &rpc).unwrap_err();
        assert!(err.to_string().contains("RPC error"), "got: {err}");
    }

    // ── 10. Invalid contract address ─────────────────────────────────────────

    #[test]
    fn invalid_contract_address_returns_usage_error() {
        let mut args = base_args();
        args.contract = "NOT_A_VALID_CONTRACT_ID".to_owned();
        let rpc = MockRpc::new(500_000);
        let err = run_audit(&args, &rpc).unwrap_err();
        assert!(err.to_string().contains("usage error"), "got: {err}");
    }

    // ── 11. JSON output is deterministic and sorted ──────────────────────────

    #[test]
    fn json_output_is_deterministic_and_sorted() {
        let dir = tempdir().unwrap();
        let spec_path = dir.path().join("keys.json");

        let fake_xdr_a = "AAAA_alpha==";
        let fake_xdr_b = "BBBB_beta==";
        let spec_json = serde_json::json!([
            { "xdr": fake_xdr_b, "label": "ZKeyB" },
            { "xdr": fake_xdr_a, "label": "AKeyA" },
        ]);
        fs::write(&spec_path, serde_json::to_string(&spec_json).unwrap()).unwrap();

        let current = 2_000_000_u32;
        let instance = instance_xdr();
        let rpc = MockRpc::new(current)
            .with_entry(&instance, Some(current + DEFAULT_THRESHOLD + 1))
            .with_entry(fake_xdr_a, Some(current + DEFAULT_THRESHOLD + 1))
            .with_entry(fake_xdr_b, Some(current + DEFAULT_THRESHOLD + 1));

        let mut args = base_args();
        args.keys = Some(spec_path.clone());

        let (report1, _) = run_audit(&args, &rpc).unwrap();
        let (report2, _) = run_audit(&args, &rpc).unwrap();

        let json1 = serde_json::to_string_pretty(&report1).unwrap();
        let json2 = serde_json::to_string_pretty(&report2).unwrap();
        assert_eq!(json1, json2, "JSON output must be deterministic");

        // Entries must be sorted by key.
        let keys: Vec<&str> = report1.entries.iter().map(|e| e.key.as_str()).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "entries must be sorted by key");
    }

    // ── 12. Plan file is generated and validates ─────────────────────────────

    #[test]
    fn plan_file_generated_for_expiring_entries() {
        let dir = tempdir().unwrap();
        let plan_path = dir.path().join("extend.json");

        let current = 1_000_000_u32;
        let live_until = current + 50; // expiring
        let xdr = instance_xdr();
        let rpc = MockRpc::new(current).with_entry(&xdr, Some(live_until));

        let mut args = base_args();
        args.plan = Some(plan_path.clone());
        args.source = Some("alice".to_owned());

        run_with_client(args, &rpc).unwrap();

        assert!(plan_path.exists(), "plan file should be written");
        let raw = fs::read_to_string(&plan_path).unwrap();
        let plan: crate::commands::plan::Plan = serde_json::from_str(&raw).unwrap();
        assert_eq!(plan.version, "1");
        assert_eq!(plan.network, "testnet");
        assert_eq!(plan.source, "alice");
        assert!(!plan.calls.is_empty());
    }

    // ── 13. No plan file for healthy entries ─────────────────────────────────

    #[test]
    fn plan_has_no_calls_when_all_healthy() {
        let dir = tempdir().unwrap();
        let plan_path = dir.path().join("extend.json");

        let current = 1_000_000_u32;
        let live_until = current + DEFAULT_THRESHOLD + 1;
        let xdr = instance_xdr();
        let rpc = MockRpc::new(current).with_entry(&xdr, Some(live_until));

        let mut args = base_args();
        args.plan = Some(plan_path.clone());
        args.source = Some("bob".to_owned());

        run_with_client(args, &rpc).unwrap();

        // Plan is still written but with zero calls.
        let raw = fs::read_to_string(&plan_path).unwrap();
        let plan: crate::commands::plan::Plan = serde_json::from_str(&raw).unwrap();
        assert!(plan.calls.is_empty());
    }

    // ── 14. Table render does not panic ──────────────────────────────────────

    #[test]
    fn table_render_does_not_panic() {
        let current = 1_000_000_u32;
        let live_until = current + DEFAULT_THRESHOLD + 1;
        let xdr = instance_xdr();
        let rpc = MockRpc::new(current).with_entry(&xdr, Some(live_until));

        let (report, _) = run_audit(&base_args(), &rpc).unwrap();
        let mut buf = Vec::new();
        render_table(&report, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("ContractInstance"));
        assert!(text.contains("healthy"));
    }

    // ── 15. Network profile expansion ────────────────────────────────────────

    #[test]
    fn network_profiles_resolve_correctly() {
        assert!(rpc_url_for_network("testnet").contains("soroban-testnet"));
        assert!(rpc_url_for_network("mainnet").contains("mainnet"));
        assert!(rpc_url_for_network("futurenet").contains("futurenet"));
        // Custom URL is returned verbatim.
        assert_eq!(
            rpc_url_for_network("http://localhost:8000"),
            "http://localhost:8000"
        );
    }

    // ── 16. Exit-code semantics ───────────────────────────────────────────────

    #[test]
    fn exit_code_values_match_spec() {
        assert_eq!(AuditExitCode::Ok as i32, 0);
        assert_eq!(AuditExitCode::BelowThreshold as i32, 1);
        assert_eq!(AuditExitCode::Error as i32, 2);
    }
}
