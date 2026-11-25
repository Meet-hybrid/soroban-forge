#!/usr/bin/env python3
"""Live proof that Soroban Forge settles a token built with OpenZeppelin
Stellar Contracts, on Stellar testnet.

Deploys OpenZeppelin's `fungible-pausable` example token, then runs it through
Forge's escrow and vesting contracts:

  A. escrow: create -> deposit -> seller release
  B. escrow while the token is paused: release is refused and the funds stay
     in escrow; after unpause the same release succeeds
  C. escrow dispute: the arbiter rules for the buyer, who is refunded
  D. vesting: the funder pays the contract, the beneficiary claims in full

and checks that every token is accounted for. Writes a JSON receipt.

Needs the Stellar CLI with four funded identities (forge-oz-issuer,
forge-oz-buyer, forge-oz-seller, forge-oz-arbiter), the two WASM files and the
deployed escrow:

  python3 scripts/openzeppelin-token-proof.py \\
      --oz-wasm path/to/fungible_pausable_example.wasm \\
      --vesting-wasm target/wasm32v1-none/release/soroban_forge_vesting.wasm \\
      --escrow CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB \\
      --receipt docs/testnet/openzeppelin-token-receipt.json

Every transaction is signed by the Stellar CLI; no secret key is read here.
"""

import argparse
import hashlib
import json
import re
import subprocess
import sys
import time
from datetime import datetime, timezone

NET = ["--network", "testnet"]
EXPLORER = "https://stellar.expert/explorer/testnet/tx/"
SUPPLY = 1_000_000
ROLES = ("issuer", "buyer", "seller", "arbiter")

p = argparse.ArgumentParser()
p.add_argument("--oz-wasm", required=True)
p.add_argument("--vesting-wasm", required=True)
p.add_argument("--escrow", required=True)
p.add_argument("--receipt", required=True)
p.add_argument("--prefix", default="forge-oz")
args = p.parse_args()

ident = {r: f"{args.prefix}-{r}" for r in ROLES}
addr = {
    r: subprocess.run(["stellar", "keys", "address", ident[r]], check=True, capture_output=True, text=True).stdout.strip()
    for r in ROLES
}
receipt = {
    "network": "testnet",
    "date": datetime.now(timezone.utc).strftime("%Y-%m-%d"),
    "escrow": args.escrow,
    "openzeppelin": {"library": "OpenZeppelin/stellar-contracts v0.8.0", "example": "fungible-pausable"},
    "roles": addr,
    "steps": [],
}


