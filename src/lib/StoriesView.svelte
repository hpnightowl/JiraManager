<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { dateTime, shortDate } from "$lib/format";
  import { doneByMonth, doneTotals, doneTypes, filterDone, inMonth, kindOf, perMonth, recentStories } from "$lib/stories";
  import type { AdoptItem, AdoptReport, StoryStats, SuggestOptions, SuggestReport, Suggestion } from "$lib/types";

  let {
    active,
    jiraConnected,
    onadopted,
    onmessage,
    ongo,
    onopen,
  }: {
    /** True while this tab is the one on screen (the numbers are re-read each time you come back). */
    active: boolean;
    jiraConnected: boolean;
    /** Open an issue in Jira (only on your own site). */
    onopen: (jiraKey: string) => void;
    /** Drafts were saved on these task ids. */
    onadopted: (taskIds: number[]) => void;
    onmessage: (kind: "ok" | "error", text: string) => void;
    ongo: (tab: "setup") => void;
  } = $props();

  // ---- how many have I created? ----
  let stats = $state<StoryStats | null>(null);
  let refreshing = $state(false);

  async function load() {
    try {
      stats = await invoke<StoryStats>("get_story_stats");
    } catch (e) {
      onmessage("error", String(e));
    }
  }

  async function refreshFromJira() {
    refreshing = true;
    try {
      stats = await invoke<StoryStats>("refresh_reported_stories");
      onmessage("ok", `Read from Jira: ${stats.done.length} finished by you, ${stats.reported.length} reported by you. Nothing was changed in Jira.`);
    } catch (e) {
      onmessage("error", String(e));
    } finally {
      refreshing = false;
    }
  }

  // The first time you open this tab with Jira connected, read it for you (reading is free and never
  // changes anything), unless that was done in the last 6 hours. Otherwise the numbers would sit
  // empty until you found the button.
  let autoRead = false;
  async function firstVisit() {
    await load();
    if (autoRead || !jiraConnected) return;
    autoRead = true;
    const at = stats?.reported_refreshed_at;
    if (!at || Date.now() / 1000 - at > 6 * 3600) await refreshFromJira();
  }

  $effect(() => {
    if (active) firstVisit();
  });

  const app = $derived(stats?.created_with_app ?? []);
  const jira = $derived(stats?.reported ?? []);

  // ---- finished work ----
  const done = $derived(stats?.done ?? []);
  const readAlready = $derived(!!stats?.reported_refreshed_at);
  const totals = $derived(doneTotals(done));
  const doneMonths = $derived(doneByMonth(done));
  const types = $derived(doneTypes(done));
  let typeFilter = $state("");
  let monthFilter = $state("");
  let showAll = $state(false);
  const listed = $derived(filterDone(done, typeFilter, monthFilter));
  const shown = $derived(showAll ? listed : listed.slice(0, 15));
  const undated = $derived(done.filter((d) => d.done_at <= 0).length);
  const rows = $derived(perMonth(app, jira));
  const recent = $derived(recentStories(app, jira, 8));

  // ---- suggestions ----
  type Draft = Suggestion & { picked: boolean };

  let count = $state(5);
  let sources = $state({ merge_requests: true, defects: true, other_tasks: true });
  let sinceDays = $state(30);
  let useTemplate = $state(false);
  let suggesting = $state(false);
  let adopting = $state(false);
  let report = $state<SuggestReport | null>(null);
  let drafts = $state<Draft[]>([]);

  const noSource = $derived(!sources.merge_requests && !sources.defects && !sources.other_tasks);
  const countOk = $derived(Number.isInteger(count) && count >= 1 && count <= 20);
  const pickedDrafts = $derived(drafts.filter((d) => d.picked && d.title.trim() !== ""));

  async function suggest(event: Event) {
    event.preventDefault();
    suggesting = true;
    report = null;
    drafts = [];
    const options: SuggestOptions = { count, ...sources, since_days: sinceDays };
    try {
      const r = await invoke<SuggestReport>("suggest_stories", { options, useTemplate });
      report = r;
      drafts = r.suggestions.map((s) => ({ ...s, picked: true }));
    } catch (e) {
      onmessage("error", String(e));
    } finally {
      suggesting = false;
    }
  }

  async function adopt() {
    const items: AdoptItem[] = pickedDrafts.map((d) => ({
      task_id: d.task_id,
      kind: d.kind,
      title: d.title,
      description: d.description,
      source: d.source,
    }));
    if (items.length === 0) return;
    adopting = true;
    try {
      const r = await invoke<AdoptReport>("adopt_suggestions", { items });
      // Saved ones leave the list; a failed one stays so you can see which and try again.
      const failedIds = new Set(r.failed.map((f) => f.task_id));
      const saved = new Set(pickedDrafts.filter((d) => !failedIds.has(d.task_id)));
      drafts = drafts.filter((d) => !saved.has(d));
      if (r.failed.length) onmessage("error", `${r.failed.length} could not be saved: ${r.failed[0].message}`);
      if (r.task_ids.length) onadopted(r.task_ids);
    } catch (e) {
      onmessage("error", String(e));
    } finally {
      adopting = false;
    }
  }

  const kindLabel = (d: Suggestion) =>
    d.kind === "merge_request" ? "Merge request / branch" : d.kind === "defect" ? `Defect ${d.jira_key ?? ""}` : "Tracked task";
