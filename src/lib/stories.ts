// Numbers for the Stories tab. Pure functions (no Svelte, no Tauri), tested in stories.test.ts.

import { monthKeyOf, monthLabel } from "./grouping";
import type { DoneRecord, StoryRecord } from "./types";

export type MonthRow = { key: string; label: string; app: number; jira: number };

/** How many fall in the month containing `now` (the user's time zone). */
export function inMonth(records: StoryRecord[], now: Date = new Date()): number {
  const key = monthKeyOf(Math.floor(now.getTime() / 1000));
  return records.filter((r) => monthKeyOf(r.created_at) === key).length;
}

/** Month by month, newest first: how many you created with the app and how many Jira says you reported. */
export function perMonth(app: StoryRecord[], jira: StoryRecord[]): MonthRow[] {
  const rows = new Map<string, MonthRow>();
  const row = (key: string) => {
    let r = rows.get(key);
    if (!r) rows.set(key, (r = { key, label: monthLabel(key), app: 0, jira: 0 }));
    return r;
  };
  for (const r of app) row(monthKeyOf(r.created_at)).app++;
  for (const r of jira) if (r.created_at > 0) row(monthKeyOf(r.created_at)).jira++;
  return [...rows.values()].sort((a, b) => b.key.localeCompare(a.key));
}

/** Newest first, de-duplicated by key when the same issue is in both lists (the Jira copy has more detail). */
export function recentStories(app: StoryRecord[], jira: StoryRecord[], limit: number): (StoryRecord & { viaApp: boolean })[] {
  const appKeys = new Set(app.map((r) => r.jira_key));
  const merged = new Map<string, StoryRecord & { viaApp: boolean }>();
  for (const r of app) merged.set(r.jira_key, { ...r, viaApp: true });
  for (const r of jira) merged.set(r.jira_key, { ...r, viaApp: appKeys.has(r.jira_key) });
  return [...merged.values()].sort((a, b) => b.created_at - a.created_at).slice(0, limit);
}

// ---- what I have finished ---------------------------------------------------------------

export type Kind = "story" | "defect" | "other";

/** Bugs and defects are one group, stories another; Task, Epic, Improvement... are "other". */
export function kindOf(issueType: string | null): Kind {
  const t = (issueType ?? "").toLowerCase();
  if (t.includes("bug") || t.includes("defect")) return "defect";
  if (t.includes("story")) return "story";
  return "other";
}

export type Tally = { story: number; defect: number; other: number; total: number };
const empty = (): Tally => ({ story: 0, defect: 0, other: 0, total: 0 });
function add(t: Tally, kind: Kind) {
  t[kind]++;
  t.total++;
}

/** How many I finished, overall and in the month containing `now`. Issues without a date count in
 *  the total but never in a month: guessing a month would make the numbers wrong. */
export function doneTotals(done: DoneRecord[], now: Date = new Date()): { all: Tally; thisMonth: Tally } {
  const key = monthKeyOf(Math.floor(now.getTime() / 1000));
  const all = empty();
  const thisMonth = empty();
  for (const d of done) {
    const kind = kindOf(d.issue_type);
    add(all, kind);
    if (d.done_at > 0 && monthKeyOf(d.done_at) === key) add(thisMonth, kind);
  }
  return { all, thisMonth };
}

export type DoneMonth = Tally & { key: string; label: string };

/** Month by month (newest first) of what I finished. */
export function doneByMonth(done: DoneRecord[]): DoneMonth[] {
  const rows = new Map<string, DoneMonth>();
  for (const d of done) {
    if (d.done_at <= 0) continue;
    const key = monthKeyOf(d.done_at);
    let row = rows.get(key);
    if (!row) rows.set(key, (row = { key, label: monthLabel(key), ...empty() }));
    add(row, kindOf(d.issue_type));
  }
  return [...rows.values()].sort((a, b) => b.key.localeCompare(a.key));
}

/** The issue types present, with counts, biggest first (for the filter chips). */
export function doneTypes(done: DoneRecord[]): [string, number][] {
  const counts = new Map<string, number>();
  for (const d of done) counts.set(d.issue_type ?? "Unknown", (counts.get(d.issue_type ?? "Unknown") ?? 0) + 1);
  return [...counts].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
}

/** Newest first; `type` "" = every type, `month` "" = every month, "unknown" = the ones with no date. */
export function filterDone(done: DoneRecord[], type: string, month: string): DoneRecord[] {
  return done
    .filter((d) => (type === "" || (d.issue_type ?? "Unknown") === type))
    .filter((d) => month === "" || (month === "unknown" ? d.done_at <= 0 : d.done_at > 0 && monthKeyOf(d.done_at) === month))
    .sort((a, b) => b.done_at - a.done_at || a.jira_key.localeCompare(b.jira_key));
}
