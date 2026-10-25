<script lang="ts">
  import { clockDuration, humanDuration, shortDate } from "$lib/format";
  import { effectiveParent, isEpic } from "$lib/grouping";
  import type { MergeRequest, RowActions, Task } from "$lib/types";

  let {
    task,
    nowSec,
    epics,
    actions,
    canPush,
    header = false,
    selected = false,
    onselect,
    mrs = [],
    busyStory = false,
  }: {
    task: Task;
    nowSec: number;
    epics: { key: string; title: string }[];
    actions: RowActions;
    canPush: boolean;
    /** Larger styling when this row is the heading of a group (my own epic). */
    header?: boolean;
    selected?: boolean;
    onselect?: (id: number, on: boolean) => void;
    /** GitLab merge requests that belong to this task. */
    mrs?: MergeRequest[];
    /** True while a story for this task is being written. */
    busyStory?: boolean;
  } = $props();

  const hasStory = $derived(!!(task.story_title || task.story_description));
  const storyLabel = $derived(
    task.story_source === "ai" ? "AI draft" : task.story_source === "template" ? "template draft" : "your draft",
  );

  const running = $derived(task.running_since !== null);
  const elapsed = $derived(task.tracked_seconds + (task.running_since !== null ? Math.max(0, nowSec - task.running_since) : 0));
  const parentKey = $derived(effectiveParent(task));
  const staged = $derived(task.pending_parent_key !== null);
  const local = $derived(task.jira_key === null);

  // The parent may be something that isn't an epic (a story with sub-tasks): keep it selectable.
  const parentOptions = $derived(
    parentKey && !epics.some((e) => e.key === parentKey) ? [{ key: parentKey, title: "" }, ...epics] : epics,
  );

  // --- rename (only for tasks that aren't in Jira yet) ---
  let editing = $state(false);
  let draft = $state("");

  function startEdit() {
    draft = task.title;
    editing = true;
  }
  function commitEdit() {
    if (!editing) return;
    editing = false;
    if (draft.trim() && draft.trim() !== task.title) actions.rename(task.id, draft);
  }

  // --- two-step delete: the first click arms it, the second one confirms ---
  let armed = $state(false);
  let disarmTimer: ReturnType<typeof setTimeout> | undefined;
  function remove() {
    if (!armed) {
      armed = true;
      disarmTimer = setTimeout(() => (armed = false), 4000);
      return;
    }
    clearTimeout(disarmTimer);
    actions.remove(task.id);
  }
</script>

