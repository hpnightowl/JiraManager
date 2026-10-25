import { describe, expect, it } from "vitest";
import { missingRequired } from "./forms";
import type { CreateForm, FieldKind, FieldSpec } from "./types";

const spec = (id: string, name: string, kind: FieldKind): FieldSpec => ({
  id,
  name,
  required: true,
  has_default: false,
  kind,
  allowed: [],
  note: null,
});

const form = (fields: FieldSpec[]): CreateForm => ({
  fields,
  optional: [],
  me: null,
  can_set_parent: true,
  description_required: false,
  saved: {},
});

describe("missingRequired", () => {
  const f = form([spec("team", "Team", "option"), spec("points", "Story Points", "number"), spec("comps", "Components", "multi_option")]);

  it("lists every required field that has no answer", () => {
    expect(missingRequired(f, {}, {})).toEqual(["Team", "Story Points", "Components"]);
  });

  it("treats blank text as no answer, and an empty multi-select as no answer", () => {
    expect(missingRequired(f, { team: "  ", points: "3" }, { comps: [] })).toEqual(["Team", "Components"]);
  });

  it("is empty once everything is answered", () => {
    expect(missingRequired(f, { team: "201", points: "5" }, { comps: ["11"] })).toEqual([]);
  });

  it("always blocks on an unsupported required field", () => {
    const g = form([spec("weird", "Cascading thing", "unsupported")]);
    expect(missingRequired(g, { weird: "x" }, {})).toEqual(["Cascading thing"]);
  });

  it("has nothing to say before the form has loaded", () => {
    expect(missingRequired(null, {}, {})).toEqual([]);
  });
});
