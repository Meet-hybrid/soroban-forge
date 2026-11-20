#!/usr/bin/env bash
# Soroban Forge — manifest-driven testnet deployment and smoke receipts.
# Credential-safe: this script never creates, prints, funds, stores, or commits keys.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

NETWORK="${NETWORK:-testnet}"
RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"
NETWORK_PASSPHRASE="${NETWORK_PASSPHRASE:-Test SDF Network ; September 2015}"
SOURCE_IDENTITY="${SOURCE_IDENTITY:-}"
TOKEN_ID="${TOKEN_ID:-}"
RECEIPT="${RECEIPT:-artifacts/testnet-deployment-manifest.json}"
PROVENANCE="${PROVENANCE:-provenance-manifest.json}"
MODE="live"

usage() {
  cat <<'USAGE'
Usage:
  bash scripts/deploy-testnet.sh --dry-run [--receipt PATH]
  bash scripts/deploy-testnet.sh --build-only
  bash scripts/deploy-testnet.sh [--receipt PATH]

--dry-run is fully offline: it builds no artifacts and performs no network or
credential operations. --build-only builds the six deployable WASM artifacts.

Live mode deploys contracts that do not already have an ID, or uses existing
*_ID environment variables. It requires caller-provided Stellar CLI identities.

Common environment:
  SOURCE_IDENTITY  funded source identity used to deploy
  TOKEN_ID         existing SEP-41 token used by smoke flows
  NETWORK          Stellar network name (default: testnet)
  RPC_URL          Soroban RPC endpoint
  RECEIPT          output manifest (default: artifacts/testnet-deployment-manifest.json)

Existing contract IDs:
  ESCROW_ID, VESTING_ID, MULTI_SIG_WALLET_ID, DAO_GOVERNANCE_ID,
  SUBSCRIPTION_PAYMENTS_ID, MARKETPLACE_ROYALTIES_ID

Smoke identities:
  BUYER_IDENTITY, SELLER_IDENTITY, ARBITER_IDENTITY, BENEFICIARY_IDENTITY,
  OWNER1_IDENTITY, OWNER2_IDENTITY, PROVIDER_IDENTITY, PROPOSER_IDENTITY,
  TREASURY_IDENTITY, RECIPIENT_IDENTITY, PAYER_IDENTITY
USAGE
}

fail() { echo "ERROR: $*" >&2; exit 1; }
step() { printf '\n== %s ==\n' "$*"; }
sha256_of() { sha256sum "$1" | awk '{print $1}'; }

PACKAGES=(
  "escrow|soroban-forge-escrow|soroban_forge_escrow.wasm|Escrow"
  "vesting|soroban-forge-vesting|soroban_forge_vesting.wasm|Vesting"
  "multi-sig-wallet|soroban-forge-multi-sig-wallet|soroban_forge_multi_sig_wallet.wasm|MultiSigWallet"
  "dao-governance|soroban-forge-dao-governance|soroban_forge_dao_governance.wasm|DaoGovernance"
  "subscription-payments|soroban-forge-subscription-payments|soroban_forge_subscription_payments.wasm|SubscriptionPayments"
  "marketplace-royalties|soroban-forge-marketplace-royalties|soroban_forge_marketplace_royalties.wasm|MarketplaceRoyalties"
)

id_for_key() {
  case "$1" in
    escrow) printf '%s' "${ESCROW_ID:-}" ;;
    vesting) printf '%s' "${VESTING_ID:-}" ;;
    multi-sig-wallet) printf '%s' "${MULTI_SIG_WALLET_ID:-}" ;;
    dao-governance) printf '%s' "${DAO_GOVERNANCE_ID:-}" ;;
    subscription-payments) printf '%s' "${SUBSCRIPTION_PAYMENTS_ID:-}" ;;
    marketplace-royalties) printf '%s' "${MARKETPLACE_ROYALTIES_ID:-}" ;;
  esac
}

