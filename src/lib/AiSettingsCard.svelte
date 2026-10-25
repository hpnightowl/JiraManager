<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { gigabytes, percent, pullModel } from "$lib/pull";
  import type { AiStatus, LlmSettings, LlmStatus, PullProgress } from "$lib/types";

  let { onmessage }: { onmessage: (kind: "ok" | "error", text: string) => void } = $props();

  // --- starting and stopping the AI server (so it doesn't keep running when you don't need it) ---
  let ai = $state<AiStatus | null>(null);
  let aiBusy = $state(false);

  async function loadAi() {
    try {
      ai = await invoke<AiStatus>("ai_status");
    } catch (e) {
      onmessage("error", String(e));
    }
  }

  async function startAi() {
    aiBusy = true;
    try {
      ai = await invoke<AiStatus>("start_ai");
      onmessage("ok", "The AI is running.");
      await check();
    } catch (e) {
      onmessage("error", String(e));
    } finally {
      aiBusy = false;
    }
  }

  async function stopAi() {
    aiBusy = true;
    try {
      const [message, status] = await invoke<[string, AiStatus]>("stop_ai");
      ai = status;
      onmessage("ok", message);
      await check();
    } catch (e) {
      onmessage("error", String(e));
    } finally {
      aiBusy = false;
    }
  }

  async function setAutostart(on: boolean) {
    try {
      await invoke("set_ai_autostart", { on });
      if (ai) ai = { ...ai, autostart: on };
    } catch (e) {
      onmessage("error", String(e));
    }
  }

  // --- downloading a model through your local Ollama (needs a click, then a second to confirm) ---
  let confirming = $state<string | null>(null);
  let pulling = $state<string | null>(null);
  let progress = $state<PullProgress | null>(null);
  const pct = $derived(percent(progress));

  async function download(tag: string, gb: number) {
    if (confirming !== tag) {
      confirming = tag; // the first click only asks: this is a big download
      setTimeout(() => (confirming = null), 6000);
      return;
    }
    confirming = null;
    pulling = tag;
    progress = null;
    try {
      await pullModel(tag, (p) => (progress = p));
      model = tag;
      const s = await invoke<LlmSettings>("save_llm_settings", { baseUrl: url, model: tag });
      settings = s;
      onmessage("ok", `Model ${tag} downloaded (about ${gb} GB) and selected.`);
      await check();
    } catch (e) {
      onmessage("error", String(e));
    } finally {
      pulling = null;
    }
  }

  let settings = $state<LlmSettings | null>(null);
  let status = $state<LlmStatus | null>(null);
  let url = $state("");
  let model = $state("");
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

  async function load() {
    const s = await run(() => invoke<LlmSettings>("get_llm_settings"));
    if (s) {
      settings = s;
      url = s.base_url;
      model = s.model;
      await check();
    }
  }

  async function check() {
    status = (await run(() => invoke<LlmStatus>("llm_status"))) ?? status;
  }

  async function save(event: Event) {
    event.preventDefault();
    const s = await run(() => invoke<LlmSettings>("save_llm_settings", { baseUrl: url, model }));
    if (s) {
      settings = s;
      url = s.base_url;
      onmessage("ok", "AI settings saved.");
      await check();
    }
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      onmessage("ok", `Copied: ${text}`);
    } catch {
      onmessage("error", "Couldn't copy automatically. Select the command and copy it.");
    }
  }

  const modelNames = $derived(status?.models.map((m) => m.name) ?? []);

  onMount(() => {
    load();
    loadAi();
    // Coming back to the window (e.g. after installing or quitting Ollama): look again.
    const onFocus = () => !pulling && loadAi();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  });
</script>

