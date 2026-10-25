<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import CreateFields from "$lib/CreateFields.svelte";
  import { dateTime, humanDuration, recall, remember } from "$lib/format";
  import { missingRequired } from "$lib/forms";
  import type { CreateForm, Improvement, PushPreview, PushResult, Task } from "$lib/types";

  let { task, onclose, ondone }: { task: Task; onclose: () => void; ondone: (r: PushResult) => void } = $props();

  // `task` never changes while the dialog is open, so reading it once here is intended.
  // svelte-ignore state_referenced_locally
  const needsIssue = task.jira_key === null;

  let preview = $state<PushPreview | null>(null);
  let roundMinutes = $state(Number(recall("pj.round") ?? "0") || 0);

  // The issue: where it goes and what Jira requires (shared with the bulk dialog).
  let projectKey = $state("");
  let typeId = $state("");
  let typeName = $state<string | null>(null);
  let form = $state<CreateForm | null>(null);
  let answers = $state<Record<string, string>>({});
  let multi = $state<Record<string, string[]>>({});
  let remember_ = $state(true);

  // The story. It starts from the draft saved on the task, if there is one.
  // svelte-ignore state_referenced_locally
  let title = $state(task.story_title ?? task.title);
  // svelte-ignore state_referenced_locally
  let description = $state(task.story_description ?? "");
  let suggestion = $state<Improvement | null>(null);
  let suggesting = $state(false);
  let aiProblem = $state(""); // why the AI wasn't available, shown next to the template button

  let error = $state("");
  let busy = $state(false);
  let result = $state<PushResult | null>(null);

  async function call<T>(fn: () => Promise<T>): Promise<T | undefined> {
    try {
      error = "";
      return await fn();
    } catch (e) {
      error = String(e);
    }
  }

  async function loadPreview() {
    preview = (await call(() => invoke<PushPreview>("get_push_preview", { taskId: task.id, roundMinutes }))) ?? preview;
  }

  async function changeRounding(minutes: number) {
    roundMinutes = minutes;
    remember("pj.round", String(minutes));
    await loadPreview();
  }

  // --- "Improve with AI" ---
  // The AI only proposes. Nothing changes until the user presses "Use this".
  async function improveWithAi() {
    suggesting = true;
    aiProblem = "";
    suggestion = null;
    try {
      suggestion = await invoke<Improvement>("suggest_story", { title, description });
    } catch (e) {
      aiProblem = String(e); // e.g. Ollama isn't running: offer the template instead
    } finally {
      suggesting = false;
    }
  }

  async function useTemplate() {
    aiProblem = "";
    suggestion = (await call(() => invoke<Improvement>("template_story", { title, description }))) ?? null;
  }

  function applySuggestion() {
    if (!suggestion) return;
    title = suggestion.title;
    const criteria = suggestion.acceptance_criteria.map((c) => `- ${c}`).join("\n");
    description = [suggestion.description, criteria && `Acceptance criteria:\n${criteria}`].filter(Boolean).join("\n\n");
    suggestion = null;
  }

  /** Cancel keeps what you wrote as the task's story draft, so closing the dialog loses nothing. */
  async function closeAndKeepDraft() {
    const changed = title.trim() !== task.title || description.trim() !== "";
    if (needsIssue && changed) {
      try {
        await invoke("save_story_draft", { taskId: task.id, title, description });
      } catch {
        /* a failed draft save must not trap the user in the dialog */
      }
    }
    onclose();
  }

  const pendingParent = $derived(preview?.pending_parent_key ?? null);
  const parentBlocked = $derived(needsIssue && pendingParent !== null && form !== null && !form.can_set_parent);

  // What the user is about to do, in one sentence (the "dry run").
  const summary = $derived.by(() => {
    const parts: string[] = [];
    if (needsIssue) {
      parts.push(projectKey && typeName ? `create 1 ${typeName} in ${projectKey}` : "create 1 issue");
      if (pendingParent) parts.push(`put it under ${pendingParent}`);
    } else if (pendingParent) {
      parts.push(`set the parent of ${task.jira_key} to ${pendingParent}`);
    }
    const n = preview?.entries.filter((e) => e.skipped === null).length ?? 0;
    if (n > 0) {
      const log = `log ${n} worklog${n === 1 ? "" : "s"} totalling ${humanDuration(preview!.total_log_seconds)}`;
      parts.push(needsIssue || pendingParent ? log : `${log} on ${task.jira_key}`);
    }
    return parts.length ? `This will ${parts.join(", then ")}.` : "There is nothing to send.";
  });

  const nothingToDo = $derived(!needsIssue && (preview?.total_log_seconds ?? 0) === 0 && pendingParent === null);
  const missing = $derived.by(() => {
    if (!needsIssue) return [];
    const names = missingRequired(form, answers, multi);
    if (form?.description_required && !description.trim()) names.push("Description");
    return names;
  });
  const notReady = $derived(needsIssue && (!projectKey || !typeId || form === null || parentBlocked || missing.length > 0));

  async function push() {
    busy = true;
    const r = await call(() =>
      invoke<PushResult>("push_task", {
        request: {
          task_id: task.id,
          project_key: needsIssue ? projectKey : null,
          issue_type_id: needsIssue ? typeId : null,
          issue_type_name: needsIssue ? typeName : null,
          title: needsIssue ? title : null,
          description,
          fields: { ...answers, ...multi },
          round_minutes: roundMinutes,
          remember: remember_,
        },
      }),
    );
    busy = false;
    if (r) result = r;
  }

  onMount(loadPreview);
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && !busy && (result ? ondone(result) : closeAndKeepDraft())} />

