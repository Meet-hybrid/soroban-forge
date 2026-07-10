# Architecture — Storage Patterns

- **Persistent storage**: Long-lived state keyed by `u64` or `Symbol`.
- **Temporary storage**: Transient state (e.g., pending transactions) keyed by `u64`.
- **Instance storage**: Contract configuration using `Instance` map.
- **TTL enforcement**: Every entry must define a TTL strategy.

Naming conventions:
- `ESCROW_<id>_DATA` for persistent entries
- `ESCROW_<id>_PARTIES` for party lists