</script>

<div class="stories">
  <section class="card pad" aria-label="Finished work">
    <div class="head">
      <h2>Finished by me</h2>
      {#if jiraConnected}
        <button class="btn small" onclick={refreshFromJira} disabled={refreshing} title="Reads from Jira. Nothing is changed in Jira.">
          {refreshing ? "Reading…" : "↻ Refresh from Jira"}
        </button>
      {/if}
    </div>
    <p class="hint tight">Issues assigned to you that Jira shows in a Done status (whatever your workflow calls it), counted by the day they got there.</p>

    {#if !jiraConnected}
      <p class="muted">Connect Jira to see this. <button class="link" onclick={() => ongo("setup")}>Open Setup</button></p>
    {:else if !readAlready}
      <p class="muted">{refreshing ? "Reading your issues from Jira…" : "Not read from Jira yet. Press Refresh from Jira."}</p>
    {:else if done.length === 0}
      <p class="muted">Jira lists nothing assigned to you in a Done status.</p>
    {:else}
      <div class="tiles">
        <div class="tile"><span class="label">Stories done</span><span class="value">{totals.all.story}</span><span class="caption">{totals.thisMonth.story} this month</span></div>
        <div class="tile defect"><span class="label">Defects done</span><span class="value">{totals.all.defect}</span><span class="caption">{totals.thisMonth.defect} this month</span></div>
        <div class="tile other"><span class="label">Other (tasks…)</span><span class="value">{totals.all.other}</span><span class="caption">{totals.thisMonth.other} this month</span></div>
        <div class="tile"><span class="label">All finished</span><span class="value">{totals.all.total}</span><span class="caption">{totals.thisMonth.total} this month</span></div>
      </div>
      {#if done.length >= 500}<p class="warn small">Jira has more than 500; this shows the latest 500.</p>{/if}

      <table>
        <thead><tr><th>Month finished</th><th class="num">Stories</th><th class="num">Defects</th><th class="num">Other</th><th class="num">Total</th></tr></thead>
        <tbody>
          {#each doneMonths as m (m.key)}
            <tr class:picked={monthFilter === m.key}>
              <td><button class="link" onclick={() => ((monthFilter = monthFilter === m.key ? "" : m.key), (showAll = false))} title="Show only this month">{m.label}</button></td>
              <td class="num">{m.story}</td><td class="num">{m.defect}</td><td class="num">{m.other}</td><td class="num"><strong>{m.total}</strong></td>
            </tr>
          {/each}
          {#if undated > 0}
            <tr class:picked={monthFilter === "unknown"}>
              <td><button class="link" onclick={() => ((monthFilter = monthFilter === "unknown" ? "" : "unknown"), (showAll = false))}>Date unknown</button></td>
              <td class="num" colspan="3"></td><td class="num"><strong>{undated}</strong></td>
            </tr>
          {/if}
        </tbody>
      </table>

      <div class="chips" role="group" aria-label="Filter by issue type">
        <button class="chip" class:active={typeFilter === ""} onclick={() => ((typeFilter = ""), (showAll = false))}>All {done.length}</button>
        {#each types as [name, count] (name)}
          <button class="chip" class:active={typeFilter === name} onclick={() => ((typeFilter = typeFilter === name ? "" : name), (showAll = false))}>{name} {count}</button>
        {/each}
        {#if monthFilter}<button class="chip active" onclick={() => (monthFilter = "")}>✕ month filter</button>{/if}
      </div>

      <ul class="donelist">
        {#each shown as d (d.jira_key)}
          <li>
            <button class="key" onclick={() => onopen(d.jira_key)} title="Open in Jira">{d.jira_key}</button>
            <span class="rtitle">{d.title}</span>
            <span class="kind {kindOf(d.issue_type)}">{d.issue_type ?? "Unknown"}</span>
            <span class="muted small when">{d.done_at > 0 ? shortDate(d.done_at) : "date unknown"}</span>
          </li>
        {:else}
          <li class="muted">Nothing matches that filter.</li>
        {/each}
      </ul>
      {#if listed.length > 15}
        <button class="btn small subtle more" onclick={() => (showAll = !showAll)}>{showAll ? "Show fewer" : `Show all ${listed.length}`}</button>
      {/if}
    {/if}
  </section>

  <section class="counts" aria-label="Stories created">
    <div class="card count">
      <span class="label">Created with ProjectJira</span>
      <span class="value">{app.length}</span>
      <span class="caption">{inMonth(app)} this month</span>
      <p class="hint">An exact count of the issues this app created in Jira, from the first push made after this feature arrived. Earlier ones can't be recovered here; the number on the right has them.</p>
    </div>
    <div class="card count">
      <span class="label">Reported in Jira</span>
      <span class="value">{stats?.reported_refreshed_at ? jira.length : "–"}</span>
      <span class="caption">
        {#if stats?.reported_refreshed_at}{inMonth(jira)} this month · read {dateTime(stats.reported_refreshed_at)}{:else}not read yet{/if}
      </span>
      <p class="hint">Every issue Jira lists with you as the <em>reporter</em>, history included. It also counts issues you made in Jira itself, so it can be higher.</p>
      {#if jiraConnected}
        <button class="btn small" onclick={refreshFromJira} disabled={refreshing} title="Reads from Jira. Nothing is changed in Jira.">
          {refreshing ? "Reading…" : stats?.reported_refreshed_at ? "↻ Refresh from Jira" : "Read from Jira"}
        </button>
      {:else}
        <button class="btn small" onclick={() => ongo("setup")}>Connect Jira first</button>
      {/if}
    </div>
  </section>

  {#if rows.length > 0}
    <section class="card pad" aria-label="Stories per month">
      <h2>Month by month</h2>
      <table>
        <thead><tr><th>Month</th><th class="num">With ProjectJira</th><th class="num">Reported in Jira</th></tr></thead>
        <tbody>
          {#each rows as r (r.key)}
            <tr><td>{r.label}</td><td class="num">{r.app}</td><td class="num">{r.jira || "–"}</td></tr>
          {/each}
        </tbody>
      </table>
      {#if recent.length > 0}
        <h3>Most recent</h3>
        <ul class="recent">
          {#each recent as s (s.jira_key)}
            <li>
              <strong>{s.jira_key}</strong>
              <span class="rtitle">{s.title}</span>
              <span class="muted small">{s.issue_type ?? ""} · {shortDate(s.created_at)}{s.viaApp ? " · via ProjectJira" : ""}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  {/if}

  <section class="card pad suggest" aria-label="Suggest stories">
    <h2>Suggest stories</h2>
    <p class="muted">
      Choose how many stories you need. The app picks the pieces of your work that most deserve one and drafts each with the local AI. Nothing goes to Jira until you review and push.
    </p>

    <form onsubmit={suggest}>
      <div class="row">
        <label class="num-field">
          How many stories do you need?
          <input class="input" type="number" min="1" max="20" bind:value={count} aria-label="How many stories" />
        </label>
        <label>
          Look at work from
          <select class="input" bind:value={sinceDays}>
            <option value={14}>the last 14 days</option>
            <option value={30}>the last 30 days</option>
            <option value={60}>the last 60 days</option>
            <option value={90}>the last 90 days</option>
            <option value={0}>any time</option>
          </select>
        </label>
      </div>

      <fieldset>
        <legend>Where to look</legend>
        <label class="check"><input type="checkbox" bind:checked={sources.merge_requests} /> Merge requests &amp; branches without a Jira issue</label>
        <label class="check"><input type="checkbox" bind:checked={sources.defects} /> Defects assigned to me <span class="muted">(their description and comments are read from Jira)</span></label>
        <label class="check"><input type="checkbox" bind:checked={sources.other_tasks} /> Other tasks I tracked time on</label>
      </fieldset>

      <div class="row">
        <button class="btn primary" type="submit" disabled={suggesting || noSource || !countOk}>
          {suggesting ? "Choosing and drafting…" : `Suggest ${countOk ? count : "…"} ${count === 1 ? "story" : "stories"}`}
        </button>
        <label class="check"><input type="checkbox" bind:checked={useTemplate} /> Don't use the AI (simple templates)</label>
      </div>
      {#if !countOk}<p class="warn small">Choose a whole number from 1 to 20.</p>{/if}
      {#if noSource}<p class="warn small">Tick at least one place to look.</p>{/if}
      {#if suggesting}<p class="muted small">The local AI writes each story one by one, which can take a little while on a CPU.</p>{/if}
    </form>

    {#if report}
      {#if report.stopped}
        <div class="note warn-box">{report.stopped} The drafts below use the simple template instead.</div>
      {/if}
      {#each report.notes as n}<div class="note">{n}</div>{/each}

      {#if drafts.length === 0 && report.suggestions.length === 0}
        <p class="muted">
          Nothing to suggest{sinceDays ? ` in the last ${sinceDays} days` : ""}. Work that already has a story draft or a Jira issue is left out.
          Import merge requests or track some time, or widen the dates.
        </p>
      {:else}
        <p class="result-head">
          <strong>The best {report.suggestions.length}</strong> of {report.available} piece{report.available === 1 ? "" : "s"} of work found
          {#if report.suggestions.length < count}<span class="muted"> (you asked for {count}; that is all there is)</span>{/if}
        </p>

        <div class="cards">
          {#each drafts as d, i (d.task_id + d.kind)}
            <article class="sug" class:off={!d.picked}>
              <header>
                <label class="check pick"><input type="checkbox" bind:checked={d.picked} aria-label="Use this suggestion" /> <span class="n">#{i + 1}</span></label>
                <span class="kind {d.kind}">{kindLabel(d)}</span>
                <span class="muted small src">{d.source === "ai" ? "written by the AI" : "from a template"}</span>
              </header>
              <p class="from muted small" title="The work this was drafted from">From: {d.source_title}</p>
              <ul class="evidence">{#each d.evidence as e}<li>{e}</li>{/each}</ul>
              <input class="input" bind:value={d.title} aria-label="Story title" />
              <textarea class="input" rows="6" bind:value={d.description} aria-label="Story description"></textarea>
            </article>
          {/each}
        </div>

        {#if drafts.length > 0}
          <div class="adopt">
            <button class="btn primary" onclick={adopt} disabled={adopting || pickedDrafts.length === 0}>
              {adopting ? "Saving…" : `Save ${pickedDrafts.length} as drafts on my tasks`}
            </button>
            <span class="muted small">Drafts appear on the Tasks tab, selected. Push is still a separate, explicit step.</span>
          </div>
        {/if}
      {/if}
    {/if}
  </section>
</div>

<style>
  .stories {
    display: grid;
    gap: 1rem;
  }
  .counts {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(17rem, 1fr));
    gap: 0.8rem;
  }
  .count {
    padding: 0.9rem 1.1rem 1rem;
    display: grid;
    gap: 0.15rem;
    align-content: start;
    justify-items: start;
  }
  .label {
    font-size: 0.78rem;
    font-weight: 600;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .value {
    font-size: 2.2rem;
    font-weight: 800;
    line-height: 1.1;
    font-variant-numeric: tabular-nums;
    color: var(--accent);
  }
  .caption {
    font-size: 0.85rem;
    color: var(--muted);
  }
  .hint {
    font-size: 0.8rem;
    color: var(--muted);
    margin: 0.4rem 0 0.5rem;
  }
  .pad {
    padding: 1rem 1.2rem 1.2rem;
    display: grid;
    gap: 0.7rem;
  }
  h2 {
    font-size: 1.05rem;
  }
  h3 {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--muted);
    margin-top: 0.4rem;
  }
  .small {
    font-size: 0.82rem;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.92rem;
  }
  th,
  td {
    text-align: left;
    padding: 0.35rem 0.5rem;
    border-bottom: 1px solid var(--border);
  }
  th {
    font-size: 0.78rem;
    color: var(--muted);
    font-weight: 600;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.8rem;
  }
  .hint.tight {
    margin: 0;
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr));
    gap: 0.6rem;
  }
  .tile {
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 0.6rem 0.9rem;
    display: grid;
    gap: 0.1rem;
    background: var(--surface);
  }
  .tile .value {
    font-size: 1.8rem;
  }
  .tile.defect .value {
    color: var(--bad);
  }
  .tile.other .value {
    color: var(--muted);
  }
  tr.picked td {
    background: var(--accent-soft);
  }
  .link {
    border: 0;
    background: transparent;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
    font: inherit;
    text-align: left;
  }
  .link:hover {
    text-decoration: underline;
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
    padding: 0.15rem 0.7rem;
    font-size: 0.82rem;
    font-weight: 600;
    color: var(--muted);
    cursor: pointer;
  }
  .chip.active {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .donelist {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.25rem;
  }
  .donelist li {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 0.2rem 0.6rem;
    padding: 0.3rem 0.5rem;
    border-radius: 8px;
    background: var(--surface-2);
  }
  .key {
    border: 0;
    background: transparent;
    color: var(--accent);
    font-weight: 700;
    cursor: pointer;
    padding: 0;
  }
  .key:hover {
    text-decoration: underline;
  }
  .when {
    margin-left: auto;
  }
  .more {
    justify-self: start;
  }
  .recent {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.3rem;
  }
  .recent li {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 0.2rem 0.6rem;
  }
  .rtitle {
    flex: 1;
    min-width: 8rem;
    overflow-wrap: anywhere;
  }

  .suggest form {
    display: grid;
    gap: 0.8rem;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: end;
    gap: 0.8rem 1.2rem;
  }
  label {
    display: grid;
    gap: 0.3rem;
    font-size: 0.88rem;
    font-weight: 600;
  }
  .num-field input {
    width: 6rem;
  }
  fieldset {
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 0.6rem 0.9rem 0.8rem;
    display: grid;
    gap: 0.4rem;
  }
  legend {
    font-size: 0.78rem;
    font-weight: 600;
    color: var(--muted);
    padding: 0 0.3rem;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-weight: 500;
  }
  .warn {
    color: var(--warn);
  }
  .note {
    background: var(--surface-2);
    border-radius: 8px;
    padding: 0.5rem 0.8rem;
    font-size: 0.88rem;
    overflow-wrap: anywhere;
  }
  .warn-box {
    background: var(--warn-soft);
    color: var(--warn);
  }
  .result-head {
    margin-top: 0.3rem;
  }
  .cards {
    display: grid;
    gap: 0.8rem;
  }
  .sug {
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 0.8rem 1rem 1rem;
    display: grid;
    gap: 0.5rem;
    background: var(--surface);
  }
  .sug.off {
    opacity: 0.55;
  }
  .sug header {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.6rem;
  }
  .pick {
    display: inline-flex;
  }
  .n {
    font-weight: 800;
    color: var(--muted);
  }
  .src {
    margin-left: auto;
  }
  .kind {
    font-size: 0.78rem;
    font-weight: 700;
    border-radius: 999px;
    padding: 0.1rem 0.6rem;
    background: var(--accent-soft);
    color: var(--accent);
  }
  .kind.defect {
    background: var(--bad-soft);
    color: var(--bad);
  }
  .kind.task,
  .kind.other {
    background: var(--surface-2);
    color: var(--muted);
  }
  .from {
    overflow-wrap: anywhere;
  }
  .evidence {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
  }
  .evidence li {
    font-size: 0.8rem;
    background: var(--surface-2);
    border-radius: 999px;
    padding: 0.1rem 0.6rem;
  }
  textarea.input {
    resize: vertical;
    font: inherit;
  }
  .adopt {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.8rem;
  }
</style>
