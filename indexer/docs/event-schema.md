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

## Checkpoints and recovery

The TypeScript SDK exposes a versioned JSON checkpoint for consumers that persist
their own indexed events. A checkpoint stores the exact network identity, the
sorted contract ID set, and the last indexed cursor (`ledger`, `eventIndex`,
and `pagingToken`). Pass the same identity and the saved checkpoint when
restarting:

```ts
const identity = {
  network: "Test SDF Network ; September 2015",
  contractIds: ["<CONTRACT_ID>"],
};
const indexer = createIndexer({ ...identity, checkpoint: savedCheckpointJson });
const resumeAfter = indexer.cursor();
```

Save progress using `serializeIndexerCheckpoint(indexer.checkpoint()!)`. The
checkpoint is JSON and can be loaded with `parseIndexerCheckpoint(json,
identity)` before startup. Checkpoint loading fails closed if the JSON or
cursor is malformed, the network or contract set differs, or the schema
version is unsupported. The error identifies the mismatch and instructs the
operator to restore a compatible checkpoint or deliberately discard it.

On an incompatibility, do not edit the checkpoint to force startup. To recover,
first decide whether existing indexed events can be retained; then deliberately
remove the checkpoint and rebuild from the required starting ledger, or restore
the matching network/contract configuration and a compatible checkpoint.
Rebuilding from an earlier ledger is safe because normalized event IDs are
deduplicated. Persist the returned checkpoint only after the corresponding
events have been durably stored.

## Fixture mode

The project keeps canonical fixture snapshots under `indexer/fixtures/*.json`. The TypeScript indexer can load them directly without a live RPC connection:

```ts
import { loadFixtureEvents, createIndexer } from "@soroban-forge/escrow-client";

const events = loadFixtureEvents("escrow");
const indexer = createIndexer({ events });
const filtered = indexer.filter({ fromLedger: 1000, toLedger: 1005 });
```

The output of fixture mode is intentionally stable so any event-schema drift fails loudly during CI and local checks.
