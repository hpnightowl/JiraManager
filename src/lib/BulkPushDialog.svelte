<script lang="ts">
  // Push several tasks from one review screen: pick project, type and assignee once, check every
  // story, parent and time, confirm once. Each task succeeds or fails on its own.
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import CreateFields from "$lib/CreateFields.svelte";
  import { humanDuration, recall, remember } from "$lib/format";
  import { missingRequired } from "$lib/forms";
  import type { BulkItem, CreateForm, PushPreview, Task } from "$lib/types";

  let { tasks, onclose, ondone }: { tasks: Task[]; onclose: () => void; ondone: (items: BulkItem[]) => void } = $props();

  // The selection doesn't change while the dialog is open, so reading it once is intended.
  // svelte-ignore state_referenced_locally
  const toCreate = tasks.filter((t) => t.jira_key === null);
  const needsIssue = toCreate.length > 0;

  let previews = $state<PushPreview[]>([]);
  let roundMinutes = $state(Number(recall("pj.round") ?? "0") || 0);

  let projectKey = $state("");
  let typeId = $state("");
  let typeName = $state<string | null>(null);
  let form = $state<CreateForm | null>(null);
  let answers = $state<Record<string, string>>({});
  let multi = $state<Record<string, string[]>>({});
  let remember_ = $state(true);

  let error = $state("");
  let busy = $state(false);
  let items = $state<BulkItem[] | null>(null);

  async function loadPreviews() {
    try {
      error = "";
      previews = await invoke<PushPreview[]>("get_bulk_preview", { taskIds: tasks.map((t) => t.id), roundMinutes });
    } catch (e) {
      error = String(e);
    }
  }

  async function changeRounding(minutes: number) {
    roundMinutes = minutes;
    remember("pj.round", String(minutes));
    await loadPreviews();
  }

  const previewOf = (id: number) => previews.find((p) => p.task_id === id);

  // One row per task, with what will happen to it.
  const rows = $derived(
    tasks.map((t) => {
      const p = previewOf(t.id);
      const seconds = p?.total_log_seconds ?? 0;
      const parent = p?.pending_parent_key ?? null;
      const nothing = t.jira_key !== null && seconds === 0 && parent === null;
      return { task: t, seconds, parent, nothing, hasStory: !!(t.story_description || t.story_title), shownTitle: t.story_title ?? t.title };
    }),
  );

  const worklogs = $derived(previews.reduce((n, p) => n + p.entries.filter((e) => e.skipped === null).length, 0));
  const totalSeconds = $derived(previews.reduce((n, p) => n + p.total_log_seconds, 0));
  const parents = $derived(rows.filter((r) => r.parent).length);
  const withoutStory = $derived(rows.filter((r) => r.task.jira_key === null && !r.hasStory).length);

  const summary = $derived.by(() => {
    const parts: string[] = [];
    if (needsIssue) {
      parts.push(projectKey && typeName ? `create ${toCreate.length} ${typeName} issue${toCreate.length === 1 ? "" : "s"} in ${projectKey}` : `create ${toCreate.length} issue${toCreate.length === 1 ? "" : "s"}`);
    }
    if (parents > 0) parts.push(`set ${parents} parent${parents === 1 ? "" : "s"}`);
    if (worklogs > 0) parts.push(`log ${worklogs} worklog${worklogs === 1 ? "" : "s"} totalling ${humanDuration(totalSeconds)}`);
    return parts.length ? `This will ${parts.join(", ")}.` : "There is nothing to send.";
  });

  const nothingToDo = $derived(rows.every((r) => r.nothing));
  const missing = $derived(needsIssue ? missingRequired(form, answers, multi) : []);
  const notReady = $derived(needsIssue && (!projectKey || !typeId || form === null || missing.length > 0));

  async function push() {
    busy = true;
    try {
      error = "";
      items = await invoke<BulkItem[]>("push_tasks", {
        request: {
          task_ids: tasks.map((t) => t.id),
          project_key: needsIssue ? projectKey : null,
          issue_type_id: needsIssue ? typeId : null,
          issue_type_name: needsIssue ? typeName : null,
          fields: { ...answers, ...multi },
          round_minutes: roundMinutes,
          remember: remember_,
        },
      });
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  const okCount = $derived(items?.filter((i) => i.result).length ?? 0);
  const failCount = $derived(items?.filter((i) => i.error).length ?? 0);

  onMount(loadPreviews);
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && !busy && (items ? ondone(items) : onclose())} />

<div class="overlay">
  <div class="dialog" role="dialog" aria-modal="true" aria-label="Push selected tasks to Jira">
    {#if items}
      <h2>Push finished: {okCount} done{failCount ? `, ${failCount} failed` : ""}</h2>
      <ul class="results">
        {#each items as item (item.task_id)}
          <li class:fail={item.error}>
            <span class="mark">{item.error ? "✕" : "✓"}</span>
            <div>
              <strong>{item.title}</strong>
              {#if item.result}
                <div class="muted small">
                  {item.result.jira_key}
                  {#if item.result.created_issue}· created{/if}
                  {#if item.result.parent_set}· parent set{/if}
                  · {item.result.worklogs_logged} worklog{item.result.worklogs_logged === 1 ? "" : "s"}
                  {#if item.result.seconds_logged}({humanDuration(item.result.seconds_logged)}){/if}
                </div>
                {#each item.result.warnings as w}<div class="error-text small">{w}</div>{/each}
              {:else}
                <div class="error-text small">{item.error}</div>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
      {#if failCount > 0}
        <p class="muted small">Failed tasks were not changed. Fix the problem and push them again; finished ones won't be repeated.</p>
      {/if}
      <div class="actions"><button class="btn primary" onclick={() => ondone(items!)}>Close</button></div>
    {:else}
      <h2>Push {tasks.length} task{tasks.length === 1 ? "" : "s"} to Jira</h2>
      <p class="muted lead">One review for all of them. Nothing is sent until you confirm at the bottom.</p>

      {#if needsIssue}
        <section>
          <h3>New issues ({toCreate.length}): same settings for each</h3>
          <CreateFields bind:projectKey bind:typeId bind:typeName bind:form bind:answers bind:multi bind:remember={remember_} bind:busy bind:error />
        </section>
      {/if}

      <section>
        <h3>Time</h3>
        <label>
          Round each entry up to
          <select class="input" value={roundMinutes} onchange={(e) => changeRounding(Number(e.currentTarget.value))}>
            <option value={0}>exact time</option>
            <option value={5}>5 minutes</option>
            <option value={15}>15 minutes</option>
            <option value={30}>30 minutes</option>
            <option value={60}>1 hour</option>
          </select>
        </label>
      </section>

      <section>
        <h3>What will happen to each task</h3>
        <table>
          <thead><tr><th>Task</th><th>Jira</th><th>Story</th><th>Time</th></tr></thead>
          <tbody>
            {#each rows as r (r.task.id)}
              <tr class:nothing={r.nothing}>
                <td class="t">{r.shownTitle}</td>
                <td>
                  {#if r.task.jira_key}{r.task.jira_key}{:else}<span class="pill staged">new issue</span>{/if}
                  {#if r.parent}<div class="muted small">parent {r.parent}</div>{/if}
                </td>
                <td>
                  {#if r.task.jira_key}<span class="muted">n/a</span>
                  {:else if r.hasStory}✓ drafted
                  {:else}<span class="muted">title only</span>{/if}
                </td>
                <td>{r.nothing ? "nothing to send" : r.seconds ? humanDuration(r.seconds) : "–"}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        {#if withoutStory > 0}
          <p class="muted small">
            {withoutStory} new issue{withoutStory === 1 ? " has" : "s have"} no drafted story and will be created with just a title.
            Close this and use <strong>Draft stories</strong> first if you want descriptions.
          </p>
        {/if}
      </section>

      <p class="summary">{summary}</p>
      {#if missing.length > 0}<p class="warn">Still needed before you can push: {missing.join(", ")}</p>{/if}
      {#if error}<p class="error-text">{error}</p>{/if}

      <div class="actions">
        <button class="btn" onclick={onclose} disabled={busy}>Cancel</button>
        <button class="btn primary" onclick={push} disabled={busy || nothingToDo || notReady}>
          {busy ? "Pushing… please wait" : `Push ${tasks.length} task${tasks.length === 1 ? "" : "s"}`}
        </button>
      </div>
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
    width: min(760px, 100%);
    max-height: 92vh;
    overflow: auto;
    padding: 0.4rem 1.4rem 1.3rem;
    box-shadow: var(--shadow-lg);
  }
  h2 {
    font-size: 1.2rem;
    margin: 1rem 0 0.2rem;
  }
  h3 {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--muted);
    margin: 1.2rem 0 0.5rem;
  }
  .small {
    font-size: 0.82rem;
  }
  label {
    display: grid;
    gap: 0.3rem;
    font-size: 0.9rem;
    font-weight: 600;
    margin-bottom: 0.7rem;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.88rem;
  }
  th {
    text-align: left;
    font-size: 0.75rem;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 0.2rem 0.4rem;
  }
  td {
    padding: 0.4rem;
    border-top: 1px solid var(--border);
    vertical-align: top;
  }
  td.t {
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  tr.nothing {
    opacity: 0.55;
  }
  .summary {
    font-weight: 700;
    margin: 1.2rem 0 0.6rem;
  }
  .warn {
    color: var(--warn);
    font-size: 0.88rem;
    margin: 0 0 0.6rem;
  }
  .actions {
    display: flex;
    gap: 0.5rem;
    justify-content: flex-end;
    flex-wrap: wrap;
  }
  .results {
    list-style: none;
    margin: 0.8rem 0;
    padding: 0;
    display: grid;
    gap: 0.5rem;
  }
  .results li {
    display: flex;
    gap: 0.7rem;
    padding: 0.6rem 0.8rem;
    border-radius: 10px;
    background: var(--good-soft);
  }
  .results li.fail {
    background: var(--bad-soft);
  }
  .mark {
    font-weight: 800;
  }
</style>