<div class="row" class:header class:done={task.done} class:running>
  <input
    class="check"
    type="checkbox"
    checked={selected}
    onchange={(e) => onselect?.(task.id, e.currentTarget.checked)}
    aria-label="Select this task"
    title="Select (for bulk actions)"
  />

  <div class="main">
    <div class="line1">
      {#if task.issue_type}
        <span class="pill {task.issue_type.toLowerCase().replace(/\s+/g, '-')}">{task.issue_type}</span>
      {:else if local}
        <span class="pill">Not in Jira yet</span>
      {/if}
      {#if task.jira_key}
        <button class="key" onclick={() => actions.open(task.jira_key!)} title="Open in Jira">{task.jira_key}</button>
      {/if}
      {#if editing}
        <!-- svelte-ignore a11y_autofocus -->
        <input
          class="input rename"
          bind:value={draft}
          autofocus
          onblur={commitEdit}
          onkeydown={(e) => {
            if (e.key === "Enter") commitEdit();
            if (e.key === "Escape") editing = false;
          }}
        />
      {:else}
        <span class="title">{task.title}</span>
        {#if local}
          <button class="btn subtle small edit" onclick={startEdit} aria-label="Rename task" title="Rename">✎</button>
        {/if}
      {/if}
      {#if task.jira_status}<span class="pill">{task.jira_status}</span>{/if}
    </div>

    <div class="line2 muted">
      {#if task.created_at}<span>Created {shortDate(task.created_at)}</span>{/if}
      {#if !isEpic(task)}
        <label class="parent">
          Parent
          <select onchange={(e) => actions.setParent(task.id, e.currentTarget.value || null)}>
            <option value="" selected={!parentKey}>None</option>
            {#each parentOptions as p (p.key)}
              <option value={p.key} selected={p.key === parentKey}>{p.key}{p.title ? ` · ${p.title}` : ""}</option>
            {/each}
          </select>
        </label>
        {#if staged}
          <span class="pill staged" title="Chosen here. It is only sent to Jira when you press Push.">not pushed yet</span>
        {/if}
      {/if}
      {#each mrs as mr (mr.id)}
        {#if mr.source === "local"}
          <!-- a local branch has no web page to open -->
          <span class="mr branch" title="{mr.reference} · {mr.title} (local branch)">⎇ {mr.reference.split("@").pop()}</span>
        {:else}
          <button
            class="mr {mr.state}"
            onclick={() => actions.openMr(mr.web_url)}
            title="{mr.reference} · {mr.title} ({mr.state}, {mr.role})"
          >
            !{mr.reference.split("!").pop()} {mr.state}
          </button>
        {/if}
      {/each}
      {#if local}
        {#if hasStory}
          <span class="pill story-ready" title="This story is used when you push. Open Push to edit it.">Story ready · {storyLabel}</span>
        {:else}
          <button class="btn subtle small" onclick={() => actions.story(task.id)} disabled={busyStory} title="Draft a title, description and acceptance criteria with the local AI">
            {busyStory ? "Writing…" : "✨ Draft story"}
          </button>
        {/if}
      {/if}
    </div>
  </div>

  <div class="time">
    <div class="clock" title="Time tracked here">{clockDuration(elapsed)}</div>
    {#if task.jira_logged_seconds !== null}
      <div class="sub" title="Time already logged on the Jira issue">Jira {humanDuration(task.jira_logged_seconds)}</div>
    {/if}
    {#if task.unpushed_seconds >= 60}
      <div class="sub push-hint" title="Finished time not yet logged in Jira">+{humanDuration(task.unpushed_seconds)} to push</div>
    {/if}
  </div>

  <div class="acts">
    {#if running}
      <button class="btn round stop" onclick={() => actions.stop()} aria-label="Stop timer" title="Stop timer">■</button>
    {:else}
      <button class="btn round" onclick={() => actions.start(task.id)} aria-label="Start timer" title="Start timer">▶</button>
    {/if}
    {#if canPush}
      <button
        class="btn soft small"
        onclick={() => actions.push(task)}
        title={local ? "Create this in Jira, then log your time" : "Send your staged changes and unlogged time to Jira"}
      >
        Push
      </button>
    {/if}
    <button
      class="btn subtle small done-btn"
      class:is-done={task.done}
      onclick={() => actions.toggle(task.id)}
      aria-label={task.done ? "Reopen task" : "Mark as done"}
      title={task.done ? "Reopen" : "Mark as done"}
    >
      ✓
    </button>
    <button
      class="btn subtle small"
      class:danger={armed}
      onclick={remove}
      aria-label="Delete task"
      title={task.jira_key
        ? "Hide this issue: it leaves the list and won't come back on Refresh (restore it from Hidden). The Jira issue is not touched."
        : "Delete this task"}
    >
      {armed ? (task.jira_key ? "Hide?" : "Sure?") : "✕"}
    </button>
  </div>
</div>

<style>
  .row {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto auto;
    align-items: center;
    gap: 0.9rem;
    padding: 0.7rem 0.9rem;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
    transition: border-color 0.15s, box-shadow 0.15s;
  }
  .row.running {
    border-color: var(--good);
    box-shadow: 0 0 0 1px var(--good), var(--shadow);
  }
  .row.header {
    background: var(--surface-2);
  }
  .check {
    width: 1.1rem;
    height: 1.1rem;
    accent-color: var(--accent);
  }
  .main {
    min-width: 0;
  }
  .line1 {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.45rem;
  }
  .title {
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .row.header .title {
    font-size: 1.05rem;
  }
  .row.done .title {
    text-decoration: line-through;
    opacity: 0.55;
  }
  .edit {
    opacity: 0;
    transition: opacity 0.12s;
  }
  .row:hover .edit,
  .edit:focus-visible {
    opacity: 1;
  }
  .rename {
    flex: 1;
    min-width: 12rem;
    padding: 0.25rem 0.5rem;
  }
  .line2 {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.4rem 0.9rem;
    margin-top: 0.25rem;
    font-size: 0.82rem;
  }
  .parent {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
  }
  .parent select {
    max-width: 16rem;
    padding: 0.1rem 0.3rem;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface);
    font-size: 0.82rem;
  }
  .time {
    text-align: right;
    min-width: 5.5rem;
    font-variant-numeric: tabular-nums;
  }
  .clock {
    font-size: 1.1rem;
    font-weight: 700;
  }
  .row.running .clock {
    color: var(--good);
  }
  .sub {
    font-size: 0.76rem;
    color: var(--muted);
  }
  .push-hint {
    color: var(--accent);
    font-weight: 600;
  }
  .acts {
    display: flex;
    align-items: center;
    gap: 0.35rem;
  }
  .stop {
    color: var(--bad);
    border-color: var(--bad);
  }
  .done-btn.is-done {
    color: var(--good);
    background: var(--good-soft);
  }
  .mr {
    padding: 0 0.5rem;
    border: 0;
    border-radius: 6px;
    background: var(--epic-soft);
    color: var(--epic);
    font-size: 0.78rem;
    font-weight: 700;
    cursor: pointer;
  }
  .mr.merged {
    background: var(--good-soft);
    color: var(--good);
  }
  .mr.closed,
  .mr.branch {
    background: var(--surface-2);
    color: var(--muted);
  }
  .mr.branch {
    cursor: default;
    max-width: 14rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .story-ready {
    background: var(--accent-soft);
    border-color: transparent;
    color: var(--accent);
  }

  @media (max-width: 640px) {
    .row {
      grid-template-columns: auto minmax(0, 1fr);
    }
    .time,
    .acts {
      grid-column: 2;
      text-align: left;
    }
  }
</style>
