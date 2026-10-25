<script lang="ts">
  // Project, issue type, required fields and useful optional ones for creating issues. Shared by the
  // single and bulk Push dialogs.
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { recall, remember } from "$lib/format";
  import type { CreateForm, FieldSpec, IssueType, JiraUser, Project } from "$lib/types";

  let {
    projectKey = $bindable(""),
    typeId = $bindable(""),
    typeName = $bindable<string | null>(null),
    form = $bindable<CreateForm | null>(null),
    answers = $bindable<Record<string, string>>({}),
    multi = $bindable<Record<string, string[]>>({}),
    remember: rememberAnswers = $bindable(true),
    busy = $bindable(false),
    error = $bindable(""),
  }: {
    projectKey?: string;
    typeId?: string;
    typeName?: string | null;
    form?: CreateForm | null;
    answers?: Record<string, string>;
    multi?: Record<string, string[]>;
    remember?: boolean;
    busy?: boolean;
    error?: string;
  } = $props();

  let projects = $state<Project[]>([]);
  let issueTypes = $state<IssueType[]>([]);
  let userQuery = $state<Record<string, string>>({});
  let userResults = $state<Record<string, JiraUser[]>>({});
  let userChosen = $state<Record<string, string>>({}); // display name of the chosen person

  async function call<T>(fn: () => Promise<T>): Promise<T | undefined> {
    try {
      error = "";
      return await fn();
    } catch (e) {
      error = String(e);
    }
  }

  // Let the parent know the issue type's name (used in summaries like "create 3 Story in ABC").
  $effect(() => {
    typeName = issueTypes.find((t) => t.id === typeId)?.name ?? null;
  });

  async function loadProjects() {
    busy = true;
    projects = (await call(() => invoke<Project[]>("list_jira_projects"))) ?? [];
    busy = false;
    const last = recall("pj.project");
    if (last && projects.some((p) => p.key === last)) await loadTypes(last);
  }

  // The select handlers pass the NEW value from the event, so they don't depend on whether
  // Svelte has already copied it into the bound variable.
  async function loadTypes(key: string) {
    projectKey = key;
    issueTypes = [];
    typeId = "";
    form = null;
    if (!projectKey) return;
    busy = true;
    issueTypes = (await call(() => invoke<IssueType[]>("list_issue_types", { projectKey }))) ?? [];
    busy = false;
    const lastType = recall(`pj.type.${projectKey}`);
    const pick = issueTypes.find((t) => t.id === lastType) ?? issueTypes.find((t) => t.name === "Task") ?? issueTypes[0];
    if (pick) await loadForm(pick.id);
  }

  async function loadForm(id: string) {
    typeId = id;
    form = null;
    if (!projectKey || !typeId) return;
    busy = true;
    const f = await call(() => invoke<CreateForm>("get_create_form", { projectKey, issueTypeId: typeId }));
    busy = false;
    if (!f) return;
    remember("pj.project", projectKey);
    remember(`pj.type.${projectKey}`, typeId);
    form = f;
    answers = {};
    multi = {};
    userChosen = {};
    for (const spec of [...f.fields, ...f.optional]) {
      const saved = f.saved[spec.id];
      if (spec.kind === "multi_option") multi[spec.id] = Array.isArray(saved) ? saved : [];
      else if (typeof saved === "string") {
        answers[spec.id] = saved;
        if (spec.kind === "user") userChosen[spec.id] = "Previously chosen person";
      } else answers[spec.id] = "";
    }
    // New issues are assigned to you unless you change it.
    if (f.optional.some((s) => s.id === "assignee") && f.me) assignToMe("assignee");
  }

  function assignToMe(specId: string) {
    if (!form?.me) return;
    answers[specId] = form.me.account_id;
    userChosen[specId] = `${form.me.display_name} (me)`;
  }

  function clearUser(specId: string) {
    answers[specId] = "";
    delete userChosen[specId];
  }

  async function searchUsers(spec: FieldSpec) {
    const query = userQuery[spec.id] ?? "";
    userResults[spec.id] = (await call(() => invoke<JiraUser[]>("search_jira_users", { projectKey, query }))) ?? [];
  }

  function chooseUser(spec: FieldSpec, user: JiraUser) {
    answers[spec.id] = user.account_id;
    userChosen[spec.id] = user.display_name;
    userResults[spec.id] = [];
  }

  function toggleMulti(specId: string, optionId: string, on: boolean) {
    const current = multi[specId] ?? [];
    multi[specId] = on ? [...current, optionId] : current.filter((x) => x !== optionId);
  }

  onMount(loadProjects);
</script>

