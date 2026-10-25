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