def sha256(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def cli(argv, expect_ok=True):
    out = subprocess.run(argv, capture_output=True, text=True)
    m = re.search(r"([0-9a-f]{64})", out.stderr)
    if expect_ok and out.returncode != 0:
        sys.exit(f"command failed: {' '.join(argv[:6])}…\n{out.stderr[-1500:]}")
    return out.returncode == 0, out.stdout.strip(), m.group(1) if m else None, out.stderr


def invoke(contract, role, fn, *kv, expect_ok=True, send=True):
    argv = ["stellar", "contract", "invoke", "--id", contract, "--source", ident[role], *NET]
    if not send:
        argv += ["--send", "no"]
    argv += ["--", fn, *kv]
    return cli(argv, expect_ok)


def balance(token, who):
    _, out, _, _ = invoke(token, "issuer", "balance", "--account", who, send=False)
    return int(out.strip('"'))


def step(what, **data):
    if "hash" in data and data["hash"]:
        data["tx"] = EXPLORER + data["hash"]
    receipt["steps"].append({"what": what, **data})
    print("✓", what, json.dumps({k: v for k, v in data.items() if k != "tx"}))


# ------------------------------------------------------------------ setup
ok, token, h, _ = cli(["stellar", "contract", "deploy", "--wasm", args.oz_wasm, "--source", ident["issuer"], *NET, "--",
                       "--name", "Forge Demo Token", "--symbol", "FDT", "--owner", addr["issuer"],
                       "--initial_supply", str(SUPPLY)])
token = token.splitlines()[-1]
receipt["openzeppelin"].update(token=token, wasm_sha256=sha256(args.oz_wasm))
step("deploy OpenZeppelin fungible-pausable token", hash=h, token=token, supply=SUPPLY)

ok, vesting, h, _ = cli(["stellar", "contract", "deploy", "--wasm", args.vesting_wasm, "--source", ident["issuer"], *NET])
vesting = vesting.splitlines()[-1]
receipt["vesting"] = {"contract": vesting, "wasm_sha256": sha256(args.vesting_wasm)}
step("deploy Forge vesting", hash=h, contract=vesting)

_, _, h, _ = invoke(token, "issuer", "transfer", "--from", addr["issuer"], "--to", addr["buyer"], "--amount", "10000")
step("issuer funds the buyer with 10,000 FDT", hash=h)

TIMEOUT = "86400"


def new_escrow(amount):
    _, out, h1, _ = invoke(args.escrow, "buyer", "create_escrow", "--buyer", addr["buyer"], "--seller", addr["seller"],
                           "--arbiter", addr["arbiter"], "--token", token, "--amount", str(amount), "--timeout", TIMEOUT)
    eid = out.strip('"')
    _, _, h2, _ = invoke(args.escrow, "buyer", "deposit", "--escrow_id", eid)
    return eid, h1, h2


escrow_before = balance(token, args.escrow)

# ------------------------------------------------------------------ A: normal release
eid, h1, h2 = new_escrow(1000)
held = balance(token, args.escrow) - escrow_before
_, _, h3, _ = invoke(args.escrow, "seller", "release", "--escrow_id", eid)
step("A. escrow: create, deposit 1,000, seller releases", escrow_id=eid, create=h1, deposit=h2, hash=h3,
     held_in_escrow_after_deposit=held, ok=held == 1000)

# ------------------------------------------------------------------ B: token paused
eid, h1, h2 = new_escrow(500)
_, _, hp, _ = invoke(token, "issuer", "pause", "--caller", addr["issuer"])
seller_before = balance(token, addr["seller"])
ok_paused, _, _, err = invoke(args.escrow, "seller", "release", "--escrow_id", eid, expect_ok=False)
still_held = balance(token, args.escrow) - escrow_before
_, status_paused, _, _ = invoke(args.escrow, "issuer", "get_status", "--escrow_id", eid, send=False)
_, _, hu, _ = invoke(token, "issuer", "unpause", "--caller", addr["issuer"])
_, _, hr, _ = invoke(args.escrow, "seller", "release", "--escrow_id", eid)
step("B. token paused by its owner: release is refused, funds stay in escrow; after unpause it succeeds",
     escrow_id=eid, create=h1, deposit=h2, pause=hp, unpause=hu, hash=hr,
     release_while_paused_refused=not ok_paused, error=(re.findall(r"Error\([^)]*\)", err) or ["?"])[0],
     escrow_status_while_paused=status_paused.strip('"'), held_while_paused=still_held,
     seller_unchanged_while_paused=balance(token, addr["seller"]) - seller_before - 500 == 0,
     ok=(not ok_paused) and still_held == 500)

# ------------------------------------------------------------------ C: dispute
eid, h1, h2 = new_escrow(300)
buyer_before = balance(token, addr["buyer"])
_, _, hd, _ = invoke(args.escrow, "buyer", "dispute", "--escrow_id", eid, "--claimant", addr["buyer"])
_, _, hz, _ = invoke(args.escrow, "arbiter", "resolve", "--escrow_id", eid, "--in_favor_of_seller", "false")
refunded = balance(token, addr["buyer"]) - buyer_before
step("C. dispute: buyer disputes, arbiter rules for the buyer, buyer refunded", escrow_id=eid, create=h1,
     deposit=h2, dispute=hd, hash=hz, refunded=refunded, ok=refunded == 300)

# ------------------------------------------------------------------ D: vesting
_, _, hf, _ = invoke(token, "issuer", "transfer", "--from", addr["issuer"], "--to", vesting, "--amount", "1200")
_, sid, hs, _ = invoke(vesting, "seller", "create_schedule", "--funder", addr["issuer"], "--beneficiary",
                       addr["seller"], "--token", token, "--total_amount", "1200", "--cliff", "0", "--duration", "30")
sid = sid.strip('"')
time.sleep(40)
_, claimed, hc, _ = invoke(vesting, "seller", "claim", "--schedule_id", sid)
step("D. vesting: funder pays the contract 1,200, schedule of 30 s, beneficiary claims", schedule_id=sid,
     fund=hf, schedule=hs, hash=hc, claimed=int(claimed.strip('"')), ok=int(claimed.strip('"')) == 1200)

# ------------------------------------------------------------------ conservation
final = {r: balance(token, addr[r]) for r in ("issuer", "buyer", "seller")}
final["escrow_contract_change"] = balance(token, args.escrow) - escrow_before
final["vesting_contract"] = balance(token, vesting)
total = final["issuer"] + final["buyer"] + final["seller"] + final["escrow_contract_change"] + final["vesting_contract"]
expected = {"issuer": SUPPLY - 10000 - 1200, "buyer": 10000 - 1000 - 500, "seller": 1000 + 500 + 1200,
            "escrow_contract_change": 0, "vesting_contract": 0}
step("every token accounted for", balances=final, expected=expected, total=total,
     ok=final == expected and total == SUPPLY)

receipt["all_ok"] = all(s.get("ok", True) for s in receipt["steps"])
with open(args.receipt, "w") as f:
    json.dump(receipt, f, indent=2)
    f.write("\n")
print("receipt written:", args.receipt, "all_ok =", receipt["all_ok"])
sys.exit(0 if receipt["all_ok"] else 1)
