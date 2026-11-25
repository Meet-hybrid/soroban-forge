# Soroban Forge and OpenZeppelin Stellar Contracts

Use them together. [OpenZeppelin Stellar Contracts](https://github.com/OpenZeppelin/stellar-contracts)
gives you audited tokens, access control, governance and smart accounts.
Soroban Forge gives you the payment flows that move those tokens: escrow,
vesting, subscriptions and royalty settlement. Forge works with any SEP-41
token, and it has been run on testnet with a token built from OpenZeppelin's
own example code.

## What each library covers

Compared against OpenZeppelin Stellar Contracts v0.8.0 (October 2026).

| Need | OpenZeppelin | Soroban Forge |
| --- | --- | --- |
| Fungible and non-fungible tokens, with pausable, allowlist, blocklist and capped variants | ✅ | — (use OpenZeppelin, or any SEP-41 token) |
| Ownership and role-based access control | ✅ | — |
| Pausable, upgradeable, math and crypto utilities | ✅ | — |
| **Escrow**: buyer, seller and arbiter, partial release, disputes, refund after the deadline | — | ✅ flagship, live on testnet |
| **Vesting**: cliff and linear or tranche release, claims | — | ✅ |
| **Subscriptions**: recurring charges, pause and resume, cancellation after failed payments | — | ✅ |
| **Royalty settlement**: pay the seller and the royalty holder per sale, up to 20 sales in one call | Royalty *information* on NFTs (who is owed what) | ✅ moves the money |
| Governance: proposals, votes, timelock | ✅ Governor, votes, timelock | DAO with a SEP-41 bond per proposal |
| Multi-signature control | ✅ Smart accounts with policies | Multi-sig wallet with rolling withdrawal limits |

Where both libraries have something (governance and multi-signature), they
differ: OpenZeppelin's are audited and general; Forge's DAO adds a proposal
bond that is returned or forfeited, and Forge's multi-sig adds rolling
withdrawal limits. If you only need standard governance or a smart account,
use OpenZeppelin's.

## Using them together

A typical setup:

1. Issue your token with OpenZeppelin (for example `fungible-pausable` or
   `fungible-allowlist`), with OpenZeppelin access control on its admin
   functions.
2. Settle payments in that token with Forge: hold it in escrow, vest it to a
   team, charge it as a subscription, or split a sale between seller and
   royalty holder.

Forge calls the token only through the standard SEP-41 interface, so nothing
in Forge depends on OpenZeppelin's code, and the two can use different
Soroban SDK versions (OpenZeppelin v0.8.0 uses 28, Forge uses 29).

Things to know when combining them:

- **Paused tokens.** When the token's owner pauses it, every Forge payout in
  that token fails with `TokenTransferFailed` (code 11) and nothing changes:
  the escrow stays `Funded` and keeps the funds. Retry after the token is
  unpaused. This was checked on testnet (round B below).
- **Allowlist and blocklist tokens.** The Forge contract address must be
  allowed to hold and send the token, and so must every party it pays.
  Otherwise deposits or payouts fail the same way, safely.
- **Decimals.** Forge amounts are in the token's smallest unit. OpenZeppelin's
  example tokens use 18 decimals.

## Testnet proof

[`docs/testnet/openzeppelin-token-receipt.json`](testnet/openzeppelin-token-receipt.json)
records every transaction. On 13 November 2026, an OpenZeppelin
`fungible-pausable` token (OpenZeppelin Stellar Contracts v0.8.0, contract
`CBPDG57HYK5NQM3P5KISW4MG7SWQ7OJ4VO44PBOP6ERONOJJXYAVJTEY`) went through
Forge's deployed escrow (`CC227UDF…NBZHSB`) and a freshly deployed Forge
vesting contract:

| Round | What happened | Result |
| --- | --- | --- |
| A | Buyer created and funded an escrow of 1,000; the seller released it | Seller paid 1,000 |
| B | Escrow of 500 funded, then the token's owner paused the token. The seller's release was refused (`TokenTransferFailed`); the escrow stayed `Funded` holding 500 and the seller received nothing. After unpause, the same release succeeded | Funds safe while paused; seller paid 500 after |
| C | Escrow of 300; the buyer disputed and the arbiter ruled for the buyer | Buyer refunded 300 |
| D | The funder paid 1,200 into vesting; a 30-second schedule was created and the beneficiary claimed | Beneficiary received 1,200 |

Afterwards every token was accounted for: issuer 988,800 + buyer 8,500 +
seller 2,700 = 1,000,000 minted, with the escrow and vesting contracts holding
nothing extra.

### Reproduce it

Build OpenZeppelin's example token and Forge's vesting contract, create four
funded testnet identities, then run the script:

```bash
git clone --branch v0.8.0 https://github.com/OpenZeppelin/stellar-contracts
(cd stellar-contracts && stellar contract build --package fungible-pausable-example)
stellar contract build --package soroban-forge-vesting
for r in issuer buyer seller arbiter; do stellar keys generate forge-oz-$r --network testnet --fund; done

python3 scripts/openzeppelin-token-proof.py \
  --oz-wasm stellar-contracts/target/wasm32v1-none/release/fungible_pausable_example.wasm \
  --vesting-wasm target/wasm32v1-none/release/soroban_forge_vesting.wasm \
  --escrow CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB \
  --receipt artifacts/openzeppelin-token-receipt.json
```

The script signs through the Stellar CLI and never reads a secret key. It
exits non-zero if any check fails.

## Limits

- Testnet only. Neither this combination nor Forge's contracts have been
  independently audited; OpenZeppelin's contracts have their own audits.
- This proof covers escrow and vesting. Subscriptions and royalty settlement
  use the same SEP-41 calls but were not part of this run.
