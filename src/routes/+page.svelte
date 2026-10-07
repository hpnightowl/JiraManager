<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

  // Mirrors the Rust `Task` struct (serde turns Rust fields into these JSON keys).
  type Task = { id: number; title: string; done: boolean };

  let tasks = $state<Task[]>([]);
  let title = $state("");
  let error = $state("");

  // Every backend call can fail (Rust returns Result<_, String>), so we wrap them.
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

  onMount(refresh);
</script>

<main>
  <h1>Tasks</h1>

  <form onsubmit={addTask}>
    <input placeholder="What are you working on?" bind:value={title} />
    <button type="submit">Add</button>
  </form>

  {#if error}<p class="error">{error}</p>{/if}

  <ul>
    {#each tasks as task (task.id)}
      <li class:done={task.done}>
        <input type="checkbox" checked={task.done} onchange={() => toggle(task.id)} />
        <span>{task.title}</span>
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
  li span {
    flex: 1;
  }
  li.done span {
    text-decoration: line-through;
    opacity: 0.55;
  }
  li.empty {
    justify-content: center;
    opacity: 0.6;
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
    .del {
      background: transparent;
      border-color: transparent;
    }
  }
</style>
