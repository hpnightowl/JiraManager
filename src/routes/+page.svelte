<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { clockDuration, clockTime, humanDuration, recall, remember } from "$lib/format";
  import {
    NO_PARENT,
    epicGroupStartsOpen,
    epicOptions,
    filterGroups,
    groupByMonth,
    groupByParent,
    monthKeyOf,
    monthStartsOpen,
    periodBounds,
    rowsToShow,
    ROWS_SHOWN,
    type Group,
    type MonthGroup,
  } from "$lib/grouping";
  import BulkPushDialog from "$lib/BulkPushDialog.svelte";
  import HelpView from "$lib/HelpView.svelte";
  import ImportMrDialog from "$lib/ImportMrDialog.svelte";
  import PushDialog from "$lib/PushDialog.svelte";
  import SetupView from "$lib/SetupView.svelte";
  import StoriesView from "$lib/StoriesView.svelte";
  import TaskRow from "$lib/TaskRow.svelte";
  import type {
    BulkItem,
    GenerateReport,
    GitConnection,
    HiddenIssue,
    ImportSummary,
    JiraSettings,
    MergeRequest,
    PushResult,
    RowActions,
    SyncReport,
    Task,
  } from "$lib/types";

  type StaleTimer = { task_id: number; title: string; started_at: number; last_seen: number };
  type View = "epic" | "month";

  // ---- state ----
  let tasks = $state<Task[]>([]);
  let title = $state("");
  let stale = $state<StaleTimer | null>(null);
  let settings = $state<JiraSettings>({ base_url: "", email: "", has_token: false, token_storage: null, fetch_days: 30 });
  // Which tab is open. The first time (Jira not connected yet) we open Setup for you.
  type Tab = "tasks" | "stories" | "setup" | "help";
  let tab = $state<Tab>("tasks");
  const needsSetup = $derived(!settings.base_url || !settings.has_token);
  let pushTarget = $state<Task | null>(null);
  let refreshing = $state(false);

  // GitLab merge requests, and the bulk-action state (selection, dialogs, story writing).
  let connections = $state<GitConnection[]>([]);
  let mrs = $state<MergeRequest[]>([]);
  let selected = $state<Record<number, boolean>>({});
  let showImport = $state(false);
  let showBulk = $state(false);
  let writing = $state<Record<number, boolean>>({}); // task ids whose story is being written
  let storyStopped = $state<{ message: string; ids: number[] } | null>(null);
  let lastRefresh = $state<string | null>(recall("pj.lastRefresh"));

  let view = $state<View>(recall("pj.view") === "month" ? "month" : "epic");
  let query = $state("");
  let typeFilter = $state(""); // "" = every type
  let hidden = $state<HiddenIssue[]>([]);
  let showHidden = $state(false);
  let hideDone = $state(false);
  let monthFilter = $state("");
  // Groups start closed (see grouping.ts); this holds only the choices made with the arrows.
  let expanded = $state<Record<string, boolean>>({});
  let showAllRows = $state<Record<string, boolean>>({}); // groups whose rows are not cut at ROWS_SHOWN

  let message = $state<{ kind: "ok" | "error"; text: string } | null>(null);
  let messageTimer: ReturnType<typeof setTimeout> | undefined;

  // The clock the UI re-reads every second. Rust only stores *when* a timer started;
  // the ticking number on screen is computed here: tracked + (now - running_since).
  let nowSec = $state(Math.floor(Date.now() / 1000));
  let todaySec = $state(0);
  let monthSec = $state(0);
  let totalsAt = $state(Math.floor(Date.now() / 1000));

  // ---- helpers ----
  function notify(kind: "ok" | "error", text: string) {
    message = { kind, text };
    clearTimeout(messageTimer);
    if (kind === "ok") messageTimer = setTimeout(() => (message = null), 7000);
  }

  // Every backend call can fail (Rust returns Result<_, AppError>), so we wrap them.
  async function call<T>(fn: () => Promise<T>): Promise<T | undefined> {
    try {
      return await fn();
    } catch (e) {
      notify("error", String(e));
    }
  }

  function elapsedOf(t: Task): number {
    return t.tracked_seconds + (t.running_since !== null ? Math.max(0, nowSec - t.running_since) : 0);
  }

  const sumOf = (list: Task[], f: (t: Task) => number) => list.reduce((n, t) => n + f(t), 0);

  /** 0 -> "0m", 20 -> "<1m", 4500 -> "1h 15m". */
  function statTime(seconds: number): string {
    if (seconds <= 0) return "0m";
    return seconds < 60 ? "<1m" : humanDuration(seconds);
  }

  // ---- loading ----
  async function refreshTotals() {
    const b = periodBounds();
    const [day, month] = await Promise.all([
      call(() => invoke<number>("tracked_between", { from: b.dayStart, to: b.dayEnd })),
      call(() => invoke<number>("tracked_between", { from: b.monthStart, to: b.monthEnd })),
    ]);
    todaySec = day ?? todaySec;
    monthSec = month ?? monthSec;
    totalsAt = Math.floor(Date.now() / 1000);
  }

  async function refresh() {
    tasks = (await call(() => invoke<Task[]>("list_tasks"))) ?? tasks;
    mrs = (await call(() => invoke<MergeRequest[]>("list_merge_requests"))) ?? mrs;
    await refreshTotals();
  }

  async function loadSettings() {
    const s = await call(() => invoke<JiraSettings>("get_jira_settings"));
    if (s) {
      settings = s;
      // First run: open the Setup tab by itself.
      if (!s.base_url || !s.has_token) tab = "setup";
    }
    await loadConnections();
  }

  async function loadConnections() {
    connections = (await call(() => invoke<GitConnection[]>("list_git_connections"))) ?? connections;
  }

  async function checkStale() {
    stale = (await call(() => invoke<StaleTimer | null>("stale_timer"))) ?? null;
  }

  /** Reads from Jira only. Nothing is written to Jira here. */
  async function refreshFromJira() {
    refreshing = true;
    const report = await call(() => invoke<SyncReport>("fetch_jira_issues"));
    refreshing = false;
    if (report) {
      lastRefresh = new Date().toISOString();
      remember("pj.lastRefresh", lastRefresh);
      // Say WHAT came back, by type, so you can see for yourself that defects are included.
      const kinds = report.by_type.map((t) => `${t.count} ${t.name}`).join(", ");
      const hiddenNote = report.hidden_skipped ? ` ${report.hidden_skipped} you removed earlier stayed hidden.` : "";
      const scope = settings.fetch_days === 0 ? "open issues only" : `open + resolved in the last ${settings.fetch_days} days`;
      notify(
        "ok",
        `Refreshed from Jira (${scope}): ${report.fetched} issue(s)${kinds ? ` (${kinds})` : ""}, ${report.added} new, ${report.updated} updated.${hiddenNote} Nothing was changed in Jira.`,
      );
      await refresh();
      await loadHidden();
    }
  }

  // ---- task actions ----
  async function addTask(event: Event) {
    event.preventDefault();
    const added = await call(() => invoke<Task>("add_task", { title }));
    if (added) {
      title = "";
      await refresh();
    }
  }

  const actions: RowActions = {
    start: async (id) => {
      await call(() => invoke<void>("start_timer", { taskId: id })); // Rust: task_id
      await refresh();
    },
    stop: async () => {
      await call(() => invoke<void>("stop_timer", { at: null }));
      await refresh();
    },
    toggle: async (id) => {
      await call(() => invoke<Task>("toggle_task", { id }));
      await refresh();
    },
    remove: async (id) => {
      const t = tasks.find((x) => x.id === id);
      await call(() => invoke<void>("delete_task", { id }));
      await refresh();
      if (t?.jira_key) {
        await loadHidden();
        notify("ok", `${t.jira_key} is hidden and won't come back on Refresh. Restore it from "Hidden". The Jira issue itself is untouched.`);
      }
    },
    push: (task) => (pushTarget = task),
    open: async (key) => {
      await call(() => invoke<void>("open_link", { url: `${settings.base_url}/browse/${encodeURIComponent(key)}` }));
    },
    setParent: async (id, parentKey) => {
      await call(() => invoke<Task>("set_task_parent", { id, parentKey }));
      await refresh();
    },
    rename: async (id, newTitle) => {
      await call(() => invoke<Task>("rename_task", { id, title: newTitle }));
      await refresh();
    },
    story: (id) => generateFor([id], false),
    openMr: async (url) => {
      // The backend opens only addresses on a server you set up (the link came from that server's
      // data, but we don't trust any address we didn't set up ourselves) and says so if it refuses.
      await call(() => invoke<void>("open_link", { url }));
    },
  };

  // ---- stories (drafts kept on each task) ----
  /** Draft stories for these tasks, one after another. Nothing is sent anywhere but your local AI. */
  async function generateFor(ids: number[], useTemplate: boolean) {
    if (ids.length === 0) return;
    for (const id of ids) writing[id] = true;
    storyStopped = null;
    try {
      const r = await invoke<GenerateReport>("generate_stories", { taskIds: ids, useTemplate });
      const failed = r.failed.length ? ` ${r.failed.length} failed: ${r.failed[0].message}` : "";
      if (r.stopped) {
        storyStopped = { message: r.stopped, ids: ids.filter((id) => !r.done.includes(id)) };
        notify("error", `Drafted ${r.done.length} of ${ids.length} stories, then stopped.${failed}`);
      } else {
        notify(r.failed.length ? "error" : "ok", `Drafted ${r.done.length} ${r.done.length === 1 ? "story" : "stories"}${useTemplate ? " from templates" : " with the AI"}. Review them in Push before sending.${failed}`);
      }
    } catch (e) {
      notify("error", String(e));
    } finally {
      for (const id of ids) delete writing[id];
      await refresh();
    }
  }

  // ---- selection and bulk actions ----
  function select(id: number, on: boolean) {
    selected[id] = on;
  }

  function clearSelection() {
    selected = {};
  }

  const selectedTasks = $derived(tasks.filter((t) => selected[t.id] && !t.context_only));
  const selectedNeedingStory = $derived(selectedTasks.filter((t) => t.jira_key === null));
  const selectedPushable = $derived(selectedTasks.filter(canPush));

  function selectAllShown() {
    for (const t of visibleTasks) selected[t.id] = true;
  }

  async function bulkPushDone(items: BulkItem[]) {
    showBulk = false;
    const ok = items.filter((i) => i.result).length;
    const failed = items.filter((i) => i.error).length;
    notify(failed ? "error" : "ok", `Bulk push: ${ok} done${failed ? `, ${failed} failed` : ""}.`);
    for (const i of items) if (i.result) delete selected[i.task_id]; // finished ones leave the selection
    await refresh();
  }

  async function importDone(r: ImportSummary | null) {
    showImport = false;
    if (r) {
      const failed = r.connections.filter((c) => c.error);
      const problem = failed.length ? ` ${failed.length} source(s) failed: ${failed[0].name || "a connection"}: ${failed[0].error}` : "";
      notify(failed.length ? "error" : "ok", `Imported ${r.fetched} change(s): ${r.new_tasks} new task(s), ${r.linked_to_existing} attached to existing Jira tasks.${problem}`);
    }
    await refresh();
  }

  // Import is offered when at least one source can actually be read.
  const canImport = $derived(connections.some((c) => c.kind === "local" || c.has_token));

  // Offer "Push" when something would actually be sent to Jira.
  function canPush(t: Task): boolean {
    return (
      settings.has_token &&
      !t.context_only &&
      (t.jira_key === null || t.unpushed_seconds >= 60 || t.pending_parent_key !== null)
    );
  }

  async function pushDone(r: PushResult) {
    pushTarget = null;
    const parts = [
      r.created_issue ? "issue created" : null,
      r.parent_set ? "parent set" : null,
      `${r.worklogs_logged} worklog(s) logged`,
    ].filter(Boolean);
    notify(r.warnings.length ? "error" : "ok", `${r.jira_key}: ${parts.join(", ")}.${r.warnings.length ? " " + r.warnings.join(" ") : ""}`);
    await refresh();
  }

  // ---- timer recovery (after a crash / closing the app mid-timer) ----
  async function stopAtLastSeen() {
    if (!stale) return;
    await call(() => invoke<void>("stop_timer", { at: stale!.last_seen }));
    stale = null;
    await refresh();
  }

  async function keepRunning() {
    await call(() => invoke<void>("heartbeat"));
    stale = null;
  }

  // ---- what to show ----
  const NOT_IN_JIRA = "Not in Jira yet";
  const typeOf = (t: Task) => t.issue_type ?? NOT_IN_JIRA;

  function keep(t: Task): boolean {
    const q = query.trim().toLowerCase();
    const matches = !q || t.title.toLowerCase().includes(q) || (t.jira_key ?? "").toLowerCase().includes(q);
    const typeOk = typeFilter === "" || typeOf(t) === typeFilter;
    return matches && typeOk && (!hideDone || !t.done || t.context_only);
  }

  // Chips for the issue types you actually have (Bug, Story...), with counts.
  const typeChips = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const t of tasks) if (!t.context_only) counts.set(typeOf(t), (counts.get(typeOf(t)) ?? 0) + 1);
    return [...counts].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  });

  // ---- removed ("hidden") Jira issues ----
  async function loadHidden() {
    hidden = (await call(() => invoke<HiddenIssue[]>("list_hidden_issues"))) ?? hidden;
  }

  async function restoreHidden(key: string) {
    await call(() => invoke<void>("unhide_issue", { jiraKey: key }));
    await loadHidden();
    notify("ok", `${key} will come back the next time you press Refresh from Jira.`);
  }

  const epics = $derived(epicOptions(tasks));
  const groups = $derived(filterGroups(groupByParent(tasks), keep));
  const allMonths = $derived(groupByMonth(tasks));
  const months = $derived(
    groupByMonth(tasks.filter(keep)).filter((m) => !monthFilter || m.key === monthFilter),
  );

  // The merge requests of each task, for the chips on its row.
  const mrsByTask = $derived.by(() => {
    const byTask = new Map<number, MergeRequest[]>();
    for (const mr of mrs) if (mr.task_id !== null) byTask.set(mr.task_id, [...(byTask.get(mr.task_id) ?? []), mr]);
    return byTask;
  });

  // Open or closed: your own choice wins, otherwise the rule in grouping.ts.
  const searching = $derived(query.trim() !== "" || typeFilter !== "");
  const currentMonth = monthKeyOf(Math.floor(Date.now() / 1000));
  const epicOpen = (g: Group) => expanded[g.id] ?? epicGroupStartsOpen(g, searching);
  const monthOpen = (m: MonthGroup) => expanded[`m:${m.key}`] ?? monthStartsOpen(m, currentMonth, searching);
  const rowsOf = (id: string, rows: Task[]) => rowsToShow(rows, !!showAllRows[id]);

  // The tasks currently on screen (closed groups and cut-off rows excluded), for "Select all shown".
  const visibleTasks = $derived(
    view === "epic"
      ? groups.flatMap((g) => [
          ...(g.header && !g.header.context_only ? [g.header] : []),
          ...(epicOpen(g) ? rowsOf(g.id, g.children) : []),
        ])
      : months.flatMap((m) => (monthOpen(m) ? rowsOf(`m:${m.key}`, m.tasks) : [])),
  );

  const running = $derived(tasks.find((t) => t.running_since !== null));
  const liveExtra = $derived(running ? Math.max(0, nowSec - totalsAt) : 0);

  const mine = $derived(tasks.filter((t) => !t.context_only));
  const unpushedSeconds = $derived(sumOf(mine, (t) => t.unpushed_seconds));
  const toCreate = $derived(mine.filter((t) => t.jira_key === null).length);
  const parentChanges = $derived(mine.filter((t) => t.jira_key !== null && t.pending_parent_key !== null).length);

  function monthTypeChips(list: Task[]): string {
    const counts = new Map<string, number>();
    for (const t of list) {
      const label = t.issue_type ?? "Not in Jira yet";
      counts.set(label, (counts.get(label) ?? 0) + 1);
    }
    return [...counts].map(([label, n]) => `${n} ${label}`).join(" · ");
  }

  function toggleGroup(id: string, open: boolean) {
    expanded[id] = !open;
  }

  function setAllGroups(open: boolean) {
    if (view === "epic") for (const g of groups) expanded[g.id] = open;
    else for (const m of months) expanded[`m:${m.key}`] = open;
  }

  const anyOpen = $derived(view === "epic" ? groups.some(epicOpen) : months.some(monthOpen));

  /** Drafts adopted from the Stories tab: show them on Tasks, ticked, ready for "Push selected". */
  async function storiesAdopted(ids: number[]) {
    await refresh();
    selected = Object.fromEntries(ids.map((id) => [id, true]));
    if (typeFilter) typeFilter = ""; // make sure they are not filtered out of sight
    query = "";
    tab = "tasks";
    notify("ok", `${ids.length} story ${ids.length === 1 ? "draft is" : "drafts are"} ready and selected below. Review, then Push selected. Nothing was sent to Jira.`);
  }

  function setView(v: View) {
    view = v;
    remember("pj.view", v);
  }

  onMount(() => {
    refresh();
    checkStale();
    loadSettings();
    loadHidden();
    const tick = setInterval(() => (nowSec = Math.floor(Date.now() / 1000)), 1000);
    // While a timer runs, tell Rust "still alive" every 15 s (see STALE_AFTER_SECS there).
    const beat = setInterval(() => {
      if (running && !stale) call(() => invoke<void>("heartbeat"));
    }, 15000);
    return () => {
      clearInterval(tick);
      clearInterval(beat);
    };
  });