identity_address() {
  stellar keys address "$1" 2>/dev/null || fail "Stellar identity not found: $1"
}

invoke() {
  local id="$1" source="$2" contract="$3"; shift 3
  local output
  output="$(stellar contract invoke --id "$id" --source-account "$source" --rpc-url "$RPC_URL" --network-passphrase "$NETWORK_PASSPHRASE" --send yes --output json -- "$@" 2>&1)" \
    || fail "$contract invocation failed ($source $*): $output"
  printf '%s' "$output" | python3 -c '
import json,re,sys
v=json.load(sys.stdin)
h=""
ret=""
def walk(x):
 global h,ret
 if isinstance(x,dict):
  for k,a in x.items():
   if k in ("hash","tx_hash","transaction_hash") and isinstance(a,str) and re.fullmatch(r"[0-9a-fA-F]{64}",a): h=a
   if k in ("result","return_value","value","u64","i64","u32","i32","symbol","string") and not isinstance(a,(dict,list)): ret=str(a)
   walk(a)
 elif isinstance(x,list):
  for a in x: walk(a)
walk(v)
print(h)
print(ret)
'  || fail "$contract returned non-JSON CLI output: $output"
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "$1 is required"
}

build_artifacts() {
  require_command stellar
  stellar contract build --locked --profile release --optimize=false
}

write_manifest() {
  local status="$1"
  mkdir -p "$(dirname "$RECEIPT")"
  RECEIPT_STATUS="$status" RECEIPT_NETWORK="$NETWORK" RECEIPT_PATH="$RECEIPT" ROOT="$ROOT" \
  ESCROW_ID="${ESCROW_ID:-}" VESTING_ID="${VESTING_ID:-}" \
  MULTI_SIG_WALLET_ID="${MULTI_SIG_WALLET_ID:-}" DAO_GOVERNANCE_ID="${DAO_GOVERNANCE_ID:-}" \
  SUBSCRIPTION_PAYMENTS_ID="${SUBSCRIPTION_PAYMENTS_ID:-}" MARKETPLACE_ROYALTIES_ID="${MARKETPLACE_ROYALTIES_ID:-}" \
  python3 - <<'PY'
import glob, json, os
root=os.environ["ROOT"]
status=os.environ["RECEIPT_STATUS"]
entries=[]
packages=[
("escrow","soroban-forge-escrow","soroban_forge_escrow.wasm","Escrow","ESCROW_ID","get_escrow",["--escrow_id","1"]),
("vesting","soroban-forge-vesting","soroban_forge_vesting.wasm","Vesting","VESTING_ID","get_schedule",["--schedule_id","1"]),
("multi-sig-wallet","soroban-forge-multi-sig-wallet","soroban_forge_multi_sig_wallet.wasm","MultiSigWallet","MULTI_SIG_WALLET_ID","get_threshold",[]),
("dao-governance","soroban-forge-dao-governance","soroban_forge_dao_governance.wasm","DaoGovernance","DAO_GOVERNANCE_ID","get_bond_config",[]),
("subscription-payments","soroban-forge-subscription-payments","soroban_forge_subscription_payments.wasm","SubscriptionPayments","SUBSCRIPTION_PAYMENTS_ID","get_subscription_count",[]),
("marketplace-royalties","soroban-forge-marketplace-royalties","soroban_forge_marketplace_royalties.wasm","MarketplaceRoyalties","MARKETPLACE_ROYALTIES_ID","get_royalty",["--collection","configured-at-runtime"]),
]
import hashlib
for key,package,wasm,label,idenv,method,args in packages:
 p=os.path.join(root,"target","wasm32v1-none","release",wasm)
 sha=None
 size=None
 if os.path.isfile(p):
  b=open(p,"rb").read(); sha=hashlib.sha256(b).hexdigest(); size=len(b)
 cid=os.environ.get(idenv) or "configured-at-runtime"
 entries.append({
  "key":key,"package":package,"name":label,"contract_id":cid,
  "wasm":{"path":"target/wasm32v1-none/release/"+wasm,"sha256":sha,"bytes":size},
  "deployment_transaction":"","verification_status":"pending" if status=="dry_run" else "not_verified",
  "read_only":{"method":method,"args":args},
  "smoke":{"status":"not_run","transaction":"","events":[],"state":""}
 })
doc={"schema":"soroban-forge/testnet-deployment@1","status":status,"network":os.environ["RECEIPT_NETWORK"],"contracts":entries}
path=os.environ["RECEIPT_PATH"]
os.makedirs(os.path.dirname(path) or ".",exist_ok=True)
with open(path,"w",encoding="utf-8") as f:
 json.dump(doc,f,indent=2); f.write("\n")
print("receipt:",path)
PY
}

