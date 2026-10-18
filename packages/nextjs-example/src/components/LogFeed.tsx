"use client";

export interface LogEntry {
  id: number;
  time: string;
  text: string;
  kind: "info" | "ok" | "err";
}

interface LogFeedProps {
  logs: LogEntry[];
  onClear: () => void;
}

export function LogFeed({ logs, onClear }: LogFeedProps) {
  return (
    <section className="mt-8">
      <div className="flex items-center justify-between mb-2">
        <h3 className="font-semibold text-sm text-slate-800 dark:text-slate-200">
          Activity &amp; Transaction Log ({logs.length})
        </h3>
        {logs.length > 0 && (
          <button
            type="button"
            onClick={onClear}
            className="text-xs text-slate-400 hover:text-slate-600 dark:hover:text-slate-200"
          >
            Clear Log
          </button>
        )}
      </div>

      <div className="max-h-64 overflow-y-auto rounded-xl border border-slate-900 bg-slate-950 p-3.5 font-mono text-xs text-slate-300 shadow-inner">
        {logs.length === 0 ? (
          <p className="text-slate-600">No activity logged yet...</p>
        ) : (
          logs.map((entry) => (
            <p
              key={entry.id}
              className={`mb-1 break-all ${
                entry.kind === "err"
                  ? "text-rose-400"
                  : entry.kind === "ok"
                    ? "text-emerald-400"
                    : "text-slate-300"
              }`}
            >
              <span className="text-slate-600">[{entry.time}] </span>
              {entry.text}
            </p>
          ))
        )}
      </div>
    </section>
  );
}