<div class="overlay">
  <div class="dialog" role="dialog" aria-modal="true" aria-label="Push to Jira">
    {#if result}
      <h2>{result.created_issue ? "Created" : "Updated"} {result.jira_key}</h2>
      <p>
        {#if result.created_issue}Issue created. {/if}
        {#if result.parent_set}Parent set. {/if}
        {#if result.worklogs_logged > 0}
          Logged {result.worklogs_logged} worklog{result.worklogs_logged === 1 ? "" : "s"}
          ({humanDuration(result.seconds_logged)}).
        {:else}
          No time was logged.
        {/if}
        {#if result.skipped > 0}{result.skipped} entr{result.skipped === 1 ? "y was" : "ies were"} too short to log.{/if}
      </p>
      {#each result.warnings as w}<p class="error-text">{w}</p>{/each}
      <div class="actions"><button class="btn primary" onclick={() => ondone(result!)}>Close</button></div>
    {:else}
      <h2>Push “{task.title}” to Jira</h2>
      <p class="muted lead">Review what will be sent. Nothing happens until you confirm at the bottom.</p>

      {#if needsIssue}
        <section>
          <h3>New Jira issue</h3>
          <CreateFields bind:projectKey bind:typeId bind:typeName bind:form bind:answers bind:multi bind:remember={remember_} bind:busy bind:error />

          <h3>Story</h3>
          <label>
            Title
            <input class="input" bind:value={title} maxlength="255" />
          </label>
          <label>
            Description {form?.description_required ? "(required)" : "(optional)"}
            <textarea class="input" rows="5" bind:value={description} placeholder="What and why. Blank lines make paragraphs; lines starting with “- ” become a bullet list."></textarea>
          </label>
          {#if task.story_source && task.story_description}
            <p class="muted small draft-note">Started from the saved {task.story_source === "ai" ? "AI" : task.story_source} draft for this task.</p>
          {/if}

          <div class="ai-row">
            <button class="btn soft" type="button" onclick={improveWithAi} disabled={suggesting || busy || !title.trim()}>
              {suggesting ? "Thinking… (can take a few seconds)" : "Improve with AI"}
            </button>
            <span class="muted small">Runs locally. You review the result before anything is used.</span>
          </div>
          {#if aiProblem}
            <div class="ai-problem">
              <p>{aiProblem}</p>
              <button class="btn small" type="button" onclick={useTemplate}>Use a simple template instead</button>
            </div>
          {/if}
          {#if suggestion}
            <div class="suggestion">
              <div class="suggestion-head">
                {#if suggestion.source === "ai"}
                  <span class="pill">AI suggestion · {suggestion.model}</span>
                {:else}
                  <span class="pill">Template (no AI)</span>
                {/if}
                <span class="muted small">Check it for accuracy: small models can get things wrong.</span>
              </div>
              <p class="s-title">{suggestion.title}</p>
              <p class="s-desc">{suggestion.description}</p>
              {#if suggestion.acceptance_criteria.length > 0}
                <p class="field-label">Acceptance criteria</p>
                <ul>
                  {#each suggestion.acceptance_criteria as c}<li>{c}</li>{/each}
                </ul>
              {/if}
              <div class="actions">
                <button class="btn subtle" type="button" onclick={() => (suggestion = null)}>Dismiss</button>
                <button class="btn" type="button" onclick={improveWithAi} disabled={suggesting || suggestion.source !== "ai"}>Try again</button>
                <button class="btn primary" type="button" onclick={applySuggestion}>Use this</button>
              </div>
            </div>
          {/if}

          {#if pendingParent}
            <p class:error-text={parentBlocked} class="parent-note">
              {#if parentBlocked}
                Parent <strong>{pendingParent}</strong> can't be set for this issue type or project. Clear the parent on
                the task first.
              {:else}
                Parent: <strong>{pendingParent}</strong> (sent together with the new issue)
              {/if}
            </p>
          {/if}
        </section>
      {:else}
        <p>Linked to <strong>{task.jira_key}</strong>. Only your staged changes and unlogged time will be sent.</p>
        {#if pendingParent}<p class="parent-note">Parent: <strong>{pendingParent}</strong> (will be set on push)</p>{/if}
      {/if}

      <section>
        <h3>Time to log</h3>
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
        {#if preview && preview.entries.length > 0}
          <table>
            <tbody>
              {#each preview.entries as e (e.entry_id)}
                <tr class:skipped={e.skipped !== null}>
                  <td>{dateTime(e.started_at)}</td>
                  <td>worked {humanDuration(e.seconds)}</td>
                  <td>{e.skipped ? `skipped: ${e.skipped}` : `logs ${humanDuration(e.log_seconds)}`}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {:else}
          <p class="muted small">No finished time to log. A timer that is still running is not included.</p>
        {/if}
      </section>

      <p class="summary">{summary}</p>
      {#if missing.length > 0}<p class="need">Still needed before you can push: {missing.join(", ")}</p>{/if}
      {#if error}<p class="error-text">{error}</p>{/if}

      <div class="actions">
        <button class="btn" onclick={closeAndKeepDraft} disabled={busy}>Cancel</button>
        <button class="btn primary" onclick={push} disabled={busy || nothingToDo || notReady}>
          {busy ? "Working…" : needsIssue ? "Create issue and log time" : "Send to Jira"}
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
    width: min(680px, 100%);
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
  .lead {
    margin-bottom: 0.2rem;
  }
  .small {
    font-size: 0.85rem;
  }
  label {
    display: grid;
    gap: 0.3rem;
    font-size: 0.9rem;
    font-weight: 600;
    margin-bottom: 0.7rem;
  }
  .field-label {
    font-size: 0.9rem;
    font-weight: 600;
  }
  .actions {
    display: flex;
    gap: 0.5rem;
    justify-content: flex-end;
    flex-wrap: wrap;
  }
  .draft-note {
    margin: -0.3rem 0 0.6rem;
  }
  .ai-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.6rem;
    margin-bottom: 0.7rem;
  }
  .ai-problem {
    display: grid;
    gap: 0.4rem;
    justify-items: start;
    padding: 0.6rem 0.8rem;
    margin-bottom: 0.7rem;
    border-radius: 8px;
    background: var(--warn-soft);
    color: var(--warn);
    font-size: 0.88rem;
  }
  .suggestion {
    display: grid;
    gap: 0.4rem;
    padding: 0.8rem 1rem;
    margin-bottom: 0.8rem;
    border: 1px solid var(--accent);
    border-radius: 10px;
    background: var(--accent-soft);
  }
  .suggestion-head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
  .suggestion ul {
    margin: 0;
    padding-left: 1.2rem;
  }
  .s-title {
    font-weight: 700;
  }
  .parent-note {
    margin: 0 0 0.7rem;
    padding: 0.5rem 0.7rem;
    border-radius: 8px;
    background: var(--accent-soft);
  }
  .parent-note.error-text {
    background: var(--bad-soft);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.85rem;
  }
  td {
    padding: 0.3rem 0.4rem;
    border-bottom: 1px solid var(--border);
  }
  tr.skipped {
    opacity: 0.55;
  }
  .summary {
    font-weight: 700;
    margin: 1.2rem 0 0.6rem;
  }
  .need {
    color: var(--warn);
    font-size: 0.88rem;
    margin: 0 0 0.6rem;
  }
</style>
