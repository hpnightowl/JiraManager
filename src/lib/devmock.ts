// Dev only (see routes/+layout.ts): a fake backend with sample data so the UI can be opened in a
// normal browser at http://localhost:1420. Not part of the packaged app.

import type { GitConnection, Task } from "./types";

const now = () => Math.floor(Date.now() / 1000);
const DAY = 86400;

let nextId = 100;
const task = (over: Partial<Task>): Task => ({
  id: nextId++,
  title: "",
  done: false,
  tracked_seconds: 0,
  running_since: null,
  jira_key: null,
  jira_status: null,
  unpushed_seconds: 0,
  created_at: now(),
  issue_type: null,
  parent_key: null,
  pending_parent_key: null,
  jira_logged_seconds: null,
  context_only: false,
  story_title: null,
  story_description: null,
  story_source: null,
  ...over,
});

const tasks: Task[] = [
  task({ title: "Checkout redesign", jira_key: "ABC-1", issue_type: "Epic", context_only: true, created_at: 0 }),
  task({ title: "Platform reliability", jira_key: "ABC-10", issue_type: "Epic", context_only: true, created_at: 0 }),
  task({
    title: "Add Apple Pay to checkout", jira_key: "ABC-12", issue_type: "Story", jira_status: "In Progress",
    parent_key: "ABC-1", created_at: now() - 3 * DAY, tracked_seconds: 5400, jira_logged_seconds: 3600, unpushed_seconds: 1800,
  }),
  task({
    title: "Fix address validation", jira_key: "ABC-13", issue_type: "Story", jira_status: "Done", done: true,
    parent_key: "ABC-1", created_at: now() - 36 * DAY, tracked_seconds: 7200, jira_logged_seconds: 7200,
  }),
  task({
    title: "Alert on queue lag", jira_key: "ABC-21", issue_type: "Story", jira_status: "In Progress",
    parent_key: "ABC-10", created_at: now() - 8 * DAY, tracked_seconds: 2700, jira_logged_seconds: 1800,
    running_since: now() - 12 * 60,
  }),
  task({ title: "Update the on-call runbook", jira_key: "ABC-30", issue_type: "Task", jira_status: "To Do", created_at: now() - 40 * DAY, jira_logged_seconds: 0 }),
  task({ title: "Prepare sprint demo notes", pending_parent_key: "ABC-1", tracked_seconds: 900, unpushed_seconds: 900, created_at: now() - DAY }),
  task({ title: "Investigate flaky checkout test", created_at: now() - 2 * 3600 }),
  // Created from a GitLab merge request, with a story already drafted by the AI.
  task({
    title: "Speed up the address lookup", created_at: now() - 5 * DAY, tracked_seconds: 1800, unpushed_seconds: 1800,
    story_title: "Speed up address lookup on the checkout page",
    story_description: "Address lookups currently feel slow.\n\nAcceptance criteria:\n- Suggestions appear within 300 ms\n- Slow lookups show a loading state",
    story_source: "ai",
  }),
  // Created from a merge request, no story yet.
  task({ title: "Migrate the invoice job to the new queue", created_at: now() - 4 * DAY }),
];

// Two GitLab servers (each with its own token) and a local repository folder.
let connections: GitConnection[] = [
  { id: 1, name: "Work GitLab", kind: "gitlab", base_url: "https://gitlab.com", author: null, has_token: true, token_storage: "OS keychain" },
  { id: 2, name: "Client GitLab", kind: "gitlab", base_url: "https://git.client.example", author: null, has_token: false, token_storage: null },
  { id: 3, name: "payments-service", kind: "local", base_url: "/home/you/projects/payments-service", author: "you@company.com", has_token: false, token_storage: null },
];

