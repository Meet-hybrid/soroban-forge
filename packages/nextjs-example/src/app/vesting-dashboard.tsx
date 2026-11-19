"use client";

import { useCallback, useEffect, useState } from "react";
import { vesting } from "@soroban-forge/escrow-client";

type VestingClient = InstanceType<typeof vesting.Client>;
type VestingSchedule = vesting.VestingSchedule;
type VestingStatus = vesting.VestingStatus;

const ZERO = BigInt(0);
const MAX_SCHEDULE_ID = BigInt("18446744073709551615");

interface VestingDashboardProps {
  client: VestingClient | null;
  account: string | null;
  enabled: boolean;
  ensureTestnetWallet: () => Promise<boolean>;
}

interface ScheduleView {
  id: bigint;
  schedule: VestingSchedule;
  status: VestingStatus;
  claimable: bigint;
}

const STATUS_STYLES: Record<VestingStatus["tag"], string> = {
  Locked: "bg-amber-100 text-amber-800",
  Vesting: "bg-blue-100 text-blue-800",
  Completed: "bg-emerald-100 text-emerald-800",
  Revoked: "bg-red-100 text-red-800",
};

function messageForError(error: unknown): string {
  const message =
    error instanceof Error
      ? error.message
      : typeof error === "object" &&
          error !== null &&
          "message" in error &&
          typeof error.message === "string"
        ? error.message
        : String(error);
  if (
    /(?:network|passphrase|testnet).*(?:mismatch|does not match|different|wrong|expected|switch)|(?:mismatch|does not match|different|wrong).*(?:network|passphrase)/i.test(
      message,
    )
  ) {
    return `Wallet network mismatch. Switch the connected wallet to Stellar Testnet and try again. ${message}`;
  }
  return message || "The vesting request failed.";
}

function amountPercentage(amount: bigint, total: bigint): number {
  if (total <= ZERO || amount <= ZERO) {
    return 0;
  }
  const boundedAmount = amount > total ? total : amount;
  return Number((boundedAmount * BigInt(10_000)) / total) / 100;
}

function unlockedAmount(schedule: VestingSchedule, now: bigint): bigint {
  if (schedule.revoked_vested !== undefined) {
    return schedule.revoked_vested;
  }

  const cliffTime = schedule.start + schedule.cliff;
  if (now < cliffTime) {
    return ZERO;
  }

  const endTime = schedule.start + schedule.duration;
  if (now >= endTime) {
    return schedule.total_amount;
  }

  const vestingPeriod = endTime - cliffTime;
  if (vestingPeriod <= ZERO) {
    return schedule.total_amount;
  }

  return (schedule.total_amount * (now - cliffTime)) / BigInt(vestingPeriod);
}

function formatStart(timestamp: bigint): string {
  const milliseconds = Number(timestamp) * 1000;
  if (!Number.isFinite(milliseconds)) {
    return `${timestamp.toString()} seconds`;
  }
  const date = new Date(milliseconds);
  return Number.isNaN(date.getTime())
    ? `${timestamp.toString()} seconds`
    : date.toLocaleString();
}

async function readSchedule(
  client: VestingClient,
  id: bigint,
): Promise<ScheduleView> {
  const [scheduleTransaction, statusTransaction, claimableTransaction] =
    await Promise.all([
      client.get_schedule({ schedule_id: id }),
      client.get_status({ schedule_id: id }),
      client.claimable({ schedule_id: id }),
    ]);

  if (!scheduleTransaction.result.isOk()) {
    throw new Error(
      `Could not load schedule #${id.toString()}: ${scheduleTransaction.result.unwrapErr().message}`,
    );
  }
  if (!statusTransaction.result.isOk()) {
    throw new Error(
      `Could not read the status for schedule #${id.toString()}: ${statusTransaction.result.unwrapErr().message}`,
    );
  }
  if (!claimableTransaction.result.isOk()) {
    throw new Error(
      `Could not read claimable tokens for schedule #${id.toString()}: ${claimableTransaction.result.unwrapErr().message}`,
    );
  }

  return {
    id,
    schedule: scheduleTransaction.result.unwrap(),
    status: statusTransaction.result.unwrap(),
    claimable: claimableTransaction.result.unwrap(),
  };
}