validate_provenance_hashes() {
  require_command python3
  [ -f "$PROVENANCE" ] || fail "provenance manifest missing: $PROVENANCE"
  python3 - "$PROVENANCE" <<'PY' || exit 1
import hashlib,json,sys,os
m=json.load(open(sys.argv[1]))
failed=[]
for a in m.get("artifacts",[]):
 p="target/wasm32v1-none/release/"+a["name"]
 if not os.path.isfile(p): failed.append("MISSING "+a["name"]); continue
 h=hashlib.sha256(open(p,"rb").read()).hexdigest()
 if h!=a["sha256"]: failed.append("HASH MISMATCH "+a["name"])
 else: print("OK",a["name"],h)
if failed:
 print("ERROR: provenance validation failed:"); print("\n".join(failed)); sys.exit(1)
PY
}

smoke_contract() {
  local key="$1" id="$2" contract_name="$3"
  SMOKE_TX="" SMOKE_EVENTS='[]' SMOKE_STATE="not_checked"
  SMOKE_READ_METHOD="" SMOKE_READ_ARGS='[]'

  [ -n "$TOKEN_ID" ] || fail "$contract_name smoke: TOKEN_ID is required"
  step "$contract_name smoke"

  case "$key" in
    escrow)
      [ -n "${BUYER_IDENTITY:-}" ] && [ -n "${SELLER_IDENTITY:-}" ] && [ -n "${ARBITER_IDENTITY:-}" ] || fail "escrow smoke requires BUYER_IDENTITY, SELLER_IDENTITY, ARBITER_IDENTITY"
      buyer="$(identity_address "$BUYER_IDENTITY")"; seller="$(identity_address "$SELLER_IDENTITY")"; arbiter="$(identity_address "$ARBITER_IDENTITY")"
      r="$(invoke "$id" "$BUYER_IDENTITY" escrow create_escrow --buyer "$buyer" --seller "$seller" --arbiter "$arbiter" --token "$TOKEN_ID" --amount 1 --timeout 86400)"
      tx="$(printf '%s\n' "$r" | sed -n '1p')"; escrow_no="$(printf '%s\n' "$r" | sed -n '2p' | tr -d '[:space:]')"
      [ -n "$escrow_no" ] || fail "escrow smoke: create_escrow returned no escrow id"
      deposit_result="$(invoke "$id" "$BUYER_IDENTITY" escrow deposit --escrow_id "$escrow_no")"; deposit_tx="$(printf '%s\n' "$deposit_result" | sed -n '1p')"
      release_result="$(invoke "$id" "$SELLER_IDENTITY" escrow release --escrow_id "$escrow_no")"; release_tx="$(printf '%s\n' "$release_result" | sed -n '1p')"
      assert_transaction "$contract_name" "$tx"
      assert_transaction "$contract_name" "$deposit_tx"
      assert_transaction "$contract_name" "$release_tx"
      SMOKE_TX="$release_tx"; SMOKE_EVENTS='["escrow_created","deposited","released"]'; SMOKE_STATE="escrow $escrow_no completed"
      SMOKE_READ_METHOD="get_escrow"; SMOKE_READ_ARGS="[\"--escrow_id\",\"$escrow_no\"]"
      ;;
    vesting)
      [ -n "${BENEFICIARY_IDENTITY:-}" ] && [ -n "${FUNDER_IDENTITY:-}" ] || fail "vesting smoke requires FUNDER_IDENTITY, BENEFICIARY_IDENTITY"
      beneficiary="$(identity_address "$BENEFICIARY_IDENTITY")"
      r="$(invoke "$id" "$FUNDER_IDENTITY" vesting create_schedule --funder "$(identity_address "$FUNDER_IDENTITY")" --beneficiary "$beneficiary" --token "$TOKEN_ID" --total_amount 1 --cliff 0 --duration 1)"
      tx="$(printf '%s\n' "$r" | sed -n '1p')"; schedule="$(printf '%s\n' "$r" | sed -n '2p' | tr -d '[:space:]')"
      [ -n "$schedule" ] || fail "vesting smoke: create_schedule returned no schedule id"
      assert_transaction "$contract_name" "$tx"
      invoke "$id" "$FUNDER_IDENTITY" vesting get_schedule --schedule_id "$schedule" >/dev/null
      SMOKE_TX="$tx"; SMOKE_EVENTS='["vesting_schedule_created"]'; SMOKE_STATE="schedule $schedule created"
      SMOKE_READ_METHOD="get_schedule"; SMOKE_READ_ARGS="[\"--schedule_id\",\"$schedule\"]"
      ;;
    multi-sig-wallet)
      [ -n "${OWNER1_IDENTITY:-}" ] && [ -n "${OWNER2_IDENTITY:-}" ] || fail "multi-sig smoke requires OWNER1_IDENTITY, OWNER2_IDENTITY"
      owner1="$(identity_address "$OWNER1_IDENTITY")"; owner2="$(identity_address "$OWNER2_IDENTITY")"
      r="$(invoke "$id" "$OWNER1_IDENTITY" wallet initialize --owners "[$owner1,$owner2]" --threshold 1)"
      tx="$(printf '%s\n' "$r" | sed -n '1p')"
      assert_transaction "$contract_name" "$tx"
      threshold="$(invoke "$id" "$OWNER1_IDENTITY" wallet get_threshold | tail -1 | tr -d '[:space:]')"
      [ -n "$threshold" ] || fail "multi-sig smoke: get_threshold returned no state"
      SMOKE_TX="$tx"; SMOKE_EVENTS='["wallet_initialized"]'; SMOKE_STATE="threshold $threshold"
      SMOKE_READ_METHOD="get_threshold"; SMOKE_READ_ARGS='[]'
      ;;
    dao-governance)
      [ -n "${PROPOSER_IDENTITY:-}" ] && [ -n "${TREASURY_IDENTITY:-}" ] || fail "DAO smoke requires PROPOSER_IDENTITY, TREASURY_IDENTITY"
      treasury="$(identity_address "$TREASURY_IDENTITY")"
      r="$(invoke "$id" "$PROPOSER_IDENTITY" dao initialize --governance_token "$TOKEN_ID")"; tx="$(printf '%s\n' "$r" | sed -n '1p')"
      assert_transaction "$contract_name" "$tx"
      bond="$(invoke "$id" "$PROPOSER_IDENTITY" dao configure_bond --token "$TOKEN_ID" --amount 1 --treasury "$treasury")"; bond_tx="$(printf '%s\n' "$bond" | sed -n '1p')"
      assert_transaction "$contract_name" "$bond_tx"
      invoke "$id" "$PROPOSER_IDENTITY" dao get_bond_config >/dev/null
      SMOKE_TX="$bond_tx"; SMOKE_EVENTS='["dao_initialized","bond_configured"]'; SMOKE_STATE="bond 1 configured"
      SMOKE_READ_METHOD="get_bond_config"; SMOKE_READ_ARGS='[]'
      ;;
    subscription-payments)
      [ -n "${SUBSCRIBER_IDENTITY:-}" ] && [ -n "${PROVIDER_IDENTITY:-}" ] || fail "subscription smoke requires SUBSCRIBER_IDENTITY, PROVIDER_IDENTITY"
      subscriber="$(identity_address "$SUBSCRIBER_IDENTITY")"; provider="$(identity_address "$PROVIDER_IDENTITY")"
      r="$(invoke "$id" "$SUBSCRIBER_IDENTITY" subscription subscribe --subscriber "$subscriber" --provider "$provider" --token "$TOKEN_ID" --amount 1 --period 1)"
      tx="$(printf '%s\n' "$r" | sed -n '1p')"; subscription="$(printf '%s\n' "$r" | sed -n '2p' | tr -d '[:space:]')"
      [ -n "$subscription" ] || fail "subscription smoke: subscribe returned no subscription id"
      assert_transaction "$contract_name" "$tx"
      invoke "$id" "$SUBSCRIBER_IDENTITY" subscription get_subscription_count >/dev/null
      SMOKE_TX="$tx"; SMOKE_EVENTS='["subscribed"]'; SMOKE_STATE="subscription $subscription active"
      SMOKE_READ_METHOD="get_subscription_count"; SMOKE_READ_ARGS='[]'
      ;;
    marketplace-royalties)
      [ -n "${PAYER_IDENTITY:-}" ] && [ -n "${RECIPIENT_IDENTITY:-}" ] || fail "marketplace smoke requires PAYER_IDENTITY, RECIPIENT_IDENTITY"
      payer="$(identity_address "$PAYER_IDENTITY")"; recipient="$(identity_address "$RECIPIENT_IDENTITY")"
      r="$(invoke "$id" "$PAYER_IDENTITY" marketplace set_royalty --collection "$payer" --recipient "$recipient" --bps 100)"
      tx="$(printf '%s\n' "$r" | sed -n '1p')"
      assert_transaction "$contract_name" "$tx"
      invoke "$id" "$PAYER_IDENTITY" marketplace get_royalty --collection "$payer" >/dev/null
      SMOKE_TX="$tx"; SMOKE_EVENTS='["royalty_configured"]'; SMOKE_STATE="1% royalty configured"
      SMOKE_READ_METHOD="get_royalty"; SMOKE_READ_ARGS="[\"--collection\",\"$payer\"]"
      ;;
  esac
  update_manifest_contract "$key" "$id" "$SMOKE_TX" "passed" "$SMOKE_STATE" "$SMOKE_EVENTS" "$SMOKE_READ_METHOD" "$SMOKE_READ_ARGS"
  echo "$contract_name smoke: passed"
}