const mergeRequests = () => [
  { id: 1, connection_id: 1, source: "gitlab", task_id: tasks.find((t) => t.title.startsWith("Speed up"))!.id, reference: "shop/web!412", title: "Speed up the address lookup", state: "merged", web_url: "https://gitlab.com/shop/web/-/merge_requests/412", created_at: now() - 5 * DAY, merged_at: now() - 4 * DAY, role: "author", jira_key: null },
  { id: 2, connection_id: 1, source: "gitlab", task_id: tasks.find((t) => t.title.startsWith("Migrate"))!.id, reference: "shop/jobs!77", title: "Migrate the invoice job to the new queue", state: "opened", web_url: "https://gitlab.com/shop/jobs/-/merge_requests/77", created_at: now() - 4 * DAY, merged_at: null, role: "author", jira_key: null },
  { id: 3, connection_id: 1, source: "gitlab", task_id: tasks.find((t) => t.jira_key === "ABC-12")!.id, reference: "shop/web!398", title: "ABC-12: Add Apple Pay", state: "merged", web_url: "https://gitlab.com/shop/web/-/merge_requests/398", created_at: now() - 3 * DAY, merged_at: now() - 2 * DAY, role: "assignee", jira_key: "ABC-12" },
  { id: 4, connection_id: 3, source: "local", task_id: tasks.find((t) => t.title.startsWith("Investigate flaky"))!.id, reference: "payments-service@feature/flaky-checkout-test", title: "Flaky checkout test", state: "local", web_url: "local:///home/you/projects/payments-service#feature/flaky-checkout-test", created_at: now() - 2 * 3600, merged_at: null, role: "author", jira_key: null },
];

const find = (id: number) => tasks.find((t) => t.id === id)!;

let fetchDays = 30;
const ai = { running: true, started_by_app: true, loaded: ["qwen2.5:3b"], autostart: true };
const aiStatus = () => ({
  installed: true, running: ai.running, started_by_app: ai.started_by_app,
  loaded_models: ai.running ? [...ai.loaded] : [], autostart: ai.autostart, local: true,
});
let hiddenIssues: { jira_key: string; title: string; hidden_at: number }[] = [];

// Two defects assigned to me: one open, one already fixed (resolved).
tasks.push(
  task({ title: "Crash when saving an empty invoice", jira_key: "ABC-41", issue_type: "Bug", jira_status: "In Progress", created_at: now() - 6 * DAY, jira_logged_seconds: 1800 }),
  task({ title: "Totals wrong after a refund", jira_key: "ABC-37", issue_type: "Bug", jira_status: "Done", done: true, created_at: now() - 12 * DAY, tracked_seconds: 3600, jira_logged_seconds: 3600 }),
);