{#snippet control(spec: FieldSpec)}
  {#if spec.kind === "unsupported"}
    <p class="error-text">
      <strong>{spec.name}</strong> is required but can't be filled in here yet ({spec.note}). Create this issue in Jira
      instead.
    </p>
  {:else if spec.kind === "option"}
    <label>
      {spec.name}
      <select class="input" bind:value={answers[spec.id]}>
        <option value="">{spec.required ? "Choose…" : "Project default"}</option>
        {#each spec.allowed as a (a.id)}<option value={a.id}>{a.label}</option>{/each}
      </select>
    </label>
  {:else if spec.kind === "multi_option"}
    <fieldset>
      <legend>{spec.name}</legend>
      <div class="checks">
        {#each spec.allowed as a (a.id)}
          <label class="check">
            <input
              type="checkbox"
              checked={(multi[spec.id] ?? []).includes(a.id)}
              onchange={(e) => toggleMulti(spec.id, a.id, e.currentTarget.checked)}
            />
            {a.label}
          </label>
        {/each}
      </div>
    </fieldset>
  {:else if spec.kind === "user"}
    <div class="user">
      <span class="field-label">
        {spec.name}: <strong>{userChosen[spec.id] ?? (spec.id === "assignee" ? "project default" : "nobody chosen")}</strong>
      </span>
      <div class="row">
        <input
          class="input"
          placeholder="Search by name"
          bind:value={userQuery[spec.id]}
          onkeydown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              searchUsers(spec);
            }
          }}
        />
        <button class="btn" type="button" onclick={() => searchUsers(spec)}>Search</button>
        {#if spec.id === "assignee" && form?.me}
          <button class="btn subtle" type="button" onclick={() => assignToMe(spec.id)}>Me</button>
        {/if}
        <button class="btn subtle" type="button" onclick={() => clearUser(spec.id)}>Clear</button>
      </div>
      {#each userResults[spec.id] ?? [] as u (u.account_id)}
        <button class="btn result" type="button" onclick={() => chooseUser(spec, u)}>{u.display_name}</button>
      {/each}
    </div>
  {:else if spec.kind === "text_area"}
    <label>{spec.name}<textarea class="input" rows="3" bind:value={answers[spec.id]}></textarea></label>
  {:else if spec.kind === "number"}
    <label>{spec.name}<input class="input" type="number" step="any" bind:value={answers[spec.id]} /></label>
  {:else if spec.kind === "date"}
    <label>{spec.name}<input class="input" type="date" bind:value={answers[spec.id]} /></label>
  {:else if spec.kind === "labels"}
    <label>
      {spec.name} (separate with spaces or commas)
      <input class="input" bind:value={answers[spec.id]} />
    </label>
  {:else}
    <label>{spec.name}<input class="input" bind:value={answers[spec.id]} /></label>
  {/if}
{/snippet}

<label>
  Project
  <select class="input" value={projectKey} onchange={(e) => loadTypes(e.currentTarget.value)} disabled={busy}>
    <option value="">Choose a project…</option>
    {#each projects as p (p.key)}<option value={p.key} selected={p.key === projectKey}>{p.name} ({p.key})</option>{/each}
  </select>
</label>
<label>
  Issue type
  <select class="input" onchange={(e) => loadForm(e.currentTarget.value)} disabled={busy || issueTypes.length === 0}>
    {#each issueTypes as t (t.id)}<option value={t.id} selected={t.id === typeId}>{t.name}</option>{/each}
  </select>
</label>

{#if form && form.fields.length > 0}
  <h3>Required by this project</h3>
  {#each form.fields as spec (spec.id)}
    {@render control(spec)}
  {/each}
{/if}
{#if form && form.optional.length > 0}
  <h3>Also on the create screen</h3>
  {#each form.optional as spec (spec.id)}
    {@render control(spec)}
  {/each}
{/if}
{#if form && (form.fields.length > 0 || form.optional.length > 0)}
  <label class="check">
    <input type="checkbox" bind:checked={rememberAnswers} /> Remember the required answers for this project and issue type
  </label>
{/if}

<style>
  h3 {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--muted);
    margin: 1.2rem 0 0.5rem;
  }
  label,
  fieldset {
    display: grid;
    gap: 0.3rem;
    font-size: 0.9rem;
    font-weight: 600;
    margin-bottom: 0.7rem;
  }
  fieldset {
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 0.5rem 0.7rem;
  }
  .checks {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem 1rem;
  }
  label.check {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0;
    font-weight: 500;
  }
  .field-label {
    font-size: 0.9rem;
    font-weight: 600;
  }
  .row {
    display: flex;
    gap: 0.5rem;
    flex-wrap: nowrap;
  }
  .row .input {
    flex: 1;
  }
  .user {
    display: grid;
    gap: 0.4rem;
    margin-bottom: 0.7rem;
  }
  .result {
    justify-content: flex-start;
  }
</style>
