<script lang="ts">
  // The Setup tab: a checklist at the top, then one section per thing to set up.
  import AiSettingsCard from "$lib/AiSettingsCard.svelte";
  import GitConnectionsCard from "$lib/GitConnectionsCard.svelte";
  import SettingsCard from "$lib/SettingsCard.svelte";
  import SetupGuide from "$lib/SetupGuide.svelte";
  import type { GitConnection, JiraSettings } from "$lib/types";

  let {
    jira,
    connections,
    onjirachange,
    onconnectionschange,
    onmessage,
  }: {
    jira: JiraSettings;
    connections: GitConnection[];
    onjirachange: (s: JiraSettings) => void;
    onconnectionschange: () => void;
    onmessage: (kind: "ok" | "error", text: string) => void;
  } = $props();

  function jump(section: "jira" | "ai" | "git") {
    document.getElementById(`setup-${section}`)?.scrollIntoView({ behavior: "smooth", block: "start" });
  }
</script>

<div class="setup">
  <div class="intro">
    <h1>Setup</h1>
    <p class="muted">
      Only Jira is needed to start. The local AI and git sources are optional extras you can add any time.
      Need a hand? See the <strong>How to use</strong> tab.
    </p>
  </div>

  <SetupGuide {jira} {connections} onjump={jump} {onmessage} />

  <section id="setup-jira">
    <h2><span class="num">1</span> Jira</h2>
    <SettingsCard settings={jira} onchange={onjirachange} {onmessage} />
  </section>

  <section id="setup-ai">
    <h2><span class="num">2</span> Local AI</h2>
    <AiSettingsCard {onmessage} />
  </section>

  <section id="setup-git">
    <h2><span class="num">3</span> Git sources</h2>
    <GitConnectionsCard {connections} onchange={onconnectionschange} {onmessage} />
  </section>
</div>

<style>
  .setup {
    display: grid;
    gap: 1.4rem;
  }
  h1 {
    font-size: 1.5rem;
    letter-spacing: -0.01em;
  }
  .intro p {
    margin-top: 0.3rem;
    max-width: 44rem;
  }
  section {
    display: grid;
    gap: 0.6rem;
    scroll-margin-top: 4.5rem; /* keep the heading clear of the sticky top bar */
  }
  h2 {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    font-size: 1.1rem;
  }
  .num {
    display: grid;
    place-items: center;
    width: 1.6rem;
    height: 1.6rem;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 0.85rem;
    font-weight: 800;
  }
</style>
