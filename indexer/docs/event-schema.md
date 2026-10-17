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
