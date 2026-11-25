# `ttl-audit` — TTL Audit Command

The `ttl-audit` subcommand inspects the time-to-live (TTL) of a Soroban
contract's on-chain storage entries.  It calls the Soroban RPC
`getLedgerEntries` and `getLatestLedger` methods directly, with no
transactions submitted.

## Synopsis

```text
soroban-forge ttl-audit --contract <CONTRACT_ID> [OPTIONS]
```

## Options

| Option | Default | Description |
|--------|---------|-------------|
| `--contract <ID>` | *(required)* | Stellar contract address (strkey, starts with `C`). |
| `--network <PROFILE\|URL>` | `testnet` | Network profile (`testnet`, `mainnet`, `futurenet`) or a full Soroban RPC URL (e.g. `http://localhost:8000`). |
| `--keys <FILE>` | *(none)* | JSON key-spec file listing additional persistent `LedgerKey` entries to audit (see below). |
| `--threshold <LEDGERS>` | `120960` | Minimum remaining ledgers before an entry is flagged (≈ 1 week at 5 s/ledger). |
| `--format table\|json` | `table` | Output format. |
| `--plan <FILE>` | *(none)* | Write an extend-TTL plan (version-1 plan format) to this path. |
| `--source <ACCOUNT>` | `__source__` | Source account written into the generated plan file. Has no effect without `--plan`. |

## Network profiles

| Profile | RPC URL |
|---------|---------|
| `testnet` | `https://soroban-testnet.stellar.org` |
| `mainnet` | `https://mainnet.stellar.validationcloud.io/v1/stellar/rpc` |
| `futurenet` | `https://rpc-futurenet.stellar.org` |

Any string that does not match a known profile is treated as a literal URL.

## Key-spec file format

The optional `--keys` file is a JSON array of objects, one per additional
ledger key to audit:

```json
[
  {
    "xdr": "<base64-encoded LedgerKey XDR>",
    "label": "EscrowRecord"
  },
  {
    "xdr": "<base64-encoded LedgerKey XDR>"
  }
]
```

`"label"` is optional — when omitted, the first 24 characters of the XDR
string are used as the display name.

The contract instance key (`ContractDataDurability::Persistent`,
`ScVal::LedgerKeyContractInstance`) is always audited regardless of whether
`--keys` is supplied.

## Table output (default)

```
Contract : CABC...XYZ
Network  : testnet
Ledger   : 1234567
Threshold: 120960

KEY               DURABILITY   LIVE_UNTIL      REMAINING_LEDGERS  THRESHOLD  STATUS
----------------  -----------  --------------  -----------------  ---------  --------
ContractInstance  persistent   1355527         120960             OK         healthy
EscrowRecord      persistent   1241234         6667               ⚠ BELOW    expiring

Summary: 1 healthy, 1 expiring/archived, 0 missing
```

## JSON output (`--format json`)

Fields are always in the same order and entries are sorted by `key` for
deterministic diffs.

```json
{
  "contract": "CABC...XYZ",
  "network": "testnet",
  "current_ledger": 1234567,
  "threshold": 120960,
  "entries": [
    {
      "key": "ContractInstance",
      "durability": "persistent",
      "live_until_ledger": 1355527,
      "remaining_ledgers": 120960,
      "below_threshold": false,
      "status": "healthy"
    },
    {
      "key": "EscrowRecord",
      "durability": "persistent",
      "live_until_ledger": 1241234,
      "remaining_ledgers": 6667,
      "below_threshold": true,
      "status": "expiring"
    }
  ],
  "below_threshold_count": 1,
  "healthy_count": 1,
  "missing_count": 0,
  "archived_count": 0
}
```

## Entry status values

| Status | Meaning |
|--------|---------|
| `healthy` | Remaining ledgers ≥ threshold. |
| `expiring` | Entry exists but remaining ledgers < threshold. |
| `archived` | Entry exists on-chain but has no live-until ledger (past the persistent TTL window). |
| `missing` | Entry is not present on-chain at the queried ledger. |

Missing entries do **not** count toward the below-threshold count (they may
simply not have been created yet).  Archived entries **do** count.

## Extend-TTL plan generation (`--plan`)

When `--plan <FILE>` is supplied, the command writes a version-1 plan file
(compatible with `soroban-forge plan`) containing one `extend_ttl` call per
entry that is below the threshold.

```bash
soroban-forge ttl-audit \
  --contract CABC...XYZ \
  --network testnet \
  --threshold 100000 \
  --plan extend.json \
  --source alice
```

The generated `extend.json` can then be reviewed and executed with:

```bash
soroban-forge plan extend.json --dry-run   # inspect
soroban-forge plan extend.json --force     # execute
```

## Exit codes

| Code | Meaning |
|------|---------|
| `0` | All entries are at or above the threshold (or only missing). |
| `1` | One or more entries are below the threshold or archived. |
| `2` | Usage error (invalid contract address, bad key-spec file) or RPC error (timeout, connection refused). |

## Examples

Audit the contract instance on testnet (table output):

```bash
soroban-forge ttl-audit \
  --contract CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB
```

Audit with additional keys, JSON output, custom threshold:

```bash
soroban-forge ttl-audit \
  --contract CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB \
  --network mainnet \
  --keys my-keys.json \
  --threshold 50000 \
  --format json
```

Audit and generate an extend-TTL plan:

```bash
soroban-forge ttl-audit \
  --contract CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB \
  --plan extend.json \
  --source my-account
```

Use the exit code in a CI script:

```bash
soroban-forge ttl-audit --contract "$CONTRACT_ID" --threshold 50000
case $? in
  0) echo "All entries healthy." ;;
  1) echo "⚠ Entries need TTL extension." ;;
  2) echo "RPC or usage error." ; exit 1 ;;
esac
```
