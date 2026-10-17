"use client";

import { useCallback, useEffect, useMemo, useState } from "react";

type Issue = {
  number: number;
  title: string;
  html_url: string;
  state: "open" | "closed";
  updated_at: string;
  assignees: Array<{ login: string }>;
  labels: Array<{ name: string; description?: string }>;
};

type Filter = "open" | "needs" | "outside" | "assigned" | "people" | "closed" | "all";

const API =
  "https://api.github.com/repos/Meet-hybrid/soroban-forge/issues?state=all&per_page=100";
const FALLBACK: Issue[] = [];

function isWave(issue: Issue) {
  return issue.labels.some(
    (label) =>
      label.name.toLowerCase() === "stellar wave" ||
      /approved issue for .+ wave program/i.test(label.description ?? "")
  );
}

function age(updated: string) {
  const minutes = Math.max(0, Math.floor((Date.now() - Date.parse(updated)) / 60000));
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return days < 30 ? `${days}d ago` : new Date(updated).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function Stat({ label, value, tone = "" }: { label: string; value: number; tone?: string }) {
  return <div className="issue-stat"><div className={`issue-stat-number ${tone}`}>{value}</div><div className="issue-stat-label">{label}</div></div>;
}

function IssueCard({ issue }: { issue: Issue }) {
  const assigned = issue.assignees.map((assignee) => assignee.login);
  return (
    <article className="issue-card">
      <div className="issue-card-top">
        <a href={issue.html_url} target="_blank" rel="noreferrer" className="issue-number">#{issue.number} ↗</a>
        <span className="issue-pill">{issue.state === "open" ? "Open" : "Closed"}</span>
        <span className={`issue-pill ${isWave(issue) ? "wave" : "muted"}`}>{isWave(issue) ? "Stellar Wave" : "No Wave label"}</span>
      </div>
      <a href={issue.html_url} target="_blank" rel="noreferrer" className="issue-title">{issue.title}</a>
      <div className="issue-card-foot">
        <span className={assigned.length ? "" : "unassigned"}>◉ {assigned.length ? assigned.join(", ") : "Unassigned"}</span>
        <span>{age(issue.updated_at)}</span>
      </div>
    </article>
  );
}

export default function IssuesPage() {
  const [issues, setIssues] = useState<Issue[]>(FALLBACK);
  const [filter, setFilter] = useState<Filter>("open");
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState("Loading GitHub…");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const refresh = useCallback(async () => {
    if (loading) return;
    setLoading(true);
    setError(null);
    try {
      const response = await fetch(`${API}&_=${Date.now()}`, { headers: { Accept: "application/vnd.github+json" } });
      if (!response.ok) throw new Error(`GitHub returned ${response.status}`);
      const data = (await response.json()) as Array<Issue & { pull_request?: unknown }>;
      setIssues(data.filter((issue) => !issue.pull_request));
      setStatus(`Live · updated ${new Date().toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}`);
    } catch (cause) {
      setError(`Could not refresh GitHub issues. ${cause instanceof Error ? cause.message : "Showing the last available list."}`);
      setStatus("Showing saved data");
    } finally {
      setLoading(false);
    }
  }, [loading]);

  // The initial fetch synchronizes this client page with GitHub after hydration.
  // eslint-disable-next-line react-hooks/set-state-in-effect
  useEffect(() => { void refresh(); }, [refresh]);

  const counts = useMemo(() => {
    const open = issues.filter((issue) => issue.state === "open");
    const wave = open.filter(isWave);
    return {
      open: open.length,
      wave: wave.length,
      assigned: wave.filter((issue) => issue.assignees.length > 0).length,
      unassigned: wave.filter((issue) => issue.assignees.length === 0).length,
      outside: open.filter((issue) => !isWave(issue)).length,
      closed: issues.filter((issue) => issue.state === "closed").length,
    };
  }, [issues]);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return issues.filter((issue) => {
      const searchable = `${issue.number} ${issue.title} ${issue.assignees.map((a) => a.login).join(" ")}`.toLowerCase();
      if (needle && !searchable.includes(needle)) return false;
      if (filter === "needs") return issue.state === "open" && isWave(issue) && issue.assignees.length === 0;
      if (filter === "outside") return issue.state === "open" && !isWave(issue);
      if (filter === "assigned") return issue.state === "open" && isWave(issue) && issue.assignees.length > 0;
      if (filter === "closed") return issue.state === "closed";
      if (filter === "all" || filter === "people") return true;
      return issue.state === "open";
    }).sort((a, b) => Date.parse(b.updated_at) - Date.parse(a.updated_at));
  }, [filter, issues, query]);

  const lanes = useMemo(() => {
    if (filter === "people") {
      const people = new Map<string, Issue[]>();
      filtered.filter((issue) => issue.state === "open" && issue.assignees.length > 0).forEach((issue) => issue.assignees.forEach(({ login }) => {
        people.set(login, [...(people.get(login) ?? []), issue]);
      }));
      return [...people.entries()].map(([name, items]) => ({ name, subtitle: `${items.length} assigned issue${items.length === 1 ? "" : "s"}`, items }));
    }
    if (filter === "closed") return [{ name: "Closed issues", subtitle: "Completed or otherwise closed", items: filtered }];
    const open = filtered.filter((issue) => issue.state === "open");
    return [
      { name: "Open · no Wave label", subtitle: "Not currently labeled for Stellar Wave", items: open.filter((issue) => !isWave(issue)) },
      { name: "Wave-labeled · unassigned", subtitle: "Waiting for an assignee", items: open.filter((issue) => isWave(issue) && issue.assignees.length === 0) },
      { name: "Wave-labeled · assigned", subtitle: "Assigned on GitHub", items: open.filter((issue) => isWave(issue) && issue.assignees.length > 0) },
      ...(filter === "all" ? [{ name: "Closed", subtitle: "Closed GitHub issues", items: filtered.filter((issue) => issue.state === "closed") }] : []),
    ];
  }, [filter, filtered]);

  const tabs: Array<[Filter, string, number]> = [["open", "Open", counts.open], ["needs", "Wave unassigned", counts.unassigned], ["outside", "No Wave label", counts.outside], ["assigned", "Wave assigned", counts.assigned], ["people", "By assignee", new Set(issues.flatMap((issue) => issue.assignees.map((a) => a.login))).size], ["closed", "Closed", counts.closed], ["all", "All issues", issues.length]];

  return (
    <main className="issue-shell">
      <header className="issue-topbar">
        <div className="issue-brand"><div className="issue-mark">SF</div><div><div className="issue-eyebrow">Contributor distribution</div><h1>Soroban Forge issue board</h1></div></div>
        <div className="issue-actions"><span className="issue-sync"><span className="issue-dot" />{status}</span><button className="issue-button" onClick={() => void refresh()} disabled={loading}>{loading ? "Refreshing…" : "↻ Refresh"}</button></div>
      </header>
      <div className="issue-intro"><span><strong>One live view of the Soroban Forge queue.</strong> Track ownership, Wave labels, and contribution opportunities from GitHub.</span><code>Meet-hybrid / soroban-forge</code></div>
      <section className="issue-stats"><Stat label="Open issues" value={counts.open} tone="blue" /><Stat label="Stellar Wave label" value={counts.wave} tone="green" /><Stat label="Wave assigned" value={counts.assigned} tone="blue" /><Stat label="Wave unassigned" value={counts.unassigned} tone="amber" /><Stat label="No Wave label" value={counts.outside} tone="amber" /><Stat label="Closed" value={counts.closed} /></section>
      {error && <div className="issue-error">{error}</div>}
      <div className="issue-controls"><div className="issue-tabs">{tabs.map(([id, label, count]) => <button key={id} className={`issue-tab ${filter === id ? "selected" : ""}`} onClick={() => setFilter(id)}>{label} <span>{count}</span></button>)}</div><input className="issue-search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search title, issue number, contributor" /></div>
      <section className={`issue-board ${filter === "people" ? "people" : filter === "closed" ? "single" : filter === "all" ? "four" : ""}`}>
        {issues.length === 0 ? <div className="issue-loading">Loading issues from GitHub…</div> : lanes.map((lane) => <section className="issue-lane" key={lane.name}><div className="issue-lane-head"><div><strong>{lane.name}</strong><small>{lane.subtitle}</small></div><b>{lane.items.length}</b></div><div className="issue-cards">{lane.items.length ? lane.items.map((issue) => <IssueCard key={issue.number} issue={issue} />) : <div className="issue-empty">No issues in this view</div>}</div></section>)}
      </section>
      <p className="issue-footnote">This board reads public GitHub issues, assignees, and labels. The Stellar Wave label does not confirm Drips approval or issue availability. Data refreshes when opened and when you press Refresh. <a href="https://github.com/Meet-hybrid/soroban-forge/issues" target="_blank" rel="noreferrer">Open all issues ↗</a></p>
    </main>
  );
}