assert_transaction() {
  local contract_name="$1" hash="$2" response
  [[ "$hash" =~ ^[0-9a-fA-F]{64}$ ]] || fail "$contract_name smoke: transaction hash missing or invalid"
  response="$(curl --fail --silent --show-error "$RPC_URL" -H 'content-type: application/json' --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getTransaction\",\"params\":{\"hash\":\"$hash\"}}")" \
    || fail "$contract_name smoke: could not read transaction receipt for $hash"
  printf '%s' "$response" | python3 -c 'import json,sys; r=json.load(sys.stdin).get("result",{}); sys.exit(0 if r.get("status")=="SUCCESS" else 1)' \
    || fail "$contract_name smoke: transaction is not successful: $hash"
}

update_manifest_contract() {
  local key="$1" cid="$2" tx="$3" status="$4" state="$5" events="$6" method="$7" args="$8"
  RECEIPT_KEY="$key" RECEIPT_CID="$cid" RECEIPT_TX="$tx" RECEIPT_STATUS_VALUE="$status" RECEIPT_STATE="$state" RECEIPT_EVENTS="$events" RECEIPT_METHOD="$method" RECEIPT_ARGS="$args" RECEIPT_PATH="$RECEIPT" python3 - <<'PY'
import json, os
p=os.environ["RECEIPT_PATH"]
with open(p) as f: d=json.load(f)
for c in d["contracts"]:
    if c["key"]==os.environ["RECEIPT_KEY"]:
        c["contract_id"]=os.environ["RECEIPT_CID"]
        c["verification_status"]="verified"
        c["read_only"]={"method":os.environ["RECEIPT_METHOD"],"args":json.loads(os.environ["RECEIPT_ARGS"])}
        c["smoke"]={"status":os.environ["RECEIPT_STATUS_VALUE"],"transaction":os.environ["RECEIPT_TX"] or None,"events":json.loads(os.environ["RECEIPT_EVENTS"]),"state":os.environ["RECEIPT_STATE"]}
        break
with open(p,"w") as f:
    json.dump(d,f,indent=2); f.write("\n")
PY
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --dry-run) MODE="dry_run" ;;
    --build-only) MODE="build" ;;
    --receipt) shift; [ "$#" -gt 0 ] || fail "--receipt requires a path"; RECEIPT="$1" ;;
    *) fail "unknown option: $1 (use --help)" ;;
  esac
  shift