// Stories tab: what the app created (exact) and what Jira says I reported (read on demand).
let reportedAt: number | null = null;
const rec = (jira_key: string, title: string, issue_type: string, daysAgo: number, status: string | null = null) => ({
  jira_key, title, issue_type, status, created_at: now() - daysAgo * DAY,
});
const storyStats = () => ({
  created_with_app: [rec("ABC-52", "Add Apple Pay to checkout", "Story", 3), rec("ABC-47", "Alert on queue lag", "Story", 9)],
  reported: reportedAt
    ? [
        rec("ABC-52", "Add Apple Pay to checkout", "Story", 3, "In Progress"), rec("ABC-47", "Alert on queue lag", "Story", 9, "Done"),
        rec("ABC-31", "Export invoices as CSV", "Story", 41, "Done"), rec("ABC-22", "Fix address validation", "Story", 47, "Done"),
        rec("ABC-9", "Dark mode for the dashboard", "Story", 80, "Done"),
      ]
    : [],
  done: reportedAt
    ? [
        ["ABC-47", "Alert on queue lag", "Story", 2], ["ABC-37", "Totals wrong after a refund", "Bug", 4], ["ABC-33", "Retry failed webhooks", "Story", 9],
        ["ABC-29", "Crash on empty invoice", "Bug", 12], ["ABC-26", "Fix address validation", "Story", 36], ["ABC-24", "Slow search on orders", "Bug", 41],
        ["ABC-22", "Update the runbook", "Task", 44], ["ABC-19", "Dark mode for the dashboard", "Story", 70],
      ].map(([jira_key, title, issue_type, daysAgo]) => ({ jira_key, title, issue_type, status: "Done", done_at: now() - (daysAgo as number) * DAY }))
    : [],
  reported_refreshed_at: reportedAt,
});
const sug = (task_id: number, kind: string, jira_key: string | null, source_title: string, score: number, evidence: string[], title: string, description: string) => ({
  task_id, kind, jira_key, source_title, score, evidence, title, description, source: "ai",
});
const suggestionPool = [
  sug(0, "merge_request", null, "Migrate the invoice job to the new queue", 4200, ["1 merge request", "3 commits"], "Move the invoice job to the new queue",
    "The invoice job should run on the new queue so invoices are no longer delayed by other jobs.\n\nAcceptance criteria:\n- Invoices are processed through the new queue\n- A failed invoice is retried and does not block the others"),
  sug(0, "defect", "ABC-37", "Totals wrong after a refund", 6900, ["Defect ABC-37 (Done)", "1 h tracked here", "1 h logged in Jira", "Resolved", "4 Jira comments"],
    "Correct order totals after a refund",
    "Order totals showed the wrong amount after a partial refund. The cause was rounding in the refund calculation.\n\nAcceptance criteria:\n- A partial refund leaves the right total\n- Totals match the payment provider\n\nRelated Jira issue: ABC-37"),
  sug(0, "defect", "ABC-41", "Crash when saving an empty invoice", 2400, ["Defect ABC-41 (In Progress)", "30 min logged in Jira", "2 Jira comments"],
    "Stop the crash when an empty invoice is saved",
    "Saving an invoice with no lines crashes the page.\n\nAcceptance criteria:\n- Saving an empty invoice shows a clear message\n- No data is lost\n\nRelated Jira issue: ABC-41"),
  sug(0, "task", null, "Investigate flaky checkout test", 1800, ["30 min tracked"], "Fix the flaky checkout test",
    "The checkout test fails intermittently on CI.\n\nAcceptance criteria:\n- The test passes 20 runs in a row"),
];
// The cards need real task ids to adopt onto; fill them in once the sample tasks exist.
suggestionPool[0].task_id = tasks.find((t) => t.title.startsWith("Migrate"))!.id;
suggestionPool[1].task_id = tasks.find((t) => t.jira_key === "ABC-37")!.id;
suggestionPool[2].task_id = tasks.find((t) => t.jira_key === "ABC-41")!.id;
suggestionPool[3].task_id = tasks.find((t) => t.title.startsWith("Investigate flaky"))!.id;

function stopRunning() {
  const r = tasks.find((t) => t.running_since !== null);
  if (r) {
    const spent = Math.max(0, now() - r.running_since!);
    r.tracked_seconds += spent;
    r.unpushed_seconds += spent;
    r.running_since = null;
  }
}

