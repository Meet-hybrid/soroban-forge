# Soroban Forge event schema

This repository keeps a read-only event consumer in the TypeScript SDK. The indexer operates on the raw Soroban RPC `contract` event shape and normalizes it into a deterministic, replayable record stream.

## Record layout

Each emitted event is normalized to a record with these fields:

- `type`: the event stream type (typically `contract`)
- `contractId`: the emitting contract ID, as seen in the event record
- `ledger`: the ledger number of the event
- `ledgerClosedAt`: the ledger-close timestamp string
- `transactionHash`: the transaction hash that emitted the event
- `operationIndex`: the operation position inside the transaction
- `eventIndex`: the per-ledger event counter used for stable ordering
- `id`: deterministic id used for deduplication (`ledger-eventIndex` when present)
- `pagingToken`: raw paging token for replay and cursor handling
- `inSuccessfulContractCall`: whether the emitting call succeeded
- `topic`: raw base64-encoded topic values
- `value`: raw base64-encoded event payload
- `decodedTopic`: decoded topic payloads
- `decodedValue`: decoded payload value
- `eventType`: the meaningful event name, derived from the first topic symbol
- `entityId`: optional entity identifier when a topic or payload exposes one

## Filter semantics

The event consumer supports filtering by:

- `contractId`
- `eventType`
- `txHash`
- `entityId`
- `fromLedger` / `toLedger`
- `after` cursor (`ledger` and `eventIndex`)

The filter logic is deterministic and sorts normalized events before filtering so replays remain idempotent.

## Fixture mode

The project keeps canonical fixture snapshots under `indexer/fixtures/*.json`. The TypeScript indexer can load them directly without a live RPC connection:

```ts
import { loadFixtureEvents, createIndexer } from "@soroban-forge/escrow-client";

const events = loadFixtureEvents("escrow");
const indexer = createIndexer({ events });
const filtered = indexer.filter({ fromLedger: 1000, toLedger: 1005 });
```

The output of fixture mode is intentionally stable so any event-schema drift fails loudly during CI and local checks.

## Persisted replay checkpoints

When restarting or resuming an event indexer, persisted checkpoints (`soroban-forge/indexer-checkpoint@1`) record compatibility metadata alongside the replay cursor so a restart never silently resumes across incompatible environments:

```json
{
  "schemaVersion": "soroban-forge/indexer-checkpoint@1",
  "network": "testnet",
  "contracts": [
    "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAHK3M"
  ],
  "cursor": {
    "ledger": 1004,
    "eventIndex": 0,
    "pagingToken": "1004-0"
  },
  "updatedAt": "2026-01-01T00:05:00.000Z"
}
```

### Startup validation & error reference

`validateCheckpoint`, `loadAndValidateCheckpoint`, and `resumeIndexerFromCheckpoint` fail closed with a typed `CheckpointValidationError` whenever a persisted checkpoint does not match the active `IndexerRuntimeConfig`:

| Error Code | Cause | Operator Remediation |
| --- | --- | --- |
| `MALFORMED_CHECKPOINT` | Empty, non-JSON, or missing required `schemaVersion`, `network`, or `contracts` fields. | Discard or rebuild the corrupted checkpoint file with `discardCheckpoint(path)` or `rebuildCheckpoint(path, runtime, events)`. |
| `SCHEMA_VERSION_MISMATCH` | Persisted `schemaVersion` does not match `soroban-forge/indexer-checkpoint@1`. | Run `rebuildCheckpoint(path, runtime, events)` to migrate to the current checkpoint format. |
| `NETWORK_MISMATCH` | Checkpoint `network` differs from the configured runtime network (for example `testnet` vs `futurenet`). | Point the indexer back to the original network or call `discardCheckpoint(path)` / `rebuildCheckpoint(path, runtime, events)` to re-index on the target network. |
| `CONTRACT_SET_MISMATCH` | Deduplicated, sorted `contracts` list differs from the configured contract set. | Call `rebuildCheckpoint(path, runtime, events)` so newly added or removed contracts are replayed from a clean boundary. |
| `INVALID_CURSOR` | Cursor `ledger` or `eventIndex` is negative or non-integer, or `pagingToken` is malformed. | Reset the cursor with `discardCheckpoint(path)` or `rebuildCheckpoint(path, runtime, events)`. |

