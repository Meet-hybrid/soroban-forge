"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { Buffer } from "buffer";
import { dao } from "@soroban-forge/escrow-client";
import {
  canExecuteProposal,
  canVoteOnProposal,
  classifyDaoError,
  computeVoteBreakdown,
  deriveEffectiveLifecycle,
  formatActionPreview,
  lifecycleBadgeClass,
  validateProposalDraft,
  type DaoErrorBanner,
  type DaoProposalView,
} from "./dao-governance";

export const DEFAULT_DAO_CONTRACT_ID =
  "CD477UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZDAO";

export interface DaoGovernanceSectionProps {
  account: string | null;
  rpcUrl: string;
  networkPassphrase: string;
  defaultTargetAddress: string;
  onLog: (text: string, kind?: "info" | "ok" | "err") => void;
}

export default function DaoGovernanceSection({
  account,
  rpcUrl,
  networkPassphrase,
  defaultTargetAddress,
  onLog,
}: DaoGovernanceSectionProps) {
  const [daoContractId, setDaoContractId] = useState(DEFAULT_DAO_CONTRACT_ID);
  const [proposals, setProposals] = useState<DaoProposalView[]>([]);
  const [proposalCount, setProposalCount] = useState<number | null>(null);
  const [bondConfig, setBondConfig] = useState<dao.BondConfig | null>(null);
  const [votedMap, setVotedMap] = useState<Record<string, boolean>>({});
  const [loadingProposals, setLoadingProposals] = useState(false);

  // Proposal creation state
  const [targetAddress, setTargetAddress] = useState(defaultTargetAddress);
  const [actionInput, setActionInput] = useState("set_fee_bps:25");
  const [actionEncoding, setActionEncoding] = useState<"utf8" | "hex">("utf8");
  const [durationSecs, setDurationSecs] = useState("3600");
  const [proposing, setProposing] = useState(false);
  const [lastCreatedProposalId, setLastCreatedProposalId] = useState<
    string | null
  >(null);

  // Per-proposal action state & error banner
  const [inFlightKey, setInFlightKey] = useState<string | null>(null);
  const [errorBanner, setErrorBanner] = useState<DaoErrorBanner | null>(null);

  const daoClient = useMemo(() => {
    if (!daoContractId.trim()) return null;
    const pubkey =
      account ?? "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";
    return new dao.Client({
      rpcUrl,
      networkPassphrase,
      contractId: daoContractId.trim(),
      publicKey: pubkey,
      signTransaction:
        typeof window !== "undefined" && window.freighterApi && account
          ? (
              xdr: string,
              opts?: { networkPassphrase?: string; address?: string }
            ) =>
              window.freighterApi!.signTransaction(xdr, {
                networkPassphrase: opts?.networkPassphrase ?? networkPassphrase,
                address: opts?.address ?? account,
              })
          : undefined,
    });
  }, [account, daoContractId, networkPassphrase, rpcUrl]);

  const fetchGovernanceState = useCallback(
    async (clientInstance?: dao.Client | null) => {
      const activeClient = clientInstance ?? daoClient;
      if (!activeClient) return;
      setLoadingProposals(true);
      try {
        const [{ result: listResult }, { result: countResult }] =
          await Promise.all([
            activeClient.get_proposals({ offset: 0, limit: 25 }),
            activeClient.get_proposal_count(),
          ]);

        setProposalCount(Number(countResult));

        try {
          const { result: bondResult } = await activeClient.get_bond_config();
          if (bondResult.isOk()) {
            setBondConfig(bondResult.unwrap());
          }
        } catch {
          // Bond config may not be initialized on a fresh deployment
        }

        if (listResult.isOk()) {
          const fetched = listResult.unwrap();
          setProposals(fetched);
          onLog(
            `DAO get_proposals: loaded ${fetched.length} proposal(s) (total ${countResult.toString()})`,
            "ok"
          );

          if (account && fetched.length > 0) {
            const voteEntries = await Promise.all(
              fetched.map(async (p) => {
                try {
                  const { result: votedRes } = await activeClient.has_voted({
                    proposal_id: p.proposal_id,
                    voter: account,
                  });
                  return [
                    p.proposal_id.toString(),
                    votedRes.isOk() ? votedRes.unwrap() : false,
                  ] as const;
                } catch {
                  return [p.proposal_id.toString(), false] as const;
                }
              })
            );
            setVotedMap(Object.fromEntries(voteEntries));
          }
        } else {
          const errMsg = listResult.unwrapErr().message;
          const banner = classifyDaoError(errMsg, "load");
          setErrorBanner(banner);
          onLog(`DAO get_proposals returned error: ${errMsg}`, "err");
        }
      } catch (err) {
        const banner = classifyDaoError(err, "load");
        setErrorBanner(banner);
        onLog(`Failed to load DAO proposals: ${(err as Error).message}`, "err");
      } finally {
        setLoadingProposals(false);
      }
    },
    [account, daoClient, onLog]
  );

  useEffect(() => {
    if (account && daoClient) {
      void fetchGovernanceState(daoClient);
    }
  }, [account, daoClient, fetchGovernanceState]);

  const handleCreateProposal = useCallback(async () => {
    setErrorBanner(null);
    const validation = validateProposalDraft({
      proposer: account ?? "",
      target: targetAddress,
      actionInput,
      actionEncoding,
      durationSecs,
    });
    if (!validation.ok) {
      setErrorBanner(validation.banner);
      onLog(
        `DAO propose validation failed: ${validation.banner.message}`,
        "err"
      );
      return;
    }
    if (!daoClient || !account) {
      const banner = classifyDaoError("Unauthorized: connect wallet", "propose");
      setErrorBanner(banner);
      return;
    }

    setProposing(true);
    try {
      const tx = await daoClient.propose(
        {
          proposer: validation.proposer,
          target: validation.target,
          action: Buffer.from(validation.actionBytes),
          duration: validation.duration,
        },
        { timeoutInSeconds: 60 }
      );
      onLog("DAO propose simulated — requesting wallet signature…", "info");
      const sent = await tx.signAndSend();
      if (sent.sendTransactionResponse?.hash) {
        onLog(
          `DAO propose submitted — tx hash ${sent.sendTransactionResponse.hash}`,
          "ok"
        );
      }
      if (sent.result.isOk()) {
        const newId = sent.result.unwrap();
        setLastCreatedProposalId(newId.toString());
        onLog(`DAO proposal #${newId.toString()} created (Active)`, "ok");
        await fetchGovernanceState(daoClient);
      } else {
        const errMsg = sent.result.unwrapErr().message;
        const banner = classifyDaoError(errMsg, "propose");
        setErrorBanner(banner);
        onLog(`DAO propose error: ${errMsg}`, "err");
      }
    } catch (err) {
      const banner = classifyDaoError(err, "propose");
      setErrorBanner(banner);
      onLog(`DAO propose failed: ${(err as Error).message}`, "err");
    } finally {
      setProposing(false);
    }
  }, [
    account,
    actionEncoding,
    actionInput,
    daoClient,
    durationSecs,
    fetchGovernanceState,
    onLog,
    targetAddress,
  ]);

  const handleVote = useCallback(
    async (proposal: DaoProposalView, support: boolean) => {
      setErrorBanner(null);
      const pidStr = proposal.proposal_id.toString();
      const nowSec = BigInt(Math.floor(Date.now() / 1000));
      const alreadyVoted = Boolean(votedMap[pidStr]);
      const eligibility = canVoteOnProposal(
        proposal,
        account,
        nowSec,
        alreadyVoted
      );

      if (!eligibility.allowed) {
        const banner = classifyDaoError(
          eligibility.reason ?? "Vote not permitted",
          "vote"
        );
        setErrorBanner(banner);
        onLog(`DAO vote blocked on #${pidStr}: ${banner.message}`, "err");
        return;
      }
      if (!daoClient || !account) return;

      const flightId = `vote-${pidStr}-${support ? "for" : "against"}`;
      setInFlightKey(flightId);
      try {
        const tx = await daoClient.vote(
          {
            proposal_id: proposal.proposal_id,
            voter: account,
            support,
          },
          { timeoutInSeconds: 60 }
        );
        onLog(
          `DAO vote (${support ? "For" : "Against"}) on #${pidStr} simulated — requesting signature…`,
          "info"
        );
        const sent = await tx.signAndSend();
        if (sent.sendTransactionResponse?.hash) {
          onLog(
            `DAO vote submitted — tx hash ${sent.sendTransactionResponse.hash}`,
            "ok"
          );
        }
        if (sent.result.isOk()) {
          setVotedMap((prev) => ({ ...prev, [pidStr]: true }));
          onLog(
            `Vote (${support ? "For" : "Against"}) recorded on proposal #${pidStr}`,
            "ok"
          );
          await fetchGovernanceState(daoClient);
        } else {
          const errMsg = sent.result.unwrapErr().message;
          const banner = classifyDaoError(errMsg, "vote");
          setErrorBanner(banner);
          onLog(`DAO vote rejected on #${pidStr}: ${errMsg}`, "err");
        }
      } catch (err) {
        const banner = classifyDaoError(err, "vote");
        setErrorBanner(banner);
        onLog(
          `DAO vote failed on #${pidStr}: ${(err as Error).message}`,
          "err"
        );
      } finally {
        setInFlightKey(null);
      }
    },
    [account, daoClient, fetchGovernanceState, onLog, votedMap]
  );

  const handleExecute = useCallback(
    async (proposal: DaoProposalView) => {
      setErrorBanner(null);
      const pidStr = proposal.proposal_id.toString();
      const nowSec = BigInt(Math.floor(Date.now() / 1000));
      const execCheck = canExecuteProposal(proposal, nowSec);

      if (!execCheck.allowed) {
        const banner = classifyDaoError(
          execCheck.reason ?? "Proposal is not ready for execution",
          "execute"
        );
        setErrorBanner(banner);
        onLog(`DAO execute blocked on #${pidStr}: ${banner.message}`, "err");
        return;
      }
      if (!daoClient || !account) {
        const banner = classifyDaoError(
          "Unauthorized: connect wallet to execute proposal",
          "execute"
        );
        setErrorBanner(banner);
        return;
      }

      const flightId = `execute-${pidStr}`;
      setInFlightKey(flightId);
      try {
        const tx = await daoClient.execute(
          { proposal_id: proposal.proposal_id },
          { timeoutInSeconds: 60 }
        );
        onLog(
          `DAO execute on #${pidStr} simulated — requesting signature…`,
          "info"
        );
        const sent = await tx.signAndSend();
        if (sent.sendTransactionResponse?.hash) {
          onLog(
            `DAO execute submitted — tx hash ${sent.sendTransactionResponse.hash}`,
            "ok"
          );
        }
        if (sent.result.isOk()) {
          onLog(`Proposal #${pidStr} executed on-chain`, "ok");
          await fetchGovernanceState(daoClient);
        } else {
          const errMsg = sent.result.unwrapErr().message;
          const banner = classifyDaoError(errMsg, "execute");
          setErrorBanner(banner);
          onLog(`DAO execute rejected on #${pidStr}: ${errMsg}`, "err");
        }
      } catch (err) {
        const banner = classifyDaoError(err, "execute");
        setErrorBanner(banner);
        onLog(
          `DAO execute failed on #${pidStr}: ${(err as Error).message}`,
          "err"
        );
      } finally {
        setInFlightKey(null);
      }
    },
    [account, daoClient, fetchGovernanceState, onLog]
  );

  const nowSec = BigInt(Math.floor(Date.now() / 1000));

  return (
    <section
      aria-label="DAO Governance Portal"
      className="mt-8 rounded-xl border border-gray-300 p-5"
    >
      <div className="flex flex-wrap items-center justify-between gap-3 mb-4">
        <div>
          <h2 className="text-xl font-semibold">
            6 · DAO Governance — Proposals &amp; Voting
          </h2>
          <p className="text-xs text-gray-500">
            Create governance proposals, cast token-weighted For/Against votes,
            and execute Succeeded proposals after voting concludes.
          </p>
        </div>
        <div className="flex items-center gap-2">
          {proposalCount !== null && (
            <span className="rounded-full bg-gray-100 px-2.5 py-1 text-xs font-medium text-gray-700">
              {proposalCount} total proposal{proposalCount === 1 ? "" : "s"}
            </span>
          )}
          <button
            type="button"
            onClick={() => void fetchGovernanceState()}
            disabled={!daoClient || loadingProposals}
            className="rounded bg-gray-800 px-3 py-1.5 text-sm text-white hover:bg-gray-900 disabled:opacity-50"
          >
            {loadingProposals ? "Refreshing…" : "Refresh proposals"}
          </button>
        </div>
      </div>

      {/* DAO Contract Configuration bar */}
      <div className="mb-4 rounded-lg bg-gray-50 p-3 border border-gray-200">
        <label className="block text-xs font-medium text-gray-600">
          DAO Governance Contract ID (C…)
          <input
            value={daoContractId}
            onChange={(e) => setDaoContractId(e.target.value)}
            placeholder="DAO contract ID on testnet"
            className="mt-1 w-full rounded border border-gray-300 bg-white px-2 py-1 font-mono text-xs"
          />
        </label>
        {bondConfig && (
          <p className="mt-1.5 text-xs text-gray-600">
            Proposal bond:{" "}
            <span className="font-mono font-medium">
              {bondConfig.amount.toString()} stroops
            </span>{" "}
            of token <code className="break-all">{bondConfig.token}</code> ·
            Treasury <code className="break-all">{bondConfig.treasury}</code>
          </p>
        )}
      </div>

      {/* Error banner for duplicate votes, expired proposals, unauthorized accounts */}
      {errorBanner && (
        <div
          role="alert"
          className="mb-4 rounded-lg border border-red-300 bg-red-50 p-3 text-sm text-red-900"
        >
          <div className="flex items-start justify-between gap-2">
            <div>
              <div className="flex items-center gap-2">
                <span className="font-semibold">{errorBanner.title}</span>
                <span className="rounded bg-red-200 px-1.5 py-0.5 font-mono text-xs text-red-800">
                  {errorBanner.category}
                  {errorBanner.code !== null ? ` (#${errorBanner.code})` : ""}
                </span>
              </div>
              <p className="mt-1 text-xs text-red-800">{errorBanner.message}</p>
            </div>
            <button
              type="button"
              onClick={() => setErrorBanner(null)}
              className="rounded px-2 py-0.5 text-xs text-red-700 hover:bg-red-100"
            >
              Dismiss
            </button>
          </div>
        </div>
      )}

      <div className="grid gap-6 lg:grid-cols-12">
        {/* Create Proposal Subview */}
        <div className="lg:col-span-5 rounded-lg border border-gray-200 p-4">
          <h3 className="text-base font-semibold mb-3">Create DAO Proposal</h3>
          <div className="space-y-3">
            <label className="block text-sm">
              Target contract address (C… / G…)
              <input
                value={targetAddress}
                onChange={(e) => setTargetAddress(e.target.value)}
                placeholder="Target contract invoked on execution"
                className="mt-1 w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-sm"
              />
            </label>

            <div>
              <div className="flex items-center justify-between text-sm">
                <span>Encoded action payload</span>
                <div className="flex items-center gap-2 text-xs">
                  <label className="flex items-center gap-1">
                    <input
                      type="radio"
                      name="dao-action-encoding"
                      checked={actionEncoding === "utf8"}
                      onChange={() => setActionEncoding("utf8")}
                    />
                    UTF-8
                  </label>
                  <label className="flex items-center gap-1">
                    <input
                      type="radio"
                      name="dao-action-encoding"
                      checked={actionEncoding === "hex"}
                      onChange={() => setActionEncoding("hex")}
                    />
                    Hex
                  </label>
                </div>
              </div>
              <input
                value={actionInput}
                onChange={(e) => setActionInput(e.target.value)}
                placeholder={
                  actionEncoding === "hex"
                    ? "0xdeadbeef"
                    : "e.g. set_fee_bps:25"
                }
                className="mt-1 w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-sm"
              />
            </div>

            <label className="block text-sm">
              Voting duration (seconds)
              <input
                value={durationSecs}
                onChange={(e) => setDurationSecs(e.target.value)}
                placeholder="3600"
                className="mt-1 w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-sm"
              />
            </label>

            <button
              type="button"
              onClick={() => void handleCreateProposal()}
              disabled={!account || proposing}
              className="w-full rounded bg-blue-600 px-4 py-2 text-sm text-white hover:bg-blue-700 disabled:opacity-50"
            >
              {proposing
                ? "Submitting proposal…"
                : account
                  ? "Create proposal"
                  : "Connect wallet to propose"}
            </button>

            {lastCreatedProposalId && (
              <p className="text-xs text-green-700">
                Created proposal{" "}
                <span className="font-mono font-semibold">
                  #{lastCreatedProposalId}
                </span>{" "}
                (Active)
              </p>
            )}
          </div>
        </div>

        {/* Proposals & Voting List Subview */}
        <div className="lg:col-span-7 rounded-lg border border-gray-200 p-4">
          <h3 className="text-base font-semibold mb-3">
            Active &amp; Historical Proposals
          </h3>

          {proposals.length === 0 ? (
            <p className="text-sm text-gray-500">
              No proposals loaded yet. Connect your wallet or click{" "}
              <span className="font-medium">Refresh proposals</span> to query
              the DAO contract.
            </p>
          ) : (
            <div className="space-y-4">
              {proposals.map((proposal) => {
                const pidStr = proposal.proposal_id.toString();
                const effectiveTag = deriveEffectiveLifecycle(proposal, nowSec);
                const badgeCss = lifecycleBadgeClass(effectiveTag);
                const breakdown = computeVoteBreakdown(
                  proposal.for_votes,
                  proposal.against_votes
                );
                const actionPreview = formatActionPreview(proposal.action);
                const alreadyVoted = Boolean(votedMap[pidStr]);
                const voteEligibility = canVoteOnProposal(
                  proposal,
                  account,
                  nowSec,
                  alreadyVoted
                );
                const execEligibility = canExecuteProposal(proposal, nowSec);
                const votingEnded = nowSec >= proposal.voting_ends;

                return (
                  <div
                    key={pidStr}
                    className="rounded-lg border border-gray-200 p-3.5 shadow-2xs"
                  >
                    <div className="flex flex-wrap items-center justify-between gap-2 mb-2">
                      <div className="flex items-center gap-2">
                        <span className="font-mono text-sm font-bold">
                          Proposal #{pidStr}
                        </span>
                        <span
                          className={`rounded-full px-2.5 py-0.5 text-xs font-medium ${badgeCss}`}
                        >
                          {effectiveTag}
                        </span>
                        {alreadyVoted && (
                          <span className="rounded-full bg-indigo-50 px-2 py-0.5 text-xs text-indigo-700">
                            Voted
                          </span>
                        )}
                      </div>
                      <span className="font-mono text-xs text-gray-500">
                        {votingEnded ? "Ended" : "Ends"}{" "}
                        {new Date(
                          Number(proposal.voting_ends) * 1000
                        ).toLocaleString()}
                      </span>
                    </div>

                    <dl className="grid grid-cols-1 gap-x-4 gap-y-1 text-xs sm:grid-cols-2 mb-3">
                      <div>
                        <dt className="font-medium text-gray-500">
                          Target address
                        </dt>
                        <dd className="font-mono break-all">
                          {proposal.target}
                        </dd>
                      </div>
                      <div>
                        <dt className="font-medium text-gray-500">Proposer</dt>
                        <dd className="font-mono break-all">
                          {proposal.proposer}
                        </dd>
                      </div>
                      <div className="sm:col-span-2">
                        <dt className="font-medium text-gray-500">
                          Action payload
                        </dt>
                        <dd className="font-mono break-all">
                          {actionPreview.text
                            ? `${actionPreview.text} (${actionPreview.hex})`
                            : actionPreview.hex}
                        </dd>
                      </div>
                    </dl>

                    {/* Vote tally bar */}
                    <div className="mb-3">
                      <div className="flex justify-between text-xs mb-1">
                        <span className="font-medium text-green-700">
                          For: {proposal.for_votes.toString()} (
                          {breakdown.forPct}%)
                        </span>
                        <span className="font-medium text-red-700">
                          Against: {proposal.against_votes.toString()} (
                          {breakdown.againstPct}%)
                        </span>
                      </div>
                      <div className="h-2 w-full overflow-hidden rounded-full bg-gray-200 flex">
                        <div
                          style={{ width: `${breakdown.forPct}%` }}
                          className="bg-green-600"
                        />
                        <div
                          style={{ width: `${breakdown.againstPct}%` }}
                          className="bg-red-500"
                        />
                      </div>
                    </div>

                    {/* Voting & Execution Controls */}
                    <div className="flex flex-wrap items-center gap-2">
                      <button
                        type="button"
                        onClick={() => void handleVote(proposal, true)}
                        disabled={inFlightKey !== null}
                        className="rounded border border-green-600 px-3 py-1 text-xs font-medium text-green-700 hover:bg-green-50 disabled:opacity-50"
                      >
                        {inFlightKey === `vote-${pidStr}-for`
                          ? "Voting For…"
                          : "Vote For"}
                      </button>
                      <button
                        type="button"
                        onClick={() => void handleVote(proposal, false)}
                        disabled={inFlightKey !== null}
                        className="rounded border border-red-600 px-3 py-1 text-xs font-medium text-red-700 hover:bg-red-50 disabled:opacity-50"
                      >
                        {inFlightKey === `vote-${pidStr}-against`
                          ? "Voting Against…"
                          : "Vote Against"}
                      </button>
                      <button
                        type="button"
                        onClick={() => void handleExecute(proposal)}
                        disabled={!execEligibility.allowed || inFlightKey !== null}
                        title={execEligibility.reason ?? "Execute proposal"}
                        className="ml-auto rounded bg-purple-600 px-3 py-1 text-xs font-medium text-white hover:bg-purple-700 disabled:opacity-40"
                      >
                        {inFlightKey === `execute-${pidStr}`
                          ? "Executing…"
                          : "Execute"}
                      </button>
                    </div>
                    {!voteEligibility.allowed && voteEligibility.reason && (
                      <p className="mt-1.5 text-xs text-gray-500">
                        {voteEligibility.reason}
                      </p>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </div>
    </section>
  );
}