const handlers: Record<string, (a: Record<string, unknown>) => unknown> = {
  list_tasks: () => structuredClone(tasks),
  tracked_between: () => 3 * 3600 + 25 * 60,
  get_jira_settings: () => ({ base_url: "https://demo.atlassian.net", email: "you@demo.com", has_token: true, token_storage: "OS keychain", fetch_days: fetchDays }),
  set_fetch_days: (a) => {
    fetchDays = a.days as number;
    return { base_url: "https://demo.atlassian.net", email: "you@demo.com", has_token: true, token_storage: "OS keychain", fetch_days: fetchDays };
  },
  list_hidden_issues: () => structuredClone(hiddenIssues),
  unhide_issue: (a) => void (hiddenIssues = hiddenIssues.filter((h) => h.jira_key !== a.jiraKey)),
  stale_timer: () => null,
  heartbeat: () => null,
  add_task: (a) => {
    const t = task({ title: String(a.title).trim() });
    if (!t.title) throw "Task title can't be empty";
    tasks.push(t);
    return t;
  },
  toggle_task: (a) => {
    const t = find(a.id as number);
    t.done = !t.done;
    return t;
  },
  delete_task: (a) => {
    const i = tasks.findIndex((t) => t.id === a.id);
    const t = tasks[i];
    if (t?.jira_key) hiddenIssues.unshift({ jira_key: t.jira_key, title: t.title, hidden_at: now() }); // stays removed on Refresh
    tasks.splice(i, 1);
  },
  rename_task: (a) => Object.assign(find(a.id as number), { title: String(a.title) }),
  set_task_parent: (a) => Object.assign(find(a.id as number), { pending_parent_key: (a.parentKey as string | null) ?? null }),
  start_timer: (a) => {
    stopRunning();
    find(a.taskId as number).running_since = now();
  },
  stop_timer: () => stopRunning(),
  fetch_jira_issues: () => ({
    fetched: 7, added: 0, updated: 7, hidden_skipped: hiddenIssues.length,
    by_type: [{ name: "Bug", count: 3 }, { name: "Story", count: 3 }, { name: "Task", count: 1 }],
  }),
  get_push_preview: (a) => {
    const t = find(a.taskId as number);
    const round = (a.roundMinutes as number) * 60;
    const entries = [
      { entry_id: 1, started_at: now() - 5 * 3600, seconds: 1500, skipped: null as string | null },
      { entry_id: 2, started_at: now() - 3 * 3600, seconds: 300, skipped: null as string | null },
      { entry_id: 3, started_at: now() - 3600, seconds: 40, skipped: "shorter than 1 minute (Jira's minimum)" },
    ].map((e) => ({ ...e, log_seconds: round && !e.skipped ? Math.ceil(e.seconds / round) * round : e.seconds }));
    return {
      task_id: t.id, title: t.title, jira_key: t.jira_key, pending_parent_key: t.pending_parent_key, entries,
      total_log_seconds: entries.filter((e) => !e.skipped).reduce((n, e) => n + e.log_seconds, 0),
    };
  },
  list_jira_projects: () => [{ key: "ABC", name: "Alpha Platform" }, { key: "WEB", name: "Website" }],
  list_issue_types: () => [{ id: "10001", name: "Story", subtask: false }, { id: "10002", name: "Task", subtask: false }, { id: "10003", name: "Epic", subtask: false }],
  get_create_form: () => ({
    fields: [
      { id: "customfield_10050", name: "Team", required: true, has_default: false, kind: "option", note: null,
        allowed: [{ id: "201", label: "Platform" }, { id: "202", label: "Mobile" }] },
      { id: "customfield_10016", name: "Story Points", required: true, has_default: false, kind: "number", allowed: [], note: null },
    ],
    optional: [
      { id: "assignee", name: "Assignee", required: false, has_default: false, kind: "user", allowed: [], note: null },
      { id: "priority", name: "Priority", required: false, has_default: true, kind: "option", note: null,
        allowed: [{ id: "2", label: "High" }, { id: "3", label: "Medium" }, { id: "4", label: "Low" }] },
      { id: "labels", name: "Labels", required: false, has_default: false, kind: "labels", allowed: [], note: null },
    ],
    me: { account_id: "me-1", display_name: "Demo User" },
    can_set_parent: true,
    description_required: false,
    saved: {},
  }),
  search_jira_users: () => [{ account_id: "u2", display_name: "Grace Hopper" }],
  ai_status: () => aiStatus(),
  start_ai: async () => {
    await new Promise((r) => setTimeout(r, 900));
    ai.running = true;
    ai.started_by_app = true;
    return aiStatus();
  },
  stop_ai: () => {
    const was = ai.started_by_app;
    ai.running = false;
    ai.started_by_app = false;
    ai.loaded = [];
    return [was ? "The AI was stopped and its memory freed." : "Unloaded 1 model(s) from memory. Ollama itself keeps running because you started it yourself.", aiStatus()];
  },
  set_ai_autostart: (a) => void (ai.autostart = a.on as boolean),
  open_link: () => null,
  get_story_stats: () => storyStats(),
  refresh_reported_stories: async () => {
    await new Promise((r) => setTimeout(r, 700));
    reportedAt = now();
    return storyStats();
  },
  suggest_stories: async (a) => {
    await new Promise((r) => setTimeout(r, 1500)); // a real model takes a few seconds per story
    const o = a.options as { count: number; merge_requests: boolean; defects: boolean; other_tasks: boolean };
    if (o.count < 1 || o.count > 20) throw "Ask for between 1 and 20 stories";
    const all = suggestionPool.filter(
      (s) => (s.kind === "merge_request" && o.merge_requests) || (s.kind === "defect" && o.defects) || (s.kind === "task" && o.other_tasks),
    );
    const picked = [...all].sort((x, y) => y.score - x.score).slice(0, o.count);
    return {
      suggestions: picked.map((s) => ({ ...s, source: a.useTemplate ? "template" : "ai" })),
      available: all.length,
      stopped: null,
      notes: [],
    };
  },
  adopt_suggestions: (a) => {
    const ids: number[] = [];
    for (const it of a.items as { task_id: number; kind: string; title: string; description: string; source: string }[]) {
      if (it.kind === "defect") {
        const t = task({ title: it.title, story_title: it.title, story_description: it.description, story_source: it.source as Task["story_source"] });
        tasks.push(t);
        ids.push(t.id);
      } else {
        Object.assign(find(it.task_id), { story_title: it.title, story_description: it.description, story_source: it.source });
        ids.push(it.task_id);
      }
    }
    return { task_ids: ids, failed: [] };
  },
  list_merge_requests: () => mergeRequests(),
  list_git_connections: () => structuredClone(connections),
  add_git_connection: (a) => {
    const c = {
      id: connections.length + 1, name: String(a.name || a.baseUrl), kind: a.kind as GitConnection["kind"], base_url: String(a.baseUrl),
      author: (a.author as string | null) ?? null, has_token: a.kind === "gitlab" && !!a.token, token_storage: a.token ? "OS keychain" : null,
    };
    connections.push(c);
    return c;
  },
  update_git_connection: (a) => {
    const c = connections.find((x) => x.id === a.id)!;
    Object.assign(c, { name: a.name, base_url: a.baseUrl, author: a.author ?? null });
    if (a.token) Object.assign(c, { has_token: true, token_storage: "OS keychain" });
    return c;
  },
  remove_git_connection: (a) => void (connections = connections.filter((x) => x.id !== a.id)),
  clear_git_token: (a) => Object.assign(connections.find((x) => x.id === a.id)!, { has_token: false, token_storage: null }),
  test_git_connection: (a) => (connections.find((x) => x.id === a.id)!.kind === "gitlab" ? "Connected to Work GitLab as Demo User (@demo)" : "payments-service is a git repository. Looking for commits by you@company.com"),
  import_git_changes: async (a) => {
    await new Promise((r) => setTimeout(r, 800));
    const ids = a.connectionIds as number[];
    const fail = ids.includes(2);
    const ok = ids.filter((id) => id !== 2);
    return {
      connections: ids.map((id) =>
        id === 2
          ? { connection_id: 2, name: "Client GitLab", error: "Add an access token for \"Client GitLab\" in Settings first", report: null }
          : { connection_id: id, name: connections.find((c) => c.id === id)!.name, error: null, report: { fetched: 3, new_tasks: 1, linked_to_existing: 1, refreshed: 1, new_task_ids: [] } },
      ),
      fetched: ok.length * 3, new_tasks: ok.length, linked_to_existing: ok.length, refreshed: ok.length,
      new_task_ids: fail && ok.length === 0 ? [] : tasks.slice(-2).map((t) => t.id),
    };
  },
  generate_stories: async (a) => {
    await new Promise((r) => setTimeout(r, 900));
    const ids = a.taskIds as number[];
    for (const id of ids) {
      const t = find(id);
      t.story_title = `${String(t.title).replace(/\.$/, "")}`;
      t.story_description = "Describes what is needed in one or two sentences.\n\nAcceptance criteria:\n- It works as described\n- Edge cases are handled";
      t.story_source = a.useTemplate ? "template" : "ai";
    }
    return { done: ids, failed: [], stopped: null };
  },
  save_story_draft: (a) => Object.assign(find(a.taskId as number), { story_title: a.title, story_description: a.description, story_source: "manual" }),
  get_bulk_preview: (a) =>
    (a.taskIds as number[]).map((id) => {
      const t = find(id);
      return {
        task_id: id, title: t.title, jira_key: t.jira_key, pending_parent_key: t.pending_parent_key,
        entries: t.unpushed_seconds >= 60 ? [{ entry_id: id, started_at: now() - 3600, seconds: t.unpushed_seconds, log_seconds: t.unpushed_seconds, skipped: null }] : [],
        total_log_seconds: t.unpushed_seconds >= 60 ? t.unpushed_seconds : 0,
      };
    }),
  push_tasks: async (a) => {
    await new Promise((r) => setTimeout(r, 900));
    const req = a.request as { task_ids: number[] };
    let n = 200;
    return req.task_ids.map((id, i) => {
      const t = find(id);
      if (i === 1 && t.jira_key === null) return { task_id: id, title: t.title, result: null, error: "Jira rejected the request (400): Summary is too long." };
      const created = t.jira_key === null;
      if (created) t.jira_key = `ABC-${n++}`;
      return { task_id: id, title: t.title, result: { jira_key: t.jira_key, created_issue: created, worklogs_logged: 1, seconds_logged: t.unpushed_seconds, skipped: 0, parent_set: false, warnings: [] }, error: null };
    });
  },
  get_llm_settings: () => ({
    base_url: "http://127.0.0.1:11434", model: "qwen2.5:3b", ram_gb: 23.3,
    recommended: [
      { tag: "qwen2.5:3b", download_gb: 1.9, note: "Recommended: fast on a normal CPU" },
      { tag: "qwen2.5:7b", download_gb: 4.7, note: "Higher quality, noticeably slower without a GPU" },
    ],
  }),
  save_llm_settings: (a) => ({ base_url: a.baseUrl, model: a.model, ram_gb: 23.3, recommended: [] }),
  llm_status: () => ({ reachable: true, error: null, model_ready: true, models: [{ name: "qwen2.5:3b", size_bytes: 1_900_000_000 }] }),
  suggest_story: async (a) => {
    await new Promise((r) => setTimeout(r, 1200)); // a real model takes a few seconds
    return {
      title: `Add ${String(a.title).toLowerCase()} support`,
      description: "Customers can use the feature end to end without leaving the page.",
      acceptance_criteria: ["The option is visible on the checkout page", "A successful payment shows a confirmation", "Failures show a clear error"],
      source: "ai", model: "qwen2.5:3b",
    };
  },
  template_story: (a) => ({
    title: String(a.title).trim(), description: `Work item: ${String(a.title).trim()}.`,
    acceptance_criteria: ["The change works as described", "Tests are added or updated", "The change is reviewed and merged"],
    source: "template", model: null,
  }),
  push_task: (a) => {
    const req = a.request as { task_id: number };
    const t = find(req.task_id);
    const created = t.jira_key === null;
    if (created) t.jira_key = "ABC-99";
    return { jira_key: t.jira_key, created_issue: created, worklogs_logged: 2, seconds_logged: 1800, skipped: 1, parent_set: t.pending_parent_key !== null, warnings: [] };
  },
};