export default function VestingDashboard({
  client,
  account,
  enabled,
  ensureTestnetWallet,
}: VestingDashboardProps) {
  const [scheduleId, setScheduleId] = useState("1");
  const [view, setView] = useState<ScheduleView | null>(null);
  const [busy, setBusy] = useState<"loading" | "claiming" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [networkVerified, setNetworkVerified] = useState<boolean | null>(null);
  const [now, setNow] = useState(ZERO);

  useEffect(() => {
    const timer = window.setInterval(
      () => setNow(BigInt(Math.floor(Date.now() / 1000))),
      30_000,
    );
    return () => window.clearInterval(timer);
  }, []);

  const loadSchedule = useCallback(
    async (id: bigint) => {
      if (!client || !enabled) {
        setError("Switch to Live Testnet and connect a wallet to inspect schedules.");
        return null;
      }

      setBusy("loading");
      setError(null);
      setNotice(null);
      try {
        const verified = await ensureTestnetWallet();
        setNetworkVerified(verified);
        const result = await readSchedule(client, id);
        setNow(BigInt(Math.floor(Date.now() / 1000)));
        setView(result);
        return result;
      } catch (requestError) {
        setView(null);
        setError(messageForError(requestError));
        return null;
      } finally {
        setBusy(null);
      }
    },
    [client, enabled, ensureTestnetWallet],
  );

  const handleLookup = useCallback(async () => {
    const rawId = scheduleId.trim();
    if (!/^\d+$/.test(rawId)) {
      setError("Enter a positive schedule ID.");
      setView(null);
      setNotice(null);
      return;
    }
    const id = BigInt(rawId);
    if (id === ZERO || id > MAX_SCHEDULE_ID) {
      setError("Enter a schedule ID between 1 and 18,446,744,073,709,551,615.");
      setView(null);
      setNotice(null);
      return;
    }
    await loadSchedule(id);
  }, [loadSchedule, scheduleId]);

  const handleClaim = useCallback(async () => {
    if (!client || !view || !account || !enabled) {
      setError("Switch to Live Testnet and connect the schedule beneficiary wallet to claim.");
      return;
    }
    if (view.schedule.beneficiary !== account) {
      setError("Only the schedule beneficiary can claim with this wallet.");
      return;
    }
    if (view.claimable <= ZERO) {
      setError("This schedule has no tokens available to claim.");
      return;
    }

    setBusy("claiming");
    setError(null);
    setNotice(null);
    try {
      const verified = await ensureTestnetWallet();
      setNetworkVerified(verified);
      const transaction = await client.claim({ schedule_id: view.id });
      if (!transaction.result.isOk()) {
        throw new Error(
          `Claim simulation failed: ${transaction.result.unwrapErr().message}`,
        );
      }

      const sent = await transaction.signAndSend();
      const claimed = sent.result.unwrap();
      setNow(BigInt(Math.floor(Date.now() / 1000)));
      setNotice(
        `Claim confirmed. ${claimed.toString()} token base units were claimed.${verified ? "" : " Confirm the wallet is set to Testnet; this wallet cannot report its selected network."}`,
      );

      try {
        const refreshed = await readSchedule(client, view.id);
        setView(refreshed);
      } catch (refreshError) {
        setError(
          `Claim confirmed, but the schedule refresh failed: ${messageForError(refreshError)}`,
        );
      }
    } catch (claimError) {
      setError(messageForError(claimError));
    } finally {
      setBusy(null);
    }
  }, [account, client, enabled, ensureTestnetWallet, view]);

  const unlocked = view
    ? unlockedAmount(view.schedule, now)
    : ZERO;
  const unlockedPercent = view
    ? amountPercentage(unlocked, view.schedule.total_amount)
    : 0;
  const claimedPercent = view
    ? amountPercentage(view.schedule.claimed, view.schedule.total_amount)
    : 0;
  const canClaim =
    enabled &&
    Boolean(account) &&
    view !== null &&
    view.schedule.beneficiary === account &&
    view.claimable > ZERO &&
    busy === null;

  return (
    <section className="mt-6 rounded-xl border border-gray-300 bg-white p-5 shadow-sm">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div>
          <h2 className="text-lg font-semibold">Vesting Schedule Dashboard</h2>
          <p className="mt-1 text-xs text-gray-500">
            Inspect a linear schedule and claim unlocked tokens with its beneficiary wallet.
          </p>
        </div>
        <span className="rounded bg-gray-100 px-2 py-0.5 text-[11px] font-mono text-gray-600">
          {enabled ? "Live Testnet" : "Connect to Testnet"}
        </span>
      </div>

      <form
        className="mb-4 flex flex-wrap gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          void handleLookup();
        }}
      >
        <label className="sr-only" htmlFor="vesting-schedule-id">
          Schedule ID
        </label>
        <input
          id="vesting-schedule-id"
          inputMode="numeric"
          value={scheduleId}
          onChange={(event) => setScheduleId(event.target.value)}
          placeholder="Schedule ID"
          className="min-w-0 flex-1 rounded border border-gray-300 px-2 py-1.5 font-mono text-xs focus:border-blue-500 focus:outline-none"
        />
        <button
          type="submit"
          disabled={!enabled || !client || !account || busy !== null}
          className="rounded bg-gray-800 px-4 py-1.5 text-xs font-bold text-white hover:bg-gray-900 disabled:opacity-50"
        >
          {busy === "loading" ? "Loading…" : "Inspect Schedule"}
        </button>
      </form>

      {!enabled && (
        <p className="mb-3 rounded border border-amber-200 bg-amber-50 p-3 text-xs text-amber-800">
          Switch to Live Testnet to query and claim vesting schedules.
        </p>
      )}
      {enabled && !account && (
        <p className="mb-3 rounded border border-amber-200 bg-amber-50 p-3 text-xs text-amber-800">
          Connect a Stellar Testnet wallet using the wallet controls above to inspect schedules.
        </p>
      )}
      {enabled && account && networkVerified === false && (
        <p className="mb-3 rounded border border-amber-200 bg-amber-50 p-3 text-xs text-amber-800">
          This wallet cannot report its selected network. Confirm it is set to Stellar Testnet; requests use the Testnet RPC.
        </p>
      )}
      {error && (
        <p role="alert" className="mb-3 rounded border border-red-200 bg-red-50 p-3 text-xs text-red-800">
          {error}
        </p>
      )}
      {notice && (
        <p role="status" className="mb-3 rounded border border-emerald-200 bg-emerald-50 p-3 text-xs text-emerald-800">
          {notice}
        </p>
      )}

      {view && (
        <div className="space-y-4">
          <dl className="grid grid-cols-1 gap-x-4 gap-y-2 text-xs sm:grid-cols-2">
            <dt className="font-semibold text-gray-500">Schedule ID</dt>
            <dd className="font-mono font-bold">#{view.id.toString()}</dd>

            <dt className="font-semibold text-gray-500">Status</dt>
            <dd>
              <span
                className={`rounded-full px-2 py-0.5 text-[11px] font-bold ${STATUS_STYLES[view.status.tag]}`}
              >
                {view.status.tag}
              </span>
            </dd>

            <dt className="font-semibold text-gray-500">Funder</dt>
            <dd className="break-all font-mono text-gray-700">{view.schedule.funder}</dd>

            <dt className="font-semibold text-gray-500">Beneficiary</dt>
            <dd className="break-all font-mono text-gray-700">{view.schedule.beneficiary}</dd>

            <dt className="font-semibold text-gray-500">Token</dt>
            <dd className="break-all font-mono text-gray-700">{view.schedule.token}</dd>

            <dt className="font-semibold text-gray-500">Start</dt>
            <dd>{formatStart(view.schedule.start)}</dd>

            <dt className="font-semibold text-gray-500">Cliff</dt>
            <dd>{view.schedule.cliff.toString()} seconds after start</dd>

            <dt className="font-semibold text-gray-500">Duration</dt>
            <dd>{view.schedule.duration.toString()} seconds</dd>

            <dt className="font-semibold text-gray-500">Claimable now</dt>
            <dd className="font-mono">{view.claimable.toString()} base units</dd>
          </dl>

          <div>
            <div className="mb-1 flex justify-between text-xs">
              <span className="font-semibold text-gray-700">Unlock and claim progress</span>
              <span className="text-gray-600">
                {unlockedPercent.toFixed(2)}% unlocked · {claimedPercent.toFixed(2)}% claimed
              </span>
            </div>
            <div
              role="progressbar"
              aria-label="Percentage of schedule unlocked"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={unlockedPercent}
              aria-valuetext={`${unlockedPercent.toFixed(2)}% unlocked and ${claimedPercent.toFixed(2)}% claimed`}
              className="relative h-3 overflow-hidden rounded-full bg-gray-200"
            >
              <div
                className="h-full rounded-full bg-emerald-200"
                style={{ width: `${unlockedPercent}%` }}
              />
              <div
                className="absolute inset-y-0 left-0 rounded-full bg-emerald-600"
                style={{ width: `${claimedPercent}%` }}
              />
            </div>
            <div className="mt-1 flex gap-4 text-[11px] text-gray-500">
              <span><span className="mr-1 inline-block h-2 w-2 rounded-full bg-emerald-200" />Unlocked</span>
              <span><span className="mr-1 inline-block h-2 w-2 rounded-full bg-emerald-600" />Claimed</span>
            </div>
          </div>

          {account && view.schedule.beneficiary !== account && (
            <p className="text-xs text-gray-600">
              Connect the beneficiary wallet to claim this schedule.
            </p>
          )}
          <button
            type="button"
            onClick={() => void handleClaim()}
            disabled={!canClaim}
            className="rounded bg-blue-600 px-4 py-2 text-xs font-bold text-white hover:bg-blue-700 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {busy === "claiming" ? "Claiming…" : "Claim"}
          </button>
        </div>
      )}
    </section>
  );
}
