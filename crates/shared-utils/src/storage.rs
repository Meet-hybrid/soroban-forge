use soroban_sdk::contracttype;

/// Audit metadata attached to a persisted value.
///
/// Contracts store domain data in Soroban instance storage; wrapping it with
/// this record lets callers (and off-chain indexers) see when a value was last
/// written. The payload is stored as a `soroban_sdk::Val`; `Val` does not
/// implement `Eq`/`PartialEq`/`Debug`, so this record intentionally derives
/// only `Clone`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct StorageEntry {
    /// The stored payload, serialised as a Soroban `Val`.
    pub value: soroban_sdk::Val,
    /// Unix timestamp (seconds) at which the entry was last written.
    pub updated_at: u64,
}
