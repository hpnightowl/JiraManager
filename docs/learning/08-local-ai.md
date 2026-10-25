# Milestone 7 — "Improve with AI" with a local model (Ollama)

Turn a rough note ("apple pay") into a clear story: title, short description, acceptance criteria. It runs **on your computer**, it only **suggests**, and the app works fully without it.

## How it fits
```
Push dialog ▶ "✨ Improve with AI" ──suggest_story──▶ llm.rs ──HTTP──▶ Ollama (127.0.0.1:11434)
                                                                    ◀── JSON {title, description, acceptance_criteria}
            suggestion card (review) ──"Use this"──▶ fills Title + Description (criteria as a bullet list)
            Ollama off? ──▶ "Use a simple template instead"  (llm::template_improvement, no AI)
```

## Files
| File | Role |
|---|---|
| `llm.rs` | Local-only URL check, `OllamaClient` (models list, improve), prompt, JSON schema, RAM → model advice, template fallback, commands |
| `adf.rs` | Now also turns lines starting with `- ` into a real Jira bullet list |
| `AiSettingsCard.svelte` | Address, model, status ("Ready" / "Not connected"), suggested models, copy install command |
| `PushDialog.svelte` | Editable title, ✨ button, suggestion card, template fallback |
| `push.rs` | `PushRequest.title`: a reworded title becomes the Jira summary and the task name |

## Ideas

**1. Local only, enforced in code.** `normalize_llm_url` accepts `localhost`, loopback, private LAN IPs (10.x, 172.16–31.x, 192.168.x), link-local, `*.local`, `*.lan`, and bare host names. A public address such as `https://api.openai.com` is *refused*, so task text can't be sent over the internet by a typo. The HTTP client also ignores proxy settings (`no_proxy()`).

**2. Make small models reliable with a schema.** Small models are bad at "reply in this format". Ollama's `format` option takes a JSON schema and constrains generation token by token, so the answer is always parseable. We still clean the result (one-line title, no trailing full stop, strip leading `-` from criteria, drop empties, cap lengths) because valid JSON can still be messy.

**3. Prompt design for small models.** Short rules, *"use only facts in the note, never invent"*, low temperature (0.2), a hard output cap (`num_predict`). The user's text sits between triple quotes and is declared to be data, not instructions; `fence()` stops the text from closing the quotes early (prompt-injection hygiene, tested).

**4. The AI proposes, a human disposes.** A suggestion is only shown. "Use this" copies it into the form; "Dismiss" throws it away. The card says to check it, and says which model wrote it.

**5. Graceful degradation.** Not running → "Can't reach the AI server… install from ollama.com" and a template button. Model missing → "Run: `ollama pull qwen2.5:3b`". Timeouts → "try a smaller model". `llm_status` never fails: "not reachable" is a normal answer the Settings card displays.

**6. Hardware-based advice.** `sysinfo` reads RAM; `recommend_models` picks a **fast** model first and a **higher quality** one second. For this job (rewrite a few lines) speed matters more than size. Inside WSL the RAM shown is WSL's limit (20 GB in your `.wslconfig`), not the whole PC.

**7. Testing an AI feature without an AI.** `wiremock` plays Ollama: we assert on *what we send* (model, `stream:false`, the schema, low temperature, the note inside the prompt) and on how we handle good, fenced, broken and empty answers, a 404 "model not found", and a closed port.

**8. Models and speed (CPU).** Rough guide on a normal CPU: 1.5B ≈ very fast, 3B ≈ a few seconds, 7B ≈ 15–30 s for this answer, 14B+ slow without a GPU. First request also loads the model (`keep_alive: 10m` keeps it in memory afterwards).

## What a real run showed (qwen2.5:3b, this PC's CPU, no GPU)
- First call ≈ 40 s (loads the model into memory); every call after ≈ 1 s while it stays loaded (`keep_alive`).
- Titles and descriptions are good. **Acceptance criteria are the weak spot**: they can be wrong or invented (it once added "iambic pentameter" to a poem task; "Login redirects to the login page" for a redirect-loop bug). That is normal for a 3B model, which is why the card tells you to review.
- Adding **one worked example** to the prompt made the biggest difference (the first run's criteria were repetitive and off-topic). Rules alone are weak for small models; examples work.
- A note that tries to instruct the model ("ignore all previous instructions...") is treated as the thing to rewrite, not obeyed as a command.
- For better criteria try `qwen2.5:7b` (about 4.7 GB; roughly 5-10x slower on CPU). `OLLAMA_MODEL=qwen2.5:7b cargo test real_ollama -- --ignored --nocapture` compares models on the same three notes.

## Not done on purpose
- Downloading models from inside the app (it shows the `ollama pull …` command instead).
- Rewriting titles of tasks already in Jira, and summarising commits / MRs (next milestone).
- Thinking-style models (qwen3 and newer) are avoided: they add reasoning text that complicates structured output.

## Try it (needs Ollama; see below)
1. Install Ollama, pull a model: `ollama pull qwen2.5:3b`.
2. ⚙ Settings → *Local AI*: it should say **Ready: qwen2.5:3b**.
3. Push a local task → *New Jira issue* → type a rough title → **✨ Improve with AI** → review → **Use this**.

## Exercises
1. Add the parent epic's title to the prompt so the story fits its epic.
2. Add a "Shorter / More detailed" option to the suggestion card.
3. Show the model's answer time in the card.
