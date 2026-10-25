# 11 · Counting stories and suggesting new ones

What was added: a **Stories** tab (how many stories have I created? plus "I need N stories"), and a Tasks list whose groups start collapsed.

## 1. Two honest answers to "how many stories have I created?"

A single number would hide a trade-off, so the tab shows two:

| | Exact? | History? | Counts things made outside the app? |
|---|---|---|---|
| **Created with ProjectJira** (`created_issues` table) | yes | only from now on | no |
| **Reported in Jira** (`reported_issues`, read with JQL `reporter = currentUser()`) | as exact as Jira | all of it | yes |

The ledger is written in `push.rs::record_created`, in the same step that records the Jira key on the task. It uses `INSERT OR IGNORE` on the key, so pushing twice can never count twice, and it keeps its own copy of the title, so deleting the task afterwards doesn't change the count.

**Rust you meet here:**
- `Option<T>` for "never refreshed" (`reported_refreshed_at`), rather than a magic 0.
- A *transaction* (`conn.transaction()`) so replacing the "reported" copy is all-or-nothing: you never see half a refresh.
- `impl TaskStore` blocks in several files: Rust lets any module of the crate add methods to a type it can see.

## 2. "I need N stories": rank, then draft

`suggest.rs` is deliberately split into small functions you can test one at a time:

1. `collect_candidates`: pure. Looks at the local database and produces scored candidates. No network, no AI.
2. `read_defect_comments`: the only network step (Jira, read-only). It is best-effort: if Jira fails 3 times it stops and reports a *note*, it doesn't fail the whole request.
3. `rank`: sort by score, tie-break by id (so the result is stable), keep N.
4. `suggest_impl`: drafts each pick with the local model; if the AI is unavailable it falls back to the template and says so.
5. `adopt_impl`: saves the drafts. Nothing is sent to Jira.

The scoring is a plain sum, in "seconds of effort": time tracked, +900 per commit, +1200 if merged, +300 per merge request, +150 per Jira comment (capped). It is a heuristic, which is why every card shows its **evidence** instead of just a number: you can see why it was chosen and disagree.

**Why a function takes `jira: Option<&JiraClient>` and `ai: Option<&OllamaClient>`:** the logic doesn't care *where* a client comes from, so the tests pass in fake servers (wiremock) and the command passes real ones. This is the same trick as milestone 6 and is what keeps the AI/Jira parts testable without a Jira or a GPU.

## 3. Keeping a long list short

`grouping.ts` got three tiny pure functions (`epicGroupStartsOpen`, `monthStartsOpen`, `rowsToShow`). The page only stores *your* clicks (`expanded`); everything else is the rule. That is a useful pattern: **store the user's choices, derive the defaults**. If you stored the default too, you'd have to keep two copies in sync.

## Try it
- Stories tab → *Suggest 3 stories* → edit a card → *Save as drafts*. They are ticked on the Tasks tab. Open Push to see them.
- Tasks tab → *Collapse all* / *Expand all*; type in the search box and watch matching groups open.
