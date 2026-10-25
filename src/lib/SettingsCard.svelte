<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import type { JiraSettings } from "$lib/types";

  let {
    settings,
    onchange,
    onmessage,
  }: {
    settings: JiraSettings;
    /** Called with the new settings after Save / Remove token. */
    onchange: (s: JiraSettings) => void;
    onmessage: (kind: "ok" | "error", text: string) => void;
  } = $props();

  // The form is a draft: it starts from what is saved, and only Save changes anything.
  // svelte-ignore state_referenced_locally
  let url = $state(settings.base_url);
  // svelte-ignore state_referenced_locally
  let email = $state(settings.email);
  let token = $state(""); // lives here until you press Save, and is never sent back to the UI
  let busy = $state(false);

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

  async function save(event: Event) {
    event.preventDefault();
    const s = await run(() =>
      invoke<JiraSettings>("save_jira_settings", {
        baseUrl: url,
        email,
        token: token.trim() === "" ? null : token, // null = keep the saved token
      }),
    );
    if (s) {
      token = "";
      url = s.base_url;
      onchange(s);
      onmessage("ok", s.has_token ? "Settings saved." : "Settings saved. Add your API token to connect.");
    }
  }

  // Saved the moment you pick it (it isn't part of the connection details).
  async function changeFetchDays(days: number) {
    const s = await run(() => invoke<JiraSettings>("set_fetch_days", { days }));
    if (s) {
      onchange(s);
      onmessage("ok", days === 0 ? "Refresh will fetch open issues only." : `Refresh will also fetch issues you resolved in the last ${days} days.`);
    }
  }

  async function test() {
    const name = await run(() => invoke<string>("test_jira_connection"));
    if (name) onmessage("ok", `Connected to Jira as ${name}.`);
  }

  async function removeToken() {
    const s = await run(() => invoke<JiraSettings>("clear_jira_token"));
    if (s) {
      onchange(s);
      onmessage("ok", "Token removed.");
    }
  }
</script>

<section class="card settings">
  <h2>Jira connection</h2>
  <form onsubmit={save}>
    <label>
      Site URL
      <input class="input" placeholder="https://yourcompany.atlassian.net" bind:value={url} autocomplete="off" />
    </label>
    <label>
      Atlassian account email
      <input class="input" type="email" placeholder="you@company.com" bind:value={email} autocomplete="off" />
    </label>
    <label>
      API token
      <input
        class="input"
        type="password"
        placeholder={settings.has_token ? "Saved. Leave empty to keep it" : "Paste a token from id.atlassian.com"}
        bind:value={token}
        autocomplete="off"
      />
    </label>
    {#if settings.has_token}
      <p class="hint">Token stored in: {settings.token_storage}</p>
    {:else}
      <p class="hint">
        Create a token at <strong>id.atlassian.com</strong> › Security › API tokens. It stays on this computer and is
        never shown again.
      </p>
    {/if}
    <label>
      Which issues does Refresh fetch?
      <select class="input" value={settings.fetch_days} onchange={(e) => changeFetchDays(Number(e.currentTarget.value))} disabled={busy}>
        <option value={0}>Open issues assigned to me</option>
        <option value={14}>Open + resolved in the last 14 days</option>
        <option value={30}>Open + resolved in the last 30 days</option>
        <option value={60}>Open + resolved in the last 60 days</option>
        <option value={90}>Open + resolved in the last 90 days</option>
      </select>
      <span class="hint">Includes defects (Bug) and every other issue type assigned to you. Finished work is what you log time against at month end, so resolved issues are included by default.</span>
    </label>
    <div class="actions">
      <button class="btn primary" type="submit" disabled={busy}>Save</button>
      <button class="btn" type="button" onclick={test} disabled={busy || !settings.has_token}>Test connection</button>
      {#if settings.has_token}
        <button class="btn subtle danger" type="button" onclick={removeToken} disabled={busy}>Remove token</button>
      {/if}
    </div>
  </form>
</section>

<style>
  .settings {
    padding: 1rem 1.2rem 1.2rem;
  }
  h2 {
    font-size: 1rem;
    margin-bottom: 0.8rem;
  }
  form {
    display: grid;
    gap: 0.8rem;
    max-width: 34rem;
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
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
</style>
