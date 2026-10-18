"use client";

import { useState } from "react";
import { validateDaoVote } from "../../lib/validation";

interface DaoPanelProps {
  client: any;
  account: string | null;
  onLog: (text: string, kind?: "info" | "ok" | "err") => void;
  runWriteFlow: (
    actionName: string,
    validate: () => any,
    buildTx: (c: any, data: any) => Promise<any>
  ) => Promise<void>;
}

export function DaoPanel({ client, account, onLog, runWriteFlow }: DaoPanelProps) {
  // Read state
  const [proposalId, setProposalId] = useState("1");
  const [proposal, setProposal] = useState<any | null>(null);
  const [bondConfig, setBondConfig] = useState<any | null>(null);
  const [proposalCount, setProposalCount] = useState<string | null>(null);
  const [readBusy, setReadBusy] = useState(false);

  // Write state (vote)
  const [voter, setVoter] = useState(account ?? "");
  const [support, setSupport] = useState<boolean>(true);

  const handleFetchProposal = async () => {
    if (!client) {
      onLog("Connect wallet or verify client initialization first.", "err");
      return;
    }
    const id = BigInt(proposalId.trim() || "0");

    setReadBusy(true);
    try {
      onLog(`Querying DAO Proposal #${id.toString()} and Bond config...`, "info");
      const [propRes, bondRes, countRes] = await Promise.allSettled([
        client.get_proposal({ proposal_id: id }),
        client.get_bond_config(),
        client.get_proposal_count(),
      ]);

      if (propRes.status === "fulfilled" && propRes.value.result?.isOk?.()) {
        const p = propRes.value.result.unwrap();
        setProposal(p);
        onLog(`Fetched Proposal #${id.toString()} — State: ${p.state?.tag ?? "Unknown"}`, "ok");
      } else {
        setProposal(null);
      }

      if (bondRes.status === "fulfilled" && bondRes.value.result?.isOk?.()) {
        setBondConfig(bondRes.value.result.unwrap());
      }
      if (countRes.status === "fulfilled") {
        setProposalCount(countRes.value.result?.toString() ?? countRes.value.toString());
      }
    } catch (e) {
      onLog(`DAO query error: ${(e as Error).message}`, "err");
    } finally {
      setReadBusy(false);
    }
  };

  const handleCastVote = async () => {
    await runWriteFlow(
      "vote",
      () =>
        validateDaoVote({
          proposalId,
          voter: voter || account || "",
          support,
        }),
      async (c, data) => {
        return await c.vote({
          proposal_id: data.proposal_id,
          voter: data.voter,
          support: data.support,
        });
      }
    );
  };

  return (
    <div className="grid gap-6 lg:grid-cols-2">
      {/* Read Panel */}
      <div className="space-y-4 rounded-xl border border-slate-200 bg-white p-5 shadow-sm dark:border-slate-800 dark:bg-slate-900">
        <div>
          <h3 className="font-semibold text-base text-slate-900 dark:text-slate-100">
            DAO Governance Read Flows
          </h3>
          <p className="text-xs text-slate-500 dark:text-slate-400">
            Inspect proposal lifecycle states, votes tallies, posted bonds, and governance settings.
          </p>
        </div>

        <div className="rounded-lg border border-slate-200 bg-slate-50/50 p-3 dark:border-slate-800 dark:bg-slate-950/50">
          <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
            Proposal ID:
          </label>
          <div className="mt-1 flex gap-2">
            <input
              type="text"
              value={proposalId}
              onChange={(e) => setProposalId(e.target.value)}
              placeholder="e.g. 1"
              className="w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-900"
            />
            <button
              type="button"
              onClick={handleFetchProposal}
              disabled={readBusy}
              className="rounded bg-slate-800 px-3 py-1.5 text-xs font-medium text-white hover:bg-slate-700 disabled:opacity-50 dark:bg-slate-700 dark:hover:bg-slate-600"
            >
              {readBusy ? "Reading..." : "Fetch Proposal"}
            </button>
          </div>

          {proposal && (
            <div className="mt-3 space-y-2 rounded border border-slate-200 bg-white p-3 font-mono text-xs dark:border-slate-800 dark:bg-slate-900">
              <div className="flex justify-between items-center">
                <span className="text-slate-500">Proposal State:</span>
                <span className="rounded bg-indigo-50 px-2 py-0.5 font-semibold text-indigo-700 dark:bg-indigo-950 dark:text-indigo-300">
                  {proposal.state?.tag ?? "Unknown"}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-slate-500">For Votes:</span>
                <span className="text-emerald-600 dark:text-emerald-400 font-semibold">
                  {proposal.for_votes?.toString() ?? "0"}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-slate-500">Against Votes:</span>
                <span className="text-rose-600 dark:text-rose-400 font-semibold">
                  {proposal.against_votes?.toString() ?? "0"}
                </span>
              </div>
              <div className="flex justify-between truncate">
                <span className="text-slate-500">Proposer:</span>
                <span className="truncate max-w-[200px]">{proposal.proposer}</span>
              </div>
            </div>
          )}

          {proposalCount !== null && (
            <div className="mt-2 text-[11px] text-slate-500">
              Total Proposals Created: <strong className="text-slate-700 dark:text-slate-300">{proposalCount}</strong>
            </div>
          )}
          {bondConfig && (
            <div className="mt-1 text-[11px] text-slate-500 truncate">
              Proposal Bond: <strong className="text-slate-700 dark:text-slate-300">{bondConfig.amount?.toString()} stroops</strong>
            </div>
          )}
        </div>
      </div>

      {/* Write Panel */}
      <div className="space-y-4 rounded-xl border border-slate-200 bg-white p-5 shadow-sm dark:border-slate-800 dark:bg-slate-900">
        <div>
          <div className="flex items-center justify-between">
            <h3 className="font-semibold text-base text-slate-900 dark:text-slate-100">
              Write Flow: vote
            </h3>
            <span className="rounded bg-indigo-100 px-2 py-0.5 text-[10px] font-semibold text-indigo-800 dark:bg-indigo-950 dark:text-indigo-300">
              Safe &amp; Validated
            </span>
          </div>
          <p className="text-xs text-slate-500 dark:text-slate-400">
            Cast a verified vote on an active proposal before deadline. Validates voter address and ID.
          </p>
        </div>

        <div className="space-y-3">
          <div>
            <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
              Voter Address:
            </label>
            <input
              type="text"
              value={voter}
              onChange={(e) => setVoter(e.target.value)}
              placeholder={account ?? "G..."}
              className="mt-1 w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-950"
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-slate-700 dark:text-slate-300">
              Target Proposal ID:
            </label>
            <input
              type="text"
              value={proposalId}
              onChange={(e) => setProposalId(e.target.value)}
              placeholder="e.g. 1"
              className="mt-1 w-full rounded border border-slate-300 bg-white px-2.5 py-1.5 font-mono text-xs dark:border-slate-700 dark:bg-slate-950"
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1">
              Vote Decision:
            </label>
            <div className="flex gap-4">
              <label className="flex items-center gap-1.5 text-xs text-slate-700 dark:text-slate-300 cursor-pointer">
                <input
                  type="radio"
                  name="vote-support"
                  checked={support === true}
                  onChange={() => setSupport(true)}
                  className="text-indigo-600 focus:ring-indigo-500"
                />
                <span className="font-semibold text-emerald-600 dark:text-emerald-400">Vote For (Support)</span>
              </label>
              <label className="flex items-center gap-1.5 text-xs text-slate-700 dark:text-slate-300 cursor-pointer">
                <input
                  type="radio"
                  name="vote-support"
                  checked={support === false}
                  onChange={() => setSupport(false)}
                  className="text-indigo-600 focus:ring-indigo-500"
                />
                <span className="font-semibold text-rose-600 dark:text-rose-400">Vote Against</span>
              </label>
            </div>
          </div>

          <button
            type="button"
            onClick={handleCastVote}
            className="mt-4 w-full rounded-lg bg-indigo-600 px-4 py-2 text-xs font-semibold text-white shadow hover:bg-indigo-500 transition"
          >
            Simulate, Sign &amp; Cast Vote
          </button>
        </div>
      </div>
    </div>
  );
}
