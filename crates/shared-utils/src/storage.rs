use soroban_sdk::{contracttype, Bytes};

/// Audit metadata attached to a persisted value.
///
/// Contracts store domain data in Soroban instance storage; wrapping it with
/// this record lets callers (and off-chain indexers) see when a value was last
/// written. The payload is stored as opaque serialized bytes so the record is
/// valid as a Soroban contract type without coupling it to a concrete domain
/// value.
#[contracttype]
#[derive(Clone, Debug)]
pub struct StorageEntry {
    /// The stored payload, serialized as opaque Soroban bytes.
    pub value: Bytes,
    /// Unix timestamp (seconds) at which the entry was last written.
    pub updated_at: u64,
}
