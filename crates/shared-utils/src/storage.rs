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

#[cfg(test)]
mod tests {
    use super::*;

    // Persisted XDR: a two-entry ScMap with symbol keys "updated_at" (U64)
    // and "value" (Bytes). Keep these bytes independent of the current encoder
    // so a schema change cannot silently regenerate the expected records.
    const NONEMPTY: &[u8] = b"\0\0\0\x11\0\0\0\x01\0\0\0\x02\
        \0\0\0\x0f\0\0\0\x0aupdated_at\0\0\
        \0\0\0\x05\0\0\0\x01\0\0\0\x02\
        \0\0\0\x0f\0\0\0\x05value\0\0\0\
        \0\0\0\x0d\0\0\0\x03\xff\0\x80\0";
    const EMPTY: &[u8] = b"\0\0\0\x11\0\0\0\x01\0\0\0\x02\
        \0\0\0\x0f\0\0\0\x0aupdated_at\0\0\
        \0\0\0\x05\0\0\0\0\0\0\0\0\
        \0\0\0\x0f\0\0\0\x05value\0\0\0\
        \0\0\0\x0d\0\0\0\0";

    #[test]
    fn storage_entry_persisted_xdr_compatibility() {
        let env = Env::default();
        for (fixture, payload, timestamp) in [
            (NONEMPTY, &b"\xff\0\x80"[..], 0x1_0000_0002),
            (EMPTY, &b""[..], 0),
        ] {
            let encoded = Bytes::from_slice(&env, fixture);
            let entry = StorageEntry::from_xdr(&env, &encoded).unwrap();
            let expected_value = Bytes::from_slice(&env, payload);
            assert_eq!(entry.value, expected_value);
            assert_eq!(entry.updated_at, timestamp);

            let restored = xdr_round_trip(&env, entry).unwrap();
            assert_eq!(restored.value, expected_value);
            assert_eq!(restored.updated_at, timestamp);
            assert_eq!(restored.to_xdr(&env), encoded);
        }
    }
}