</script>

<header class="topbar">
  <div class="topbar-inner">
    <div class="brand"><span class="logo">◆</span> ProjectJira</div>

    <nav class="tabs" aria-label="Sections">
      <button class="tab" class:active={tab === "tasks"} aria-current={tab === "tasks" ? "page" : undefined} onclick={() => (tab = "tasks")}>Tasks</button>
      <button class="tab" class:active={tab === "stories"} aria-current={tab === "stories" ? "page" : undefined} onclick={() => (tab = "stories")}>Stories</button>
      <button class="tab" class:active={tab === "setup"} aria-current={tab === "setup" ? "page" : undefined} onclick={() => (tab = "setup")}>
        Setup
        {#if needsSetup}<span class="badge" title="Jira isn't connected yet">!</span>{/if}
      </button>
      <button class="tab" class:active={tab === "help"} aria-current={tab === "help" ? "page" : undefined} onclick={() => (tab = "help")}>How to use</button>
    </nav>

    <div class="topbar-actions">
      {#if tab === "tasks"}
        {#if lastRefresh}
          <span class="muted small">Last refreshed {clockTime(Math.floor(new Date(lastRefresh).getTime() / 1000))}</span>
        {/if}
        {#if canImport}
          <button class="btn" onclick={() => (showImport = true)} title="Bring in your merge requests and branches as tasks (read-only)">
            ⬇ Import from git
          </button>
        {/if}
        <button
          class="btn soft"
          onclick={refreshFromJira}
          disabled={refreshing || !settings.has_token}
          title="Reads your issues from Jira. Nothing is changed in Jira."
        >
          {refreshing ? "Refreshing…" : "↻ Refresh from Jira"}
        </button>
      {/if}
    </div>
  </div>
</header>

<main>
  {#if message}
    <div class="msg {message.kind}" role="status">
      <span>{message.text}</span>
      <button class="btn subtle small" onclick={() => (message = null)} aria-label="Dismiss">✕</button>
    </div>
  {/if}

  {#if tab === "setup"}
    <SetupView
      jira={settings}
      {connections}
      onjirachange={(s) => (settings = s)}
      onconnectionschange={loadConnections}
      onmessage={notify}
    />
  {:else if tab === "help"}
    <HelpView ongo={(t) => (tab = t)} />
  {/if}

  <!-- Tasks and Stories stay mounted while you look at another tab, so your filters, selection,
       running timers and the suggestions you asked for are exactly as you left them. -->
  <div class="view" hidden={tab !== "stories"}>
    <StoriesView
      active={tab === "stories"}
      jiraConnected={settings.has_token}
      onadopted={storiesAdopted}
      onmessage={notify}
      ongo={(t) => (tab = t)}
      onopen={(key) => actions.open(key)}
    />
  </div>
  <div class="view" hidden={tab !== "tasks"}>
  {#if needsSetup}
    <div class="banner">
      <p><strong>Connect Jira to get started.</strong> You can already add tasks and run timers, but Refresh and Push need your Jira details.</p>
      <div class="banner-actions"><button class="btn primary" onclick={() => (tab = "setup")}>Open Setup</button></div>
    </div>
  {/if}

  {#if storyStopped}
    <div class="banner">
      <p>{storyStopped.message}</p>
      <div class="banner-actions">
        <button class="btn" onclick={() => generateFor(storyStopped!.ids, true)}>
          Use simple templates for the remaining {storyStopped.ids.length}
        </button>
        <button class="btn" onclick={() => generateFor(storyStopped!.ids, false)}>Try the AI again</button>
        <button class="btn subtle" onclick={() => (storyStopped = null)}>Dismiss</button>
      </div>
    </div>
  {/if}

  {#if stale}
    <div class="banner">
      <p>
        The timer for <strong>{stale.title}</strong> was still running when the app last ran
        (about {clockTime(stale.last_seen)}).
      </p>
      <div class="banner-actions">
        <button class="btn" onclick={stopAtLastSeen}>Stop it at {clockTime(stale.last_seen)}</button>
        <button class="btn" onclick={keepRunning}>Keep it running</button>
      </div>
    </div>
  {/if}

  <section class="stats" aria-label="Summary">
    <div class="stat card">
      <span class="label">Today</span>
      <span class="value">{statTime(todaySec + liveExtra)}</span>
      <span class="caption">tracked</span>
    </div>
    <div class="stat card">
      <span class="label">This month</span>
      <span class="value">{statTime(monthSec + liveExtra)}</span>
      <span class="caption">tracked</span>
    </div>
    <div class="stat card" class:attention={unpushedSeconds >= 60 || toCreate > 0 || parentChanges > 0}>
      <span class="label">Waiting to be pushed</span>
      <span class="value">{statTime(unpushedSeconds)}</span>
      <span class="caption">
        {toCreate} to create · {parentChanges} parent change{parentChanges === 1 ? "" : "s"}
      </span>
    </div>
    <div class="stat card" class:live={running}>
      <span class="label">Timer</span>
      {#if running}
        <span class="value">{clockDuration(elapsedOf(running))}</span>
        <span class="caption ellipsis" title={running.title}>{running.title}</span>
      {:else}
        <span class="value muted">Idle</span>
        <span class="caption">no timer running</span>
      {/if}
    </div>
  </section>
  <p class="lock muted small">Nothing is sent to Jira until you press <strong>Push</strong> on a task.</p>

  <form class="add" onsubmit={addTask}>
    <input class="input big" placeholder="What are you working on?" bind:value={title} aria-label="New task" />
    <button class="btn primary" type="submit">Add task</button>
  </form>

  <section class="toolbar" aria-label="View options">
    <div class="segmented" role="group" aria-label="View">
      <button class:active={view === "epic"} aria-pressed={view === "epic"} onclick={() => setView("epic")}>By epic</button>
      <button class:active={view === "month"} aria-pressed={view === "month"} onclick={() => setView("month")}>By month created</button>
    </div>
    <input class="input search" placeholder="Search title or key" bind:value={query} aria-label="Search" />
    {#if view === "month"}
      <select class="input month-select" bind:value={monthFilter} aria-label="Month">
        <option value="">All months</option>
        {#each allMonths as m (m.key)}<option value={m.key}>{m.label}</option>{/each}
      </select>
    {/if}
    <label class="check"><input type="checkbox" bind:checked={hideDone} /> Hide done</label>
    <button class="btn small subtle" onclick={() => setAllGroups(!anyOpen)} title={anyOpen ? "Close every group" : "Open every group"}>
      {anyOpen ? "Collapse all" : "Expand all"}
    </button>
    <button class="btn small" onclick={selectAllShown} disabled={visibleTasks.length === 0} title="Tick every task you can see">Select all shown</button>
    <button class="btn small subtle" onclick={() => (showHidden = !showHidden)} aria-expanded={showHidden} title="Jira issues you removed from the list">
      Hidden ({hidden.length})
    </button>
  </section>

  {#if typeChips.length > 1}
    <div class="chips" role="group" aria-label="Filter by issue type">
      <button class="chip" class:active={typeFilter === ""} onclick={() => (typeFilter = "")}>All</button>
      {#each typeChips as [name, count] (name)}
        <button class="chip {name.toLowerCase().replace(/\s+/g, '-')}" class:active={typeFilter === name} onclick={() => (typeFilter = typeFilter === name ? "" : name)}>
          {name} <span class="count">{count}</span>
        </button>
      {/each}
    </div>
  {/if}

  {#if showHidden}
    <section class="card hidden-panel" aria-label="Hidden issues">
      <h3>Hidden issues</h3>
      {#if hidden.length === 0}
        <p class="muted small">Nothing hidden. Removing a Jira issue from the list (✕) hides it here so a Refresh doesn't bring it back.</p>
      {:else}
        <ul>
          {#each hidden as h (h.jira_key)}
            <li>
              <span><strong>{h.jira_key}</strong> <span class="muted">{h.title}</span></span>
              <button class="btn small" onclick={() => restoreHidden(h.jira_key)}>Restore</button>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  {/if}

  {#if tasks.filter((t) => !t.context_only).length === 0}
    <div class="empty card">
      <h2>No tasks yet</h2>
      <p class="muted">
        Type what you are working on above and press ▶ when you start.
        {#if settings.has_token}Or press <strong>Refresh from Jira</strong> to bring in your open issues.{:else}Add your Jira details in Settings to bring in your open issues.{/if}
      </p>
    </div>
  {:else if view === "epic"}
    {#each groups as g (g.id)}
      <section class="group">
        <div class="group-head">
          <button class="chev" onclick={() => toggleGroup(g.id, epicOpen(g))} aria-label="Collapse or expand" aria-expanded={epicOpen(g)}>
            {epicOpen(g) ? "▾" : "▸"}
          </button>
          {#if g.header && !g.header.context_only}
            <div class="head-row">
              <TaskRow task={g.header} {nowSec} {epics} {actions} canPush={canPush(g.header)} header selected={!!selected[g.header.id]} onselect={select} mrs={mrsByTask.get(g.header.id) ?? []} busyStory={!!writing[g.header.id]} />
            </div>
          {:else if g.header}
            <div class="ctx">
              {#if g.header.issue_type}<span class="pill {g.header.issue_type.toLowerCase()}">{g.header.issue_type}</span>{/if}
              <button class="key" onclick={() => actions.open(g.header!.jira_key!)} title="Open in Jira">{g.header.jira_key}</button>
              <span class="ctx-title">{g.header.title}</span>
            </div>
          {:else}
            <div class="ctx"><span class="ctx-title muted">No parent</span></div>
          {/if}
          <div class="group-sum muted small">
            {g.children.length} item{g.children.length === 1 ? "" : "s"}
            · {humanDuration(sumOf(g.children, elapsedOf))} tracked
            {#if g.children.some((c) => c.jira_logged_seconds !== null)}
              · Jira {humanDuration(sumOf(g.children, (c) => c.jira_logged_seconds ?? 0))}
            {/if}
            {#if g.children.some((c) => c.running_since !== null)}<span class="live-dot">● timer running</span>{/if}
            {#if g.children.some(canPush)}· <span class="topush">{g.children.filter(canPush).length} to push</span>{/if}
          </div>
        </div>
        {#if epicOpen(g)}
          <div class="children" class:flat={g.id === NO_PARENT}>
            {#each rowsOf(g.id, g.children) as t (t.id)}
              <TaskRow task={t} {nowSec} {epics} {actions} canPush={canPush(t)} selected={!!selected[t.id]} onselect={select} mrs={mrsByTask.get(t.id) ?? []} busyStory={!!writing[t.id]} />
            {:else}
              <p class="muted small pad">No stories yet. Set this epic as the parent of a task to see it here.</p>
            {/each}
            {@render moreRows(g.id, g.children.length)}
          </div>
        {/if}
      </section>
    {:else}
      <p class="muted center">Nothing matches your filters.</p>
    {/each}
  {:else}
    {#each months as m (m.key)}
      <section class="group">
        <div class="month-head">
          <button class="chev" onclick={() => toggleGroup(`m:${m.key}`, monthOpen(m))} aria-label="Collapse or expand" aria-expanded={monthOpen(m)}>
            {monthOpen(m) ? "▾" : "▸"}
          </button>
          <h2>{m.label}</h2>
          <span class="muted small">
            {m.tasks.length} created · {monthTypeChips(m.tasks)}
            · {humanDuration(sumOf(m.tasks, elapsedOf))} tracked
            {#if m.tasks.some((t) => t.jira_logged_seconds !== null)}
              · Jira {humanDuration(sumOf(m.tasks, (t) => t.jira_logged_seconds ?? 0))}
            {/if}
            {#if m.tasks.some(canPush)}· <span class="topush">{m.tasks.filter(canPush).length} to push</span>{/if}
          </span>
        </div>
        {#if monthOpen(m)}
          <div class="children flat">
            {#each rowsOf(`m:${m.key}`, m.tasks) as t (t.id)}
              <TaskRow task={t} {nowSec} {epics} {actions} canPush={canPush(t)} selected={!!selected[t.id]} onselect={select} mrs={mrsByTask.get(t.id) ?? []} busyStory={!!writing[t.id]} />
            {/each}
            {@render moreRows(`m:${m.key}`, m.tasks.length)}
          </div>
        {/if}
      </section>
    {:else}
      <p class="muted center">Nothing matches your filters.</p>
    {/each}
  {/if}
  </div>
</main>

{#snippet moreRows(id: string, total: number)}
  {#if total > ROWS_SHOWN}
    <button class="btn small subtle more" onclick={() => (showAllRows[id] = !showAllRows[id])}>
      {showAllRows[id] ? "Show fewer" : `Show ${total - ROWS_SHOWN} more`}
    </button>
  {/if}
{/snippet}

{#if selectedTasks.length > 0}
  <div class="selection-bar" role="region" aria-label="Selected tasks">
    <div class="selection-inner">
      <strong>{selectedTasks.length} selected</strong>
      <div class="selection-actions">
        <button
          class="btn soft"
          onclick={() => generateFor(selectedNeedingStory.map((t) => t.id), false)}
          disabled={selectedNeedingStory.length === 0 || Object.keys(writing).length > 0}
          title="Draft a title, description and acceptance criteria for each selected task that isn't in Jira yet (local AI)"
        >
          Draft stories ({selectedNeedingStory.length})
        </button>
        <button
          class="btn primary"
          onclick={() => (showBulk = true)}
          disabled={selectedPushable.length === 0 || !settings.has_token}
          title="Review and send the selected tasks to Jira in one go"
        >
          ⬆ Push selected ({selectedPushable.length})
        </button>
        <button class="btn subtle" onclick={clearSelection}>Clear</button>
      </div>
    </div>
  </div>
{/if}

{#if pushTarget}
  <PushDialog task={pushTarget} onclose={() => (pushTarget = null)} ondone={pushDone} />
{/if}

{#if showBulk}
  <BulkPushDialog tasks={selectedPushable} onclose={() => (showBulk = false)} ondone={bulkPushDone} />
{/if}

{#if showImport}
  <ImportMrDialog {connections} onclose={() => (showImport = false)} ondone={importDone} />
{/if}

<style>
  .topbar {
    position: sticky;
    top: 0;
    z-index: 5;
    background: color-mix(in srgb, var(--surface) 88%, transparent);
    backdrop-filter: blur(8px);
    border-bottom: 1px solid var(--border);
  }
  .topbar-inner {
    max-width: 62rem;
    margin: 0 auto;
    padding: 0.7rem 1.2rem;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    flex-wrap: wrap;
  }
  .brand {
    font-weight: 800;
    font-size: 1.1rem;
    letter-spacing: -0.01em;
  }
  .tabs {
    display: flex;
    gap: 0.2rem;
    flex: 1;
    margin-left: 0.6rem;
  }
  .tab {
    position: relative;
    border: 0;
    background: transparent;
    padding: 0.5rem 0.9rem;
    border-radius: 8px;
    font-weight: 600;
    color: var(--muted);
    cursor: pointer;
  }
  .tab:hover {
    background: var(--surface-2);
    color: var(--text);
  }
  .tab.active {
    color: var(--accent);
    background: var(--accent-soft);
  }
  .badge {
    display: inline-grid;
    place-items: center;
    width: 1.1rem;
    height: 1.1rem;
    margin-left: 0.3rem;
    border-radius: 999px;
    background: var(--warn);
    color: #fff;
    font-size: 0.7rem;
    font-weight: 800;
  }
  /* Both the tab view and `hidden` need to win over the grid display below. */
  .view {
    display: grid;
    gap: 1rem;
  }
  .view[hidden] {
    display: none;
  }
  .logo {
    color: var(--accent);
  }
  .topbar-actions {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    flex-wrap: wrap;
  }
  .small {
    font-size: 0.82rem;
  }

  main {
    max-width: 62rem;
    margin: 0 auto;
    padding: 1.2rem 1.2rem 4rem;
    display: grid;
    gap: 1rem;
  }

  .msg {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.8rem;
    padding: 0.6rem 0.9rem;
    border-radius: 10px;
    border: 1px solid transparent;
    overflow-wrap: anywhere;
  }
  .msg.ok {
    background: var(--good-soft);
    color: var(--good);
  }
  .msg.error {
    background: var(--bad-soft);
    color: var(--bad);
  }

  .banner {
    background: var(--warn-soft);
    border: 1px solid var(--warn);
    color: var(--warn);
    border-radius: 10px;
    padding: 0.8rem 1rem;
    display: grid;
    gap: 0.6rem;
  }
  .banner-actions {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  /* summary cards */
  .stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
    gap: 0.8rem;
  }
  .stat {
    display: grid;
    gap: 0.1rem;
    padding: 0.8rem 1rem;
    min-width: 0;
  }
  .stat .label {
    font-size: 0.78rem;
    font-weight: 600;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .stat .value {
    font-size: 1.6rem;
    font-weight: 800;
    font-variant-numeric: tabular-nums;
    line-height: 1.2;
  }
  .stat .caption {
    font-size: 0.8rem;
    color: var(--muted);
  }
  .ellipsis {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .stat.attention {
    border-color: var(--accent);
  }
  .stat.attention .value {
    color: var(--accent);
  }
  .stat.live {
    border-color: var(--good);
  }
  .stat.live .value {
    color: var(--good);
  }
  .lock {
    margin-top: -0.4rem;
  }

  /* add + toolbar */
  .add {
    display: flex;
    gap: 0.6rem;
  }
  .input.big {
    padding: 0.8rem 1rem;
    font-size: 1.02rem;
    border-radius: 10px;
  }
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.6rem;
  }
  .search {
    flex: 1;
    min-width: 12rem;
    width: auto;
  }
  .month-select {
    width: auto;
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.9rem;
    color: var(--muted);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }
  .chip {
    border: 1px solid var(--border);
    background: var(--surface);
    border-radius: 999px;
    padding: 0.2rem 0.75rem;
    font-size: 0.85rem;
    font-weight: 600;
    color: var(--muted);
    cursor: pointer;
  }
  .chip .count {
    margin-left: 0.2rem;
    opacity: 0.7;
    font-weight: 500;
  }
  .chip:hover {
    border-color: var(--border-strong);
  }
  .chip.active {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .chip.bug:not(.active),
  .chip.defect:not(.active) {
    color: var(--bad);
  }
  .hidden-panel {
    padding: 0.8rem 1.1rem 1rem;
    display: grid;
    gap: 0.5rem;
  }
  .hidden-panel h3 {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--muted);
  }
  .hidden-panel ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.4rem;
  }
  .hidden-panel li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.8rem;
    padding: 0.4rem 0.6rem;
    background: var(--surface-2);
    border-radius: 8px;
  }
  .segmented {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: 9px;
    overflow: hidden;
    background: var(--surface);
  }
  .segmented button {
    border: 0;
    background: transparent;
    padding: 0.45rem 0.9rem;
    cursor: pointer;
    color: var(--muted);
    font-weight: 600;
  }
  .segmented button.active {
    background: var(--accent);
    color: #fff;
  }

  /* groups */
  .group {
    display: grid;
    gap: 0.5rem;
  }
  .group-head {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    align-items: center;
    column-gap: 0.4rem;
    row-gap: 0.2rem;
  }
  .chev {
    border: 0;
    background: transparent;
    color: var(--muted);
    cursor: pointer;
    width: 1.6rem;
    height: 1.6rem;
    border-radius: 6px;
  }
  .chev:hover {
    background: var(--surface-2);
  }
  .head-row {
    min-width: 0;
  }
  .ctx {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 0.2rem;
    min-width: 0;
  }
  .ctx-title {
    font-weight: 700;
    font-size: 1.02rem;
    overflow-wrap: anywhere;
  }
  .group-sum {
    grid-column: 2;
    padding-left: 0.2rem;
  }
  .children {
    display: grid;
    gap: 0.5rem;
    margin-left: 1.1rem;
    padding-left: 0.9rem;
    border-left: 2px solid var(--border);
  }
  .children.flat {
    margin-left: 0;
    padding-left: 0;
    border-left: 0;
  }
  .pad {
    padding: 0.4rem 0.2rem;
  }
  .more {
    justify-self: start;
  }
  .live-dot {
    color: var(--good);
    font-weight: 600;
    margin-left: 0.4rem;
  }
  .topush {
    color: var(--accent);
    font-weight: 600;
  }
  .month-head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.3rem 0.9rem;
    padding: 0.4rem 0.1rem;
    border-bottom: 1px solid var(--border);
  }
  .month-head h2 {
    font-size: 1.15rem;
  }

  .selection-bar {
    position: fixed;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 8;
    background: var(--surface);
    border-top: 1px solid var(--border-strong);
    box-shadow: 0 -6px 24px rgba(16, 24, 40, 0.14);
  }
  .selection-inner {
    max-width: 62rem;
    margin: 0 auto;
    padding: 0.7rem 1.2rem;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    flex-wrap: wrap;
  }
  .selection-actions {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  .empty {
    padding: 2rem 1.4rem;
    text-align: center;
    display: grid;
    gap: 0.4rem;
  }
  .center {
    text-align: center;
    padding: 1.5rem 0;
  }
</style>
