//! Resource metrics captured for one measured top-level invocation.

use serde::{Deserialize, Serialize};
use soroban_sdk::Env;

/// Resource usage of a single top-level contract invocation, as metered by the
/// Soroban test host.
///
/// All fields are widened to `u64` so that every metric can be compared
/// against the baseline generically. The values come from the three surfaces
/// Stellar tooling exposes for the *last* top-level invocation in a test
/// [`Env`]:
///
/// - `Env::cost_estimate().budget()` — consumed CPU instructions and memory
///   bytes (the budget the network enforces per transaction).
/// - `Env::cost_estimate().resources()` — modelled transaction resources:
///   footprint (in-memory / disk reads, writes, entry counts and sizes), event
///   bytes, and rent bumps.
/// - `Env::cost_estimate().fee()` — the mainnet fee snapshot total, a
///   transaction-resource proxy that is convenient to eyeball in reports.
///
/// Caveats (documented in `docs/resource-budgets.md`): native test-host
/// metering underestimates costs that only exist for on-chain Wasm execution
/// (VM instantiation, Wasm reads, rent bumps of the code entry). Values are
/// nonetheless deterministic across runs on the pinned toolchain, which is
/// what a regression gate needs; absolute numbers are not mainnet claims.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceSample {
    /// Consumed CPU instructions (budget metering).
    pub instructions: u64,
    /// Consumed memory bytes (budget metering).
    pub mem_bytes: u64,
    /// In-memory ledger entries read by the invocation.
    pub memory_read_entries: u64,
    /// Entries that had to be read from disk (restored entries).
    pub disk_read_entries: u64,
    /// Ledger entries written by the invocation.
    pub write_entries: u64,
    /// Bytes read from disk.
    pub disk_read_bytes: u64,
    /// Bytes written to the ledger.
    pub write_bytes: u64,
    /// Total size of emitted contract events.
    pub contract_events_size_bytes: u64,
    /// Persistent entries whose rent was bumped.
    pub persistent_entry_rent_bumps: u64,
    /// Temporary entries whose rent was bumped.
    pub temporary_entry_rent_bumps: u64,
    /// Cumulative persistent rent bump in ledger-bytes.
    pub persistent_rent_ledger_bytes: u64,
    /// Cumulative temporary rent bump in ledger-bytes.
    pub temporary_rent_ledger_bytes: u64,
    /// Estimated total transaction fee (stroops, mainnet fee snapshot).
    pub fee_total_stroops: u64,
}

impl ResourceSample {
    /// Every metric as `(name, value)` pairs, in a stable order used by the
    /// comparison and report code.
    #[must_use]
    pub fn entries(&self) -> [(&'static str, u64); 13] {
        [
            ("instructions", self.instructions),
            ("mem_bytes", self.mem_bytes),
            ("memory_read_entries", self.memory_read_entries),
            ("disk_read_entries", self.disk_read_entries),
            ("write_entries", self.write_entries),
            ("disk_read_bytes", self.disk_read_bytes),
            ("write_bytes", self.write_bytes),
            (
                "contract_events_size_bytes",
                self.contract_events_size_bytes,
            ),
            (
                "persistent_entry_rent_bumps",
                self.persistent_entry_rent_bumps,
            ),
            (
                "temporary_entry_rent_bumps",
                self.temporary_entry_rent_bumps,
            ),
            (
                "persistent_rent_ledger_bytes",
                self.persistent_rent_ledger_bytes,
            ),
            (
                "temporary_rent_ledger_bytes",
                self.temporary_rent_ledger_bytes,
            ),
            ("fee_total_stroops", self.fee_total_stroops),
        ]
    }

    /// Read the resources metered during the last top-level invocation in
    /// `env`.
    ///
    /// # Panics
    ///
    /// Panics when no invocation has happened yet, or when the invocation cost
    /// metering is disabled in `env` (it is always enabled for the `Env`s the
    /// benchmark flows build).
    #[must_use]
    pub fn capture(env: &Env) -> Self {
        let estimate = env.cost_estimate();
        let resources = estimate.resources();
        let fee = estimate.fee();
        let budget = estimate.budget();
        Self {
            instructions: budget.cpu_instruction_cost(),
            mem_bytes: budget.memory_bytes_cost(),
            memory_read_entries: u64::from(resources.memory_read_entries),
            disk_read_entries: u64::from(resources.disk_read_entries),
            write_entries: u64::from(resources.write_entries),
            disk_read_bytes: u64::from(resources.disk_read_bytes),
            write_bytes: u64::from(resources.write_bytes),
            contract_events_size_bytes: u64::from(resources.contract_events_size_bytes),
            persistent_entry_rent_bumps: u64::from(resources.persistent_entry_rent_bumps),
            temporary_entry_rent_bumps: u64::from(resources.temporary_entry_rent_bumps),
            persistent_rent_ledger_bytes: nonnegative(resources.persistent_rent_ledger_bytes),
            temporary_rent_ledger_bytes: nonnegative(resources.temporary_rent_ledger_bytes),
            fee_total_stroops: nonnegative(fee.total),
        }
    }
}

/// Widen a signed host value to `u64`, clamping negatives (which only occur
/// for sub-invocation accounting and cannot happen for top-level resources).
fn nonnegative(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}
