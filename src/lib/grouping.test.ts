import { describe, expect, it } from "vitest";
import {
  NO_PARENT,
  ROWS_SHOWN,
  effectiveParent,
  epicGroupStartsOpen,
  epicOptions,
  filterGroups,
  groupByMonth,
  groupByParent,
  monthKeyOf,
  monthStartsOpen,
  periodBounds,
  rowsToShow,
} from "./grouping";
import type { Task } from "./types";

let nextId = 1;
/** A task with sensible defaults; each test overrides only what it cares about. */
function task(over: Partial<Task> = {}): Task {
  const id = nextId++;
  return {
    id,
    title: `Task ${id}`,
    done: false,
    tracked_seconds: 0,
    running_since: null,
    jira_key: null,
    jira_status: null,
    unpushed_seconds: 0,
    created_at: 1_700_000_000 + id,
    issue_type: null,
    parent_key: null,
    pending_parent_key: null,
    jira_logged_seconds: null,
    context_only: false,
    story_title: null,
    story_description: null,
    story_source: null,
    ...over,
  };
}

const epic = (key: string, over: Partial<Task> = {}) => task({ jira_key: key, issue_type: "Epic", ...over });
const story = (key: string, parent: string | null, over: Partial<Task> = {}) =>
  task({ jira_key: key, issue_type: "Story", parent_key: parent, ...over });

describe("effectiveParent", () => {
  it("prefers the parent chosen here over the one Jira has", () => {
    expect(effectiveParent(task({ parent_key: "A-1", pending_parent_key: "A-2" }))).toBe("A-2");
    expect(effectiveParent(task({ parent_key: "A-1" }))).toBe("A-1");
    expect(effectiveParent(task())).toBeNull();
  });
});

describe("groupByParent", () => {
  it("puts stories under their epic and the rest under 'no parent'", () => {
    const e = epic("A-1", { context_only: true });
    const s1 = story("A-2", "A-1");
    const s2 = story("A-3", "A-1");
    const loose = task();
    const groups = groupByParent([e, s1, s2, loose]);

    expect(groups.map((g) => g.id)).toEqual(["A-1", NO_PARENT]);
    expect(groups[0].header).toBe(e);
    expect(groups[0].children.map((c) => c.jira_key).sort()).toEqual(["A-2", "A-3"]);
    expect(groups[1].children).toEqual([loose]);
  });

  it("never lists a heading-only parent as a work item, and hides it when it has no children", () => {
    const lonelyContext = epic("A-9", { context_only: true });
    expect(groupByParent([lonelyContext])).toEqual([]);
  });

  it("shows my own epic as a heading even before it has stories", () => {
    const mine = epic("A-1");
    const groups = groupByParent([mine]);
    expect(groups).toEqual([{ id: "A-1", header: mine, children: [] }]);
  });

  it("moves a task under a parent that is only staged here", () => {
    const e = epic("A-1", { context_only: true });
    const s = task({ title: "local task", pending_parent_key: "A-1" });
    const groups = groupByParent([e, s]);
    expect(groups).toHaveLength(1);
    expect(groups[0].children).toEqual([s]);
  });

  it("a staged parent beats Jira's parent", () => {
    const e1 = epic("A-1", { context_only: true });
    const e2 = epic("A-2", { context_only: true });
    const s = story("A-3", "A-1", { pending_parent_key: "A-2" });
    const groups = groupByParent([e1, e2, s]);
    expect(groups.map((g) => g.id)).toEqual(["A-2"]);
  });

  it("ignores a parent key that isn't in the list", () => {
    const s = story("A-3", "A-404");
    const groups = groupByParent([s]);
    expect(groups).toEqual([{ id: NO_PARENT, header: null, children: [s] }]);
  });

  it("survives a parent cycle", () => {
    const a = story("A-1", "A-2");
    const b = story("A-2", "A-1");
    const groups = groupByParent([a, b]);
    const all = groups.flatMap((g) => [...(g.header ? [g.header] : []), ...g.children]);
    expect(all).toHaveLength(2); // both still shown somewhere; no infinite loop
  });

  it("puts running and open tasks first, and a group with a running timer on top", () => {
    const e1 = epic("A-1", { context_only: true });
    const e2 = epic("A-2", { context_only: true });
    const old = story("A-3", "A-1", { created_at: 1000 });
    const finished = story("A-4", "A-1", { created_at: 3000, done: true });
    const running = story("A-5", "A-1", { created_at: 2000, running_since: 5 });
    const other = story("A-6", "A-2", { created_at: 9_000_000_000 });
    const groups = groupByParent([e1, e2, old, finished, running, other]);

    expect(groups[0].id).toBe("A-1"); // has the running timer, although A-2 is newer
    expect(groups[0].children.map((c) => c.jira_key)).toEqual(["A-5", "A-3", "A-4"]);
  });
});

