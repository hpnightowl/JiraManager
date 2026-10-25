<script lang="ts">
  // Imports merge requests (GitLab) and feature branches (local repositories) as tasks and can draft
  // a story for each new one. Read-only towards GitLab and git; nothing here touches Jira.
  import { invoke } from "@tauri-apps/api/core";
  import type { GenerateReport, GitConnection, ImportSummary } from "$lib/types";

  let {
    connections,
    onclose,
    ondone,
  }: { connections: GitConnection[]; onclose: () => void; ondone: (r: ImportSummary | null) => void } = $props();

  const pad = (n: number) => String(n).padStart(2, "0");
  const firstOfMonth = (() => {
    const d = new Date();
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-01`;
  })();

  // A GitLab server without a token can't be read, so it isn't ticked by default.
  const usable = (c: GitConnection) => c.kind === "local" || c.has_token;
  // svelte-ignore state_referenced_locally
  let chosen = $state<Record<number, boolean>>(Object.fromEntries(connections.map((c) => [c.id, usable(c)])));

  let since = $state(firstOfMonth);
  let includeAssigned = $state(true);
  let mrState = $state("all"); // not called `state`: that would hide Svelte's own $state
  let busy = $state(false);
  let error = $state("");
  let summary = $state<ImportSummary | null>(null);

  let writing = $state(false);
  let stories = $state<GenerateReport | null>(null);

  const chosenIds = $derived(connections.filter((c) => chosen[c.id]).map((c) => c.id));
  const hasGitlab = $derived(connections.some((c) => chosen[c.id] && c.kind === "gitlab"));

  async function importAll() {
    busy = true;
    error = "";
    try {
      // The date is in the user's own time zone: midnight there.
      const sinceUnix = Math.floor(new Date(`${since}T00:00:00`).getTime() / 1000);
      summary = await invoke<ImportSummary>("import_git_changes", {
        connectionIds: chosenIds,
        since: sinceUnix,
        includeAssigned,
        state: mrState,
      });
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function writeStories(useTemplate: boolean) {
    if (!summary) return;
    // After an AI run only the tasks that didn't get a story yet are left to do.
    const done = new Set(stories?.done ?? []);
    const ids = summary.new_task_ids.filter((id) => !done.has(id));
    writing = true;
    error = "";
    try {
      const r = await invoke<GenerateReport>("generate_stories", { taskIds: ids, useTemplate });
      stories = { done: [...done, ...r.done], failed: r.failed, stopped: r.stopped };
    } catch (e) {
      error = String(e);
    } finally {
      writing = false;
    }
  }

  const remaining = $derived(summary ? summary.new_task_ids.length - (stories?.done.length ?? 0) : 0);
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && !busy && !writing && ondone(summary)} />

<div class="overlay">
  <div class="dialog" role="dialog" aria-modal="true" aria-label="Import from git">
    <h2>Import from your git sources</h2>

    {#if !summary}
      <p class="muted lead">
        Reads your merge requests and branches and makes one task for each. A change whose title or branch contains a
        Jira key like <strong>ABC-123</strong> is attached to that issue instead. Nothing is written to GitLab, git or Jira.
      </p>

      <fieldset>
        <legend>Read from</legend>
        {#each connections as c (c.id)}
          <label class="check" class:disabled={!usable(c)}>
            <input type="checkbox" bind:checked={chosen[c.id]} disabled={!usable(c)} />
            <span>
              <strong>{c.name}</strong>
              <span class="muted small">· {c.kind === "gitlab" ? "GitLab" : "local repository"}</span>
              {#if !usable(c)}<span class="warn small">· needs a token (Settings)</span>{/if}
            </span>
          </label>
        {/each}
      </fieldset>

      <label>
        Created / committed since
        <input class="input" type="date" bind:value={since} />
      </label>
      {#if hasGitlab}
        <label>
          GitLab merge requests
          <select class="input" bind:value={mrState}>
            <option value="all">All (open, merged, closed)</option>
            <option value="merged">Merged only</option>
            <option value="opened">Open only</option>
          </select>
        </label>
        <label class="check"><input type="checkbox" bind:checked={includeAssigned} /> Also merge requests assigned to me</label>
      {/if}
      {#if error}<p class="error-text">{error}</p>{/if}
      <div class="actions">
        <button class="btn" onclick={() => ondone(null)} disabled={busy}>Cancel</button>
        <button class="btn primary" onclick={importAll} disabled={busy || !since || chosenIds.length === 0}>
          {busy ? "Importing… (reads commits too)" : `Import from ${chosenIds.length} source${chosenIds.length === 1 ? "" : "s"}`}
        </button>
      </div>
    {:else}
      <p>
        Found <strong>{summary.fetched}</strong> change{summary.fetched === 1 ? "" : "s"}:
        <strong>{summary.new_tasks}</strong> new task{summary.new_tasks === 1 ? "" : "s"},
        <strong>{summary.linked_to_existing}</strong> attached to existing Jira task{summary.linked_to_existing === 1 ? "" : "s"},
        {summary.refreshed} already imported.
      </p>

      <ul class="results">
        {#each summary.connections as r (r.connection_id)}
          <li class:fail={r.error}>
            <strong>{r.name || "(removed connection)"}</strong>
            {#if r.error}
              <div class="error-text small">{r.error}</div>
            {:else if r.report}
              <div class="muted small">
                {r.report.fetched} found · {r.report.new_tasks} new · {r.report.linked_to_existing} attached · {r.report.refreshed} already imported
              </div>
            {/if}
          </li>
        {/each}
      </ul>

      {#if summary.new_task_ids.length > 0}
        <div class="stories">
          <h3>Stories for the new tasks</h3>
          {#if !stories}
            <p class="muted small">
              The local AI can draft a title, description and acceptance criteria for each new task from its merge
              request or commits. You can review and edit every story before anything is pushed.
            </p>
            <div class="actions left">
              <button class="btn soft" onclick={() => writeStories(false)} disabled={writing}>
                {writing ? `Writing ${summary.new_task_ids.length} stories…` : `Draft ${summary.new_task_ids.length} stories with AI`}
              </button>
              <button class="btn subtle" onclick={() => writeStories(true)} disabled={writing}>Use simple templates</button>
            </div>
          {:else}
            <p>Drafted <strong>{stories.done.length}</strong> of {summary.new_task_ids.length} stories.</p>
            {#if stories.stopped}
              <p class="warn">{stories.stopped}</p>
              <div class="actions left">
                <button class="btn" onclick={() => writeStories(false)} disabled={writing}>Try the AI again</button>
                <button class="btn" onclick={() => writeStories(true)} disabled={writing}>Use simple templates for the remaining {remaining}</button>
              </div>
            {/if}
            {#each stories.failed as f}<p class="error-text small">Task {f.task_id}: {f.message}</p>{/each}
          {/if}
        </div>
      {/if}

      {#if error}<p class="error-text">{error}</p>{/if}
      <div class="actions"><button class="btn primary" onclick={() => ondone(summary)} disabled={writing}>Done</button></div>
    {/if}
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(10, 14, 24, 0.55);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 1rem;
    z-index: 20;
  }
  .dialog {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 14px;
    width: min(580px, 100%);
    max-height: 92vh;
    overflow: auto;
    padding: 0.4rem 1.4rem 1.3rem;
    box-shadow: var(--shadow-lg);
  }
  h2 {
    font-size: 1.2rem;
    margin: 1rem 0 0.4rem;
  }
  h3 {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--muted);
    margin: 0 0 0.4rem;
  }
  .lead {
    margin-bottom: 1rem;
    font-size: 0.92rem;
  }
  fieldset {
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 0.5rem 0.8rem;
    margin: 0 0 0.9rem;
    display: grid;
    gap: 0.4rem;
  }
  legend {
    font-size: 0.9rem;
    font-weight: 600;
    padding: 0 0.3rem;
  }
  label {
    display: grid;
    gap: 0.3rem;
    font-size: 0.9rem;
    font-weight: 600;
    margin-bottom: 0.8rem;
  }
  label.check {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-weight: 500;
    margin: 0;
  }
  label.check.disabled {
    opacity: 0.6;
  }
  fieldset label.check {
    margin: 0;
  }
  .small {
    font-size: 0.85rem;
  }
  .results {
    list-style: none;
    margin: 0.6rem 0;
    padding: 0;
    display: grid;
    gap: 0.4rem;
  }
  .results li {
    padding: 0.5rem 0.8rem;
    border-radius: 8px;
    background: var(--good-soft);
  }
  .results li.fail {
    background: var(--bad-soft);
  }
  .stories {
    margin: 1rem 0;
    padding: 0.8rem 1rem;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--surface-2);
    display: grid;
    gap: 0.5rem;
  }
  .warn {
    color: var(--warn);
    font-size: 0.9rem;
  }
  .actions {
    display: flex;
    gap: 0.5rem;
    justify-content: flex-end;
    flex-wrap: wrap;
  }
  .actions.left {
    justify-content: flex-start;
  }
</style>
