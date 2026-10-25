<script lang="ts">
  // First-run checklist: Jira, the local AI (Ollama + a model) and git sources. It only
  // *checks* and *guides*. Ollama is installed by you from ollama.com; the model is downloaded
  // only when you press the button, through your own Ollama.
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { gigabytes, percent, pullModel } from "$lib/pull";
  import type { AiStatus, GitConnection, JiraSettings, LlmSettings, LlmStatus, PullProgress } from "$lib/types";

  let {
    jira,
    connections,
    onjump,
    onmessage,
  }: {
    jira: JiraSettings;
    connections: GitConnection[];
    /** Scroll to the section below that handles this step. */
    onjump: (section: "jira" | "ai" | "git") => void;
    onmessage: (kind: "ok" | "error", text: string) => void;
  } = $props();

  let llm = $state<LlmSettings | null>(null);
  let status = $state<LlmStatus | null>(null);
  let checking = $state(false);

  // The model to use: the saved choice, or the first suggestion for this computer.
  const target = $derived(llm?.model || llm?.recommended[0]?.tag || "qwen2.5:3b");
  const targetAdvice = $derived(llm?.recommended.find((m) => m.tag === target) ?? llm?.recommended[0] ?? null);

  const isWindows = typeof navigator !== "undefined" && /Windows/i.test(navigator.userAgent);
  const linuxInstall = "curl -fsSL https://ollama.com/install.sh | sh";

  const jiraOk = $derived(!!jira.base_url && jira.has_token);
  const ollamaOk = $derived(!!status?.reachable);
  const modelOk = $derived(!!status?.reachable && !!status?.model_ready);
  const sources = $derived(connections.length);

  let ai = $state<AiStatus | null>(null);
  let starting = $state(false);

  async function check() {
    checking = true;
    try {
      llm = await invoke<LlmSettings>("get_llm_settings");
      status = await invoke<LlmStatus>("llm_status");
      ai = await invoke<AiStatus>("ai_status");
    } catch (e) {
      onmessage("error", String(e));
    } finally {
      checking = false;
    }
  }

  async function startAi() {
    starting = true;
    try {
      await invoke<AiStatus>("start_ai");
      onmessage("ok", "The AI is running.");
    } catch (e) {
      onmessage("error", String(e));
    } finally {
      starting = false;
      await check();
    }
  }

  // --- downloading the model ---
  let confirming = $state(false);
  let pulling = $state(false);
  let progress = $state<PullProgress | null>(null);

  async function download() {
    if (!confirming) {
      confirming = true; // the first click only asks "are you sure?": it is a big download
      setTimeout(() => (confirming = false), 6000);
      return;
    }
    confirming = false;
    pulling = true;
    progress = null;
    try {
      await pullModel(target, (p) => (progress = p));
      // Remember it as the model to use.
      if (llm) await invoke("save_llm_settings", { baseUrl: llm.base_url, model: target });
      onmessage("ok", `Model ${target} is ready.`);
      await check();
    } catch (e) {
      onmessage("error", String(e));
    } finally {
      pulling = false;
    }
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      onmessage("ok", "Copied.");
    } catch {
      onmessage("error", "Couldn't copy automatically. Select the text and copy it.");
    }
  }

  onMount(() => {
    check();
    // Coming back from installing Ollama: check again by itself.
    const onFocus = () => !pulling && check();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  });

  const pct = $derived(percent(progress));
</script>

