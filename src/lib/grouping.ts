// How the task list is arranged. Pure functions (no Svelte, no Tauri), so they are unit-tested
// in grouping.test.ts. Two views:
//   "By epic"  : parent (epic) heading with its stories underneath, then everything without a parent
//   "By month" : tasks grouped by the month they were created

import type { Task } from "./types";

/** Where the task will sit: a parent chosen here wins over the one Jira has. */
export const effectiveParent = (t: Task): string | null => t.pending_parent_key ?? t.parent_key;

export const isEpic = (t: Task): boolean => t.issue_type?.toLowerCase() === "epic";

export type Group = {
  /** Jira key of the heading, or "__none__" for tasks without a parent. */
  id: string;
  /** The parent task shown as the group heading; null for the "No parent" group. */
  header: Task | null;
  children: Task[];
};

export const NO_PARENT = "__none__";

function newestFirstRunningFirst(a: Task, b: Task): number {
  const running = Number(b.running_since !== null) - Number(a.running_since !== null);
  if (running !== 0) return running;
  const open = Number(a.done) - Number(b.done); // open tasks before finished ones
  if (open !== 0) return open;
  return b.created_at - a.created_at;
}

/**
 * Follow parent links up to the top-most parent we know about.
 * Returns null when the task has no known parent. A cycle (A under B under A) can't loop
 * forever: we stop after `tasks.length` steps.
 */
function rootOf(task: Task, byKey: Map<string, Task>, limit: number): Task | null {
  let current = task;
  let steps = 0;
  while (steps++ <= limit) {
    const parentKey = effectiveParent(current);
    const parent = parentKey ? byKey.get(parentKey) : undefined;
    if (!parent) break;
    if (parent === task) return null; // cycle back to the start: treat as having no parent
    current = parent;
  }
  return current === task ? null : current;
}

export function groupByParent(tasks: Task[]): Group[] {
  const byKey = new Map<string, Task>();
  for (const t of tasks) if (t.jira_key) byKey.set(t.jira_key, t);

  const childrenOf = new Map<string, Task[]>();
  const loose: Task[] = [];
  const hasChildren = new Set<string>();

  for (const t of tasks) {
    if (t.context_only) continue; // headings only; never listed as a work item
    const root = rootOf(t, byKey, tasks.length);
    if (root?.jira_key) {
      hasChildren.add(root.jira_key);
      childrenOf.set(root.jira_key, [...(childrenOf.get(root.jira_key) ?? []), t]);
    } else {
      loose.push(t);
    }
  }

  const groups: Group[] = [];
  const placedAsHeader = new Set<number>();
  for (const [key, children] of childrenOf) {
    const header = byKey.get(key)!;
    placedAsHeader.add(header.id);
    groups.push({ id: key, header, children: children.sort(newestFirstRunningFirst) });
  }
  // My own epics that have no stories yet still deserve a heading.
  for (const t of loose) {
    if (isEpic(t) && t.jira_key && !hasChildren.has(t.jira_key)) {
      placedAsHeader.add(t.id);
      groups.push({ id: t.jira_key, header: t, children: [] });
    }
  }

  // Most recent activity first; a group with a running timer always on top.
  const newest = (g: Group) => Math.max(g.header?.created_at ?? 0, ...g.children.map((c) => c.created_at));
  const running = (g: Group) => [g.header, ...g.children].some((t) => t?.running_since != null);
  groups.sort((a, b) => Number(running(b)) - Number(running(a)) || newest(b) - newest(a));

  const ungrouped = loose.filter((t) => !placedAsHeader.has(t.id)).sort(newestFirstRunningFirst);
  if (ungrouped.length > 0) groups.push({ id: NO_PARENT, header: null, children: ungrouped });
  return groups;
}

/** Keep groups/children matching `keep`. A matching heading keeps all of its children. */
export function filterGroups(groups: Group[], keep: (t: Task) => boolean): Group[] {
  return groups
    .map((g) => {
      if (g.header && keep(g.header)) return g;
      return { ...g, children: g.children.filter(keep) };
    })
    .filter((g) => g.children.length > 0 || (g.header !== null && keep(g.header)));
}

// ---- keeping a long list short ----------------------------------------------------------

/** A group shows this many rows until you ask for the rest. */
export const ROWS_SHOWN = 8;

/**
 * Whether a group starts open. Groups start closed so a long list stays a short list of headings;
 * these stay open because you are in them: a running timer, tasks that aren't under any epic, or a
 * search/filter that is looking for something. A choice made with the arrow always wins over this.
 */
export function epicGroupStartsOpen(g: Group, searching: boolean): boolean {
  return searching || g.id === NO_PARENT || [g.header, ...g.children].some((t) => t?.running_since != null);
}

export function monthStartsOpen(m: { key: string; tasks: Task[] }, currentKey: string, searching: boolean): boolean {
  return searching || m.key === currentKey || m.tasks.some((t) => t.running_since !== null);
}

/** The first `ROWS_SHOWN` rows, or all of them when `all`. */
export function rowsToShow<T>(rows: T[], all: boolean): T[] {
  return all ? rows : rows.slice(0, ROWS_SHOWN);
}

/** Parents a task can be placed under: my epics and the epics Jira told us about. */
export function epicOptions(tasks: Task[]): { key: string; title: string }[] {
  return tasks
    .filter((t) => isEpic(t) && t.jira_key)
    .map((t) => ({ key: t.jira_key!, title: t.title }))
    .sort((a, b) => a.title.localeCompare(b.title));
}

// ---- months -----------------------------------------------------------------------------

export type MonthGroup = { key: string; label: string; tasks: Task[] };

/** "2026-10" in the user's own time zone (so a task created at 00:30 local counts for that day). */
export function monthKeyOf(unixSeconds: number): string {
  const d = new Date(unixSeconds * 1000);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
}

export function monthLabel(key: string, locale?: string): string {
  const [year, month] = key.split("-").map(Number);
  return new Date(year, month - 1, 1).toLocaleDateString(locale, { month: "long", year: "numeric" });
}

/** Tasks grouped by the month they were created, newest month first. */
export function groupByMonth(tasks: Task[], locale?: string): MonthGroup[] {
  const buckets = new Map<string, Task[]>();
  for (const t of tasks) {
    if (t.context_only) continue;
    const key = monthKeyOf(t.created_at);
    buckets.set(key, [...(buckets.get(key) ?? []), t]);
  }
  return [...buckets.entries()]
    .sort(([a], [b]) => b.localeCompare(a))
    .map(([key, list]) => ({
      key,
      label: monthLabel(key, locale),
      tasks: list.sort((a, b) => b.created_at - a.created_at),
    }));
}

/** [start of today, start of tomorrow, start of this month, start of next month], Unix seconds. */
export function periodBounds(now: Date = new Date()): { dayStart: number; dayEnd: number; monthStart: number; monthEnd: number } {
  const sec = (d: Date) => Math.floor(d.getTime() / 1000);
  return {
    dayStart: sec(new Date(now.getFullYear(), now.getMonth(), now.getDate())),
    dayEnd: sec(new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1)),
    monthStart: sec(new Date(now.getFullYear(), now.getMonth(), 1)),
    monthEnd: sec(new Date(now.getFullYear(), now.getMonth() + 1, 1)),
  };
}
