"use client";

import { CONTRACTS, type ContractKey } from "../lib/contracts/registry";

interface ContractSelectorProps {
  selectedContract: ContractKey;
  contractIds: Record<ContractKey, string>;
  onSelect: (key: ContractKey) => void;
  onUpdateContractId: (key: ContractKey, id: string) => void;
  onResetContractId: (key: ContractKey) => void;
}

export function ContractSelector({
  selectedContract,
  contractIds,
  onSelect,
  onUpdateContractId,
  onResetContractId,
}: ContractSelectorProps) {
  const currentConfig = CONTRACTS[selectedContract];
  const currentId = contractIds[selectedContract];
  const isDefault = currentId === currentConfig.defaultContractId;

  return (
    <div className="mb-6 space-y-4">
      {/* Contract Tabs */}
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-6">
        {(Object.keys(CONTRACTS) as ContractKey[]).map((key) => {
          const cfg = CONTRACTS[key];
          const isSelected = selectedContract === key;

          return (
            <button
              key={key}
              type="button"
              onClick={() => onSelect(key)}
              className={`flex flex-col items-start rounded-xl p-3 text-left transition border ${
                isSelected
                  ? "border-indigo-600 bg-indigo-50/50 shadow-sm ring-2 ring-indigo-500/20 dark:bg-indigo-950/40 dark:border-indigo-500"
                  : "border-slate-200 bg-white hover:border-slate-300 hover:bg-slate-50 dark:border-slate-800 dark:bg-slate-900 dark:hover:border-slate-700"
              }`}
            >
              <div className="flex w-full items-center justify-between gap-1">
                <span className="font-semibold text-xs text-slate-900 dark:text-slate-100 truncate">
                  {cfg.name}
                </span>
                <span
                  className={`rounded px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wider ${
                    isSelected
                      ? "bg-indigo-600 text-white"
                      : "bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-400"
                  }`}
                >
                  {cfg.badge}
                </span>
              </div>
              <p className="mt-1 line-clamp-2 text-[11px] text-slate-500 dark:text-slate-400">
                {cfg.tagline}
              </p>
            </button>
          );
        })}
      </div>

      {/* Contract ID Configuration */}
      <div className="rounded-xl border border-slate-200 bg-slate-50/50 p-4 dark:border-slate-800 dark:bg-slate-900/50">
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex items-center gap-2">
            <span className="font-semibold text-sm text-slate-800 dark:text-slate-200">
              {currentConfig.name} Contract ID:
            </span>
            {!isDefault && (
              <span className="rounded bg-amber-500/10 px-2 py-0.5 text-[11px] font-medium text-amber-600 dark:text-amber-400 border border-amber-500/20">
                Custom Override
              </span>
            )}
          </div>
          <div className="flex items-center gap-2">
            {!isDefault && (
              <button
                type="button"
                onClick={() => onResetContractId(selectedContract)}
                className="text-xs text-indigo-600 hover:text-indigo-700 dark:text-indigo-400 font-medium underline"
              >
                Reset to Default
              </button>
            )}
          </div>
        </div>

        <div className="mt-2 flex gap-2">
          <input
            type="text"
            value={currentId}
            onChange={(e) => onUpdateContractId(selectedContract, e.target.value)}
            placeholder="C..."
            className="w-full rounded-lg border border-slate-300 bg-white px-3 py-1.5 font-mono text-xs text-slate-900 shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500 dark:border-slate-700 dark:bg-slate-950 dark:text-slate-100"
          />
        </div>
        <p className="mt-1.5 text-xs text-slate-500 dark:text-slate-400">
          {currentConfig.writeActionDoc}
        </p>
      </div>
    </div>
  );
}