<section class="card guide" aria-label="Setup guide">
  <header>
    <h2>Checklist</h2>
    <span class="muted small">Tick all three, then head to the Tasks tab.</span>
  </header>

  <ol>
    <li class:ok={jiraOk}>
      <div class="mark">{jiraOk ? "✓" : "1"}</div>
      <div class="body">
        <strong>Connect Jira</strong>
        {#if jiraOk}
          <p class="muted small">Connected to {jira.base_url}.</p>
        {:else}
          <p class="muted small">Needs your site address, email and an API token.</p>
          <button class="btn small" onclick={() => onjump("jira")}>Go to the Jira section</button>
        {/if}
      </div>
    </li>

    <li class:ok={modelOk}>
      <div class="mark">{modelOk ? "✓" : "2"}</div>
      <div class="body">
        <strong>Local AI <span class="muted">(optional, recommended)</span></strong>
        <p class="muted small">Drafts your story titles, descriptions and acceptance criteria. It runs on this computer; nothing is sent out.</p>

        <div class="sub" class:done={ollamaOk}>
          <span class="dot">{ollamaOk ? "✓" : "○"}</span>
          <div>
            <div><strong>Ollama</strong> {ollamaOk ? "is running" : "isn't running yet"}</div>
            {#if !ollamaOk && ai?.installed}
              <!-- installed but not running: the app can start it (and stops it again when it closes) -->
              <p class="muted small">Ollama is installed but not running. The app can start it, and stops it again when you close the app.</p>
              <div class="row">
                <button class="btn primary small" onclick={startAi} disabled={starting}>{starting ? "Starting… (a few seconds)" : "▶ Start AI"}</button>
                <button class="btn small" onclick={check} disabled={checking}>{checking ? "Checking…" : "Check again"}</button>
              </div>
            {:else if !ollamaOk}
              <p class="muted small">
                {#if isWindows}
                  Install <strong>Ollama for Windows</strong> from ollama.com, then start it. Come back to this window afterwards.
                {:else}
                  Install Ollama with this command, then come back; the app can start it for you:
                {/if}
              </p>
              {#if !isWindows}
                <div class="cmd"><code>{linuxInstall}</code><button class="btn small" onclick={() => copy(linuxInstall)}>Copy</button></div>
              {/if}
              <div class="row">
                <button class="btn small" onclick={() => invoke("open_link", { url: "https://ollama.com/download" })}>Open ollama.com/download</button>
                <button class="btn small" onclick={check} disabled={checking}>{checking ? "Checking…" : "Check again"}</button>
              </div>
              {#if status?.error}<p class="muted small">{status.error}</p>{/if}
            {/if}
          </div>
        </div>

        <div class="sub" class:done={modelOk} class:blocked={!ollamaOk}>
          <span class="dot">{modelOk ? "✓" : "○"}</span>
          <div>
            <div><strong>A model</strong> {modelOk ? `(${target}) is ready` : "isn't downloaded yet"}</div>
            {#if ollamaOk && !modelOk}
              <p class="muted small">
                Suggested for this computer ({llm?.ram_gb} GB memory): <strong>{target}</strong>
                {#if targetAdvice}· about {targetAdvice.download_gb} GB. {targetAdvice.note}{/if}
              </p>
              {#if pulling}
                <div class="bar" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={pct ?? undefined}>
                  <div class="fill" class:indeterminate={pct === null} style:width={pct === null ? "100%" : `${pct}%`}></div>
                </div>
                <p class="muted small">
                  {progress?.status ?? "Starting…"}
                  {#if pct !== null && progress}· {pct}% ({gigabytes(progress.completed)} of {gigabytes(progress.total)} GB){/if}
                </p>
              {:else}
                <div class="row">
                  <button class="btn primary small" onclick={download} disabled={!llm}>
                    {confirming ? `Yes, download ${targetAdvice ? `about ${targetAdvice.download_gb} GB` : "it"}` : `Download ${target}`}
                  </button>
                </div>
              {/if}
            {:else if !ollamaOk}
              <p class="muted small">Available once Ollama is running.</p>
            {/if}
          </div>
        </div>
      </div>
    </li>

    <li class:ok={sources > 0}>
      <div class="mark">{sources > 0 ? "✓" : "3"}</div>
      <div class="body">
        <strong>Git sources <span class="muted">(optional)</span></strong>
        {#if sources > 0}
          <p class="muted small">{sources} source{sources === 1 ? "" : "s"} added.</p>
        {:else}
          <p class="muted small">Add GitLab servers or repository folders to turn your merge requests and branches into tasks.</p>
          <button class="btn small" onclick={() => onjump("git")}>Go to the Git sources section</button>
        {/if}
      </div>
    </li>
  </ol>

  {#if jiraOk && modelOk}
    <p class="ready">You're set. Open the <strong>Tasks</strong> tab, add a task, or press <strong>Refresh from Jira</strong>.</p>
  {/if}
</section>

<style>
  .guide {
    padding: 0.9rem 1.2rem 1.1rem;
    display: grid;
    gap: 0.6rem;
  }
  header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 1rem;
    flex-wrap: wrap;
  }
  h2 {
    font-size: 1rem;
  }
  ol {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.9rem;
  }
  li {
    display: flex;
    gap: 0.8rem;
  }
  .mark {
    flex: none;
    width: 1.7rem;
    height: 1.7rem;
    border-radius: 999px;
    display: grid;
    place-items: center;
    font-weight: 700;
    font-size: 0.85rem;
    background: var(--surface-2);
    border: 1px solid var(--border-strong);
    color: var(--muted);
  }
  li.ok .mark {
    background: var(--good-soft);
    border-color: transparent;
    color: var(--good);
  }
  .body {
    min-width: 0;
    display: grid;
    gap: 0.3rem;
  }
  .sub {
    display: flex;
    gap: 0.6rem;
    margin-top: 0.4rem;
    padding: 0.6rem 0.8rem;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  .sub.done .dot {
    color: var(--good);
  }
  .sub.blocked {
    opacity: 0.65;
  }
  .dot {
    font-weight: 700;
  }
  .small {
    font-size: 0.85rem;
  }
  .row {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
    margin-top: 0.3rem;
  }
  .cmd {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
    margin: 0.3rem 0;
  }
  code {
    background: var(--surface);
    border: 1px solid var(--border);
    padding: 0.15rem 0.45rem;
    border-radius: 6px;
    font-size: 0.82rem;
    overflow-wrap: anywhere;
  }
  .bar {
    height: 0.6rem;
    border-radius: 999px;
    background: var(--border);
    overflow: hidden;
    margin: 0.4rem 0 0.2rem;
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
  .ready {
    color: var(--good);
    font-weight: 600;
  }
</style>
