<script lang="ts">
  // The "How to use" tab. Plain text on purpose: nothing here talks to the backend.
  let { ongo }: { ongo: (tab: "tasks" | "setup") => void } = $props();
</script>

<div class="help">
  <div class="intro">
    <h1>How to use ProjectJira</h1>
    <p class="muted">
      It captures what you work on, times it, and (only when you say so) turns it into Jira stories and worklogs.
      Reading from Jira or GitLab never changes anything there; <strong>only the Push dialogs write to Jira</strong>.
    </p>
  </div>

  <section class="card quick">
    <h2>Quick start</h2>
    <ol>
      <li>
        <strong>Connect Jira.</strong> Open the <button class="link" onclick={() => ongo("setup")}>Setup</button> tab and
        fill in your site address, your Atlassian email and an API token.
      </li>
      <li>
        <strong>Add what you work on.</strong> On the <button class="link" onclick={() => ongo("tasks")}>Tasks</button> tab,
        type a task and press Enter. Press <span class="key">▶</span> when you start, <span class="key">■</span> when you stop.
      </li>
      <li>
        <strong>Bring in your Jira issues.</strong> Press <em>Refresh from Jira</em>. Your assigned issues (bugs and defects
        included) appear, grouped under their epics.
      </li>
      <li>
        <strong>Push when you're ready.</strong> Press <em>Push</em> on a task, read the one-line summary at the bottom, and
        confirm. The issue is created and your finished time is logged.
      </li>
    </ol>
  </section>

  <details class="card" open>
    <summary>A normal day</summary>
    <ul>
      <li>Add tasks as they come up, or <em>Refresh from Jira</em> to pull in what is assigned to you.</li>
      <li>Start the timer on the task you're on. Starting another one stops the first, so only one runs at a time.</li>
      <li>Forgot to stop? If you close the app with a timer running, it asks next time whether to stop it where you left off.</li>
      <li>Push when it suits you: at the end of the day, or later in a batch. Unpushed time is never lost.</li>
    </ul>
  </details>

  <details class="card">
    <summary>Month end (the batch way)</summary>
    <ol>
      <li><em>Refresh from Jira</em> to get your issues. By default this also includes ones you <strong>resolved in the last 30 days</strong>, so finished work is there too (change it in Setup → Jira).</li>
      <li>Optional: <em>Import from git</em> turns your GitLab merge requests and local branches into tasks.</li>
      <li>Select tasks (the checkbox on the left, or <em>Select all shown</em>) and press <em>✨ Draft stories</em> to write a title, description and acceptance criteria for each task that isn't in Jira yet.</li>
      <li>Press <em>Push selected</em>. Choose the project and issue type <strong>once</strong>, check the table (story, parent, time per task), and confirm.</li>
      <li>Each task succeeds or fails on its own. Anything that failed stays as it was; fix the problem and push it again, and finished ones won't be repeated.</li>
    </ol>
  </details>

  <details class="card">
    <summary>Stories tab: how many have I created, and "I need N stories"</summary>
    <ul>
      <li><strong>Created with ProjectJira</strong> is an exact count of the issues this app made in Jira. It starts counting from the first push after this feature arrived.</li>
      <li><strong>Reported in Jira</strong> is what Jira says you created (you as the reporter), history included, so it also has things made outside this app. Press <em>Read from Jira</em> to load it; that only reads.</li>
      <li><strong>Suggest stories</strong>: type how many you need (say 5). The app ranks your work (merge requests and branches without an issue, defects assigned to you, other tracked tasks) by time spent, commits, whether it merged and how much Jira discussion it had, takes the best 5, and drafts one story each with the local AI. Each card says <em>why</em> it was picked.</li>
      <li>For a defect the AI also reads its Jira description and latest comments. A defect already exists in Jira, so its story is saved on a <strong>new task</strong> that mentions it ("Related Jira issue: ABC-41").</li>
      <li>Edit any card, untick what you don't want, then <em>Save as drafts</em>. They land on the Tasks tab already ticked. Nothing goes to Jira until you press <em>Push selected</em>.</li>
    </ul>
  </details>

  <details class="card">
    <summary>A long list? Groups start closed</summary>
    <ul>
      <li>Epics and months start <strong>closed</strong>, showing only a heading with its totals and how many tasks are waiting to be pushed. Click the arrow to open one, or use <em>Expand all</em> / <em>Collapse all</em>.</li>
      <li>Open from the start: a group with a running timer, tasks without an epic, the current month, and everything while you search or filter.</li>
      <li>A group shows its first 8 rows; <em>Show N more</em> reveals the rest.</li>
    </ul>
  </details>

  <details class="card">
    <summary>Reading a task row</summary>
    <dl>
      <dt><span class="pill epic">Epic</span> <span class="pill story">Story</span> <span class="pill bug">Bug</span></dt>
      <dd>The Jira issue type. Click a type chip above the list to show only that type (for example only Bugs).</dd>
      <dt><span class="pill">Not in Jira yet</span></dt>
      <dd>A task that only exists here. Push creates the issue.</dd>
      <dt><span class="key">ABC-123</span></dt>
      <dd>The Jira key. Click it to open the issue in your browser.</dd>
      <dt><span class="clock">1:30:00</span> <span class="muted">Jira 1h 00m</span> <span class="push">+30m to push</span></dt>
      <dd>Time tracked here · time already logged in Jira · finished time that Push would send.</dd>
      <dt><span class="pill staged">not pushed yet</span></dt>
      <dd>You chose a parent (e.g. an epic) here. It is sent to Jira only when you Push.</dd>
      <dt><span class="mr">!412 merged</span> <span class="mr branch">⎇ feature/x</span></dt>
      <dd>A GitLab merge request, or a local branch, that this task came from.</dd>
      <dt><span class="pill story-ready">Story ready · AI draft</span></dt>
      <dd>A drafted story is saved on the task and is used when you Push. Open Push to edit it.</dd>
      <dt><span class="key">▶</span> <span class="key">■</span> <span class="key">✓</span> <span class="key">✕</span></dt>
      <dd>Start / stop the timer · mark done · remove. For a Jira issue, ✕ <strong>hides</strong> it (see below).</dd>
    </dl>
  </details>

  <details class="card">
    <summary>Epics, parents and months</summary>
    <ul>
      <li><strong>By epic</strong> shows each epic with its stories underneath. <strong>By month created</strong> groups by the month an issue was created (handy for monthly reports).</li>
      <li>Every task has a <strong>Parent</strong> menu. Choosing one is only a plan: it is sent to Jira when you Push that task.</li>
      <li>Epics that aren't yours still appear as headings, so you can see where your stories belong.</li>
    </ul>
  </details>

  <details class="card">
    <summary>Removing things</summary>
    <ul>
      <li>A task that only exists here: <strong>✕</strong> deletes it and its tracked time.</li>
      <li>A Jira issue: <strong>✕</strong> <em>hides</em> it. It leaves your list and does <strong>not come back on Refresh</strong>. The Jira issue itself is never touched.</li>
      <li>Changed your mind? Open <em>Hidden</em> (next to the list filters) and press <em>Restore</em>; it returns on the next Refresh.</li>
    </ul>
  </details>

  <details class="card">
    <summary>The local AI</summary>
    <ul>
      <li>Optional. It runs on this computer through <strong>Ollama</strong>; your text is never sent over the internet.</li>
      <li><strong>Setup → Local AI</strong> has <em>Start AI</em> and <em>Stop AI</em>. By default the app starts Ollama when it opens and <strong>stops it (and frees the memory) when you close the app</strong>. Switch that off in the same card.</li>
      <li>If Ollama was already running as a background service, the app never kills it; it only unloads the model from memory.</li>
      <li>AI suggestions are drafts: small models can get details wrong, so read them. Nothing is used until you press <em>Use this</em>.</li>
      <li>Without the AI everything else works, and "Improve" offers a simple template.</li>
    </ul>
  </details>

  <details class="card">
    <summary>Your data and privacy</summary>
    <ul>
      <li>Tasks, time and drafts are stored on this computer only (one database file).</li>
      <li>Jira and GitLab tokens go in your system keychain; where there is none, in a file only you can read. They are never shown again and never sent anywhere except to that Jira or GitLab server.</li>
      <li>The app talks to: your Jira site, your GitLab servers (read-only), and your local Ollama. Downloading a model goes through Ollama to the Ollama library.</li>
    </ul>
  </details>

  <details class="card">
    <summary>Something not working?</summary>
    <dl>
      <dt>"Jira rejected the email or API token"</dt>
      <dd>Check the email is your Atlassian account's and create a fresh token at id.atlassian.com → Security → API tokens.</dd>
      <dt>Refresh brought fewer issues than expected</dt>
      <dd>Look at the message after Refresh: it lists the count by type. Check Setup → Jira → <em>Which issues does Refresh fetch</em>, and the <em>Hidden</em> list (removed issues stay removed).</dd>
      <dt>"Can't reach the AI server"</dt>
      <dd>Press <em>Start AI</em> in Setup → Local AI. If Ollama isn't installed, the checklist shows how.</dd>
      <dt>A GitLab server says the token was rejected</dt>
      <dd>Create a new access token with only the <code>read_api</code> scope and paste it into that server's entry (each server has its own).</dd>
      <dt>A local repository finds nothing</dt>
      <dd>It needs your commit email (set it on the repository's entry) and finds branches that still exist; a merged-and-deleted branch leaves nothing behind.</dd>
    </dl>
  </details>

  <details class="card">
    <summary>Shortcuts</summary>
    <ul>
      <li><span class="key">Enter</span> in the task box adds the task.</li>
      <li><span class="key">Esc</span> closes a dialog (Cancel keeps your story draft).</li>
      <li>Hover a local task's title and click ✎ to rename it.</li>
    </ul>
  </details>
</div>

<style>
  .help {
    display: grid;
    gap: 0.8rem;
    max-width: 52rem;
  }
  h1 {
    font-size: 1.5rem;
    letter-spacing: -0.01em;
  }
  h2 {
    font-size: 1.05rem;
    margin-bottom: 0.5rem;
  }
  .intro {
    margin-bottom: 0.4rem;
  }
  .intro p {
    margin-top: 0.3rem;
  }
  .quick {
    padding: 1rem 1.3rem 1.1rem;
  }
  ol,
  ul {
    margin: 0;
    padding-left: 1.3rem;
    display: grid;
    gap: 0.45rem;
  }
  .quick ol {
    gap: 0.7rem;
  }
  details {
    padding: 0.2rem 1.3rem;
  }
  summary {
    cursor: pointer;
    font-weight: 700;
    padding: 0.7rem 0;
    list-style-position: inside;
  }
  details[open] summary {
    border-bottom: 1px solid var(--border);
    margin-bottom: 0.7rem;
  }
  details ul,
  details ol {
    padding-bottom: 0.9rem;
  }
  dl {
    margin: 0;
    padding-bottom: 0.9rem;
    display: grid;
    grid-template-columns: minmax(10rem, max-content) 1fr;
    gap: 0.5rem 1.2rem;
    align-items: baseline;
  }
  dt {
    font-weight: 600;
  }
  dd {
    margin: 0;
    color: var(--muted);
  }
  .link {
    border: 0;
    background: none;
    padding: 0;
    color: var(--accent);
    font: inherit;
    font-weight: 700;
    cursor: pointer;
    text-decoration: underline;
  }
  .key {
    display: inline-block;
    min-width: 1.4rem;
    text-align: center;
    padding: 0 0.35rem;
    border: 1px solid var(--border-strong);
    border-bottom-width: 2px;
    border-radius: 6px;
    background: var(--surface-2);
    font-size: 0.85rem;
    font-weight: 600;
    cursor: default;
  }
  .clock {
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .push {
    color: var(--accent);
    font-size: 0.8rem;
    font-weight: 600;
  }
  .mr {
    padding: 0 0.5rem;
    border-radius: 6px;
    background: var(--good-soft);
    color: var(--good);
    font-size: 0.78rem;
    font-weight: 700;
  }
  .mr.branch {
    background: var(--surface-2);
    color: var(--muted);
  }
  .story-ready {
    background: var(--accent-soft);
    border-color: transparent;
    color: var(--accent);
  }
  code {
    background: var(--surface-2);
    padding: 0.05rem 0.35rem;
    border-radius: 5px;
  }
  @media (max-width: 640px) {
    dl {
      grid-template-columns: 1fr;
      gap: 0.1rem;
    }
    dd {
      margin-bottom: 0.5rem;
    }
  }
</style>
