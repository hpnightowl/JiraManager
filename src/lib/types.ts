// Shapes of the data Rust sends to the UI. They mirror the Rust structs field by field
// (serde turns `snake_case` Rust fields into the same names here).

export type Task = {
  id: number;
  title: string;
  done: boolean;
  tracked_seconds: number; // finished sessions only
  running_since: number | null; // Unix seconds, or null when not running
  jira_key: string | null;
  jira_status: string | null;
  unpushed_seconds: number; // finished time not yet logged in Jira
  created_at: number; // Unix seconds (Jira's date for fetched issues)
  issue_type: string | null; // "Epic", "Story", "Task"...
  parent_key: string | null; // the parent Jira has for this issue
  pending_parent_key: string | null; // a parent chosen here, NOT in Jira until Push
  jira_logged_seconds: number | null; // time Jira has logged (as of last refresh/push)
  context_only: boolean; // a parent heading from Jira that isn't one of my tasks
  story_title: string | null; // drafted story, kept on the task (what Push sends)
  story_description: string | null; // description incl. an "Acceptance criteria:" list
  story_source: "ai" | "template" | "manual" | null;
};

// ---- git sources ----
/** One place your work lives: a GitLab server (own address + own token) or a repository folder. */
export type GitConnection = {
  id: number;
  name: string;
  kind: "gitlab" | "local";
  base_url: string; // server address, or the folder path for a local repository
  author: string | null; // commit email (local repositories)
  has_token: boolean;
  token_storage: string | null;
};
export type MergeRequest = {
  id: number;
  connection_id: number | null;
  source: "gitlab" | "local";
  task_id: number | null;
  reference: string; // "group/project!12", or "repo@branch" for a local branch
  title: string;
  state: string; // opened | merged | closed | local
  web_url: string; // for a local branch this is a local:// address, not a link
  created_at: number;
  merged_at: number | null;
  role: "author" | "assignee";
  jira_key: string | null;
};
export type MrImportReport = {
  fetched: number;
  new_tasks: number;
  linked_to_existing: number;
  refreshed: number;
  new_task_ids: number[];
};
export type ConnectionImport = { connection_id: number; name: string; error: string | null; report: MrImportReport | null };
export type ImportSummary = {
  connections: ConnectionImport[];
  fetched: number;
  new_tasks: number;
  linked_to_existing: number;
  refreshed: number;
  new_task_ids: number[];
};

// ---- stories and bulk actions ----
export type GenerateReport = { done: number[]; failed: { task_id: number; message: string }[]; stopped: string | null };
export type BulkItem = { task_id: number; title: string; result: PushResult | null; error: string | null };

// ---- the Stories tab ----
/** A story/issue I created: either from the app's own ledger or from what Jira says I reported. */
export type StoryRecord = {
  jira_key: string;
  title: string;
  issue_type: string | null;
  status: string | null; // only known for issues read from Jira
  created_at: number; // Unix seconds
};
export type StoryStats = {
  created_with_app: StoryRecord[]; // exact: pushed from this app (counted from when this feature arrived)
  reported: StoryRecord[]; // copy of what Jira says I reported, as of the last refresh
  reported_refreshed_at: number | null;
};
export type SuggestOptions = {
  count: number;
  merge_requests: boolean;
  defects: boolean;
  other_tasks: boolean;
  since_days: number; // 0 = no limit
};
export type Suggestion = {
  task_id: number; // the task it came from (a "defect" suggestion creates a NEW task when adopted)
  kind: "merge_request" | "defect" | "task";
  jira_key: string | null;
  source_title: string;
  score: number;
  evidence: string[];
  title: string; // drafted story
  description: string; // incl. acceptance criteria
  source: "ai" | "template";
};
export type SuggestReport = { suggestions: Suggestion[]; available: number; stopped: string | null; notes: string[] };
export type AdoptItem = { task_id: number; kind: string; title: string; description: string; source: string };
export type AdoptReport = { task_ids: number[]; failed: { task_id: number; message: string }[] };

// Note: there is deliberately no `token` here. Rust never sends it back to the UI.
export type JiraSettings = {
  base_url: string;
  email: string;
  has_token: boolean;
  token_storage: string | null;
  /** Also fetch issues resolved in the last N days; 0 = only open ones. */
  fetch_days: number;
};
export type TypeCount = { name: string; count: number };
export type SyncReport = {
  fetched: number;
  added: number;
  updated: number;
  hidden_skipped: number;
  by_type: TypeCount[];
};
/** An issue you removed from the list on purpose. */
export type HiddenIssue = { jira_key: string; title: string; hidden_at: number };

// ---- local AI ----
export type ModelAdvice = { tag: string; download_gb: number; note: string };
export type InstalledModel = { name: string; size_bytes: number };
export type LlmSettings = { base_url: string; model: string; ram_gb: number; recommended: ModelAdvice[] };
export type LlmStatus = { reachable: boolean; error: string | null; models: InstalledModel[]; model_ready: boolean };
/** The local AI server (Ollama) and what the app is doing with it. */
export type AiStatus = {
  installed: boolean; // the `ollama` program exists on this computer
  running: boolean; // something answers at the saved address
  started_by_app: boolean; // this app started it, so this app stops it
  loaded_models: string[]; // models currently held in memory
  autostart: boolean; // start it when the app opens
  local: boolean; // the saved address is on this computer
};

/** Progress of a model download (sent by Rust as "model-pull" events). */
export type PullProgress = { status: string; completed: number; total: number; done: boolean };
export type Improvement = {
  title: string;
  description: string;
  acceptance_criteria: string[];
  source: "ai" | "template";
  model: string | null;
};

/** Everything a task row can ask the page to do. */
export type RowActions = {
  start: (id: number) => void;
  stop: () => void;
  toggle: (id: number) => void;
  remove: (id: number) => void;
  push: (task: Task) => void;
  open: (jiraKey: string) => void;
  setParent: (id: number, parentKey: string | null) => void;
  rename: (id: number, title: string) => void;
  /** Draft a story for this task with the local AI. */
  story: (id: number) => void;
  /** Open a merge request in the browser (only links under your GitLab address are opened). */
  openMr: (url: string) => void;
};

export type Project = { key: string; name: string };
export type IssueType = { id: string; name: string; subtask: boolean };
export type AllowedValue = { id: string; label: string };
export type JiraUser = { account_id: string; display_name: string };

export type FieldKind =
  | "text"
  | "text_area"
  | "number"
  | "date"
  | "option"
  | "multi_option"
  | "labels"
  | "user"
  | "unsupported";

export type FieldSpec = {
  id: string;
  name: string;
  required: boolean;
  has_default: boolean;
  kind: FieldKind;
  allowed: AllowedValue[];
  note: string | null;
};

export type CreateForm = {
  fields: FieldSpec[]; // required by the project
  optional: FieldSpec[]; // useful extras on the create screen: assignee, priority, labels...
  me: JiraUser | null; // default assignee
  can_set_parent: boolean;
  description_required: boolean;
  saved: Record<string, string | string[]>;
};

export type PreviewEntry = {
  entry_id: number;
  started_at: number;
  seconds: number;
  log_seconds: number;
  skipped: string | null;
};

export type PushPreview = {
  task_id: number;
  title: string;
  jira_key: string | null;
  pending_parent_key: string | null;
  entries: PreviewEntry[];
  total_log_seconds: number;
};

export type PushResult = {
  jira_key: string;
  created_issue: boolean;
  worklogs_logged: number;
  seconds_logged: number;
  skipped: number;
  parent_set: boolean;
  warnings: string[];
};