describe("filterGroups", () => {
  const e = epic("A-1", { title: "Billing", context_only: true });
  const match = story("A-2", "A-1", { title: "Invoice PDF" });
  const other = story("A-3", "A-1", { title: "Refund email" });

  it("keeps only matching children", () => {
    const filtered = filterGroups(groupByParent([e, match, other]), (t) => t.title.includes("Invoice"));
    expect(filtered).toHaveLength(1);
    expect(filtered[0].children).toEqual([match]);
  });

  it("a matching heading keeps all its children", () => {
    const filtered = filterGroups(groupByParent([e, match, other]), (t) => t.title === "Billing");
    expect(filtered[0].children).toHaveLength(2);
  });

  it("drops groups with nothing left", () => {
    expect(filterGroups(groupByParent([e, match, other]), () => false)).toEqual([]);
  });
});

describe("epicOptions", () => {
  it("lists epics (including ones that aren't mine), sorted by title", () => {
    const options = epicOptions([epic("A-2", { title: "Zeta" }), story("A-3", null), epic("A-1", { title: "Alpha", context_only: true })]);
    expect(options).toEqual([
      { key: "A-1", title: "Alpha" },
      { key: "A-2", title: "Zeta" },
    ]);
  });
});

describe("groupByMonth", () => {
  const at = (y: number, m: number, d: number, h = 12) => Math.floor(new Date(y, m - 1, d, h).getTime() / 1000);

  it("groups by creation month, newest month first, skipping heading-only parents", () => {
    const oct1 = task({ created_at: at(2026, 10, 3) });
    const oct2 = task({ created_at: at(2026, 10, 20) });
    const sep = task({ created_at: at(2026, 9, 30) });
    const ctx = epic("A-1", { context_only: true, created_at: at(2026, 10, 1) });
    const months = groupByMonth([sep, oct1, ctx, oct2], "en-US");

    expect(months.map((m) => m.key)).toEqual(["2026-10", "2026-09"]);
    expect(months[0].tasks).toEqual([oct2, oct1]); // newest first
    expect(months[0].label).toBe("October 2026");
    expect(months[1].tasks).toEqual([sep]);
  });

  it("uses the local month, so the last evening of a month stays in that month", () => {
    expect(monthKeyOf(at(2026, 10, 31, 23))).toBe("2026-10");
    expect(monthKeyOf(at(2026, 11, 1, 0))).toBe("2026-11");
  });
});

describe("periodBounds", () => {
  it("returns the start of today, tomorrow, this month and next month", () => {
    const b = periodBounds(new Date(2026, 9, 8, 15, 30)); // 8 October 2026
    expect(b.dayEnd - b.dayStart).toBeGreaterThanOrEqual(23 * 3600); // 23-25 h around DST changes
    expect(new Date(b.monthStart * 1000).getDate()).toBe(1);
    expect(new Date(b.monthEnd * 1000).getMonth()).toBe(10); // November
  });
});

describe("keeping a long list short", () => {
  it("starts epic groups closed, except where you are working", () => {
    const quiet = { id: "ABC-1", header: epic("ABC-1"), children: [task({ parent_key: "ABC-1" })] };
    const withTimer = { id: "ABC-2", header: epic("ABC-2"), children: [task({ parent_key: "ABC-2", running_since: 5 })] };
    const loose = { id: NO_PARENT, header: null, children: [task()] };
    expect(epicGroupStartsOpen(quiet, false)).toBe(false);
    expect(epicGroupStartsOpen(withTimer, false)).toBe(true);
    expect(epicGroupStartsOpen(loose, false)).toBe(true);
    expect(epicGroupStartsOpen(quiet, true)).toBe(true); // a search wants to show its matches
  });

  it("opens the current month, a month with a running timer, and everything while searching", () => {
    const old = { key: "2026-01", tasks: [task()] };
    expect(monthStartsOpen(old, "2026-10", false)).toBe(false);
    expect(monthStartsOpen({ key: "2026-10", tasks: [] }, "2026-10", false)).toBe(true);
    expect(monthStartsOpen({ key: "2026-01", tasks: [task({ running_since: 9 })] }, "2026-10", false)).toBe(true);
    expect(monthStartsOpen(old, "2026-10", true)).toBe(true);
  });

  it("shows a limited number of rows until asked for all of them", () => {
    const rows = Array.from({ length: ROWS_SHOWN + 5 }, (_, i) => i);
    expect(rowsToShow(rows, false)).toHaveLength(ROWS_SHOWN);
    expect(rowsToShow(rows, true)).toHaveLength(ROWS_SHOWN + 5);
    expect(rowsToShow([1, 2], false)).toEqual([1, 2]);
  });
});
