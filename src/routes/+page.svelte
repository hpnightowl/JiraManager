<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

  // Mirrors the Rust structs (serde turns Rust fields into these JSON keys).
  type Task = {
    id: number;
    title: string;
    done: boolean;
    tracked_seconds: number; // finished sessions only
    running_since: number | null; // Unix seconds, or null when not running
  };
  type StaleTimer = { task_id: number; title: string; started_at: number; last_seen: number };

  let tasks = $state<Task[]>([]);
  let title = $state("");
  let error = $state("");
  let stale = $state<StaleTimer | null>(null);

  // The clock the UI re-reads every second. Rust only stores *when* a timer started;
  // the ticking number on screen is computed here: tracked + (now - running_since).
  let nowSec = $state(Math.floor(Date.now() / 1000));
  const running = $derived(tasks.find((t) => t.running_since !== null));

  function elapsed(t: Task): number {
    return t.tracked_seconds + (t.running_since !== null ? Math.max(0, nowSec - t.running_since) : 0);
  }

  function fmt(total: number): string {
    const h = Math.floor(total / 3600);
    const m = Math.floor((total % 3600) / 60);
    const s = total % 60;
    const mm = String(m).padStart(2, "0");
    const ss = String(s).padStart(2, "0");
    return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
  }

  function clock(unixSeconds: number): string {
    return new Date(unixSeconds * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  // Every backend call can fail (Rust returns Result<_, AppError>), so we wrap them.
  async function call<T>(fn: () => Promise<T>): Promise<T | undefined> {
    try {
      error = "";
      return await fn();
    } catch (e) {
      error = String(e);
    }
  }

  async function refresh() {
    tasks = (await call(() => invoke<Task[]>("list_tasks"))) ?? tasks;
  }

  async function addTask(event: Event) {
    event.preventDefault();
    const added = await call(() => invoke<Task>("add_task", { title }));
    if (added) {
      title = "";
      await refresh();
    }
  }

  async function toggle(id: number) {
    await call(() => invoke<Task>("toggle_task", { id }));
    await refresh();
  }

  async function remove(id: number) {
    await call(() => invoke<void>("delete_task", { id }));
    await refresh();
  }

  async function startTimer(taskId: number) {
    await call(() => invoke<void>("start_timer", { taskId })); // Rust: task_id
    await refresh();
  }

  async function stopTimer(at: number | null = null) {
    await call(() => invoke<void>("stop_timer", { at }));
    await refresh();
  }

  // --- left-running timer recovery (after a crash / closing the app mid-timer) ---
  async function checkStale() {
    stale = (await call(() => invoke<StaleTimer | null>("stale_timer"))) ?? null;
  }

  async function stopAtLastSeen() {
    if (!stale) return;
    await stopTimer(stale.last_seen);
    stale = null;
  }

  async function keepRunning() {
    await call(() => invoke<void>("heartbeat"));
    stale = null;
  }

  onMount(() => {
    refresh();
    checkStale();
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

<main>
  <h1>Tasks</h1>

  {#if stale}
    <div class="banner">
      <p>
        The timer for <strong>{stale.title}</strong> was still running when the app last ran
        (about {clock(stale.last_seen)}).
      </p>
      <div class="actions">
        <button onclick={stopAtLastSeen}>Stop it at {clock(stale.last_seen)}</button>
        <button onclick={keepRunning}>Keep it running</button>
      </div>
    </div>
  {/if}

  <form onsubmit={addTask}>
    <input placeholder="What are you working on?" bind:value={title} />
    <button type="submit">Add</button>
  </form>

  {#if error}<p class="error">{error}</p>{/if}

  <ul>
    {#each tasks as task (task.id)}
      <li class:done={task.done} class:running={task.running_since !== null}>
        <input type="checkbox" checked={task.done} onchange={() => toggle(task.id)} />
        <span class="title">{task.title}</span>
        <span class="time">{fmt(elapsed(task))}</span>
        {#if task.running_since !== null}
          <button class="timer stop" onclick={() => stopTimer()} aria-label="Stop timer">■</button>
        {:else}
          <button class="timer" onclick={() => startTimer(task.id)} aria-label="Start timer">▶</button>
        {/if}
        <button class="del" onclick={() => remove(task.id)} aria-label="Delete task">✕</button>
      </li>
    {:else}
      <li class="empty">No tasks yet. Add your first one above.</li>
    {/each}
  </ul>
</main>

<style>
  :global(:root) {
    font-family: Inter, Avenir, Helvetica, Arial, sans-serif;
    color: #0f0f0f;
    background: #f6f6f6;
  }
  main {
    max-width: 640px;
    margin: 0 auto;
    padding: 6vh 1rem;
  }
  form {
    display: flex;
    gap: 0.5rem;
  }
  form input {
    flex: 1;
  }
  input:not([type="checkbox"]),
  button {
    padding: 0.6em 1em;
    font: inherit;
    border-radius: 8px;
    border: 1px solid #cfcfcf;
    background: #fff;
    color: inherit;
  }
  button {
    cursor: pointer;
  }
  ul {
    list-style: none;
    padding: 0;
    margin: 1.5rem 0 0;
    display: grid;
    gap: 0.5rem;
  }
  li {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.6rem 0.8rem;
    background: #fff;
    border: 1px solid #e2e2e2;
    border-radius: 8px;
  }
  li.running {
    border-color: #2e8b57;
    box-shadow: 0 0 0 1px #2e8b57;
  }
  .title {
    flex: 1;
  }
  li.done .title {
    text-decoration: line-through;
    opacity: 0.55;
  }
  .time {
    font-variant-numeric: tabular-nums;
    opacity: 0.75;
  }
  li.running .time {
    color: #2e8b57;
    opacity: 1;
    font-weight: 600;
  }
  li.empty {
    justify-content: center;
    opacity: 0.6;
  }
  .timer {
    padding: 0.3em 0.7em;
  }
  .timer.stop {
    color: #c0392b;
  }
  .del {
    padding: 0.2em 0.6em;
    border-color: transparent;
    background: transparent;
  }
  .error {
    color: #c0392b;
    margin: 0.75rem 0 0;
  }
  .banner {
    background: #fff8e1;
    border: 1px solid #f0c36d;
    color: #4a3b00;
    border-radius: 8px;
    padding: 0.2rem 1rem 0.9rem;
    margin-bottom: 1rem;
  }
  .actions {
    display: flex;
    gap: 0.5rem;
  }
  @media (prefers-color-scheme: dark) {
    :global(:root) {
      color: #f6f6f6;
      background: #2a2a2a;
    }
    input:not([type="checkbox"]),
    button,
    li {
      background: #1f1f1f;
      border-color: #444;
    }
    li.running {
      border-color: #3fb27f;
      box-shadow: 0 0 0 1px #3fb27f;
    }
    li.running .time {
      color: #3fb27f;
    }
    .del {
      background: transparent;
      border-color: transparent;
    }
    .banner {
      background: #3a3216;
      border-color: #7a6420;
      color: #f3e6b8;
    }
  }
</style>