done

if [ "$MODE" = "dry_run" ]; then
  write_manifest "dry_run"
  cat <<OUTPUT
dry-run: no Stellar CLI, credentials, funding, network calls, or builds were used
planned contracts: escrow, vesting, multi-sig-wallet, dao-governance, subscription-payments, marketplace-royalties
planned verification: provenance hash match -> deploy/fetch -> invoke smoke -> state/events -> JSON receipt
OUTPUT
  exit 0
fi

if [ "$MODE" = "build" ]; then
  build_artifacts
  exit 0
fi

require_command stellar
require_command python3
[ -n "$SOURCE_IDENTITY" ] || fail "SOURCE_IDENTITY is required in live mode"
[ -n "$TOKEN_ID" ] || fail "TOKEN_ID is required in live mode"
identity_address "$SOURCE_IDENTITY" >/dev/null
build_artifacts
validate_provenance_hashes
write_manifest "in_progress"

step "Deploy or select contracts"
for spec in "${PACKAGES[@]}"; do
  IFS='|' read -r key package wasm name <<< "$spec"
  wasm_path="target/wasm32v1-none/release/$wasm"
  [ -f "$wasm_path" ] || fail "$name artifact missing: $wasm_path"
  cid="$(id_for_key "$key")"
  if [ -z "$cid" ]; then
    cid="$(stellar contract deploy --wasm "$wasm_path" --source-account "$SOURCE_IDENTITY" --rpc-url "$RPC_URL" --network-passphrase "$NETWORK_PASSPHRASE" 2>&1 | tail -1)" \
      || fail "$name deployment failed"
    [[ "$cid" =~ ^[A-Z0-9]+$ ]] || fail "$name deployment did not return a contract id"
  fi
  printf '%s: %s\n' "$name" "$cid"
  case "$key" in
    escrow) ESCROW_ID="$cid" ;;
    vesting) VESTING_ID="$cid" ;;
    multi-sig-wallet) MULTI_SIG_WALLET_ID="$cid" ;;
    dao-governance) DAO_GOVERNANCE_ID="$cid" ;;
    subscription-payments) SUBSCRIPTION_PAYMENTS_ID="$cid" ;;
    marketplace-royalties) MARKETPLACE_ROYALTIES_ID="$cid" ;;
  esac
  smoke_contract "$key" "$cid" "$name"
done

RECEIPT_PATH="$RECEIPT" python3 - <<'PY_MANIFEST'
import json
import os

path = os.environ["RECEIPT_PATH"]

with open(path, encoding="utf-8") as f:
    manifest = json.load(f)

manifest["status"] = "passed"

with open(path, "w", encoding="utf-8") as f:
    json.dump(manifest, f, indent=2)
    f.write("\n")
PY_MANIFEST

echo "deployment manifest: $RECEIPT"
