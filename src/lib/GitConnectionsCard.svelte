<script lang="ts">
  // GitLab servers (each with its own address and token) and local repository folders.
  import { invoke } from "@tauri-apps/api/core";
  import type { GitConnection } from "$lib/types";

  let {
    connections,
    onchange,
    onmessage,
  }: {
    connections: GitConnection[];
    onchange: () => void;
    onmessage: (kind: "ok" | "error", text: string) => void;
  } = $props();

  let busy = $state(false);

  // --- the "add" form ---
  let adding = $state<"gitlab" | "local" | null>(null);
  let name = $state("");
  let address = $state("");
  let token = $state(""); // lives here until you press Save; never sent back to the UI
  let author = $state("");

  // --- editing one connection (its token field is separate per connection) ---
  let editing = $state<number | null>(null);
  let editName = $state("");
  let editAddress = $state("");
  let editToken = $state("");
  let editAuthor = $state("");

  async function run<T>(fn: () => Promise<T>): Promise<T | undefined> {
    busy = true;
    try {
      return await fn();
    } catch (e) {
      onmessage("error", String(e));
    } finally {
      busy = false;
    }
  }

  function startAdd(kind: "gitlab" | "local") {
    adding = kind;
    name = "";
    address = kind === "gitlab" ? "https://gitlab.com" : "";
    token = "";
    author = "";
    editing = null;
  }

  async function add(event: Event) {
    event.preventDefault();
    if (!adding) return;
    const c = await run(() =>
      invoke<GitConnection>("add_git_connection", {
        kind: adding,
        name,
        baseUrl: address,
        token: token.trim() === "" ? null : token,
        author: author.trim() === "" ? null : author,
      }),
    );
    if (c) {
      adding = null;
      token = "";
      onmessage("ok", `Added "${c.name}".`);
      onchange();
    }
  }

  function startEdit(c: GitConnection) {
    editing = c.id;
    editName = c.name;
    editAddress = c.base_url;
    editToken = "";
    editAuthor = c.author ?? "";
    adding = null;
  }

  async function save(event: Event) {
    event.preventDefault();
    if (editing === null) return;
    const c = await run(() =>
      invoke<GitConnection>("update_git_connection", {
        id: editing,
        name: editName,
        baseUrl: editAddress,
        token: editToken.trim() === "" ? null : editToken, // null = keep the saved token
        author: editAuthor.trim() === "" ? null : editAuthor,
      }),
    );
    if (c) {
      editing = null;
      editToken = "";
      onmessage("ok", `Saved "${c.name}".`);
      onchange();
    }
  }

  async function test(c: GitConnection) {
    const result = await run(() => invoke<string>("test_git_connection", { id: c.id }));
    if (result) onmessage("ok", result);
  }

  async function clearToken(c: GitConnection) {
    if ((await run(() => invoke<GitConnection>("clear_git_token", { id: c.id }))) !== undefined) {
      onmessage("ok", `Token for "${c.name}" removed.`);
      onchange();
    }
  }

  // Removing is a two-step action: the first click arms it.
  let armed = $state<number | null>(null);
  async function remove(c: GitConnection) {
    if (armed !== c.id) {
      armed = c.id;
      setTimeout(() => (armed = null), 4000);
      return;
    }
    armed = null;
    await run(() => invoke<void>("remove_git_connection", { id: c.id }));
    onmessage("ok", `Removed "${c.name}". Merge requests already imported stay in your list.`);
    onchange();
  }
</script>

