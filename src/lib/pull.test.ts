import { describe, expect, it } from "vitest";
import { gigabytes, percent } from "./pull";

describe("percent", () => {
  it("is null until a file is downloading", () => {
    expect(percent(null)).toBeNull();
    expect(percent({ status: "pulling manifest", completed: 0, total: 0, done: false })).toBeNull();
  });

  it("rounds down and never passes 100", () => {
    expect(percent({ status: "pulling x", completed: 250, total: 1000, done: false })).toBe(25);
    expect(percent({ status: "pulling x", completed: 999, total: 1000, done: false })).toBe(99);
    expect(percent({ status: "pulling x", completed: 1200, total: 1000, done: false })).toBe(100);
  });
});

describe("gigabytes", () => {
  it("formats bytes with one decimal", () => {
    expect(gigabytes(1_929_912_432)).toBe("1.9");
    expect(gigabytes(0)).toBe("0.0");
  });
});