<section class="card ai">
  <h2>Local AI <span class="muted">(optional)</span></h2>
  <p class="privacy">
    The AI runs on your computer through <strong>Ollama</strong>. Your task text never leaves your machine or local
    network. The app works fine without it.
  </p>

  <div class="power">
    <div class="power-state">
      {#if !ai}
        <span class="muted small">Checking…</span>
      {:else if ai.running}
        <span class="dot on"></span>
        <div>
          <strong>The AI is running</strong>
          <div class="muted small">
            {ai.started_by_app ? "Started by this app. It stops when you close the app." : "Started outside this app (a service, or by you). Closing the app only frees its memory."}
            {#if ai.loaded_models.length > 0}Using memory: {ai.loaded_models.join(", ")}.{:else}No model loaded in memory right now.{/if}
          </div>
        </div>
      {:else}
        <span class="dot off"></span>
        <div>
          <strong>The AI is off</strong>
          <div class="muted small">
            {#if !ai.installed}Ollama isn't installed. See the checklist above.{:else if !ai.local}The saved address isn't on this computer, so the app can't start it.{:else}Press Start AI when you want story suggestions.{/if}
          </div>
        </div>
      {/if}
    </div>
    <div class="power-buttons">
      <button class="btn small" onclick={startAi} disabled={aiBusy || !ai || ai.running || !ai.installed || !ai.local}>▶ Start AI</button>
      <button
        class="btn small danger"
        onclick={stopAi}
        disabled={aiBusy || !ai || !ai.running}
        title="Frees the memory used by the model, and stops Ollama if this app started it"
      >
        ■ Stop AI
      </button>
    </div>
  </div>
  {#if ai}
    <label class="check inline">
      <input type="checkbox" checked={ai.autostart} onchange={(e) => setAutostart(e.currentTarget.checked)} />
      Start the AI automatically when the app opens (and stop it when the app closes)
    </label>
  {/if}

  {#if status}
    <p class="state" class:ok={status.reachable && status.model_ready} class:warn={status.reachable && !status.model_ready}>
      {#if !status.reachable}
        ● Not connected. {status.error}
      {:else if !model}
        ● Connected ({status.models.length} model{status.models.length === 1 ? "" : "s"} installed). Choose a model below.
      {:else if status.model_ready}
        ● Ready: {model}
      {:else}
        ● Connected, but “{model}” isn't installed. Run <code>ollama pull {model}</code>
      {/if}
    </p>
  {/if}

  <form onsubmit={save}>
    <label>
      Ollama address
      <input class="input" bind:value={url} placeholder="http://127.0.0.1:11434" autocomplete="off" />
    </label>
    <label>
      Model
      <input class="input" bind:value={model} list="installed-models" placeholder="e.g. qwen2.5:3b" autocomplete="off" />
      <datalist id="installed-models">
        {#each modelNames as name (name)}<option value={name}></option>{/each}
      </datalist>
    </label>
    <div class="actions">
      <button class="btn primary" type="submit" disabled={busy}>Save</button>
      <button class="btn" type="button" onclick={check} disabled={busy}>Check connection</button>
    </div>
  </form>

  {#if settings}
    <div class="advice">
      <h3>Suggested for this computer ({settings.ram_gb} GB memory)</h3>
      <ul>
        {#each settings.recommended as m (m.tag)}
          <li>
            <div class="what">
              <strong>{m.tag}</strong> <span class="muted">· about {m.download_gb} GB</span>
              <div class="muted small">{m.note}</div>
            </div>
            <div class="btns">
              <button class="btn small" type="button" onclick={() => (model = m.tag)}>Use</button>
              <button
                class="btn small primary"
                type="button"
                onclick={() => download(m.tag, m.download_gb)}
                disabled={pulling !== null}
                title="Downloads the model with your local Ollama (it must be running)"
              >
                {confirming === m.tag ? `Yes, download ${m.download_gb} GB` : "Download"}
              </button>
              <button class="btn small subtle" type="button" onclick={() => copy(`ollama pull ${m.tag}`)}>Copy command</button>
            </div>
          </li>
        {/each}
      </ul>
      {#if pulling}
        <div class="bar" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={pct ?? undefined}>
          <div class="fill" style:width={pct === null ? "100%" : `${pct}%`} class:indeterminate={pct === null}></div>
        </div>
        <p class="muted small">
          Downloading {pulling}: {progress?.status ?? "starting…"}
          {#if pct !== null && progress}· {pct}% ({gigabytes(progress.completed)} of {gigabytes(progress.total)} GB){/if}
        </p>
      {/if}
      <p class="muted small">
        New to Ollama? Install it from <strong>ollama.com</strong>, then run the copied command once in a terminal to
        download the model. Smaller models answer faster on a CPU.
      </p>
    </div>
  {/if}
</section>

<style>
  .ai {
    padding: 1rem 1.2rem 1.2rem;
    display: grid;
    gap: 0.8rem;
  }
  h2 {
    font-size: 1rem;
  }
  h3 {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--muted);
    margin-bottom: 0.4rem;
  }
  .privacy {
    font-size: 0.88rem;
  }
  .state {
    font-size: 0.88rem;
    color: var(--bad);
    overflow-wrap: anywhere;
  }
  .state.ok {
    color: var(--good);
  }
  .state.warn {
    color: var(--warn);
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
  .actions,
  .btns {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
  ul {
    list-style: none;
    margin: 0 0 0.5rem;
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
    padding: 0.6rem 0.8rem;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  .small {
    font-size: 0.82rem;
  }
  .power {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    flex-wrap: wrap;
    padding: 0.7rem 0.9rem;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  .power-state {
    display: flex;
    align-items: center;
    gap: 0.7rem;
    min-width: 0;
  }
  .power-buttons {
    display: flex;
    gap: 0.5rem;
  }
  .dot {
    flex: none;
    width: 0.7rem;
    height: 0.7rem;
    border-radius: 999px;
  }
  .dot.on {
    background: var(--good);
    box-shadow: 0 0 0 4px var(--good-soft);
  }
  .dot.off {
    background: var(--border-strong);
  }
  label.inline {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-weight: 500;
  }
  .bar {
    height: 0.6rem;
    border-radius: 999px;
    background: var(--border);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--accent);
    transition: width 0.2s;
  }
  .fill.indeterminate {
    background: repeating-linear-gradient(90deg, var(--accent) 0 12px, var(--accent-soft) 12px 24px);
    animation: slide 1s linear infinite;
  }
  @keyframes slide {
    to {
      background-position: 24px 0;
    }
  }
  code {
    background: var(--surface-2);
    padding: 0.05rem 0.35rem;
    border-radius: 5px;
  }
</style>