<section class="card settings">
  <h2>Git sources <span class="muted">(optional: bring in your merge requests and branches)</span></h2>
  <p class="intro muted">
    Add every place your work lives. Each GitLab server has its <strong>own address and its own token</strong>.
    A repository folder on this computer works with <em>any</em> git host. Everything is read-only.
  </p>

  {#if connections.length === 0}
    <p class="empty muted">Nothing added yet.</p>
  {/if}

  <ul class="list">
    {#each connections as c (c.id)}
      <li>
        {#if editing === c.id}
          <form class="edit" onsubmit={save}>
            <label>Name<input class="input" bind:value={editName} /></label>
            <label>
              {c.kind === "gitlab" ? "GitLab address" : "Repository folder"}
              <input class="input" bind:value={editAddress} autocomplete="off" />
            </label>
            {#if c.kind === "gitlab"}
              <label>
                New token (leave empty to keep the saved one)
                <input class="input" type="password" bind:value={editToken} autocomplete="off" placeholder={c.has_token ? "Saved" : "Paste a read_api token"} />
              </label>
            {:else}
              <label>
                Your commit email (optional, defaults to the repository's)
                <input class="input" bind:value={editAuthor} autocomplete="off" placeholder="you@company.com" />
              </label>
            {/if}
            <div class="actions">
              <button class="btn primary" type="submit" disabled={busy}>Save</button>
              <button class="btn" type="button" onclick={() => (editing = null)}>Cancel</button>
            </div>
          </form>
        {:else}
          <div class="info">
            <div class="top">
              <span class="pill">{c.kind === "gitlab" ? "GitLab" : "Local repository"}</span>
              <strong>{c.name}</strong>
              {#if c.kind === "gitlab"}
                {#if c.has_token}<span class="ok">● token saved</span>{:else}<span class="warn">● no token yet</span>{/if}
              {/if}
            </div>
            <div class="muted small addr">{c.base_url}</div>
            {#if c.kind === "gitlab" && c.has_token}<div class="muted small">Token stored in: {c.token_storage}</div>{/if}
            {#if c.kind === "local"}<div class="muted small">Looks for commits by {c.author ?? "the repository's own commit email"}</div>{/if}
          </div>
          <div class="btns">
            <button class="btn small" onclick={() => test(c)} disabled={busy || (c.kind === "gitlab" && !c.has_token)}>Test</button>
            <button class="btn small" onclick={() => startEdit(c)} disabled={busy}>Edit</button>
            {#if c.kind === "gitlab" && c.has_token}
              <button class="btn subtle small danger" onclick={() => clearToken(c)} disabled={busy}>Remove token</button>
            {/if}
            <button class="btn subtle small danger" onclick={() => remove(c)} disabled={busy}>{armed === c.id ? "Sure?" : "Remove"}</button>
          </div>
        {/if}
      </li>
    {/each}
  </ul>

  {#if adding}
    <form class="add" onsubmit={add}>
      <h3>{adding === "gitlab" ? "Add a GitLab server" : "Add a repository folder"}</h3>
      <label>
        Name (optional)
        <input class="input" bind:value={name} placeholder={adding === "gitlab" ? "e.g. Work GitLab" : "e.g. Payments service"} />
      </label>
      <label>
        {adding === "gitlab" ? "GitLab address" : "Folder of the repository"}
        <input
          class="input"
          bind:value={address}
          autocomplete="off"
          placeholder={adding === "gitlab" ? "https://gitlab.com or https://git.yourcompany.com" : "/home/you/projects/payments"}
        />
      </label>
      {#if adding === "gitlab"}
        <label>
          Personal access token
          <input class="input" type="password" bind:value={token} autocomplete="off" placeholder="Paste a token with only the read_api scope" />
        </label>
        <p class="hint">
          In GitLab: your avatar › <strong>Preferences › Access tokens</strong> › add a token with only <strong>read_api</strong>.
          It stays on this computer and can't change anything in GitLab.
        </p>
      {:else}
        <label>
          Your commit email (optional)
          <input class="input" bind:value={author} autocomplete="off" placeholder="defaults to the repository's own setting" />
        </label>
        <p class="hint">
          Finds branches (other than main) that contain commits by you. Needs <code>git</code> installed. A branch that was
          merged and deleted can't be found.
        </p>
      {/if}
      <div class="actions">
        <button class="btn primary" type="submit" disabled={busy}>Add</button>
        <button class="btn" type="button" onclick={() => (adding = null)}>Cancel</button>
      </div>
    </form>
  {:else}
    <div class="actions">
      <button class="btn" onclick={() => startAdd("gitlab")}>＋ Add a GitLab server</button>
      <button class="btn" onclick={() => startAdd("local")}>＋ Add a repository folder</button>
    </div>
  {/if}
</section>

<style>
  .settings {
    padding: 1rem 1.2rem 1.2rem;
    display: grid;
    gap: 0.8rem;
  }
  h2 {
    font-size: 1rem;
  }
  h3 {
    font-size: 0.85rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--muted);
  }
  .intro,
  .empty {
    font-size: 0.88rem;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.5rem;
  }
  li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.8rem;
    flex-wrap: wrap;
    padding: 0.7rem 0.9rem;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  .info {
    min-width: 0;
  }
  .top {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
  .addr {
    overflow-wrap: anywhere;
  }
  .small {
    font-size: 0.82rem;
  }
  .ok {
    color: var(--good);
    font-size: 0.82rem;
  }
  .warn {
    color: var(--warn);
    font-size: 0.82rem;
  }
  .btns,
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
  form.edit,
  form.add {
    display: grid;
    gap: 0.7rem;
    width: 100%;
    max-width: 36rem;
  }
  label {
    display: grid;
    gap: 0.3rem;
    font-size: 0.88rem;
    font-weight: 600;
  }
  .hint {
    font-size: 0.82rem;
    color: var(--muted);
  }
  code {
    background: var(--surface-2);
    padding: 0.05rem 0.35rem;
    border-radius: 5px;
  }
</style>
