import { describe, expect, it } from "vitest";
import { inMonth, perMonth, recentStories } from "./stories";
import type { StoryRecord } from "./types";

const at = (y: number, m: number, d: number) => Math.floor(new Date(y, m - 1, d, 12).getTime() / 1000);
const rec = (key: string, created_at: number): StoryRecord => ({ jira_key: key, title: `Title ${key}`, issue_type: "Story", status: null, created_at });

describe("inMonth", () => {
  it("counts only the month containing 'now'", () => {
    const list = [rec("A-1", at(2026, 10, 2)), rec("A-2", at(2026, 10, 30)), rec("A-3", at(2026, 9, 30))];
    expect(inMonth(list, new Date(2026, 9, 15))).toBe(2);
    expect(inMonth([], new Date(2026, 9, 15))).toBe(0);
  });
});

describe("perMonth", () => {
  it("lines the two counts up per month, newest first", () => {
    const rows = perMonth([rec("A-3", at(2026, 10, 3))], [rec("A-1", at(2026, 8, 1)), rec("A-2", at(2026, 10, 1)), rec("A-3", at(2026, 10, 3))]);
    expect(rows.map((r) => [r.key, r.app, r.jira])).toEqual([
      ["2026-10", 1, 2],
      ["2026-08", 0, 1],
    ]);
  });

  it("ignores Jira rows with no date instead of inventing 1970", () => {
    expect(perMonth([], [rec("A-1", 0)])).toEqual([]);
  });
});

describe("recentStories", () => {
  it("lists newest first, once per issue, and says which came through the app", () => {
    const app = [rec("A-3", at(2026, 10, 3))];
    const jira = [rec("A-3", at(2026, 10, 3)), rec("A-2", at(2026, 10, 1)), rec("A-1", at(2026, 9, 1))];
    const list = recentStories(app, jira, 2);
    expect(list.map((r) => [r.jira_key, r.viaApp])).toEqual([["A-3", true], ["A-2", false]]);
  });
});

import { doneByMonth, doneTotals, doneTypes, filterDone, kindOf } from "./stories";
import type { DoneRecord } from "./types";

const done = (key: string, type: string | null, done_at: number): DoneRecord => ({ jira_key: key, title: `T ${key}`, issue_type: type, status: "Done", done_at });

describe("kindOf", () => {
  it("groups bugs and defects, stories, and everything else", () => {
    expect(kindOf("Bug")).toBe("defect");
    expect(kindOf("Production Defect")).toBe("defect");
    expect(kindOf("Story")).toBe("story");
    expect(kindOf("User Story")).toBe("story");
    expect(kindOf("Task")).toBe("other");
    expect(kindOf(null)).toBe("other");
  });
});

describe("what I finished", () => {
  const list = [
    done("A-1", "Story", at(2026, 10, 3)),
    done("A-2", "Bug", at(2026, 10, 20)),
    done("A-3", "Bug", at(2026, 9, 28)),
    done("A-4", "Task", at(2026, 9, 1)),
    done("A-5", "Story", 0), // no date from Jira
  ];

  it("counts stories and defects overall and this month, without guessing months for undated ones", () => {
    const t = doneTotals(list, new Date(2026, 9, 25));
    expect(t.all).toEqual({ story: 2, defect: 2, other: 1, total: 5 });
    expect(t.thisMonth).toEqual({ story: 1, defect: 1, other: 0, total: 2 });
  });

  it("makes a month table, newest first, leaving the undated out", () => {
    const rows = doneByMonth(list);
    expect(rows.map((r) => [r.key, r.story, r.defect, r.other])).toEqual([
      ["2026-10", 1, 1, 0],
      ["2026-09", 0, 1, 1],
    ]);
  });

  it("lists the types present, and filters by type and month", () => {
    expect(doneTypes(list)).toEqual([["Bug", 2], ["Story", 2], ["Task", 1]]);
    expect(filterDone(list, "Bug", "").map((d) => d.jira_key)).toEqual(["A-2", "A-3"]);
    expect(filterDone(list, "", "2026-09").map((d) => d.jira_key)).toEqual(["A-3", "A-4"]);
    expect(filterDone(list, "", "unknown").map((d) => d.jira_key)).toEqual(["A-5"]);
    expect(filterDone(list, "Story", "2026-10").map((d) => d.jira_key)).toEqual(["A-1"]);
  });
});
