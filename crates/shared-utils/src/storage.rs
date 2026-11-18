use soroban_sdk::{
    contracttype,
    xdr::{FromXdr, ToXdr},
    Bytes, Env,
};

/// Decode a serialized Soroban record using the same XDR representation that
/// contract storage uses, and return the decoded value for assertions.
///
/// Keeping this small helper in shared-utils lets contract tests exercise a
/// legacy-record fixture and its current-schema round trip without duplicating
/// serialization plumbing in every crate.
pub fn xdr_round_trip<T>(env: &Env, value: T) -> Result<T, soroban_sdk::ConversionError>
where
    T: ToXdr + FromXdr<Error = soroban_sdk::ConversionError>,
{
    T::from_xdr(env, &value.to_xdr(env))
}

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