// A tiny stand-in for Tauri's event system: `listen(...)` registers a callback, and the fake
// backend can `emit(...)` to it (used by the model download progress).
const callbacks = new Map<number, (e: unknown) => void>();
const listeners = new Map<string, number[]>();
let callbackId = 1;
const emit = (event: string, payload: unknown) =>
  (listeners.get(event) ?? []).forEach((id) => callbacks.get(id)?.({ event, id, payload }));

handlers.pull_model = async (a) => {
  const steps = [
    { status: "pulling manifest", completed: 0, total: 0, done: false },
    ...Array.from({ length: 20 }, (_, i) => ({ status: "pulling 357c53fb659c", completed: (i + 1) * 96_000_000, total: 1_920_000_000, done: false })),
    { status: "verifying sha256 digest", completed: 0, total: 0, done: false },
    { status: "success", completed: 0, total: 0, done: true },
  ];
  for (const s of steps) {
    await new Promise((r) => setTimeout(r, 250));
    emit("model-pull", s);
  }
  void a;
};

(window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
  invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
    if (cmd === "plugin:event|listen") {
      const event = args.event as string;
      listeners.set(event, [...(listeners.get(event) ?? []), args.handler as number]);
      return args.handler;
    }
    if (cmd.startsWith("plugin:")) return null; // e.g. opening a link, un-listening
    const handler = handlers[cmd];
    if (!handler) throw `devmock: no handler for "${cmd}"`;
    return handler(args);
  },
  transformCallback: (cb: (e: unknown) => void) => {
    const id = callbackId++;
    callbacks.set(id, cb);
    return id;
  },
  unregisterCallback: (id: number) => void callbacks.delete(id),
  convertFileSrc: (p: string) => p,
};
