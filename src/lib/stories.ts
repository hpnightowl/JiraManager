// Numbers for the Stories tab. Pure functions (no Svelte, no Tauri), tested in stories.test.ts.

import { monthKeyOf, monthLabel } from "./grouping";
import type { StoryRecord } from "./types";

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
