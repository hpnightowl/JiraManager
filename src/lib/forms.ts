import type { CreateForm } from "./types";

/**
 * Names of the project's required fields that have no answer yet. An unsupported required field
 * can never be answered here, so it always counts: the push would be refused anyway, and saying
 * so up front is kinder than a Jira error afterwards.
 */
export function missingRequired(
  form: CreateForm | null,
  answers: Record<string, string>,
  multi: Record<string, string[]>,
): string[] {
  if (!form) return [];
  return form.fields
    .filter((spec) => {
      if (spec.kind === "unsupported") return true;
      if (spec.kind === "multi_option") return (multi[spec.id] ?? []).length === 0;
      return (answers[spec.id] ?? "").trim() === "";
    })
    .map((spec) => spec.name);
}
