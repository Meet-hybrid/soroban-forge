"use client";

import { useCallback, useState } from "react";
import { multisig } from "@soroban-forge/escrow-client";
import { StellarWalletsKit } from "@creit.tech/stellar-wallets-kit/sdk";

const RPC_URL = "https://soroban-testnet.stellar.org";
const NETWORK_PASSPHRASE = "Test SDF Network ; September 2015";

type Props = { account: string | null; mounted: boolean };

function statusLabel(status: multisig.TxStatus): string {
  return status.tag;
}

function kindLabel(kind: multisig.TxKind): string {
  if (kind.tag === "Call") return `Call: ${kind.values[0].fn_name}`;
  if (kind.tag === "Withdrawal") {
    return `Withdrawal: ${kind.values[0].amount.toString()} to ${kind.values[0].destination}`;
  }
  if (kind.tag === "AddOwner") return `Add owner: ${kind.values[0]}`;
  if (kind.tag === "RemoveOwner") return `Remove owner: ${kind.values[0]}`;
  if (kind.tag === "SetThreshold") return `Set threshold: ${kind.values[0]}`;
  if (kind.tag === "LimitChange") return "Withdrawal limit change";
  return "Opaque call";
}

export default function MultisigDashboard({ account, mounted }: Props) {
  const [contractId, setContractId] = useState("");
  const [owners, setOwners] = useState<string[]>([]);
  const [threshold, setThreshold] = useState<number | null>(null);
  const [transactions, setTransactions] = useState<multisig.WalletTx[]>([]);
  const [loading, setLoading] = useState(false);
  const [action, setAction] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const client = useCallback(() => {
    if (!account || !contractId.trim()) return null;
    return new multisig.Client({
      rpcUrl: RPC_URL,
      networkPassphrase: NETWORK_PASSPHRASE,
      contractId: contractId.trim(),
      publicKey: account,
      signTransaction: async (xdr: string, options?: { networkPassphrase?: string; address?: string }) => {
        const result = await StellarWalletsKit.signTransaction(xdr, {
          networkPassphrase: options?.networkPassphrase ?? NETWORK_PASSPHRASE,
          address: options?.address ?? account,
        });
        if (!result.signedTxXdr) throw new Error("Wallet did not return a signed transaction.");
        return { signedTxXdr: result.signedTxXdr };
      },
    });
  }, [account, contractId]);

  const refresh = useCallback(async () => {
    const c = client();
    if (!c) {
      setError(account ? "Enter a multi-sig wallet contract ID." : "Connect a wallet to inspect a multi-sig wallet.");
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const [ownerResult, thresholdResult, txResult] = await Promise.all([
        c.get_owners(),
        c.get_threshold(),
        c.get_transactions({ offset: 0, limit: 50 }),
      ]);
      setOwners(ownerResult.result.unwrap());
      setThreshold(Number(thresholdResult.result.unwrap()));
      setTransactions(txResult.result.unwrap());
      setNotice("Wallet state refreshed.");
    } catch (cause) {
      setError((cause as Error).message);
    } finally {
      setLoading(false);
    }
  }, [account, client]);

  const mutate = useCallback(async (txId: bigint, operation: "confirm" | "reject" | "execute") => {
    const c = client();
    if (!c || !account) return;
    setAction(`${operation}:${txId.toString()}`);
    setError(null);
    try {
      const tx = operation === "confirm"
        ? await c.confirm({ tx_id: txId, signer: account })
        : operation === "reject"
          ? await c.reject({ tx_id: txId, signer: account })
          : await c.execute({ tx_id: txId });
      await tx.signAndSend();
      setNotice(`${operation} submitted for transaction ${txId.toString()}.`);
      await refresh();
    } catch (cause) {
      setError((cause as Error).message);
    } finally {
      setAction(null);
    }
  }, [account, client, refresh]);

  const isOwner = account !== null && owners.includes(account);
  const pending = transactions.filter((tx) => tx.status.tag === "Pending");

  return (
    <section className="mt-6 rounded-xl border border-gray-300 p-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-lg font-semibold">Multi-sig console</h2>
          <p className="text-sm text-gray-500">Inspect proposals, collect approvals, and execute threshold-approved transactions.</p>
        </div>
        <span className="rounded-full bg-indigo-100 px-2 py-0.5 text-xs text-indigo-800">testnet</span>
      </div>
      <div className="mt-3 flex flex-wrap gap-2">
        <input
          value={contractId}
          onChange={(event) => setContractId(event.target.value)}
          placeholder="Multi-sig wallet contract ID (C…)"
          className="min-w-72 flex-1 rounded border border-gray-300 px-2 py-1.5 font-mono text-sm"
        />
        <button onClick={() => void refresh()} disabled={!mounted || loading} className="rounded bg-indigo-600 px-4 py-1.5 text-sm text-white hover:bg-indigo-700 disabled:opacity-50">
          {loading ? "Loading…" : "Load wallet"}
        </button>
      </div>
      {!account && <p className="mt-2 text-sm text-amber-700">Connect a wallet first. Read and write operations use the connected account.</p>}
      {error && <p role="alert" className="mt-3 rounded bg-red-50 p-2 text-sm text-red-700">{error}</p>}
      {notice && !error && <p className="mt-3 rounded bg-green-50 p-2 text-sm text-green-700">{notice}</p>}

      {threshold !== null && (
        <>
          {!isOwner && <p className="mt-3 rounded bg-amber-50 p-2 text-sm text-amber-800">The connected account is not an owner. You can inspect this wallet, but cannot confirm or reject proposals.</p>}
          <div className="mt-4 grid gap-3 sm:grid-cols-3">
            <div className="rounded border border-gray-200 p-3"><p className="text-xs uppercase text-gray-500">Threshold</p><p className="text-2xl font-semibold">{threshold}</p></div>
            <div className="rounded border border-gray-200 p-3"><p className="text-xs uppercase text-gray-500">Owners</p><p className="text-2xl font-semibold">{owners.length}</p></div>
            <div className="rounded border border-gray-200 p-3"><p className="text-xs uppercase text-gray-500">Pending</p><p className="text-2xl font-semibold">{pending.length}</p></div>
          </div>
          <div className="mt-3 rounded border border-gray-200 p-3">
            <h3 className="font-medium">Active owners</h3>
            {owners.length === 0 ? <p className="mt-1 text-sm text-gray-500">No owners returned; the wallet may be uninitialized.</p> : <ul className="mt-2 space-y-1">{owners.map((owner) => <li key={owner} className="break-all font-mono text-xs">{owner}</li>)}</ul>}
          </div>
          <div className="mt-4 space-y-3">
            <h3 className="font-medium">Pending transactions</h3>
            {pending.length === 0 ? <p className="text-sm text-gray-500">No pending transactions.</p> : pending.map((tx) => {
              const txId = BigInt(tx.tx_id);
              const ready = tx.confirmations.length >= threshold;
              const expired = tx.expiry !== null && tx.expiry !== undefined && Number(tx.expiry) < Math.floor(Date.now() / 1000);
              return <article key={tx.tx_id.toString()} className="rounded border border-gray-200 p-3">
                <div className="flex flex-wrap items-start justify-between gap-2">
                  <div><p className="font-medium">Transaction #{tx.tx_id.toString()}</p><p className="break-all text-xs text-gray-500">Target: {tx.target}</p><p className="text-sm">{kindLabel(tx.kind)}</p></div>
                  <span className={expired ? "rounded bg-red-100 px-2 py-0.5 text-xs text-red-700" : "rounded bg-yellow-100 px-2 py-0.5 text-xs text-yellow-800"}>{expired ? "Expired" : statusLabel(tx.status)}</span>
                </div>
                <p className="mt-2 text-sm">Confirmations: {tx.confirmations.length}/{threshold} · Rejections: {tx.rejections.length}</p>
                <div className="mt-2 flex flex-wrap gap-2">
                  <button disabled={!isOwner || expired || action !== null} onClick={() => void mutate(txId, "confirm")} className="rounded border border-green-600 px-3 py-1 text-sm text-green-700 disabled:opacity-50">{action === `confirm:${txId}` ? "Confirming…" : "Confirm"}</button>
                  <button disabled={!isOwner || expired || action !== null} onClick={() => void mutate(txId, "reject")} className="rounded border border-red-600 px-3 py-1 text-sm text-red-700 disabled:opacity-50">{action === `reject:${txId}` ? "Rejecting…" : "Reject"}</button>
                  <button disabled={!ready || expired || action !== null} onClick={() => void mutate(txId, "execute")} className="rounded bg-blue-600 px-3 py-1 text-sm text-white disabled:opacity-50">{action === `execute:${txId}` ? "Executing…" : "Execute"}</button>
                </div>
              </article>;
            })}
          </div>
        </>
      )}
    </section>
  );
}
