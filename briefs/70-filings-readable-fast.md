# Brief 70: Filings made readable, fast, on any device: the regulators' own data first, then one call to a small model on the GPU

## Verdict

Filings are unreadable to a person as they come from the regulators, which is why Bagholder has a model at all. The owner has decided the built-in model stays on by default (`docs/decisions.md`, brief 15 decision 4). What was carried over from the Python app, and never justified, is how it runs:
- a 2 GB Gemma 2 2B executable, downloaded at first use;
- the GPU switched off on every device (`-ngl 0`, `localmodel.rs:470`);
- two calls per filing, each reading the same 3,000 characters;
- the text sent being, as often as not, the SEC cover page rather than the news;
- a process that outlives the server: five orphans were found on one machine, bound to one port;
- structured filings sent to the model although the regulator already states what they are.

**The design, in the order to build it:**
1. Fix what is broken now.
2. Name every filing the regulators already describe without the model.
3. Read everything else with one call to Qwen3.5-2B on the GPU, checked against the filing's own text.
4. Run the model inside the server, through llama.cpp's own library, behind a plan.

## The evidence

### What the model costs today, measured

- **The model now in use**, `-ngl 0` as Bagholder runs it, on the old executable: **5½ minutes per call** on a 2-core x86 without AVX (2026-10-20, this server). That machine is slower than a Raspberry Pi 5 for this work, so it shows the floor, not the norm.
- **On an Apple M3 with the GPU used** (`briefs/70/llm-bench/results-air.md`): the same model takes **2–3 s per filing**. With the GPU off, its prefill is 3.8× slower (625 → 163 tokens a second).

### Six models on eight real press releases

The kit is in `briefs/70/llm-bench/`:
- the same llama.cpp build (b11750) on every machine;
- one JSON call per filing;
- 8 public press releases from EDGAR: 5 US companies and 3 Canadian.

Results on the Apple M3, Metal, in `results-air.md` and `results-air-qwen.md`:

| Model (Q4) | Size | Per filing | Prefill, GPU / CPU (tok/s) | Quality | Licence |
|---|---|---|---|---|---|
| Gemma 2 2B (today) | 1.6 GB | 2–3 s | 625 / 163 | good | Gemma Terms |
| Gemma 3 1B | 0.8 GB | ~1.2 s | 1697 / 575 | good; one wrong title (Shopify's sales release called an "Annual Report") | Gemma Terms |
| **Qwen3.5-2B** | 1.1 GB | **~1.9 s** | 886 / 311 | **best: all 8 right, every figure found in the filing** | **Apache-2.0** |
| Qwen3.5-0.8B | 0.5 GB | ~0.9 s | 1962 / 702 | **invented a figure** (Tesla "$2,000 per share", which is in no filing); called an agreement a "completion" | Apache-2.0 |
| LFM2.5-1.2B | 0.7 GB | ~0.9 s | 1351 / 539 | right but vague ("Merger Announcement") | LFM 1.0 (revenue cap) |
| Gemma 3 270M | 0.2 GB | ~0.5 s | 9435 / 2661 | unusable: echoes the form's header | Gemma Terms |

- **Grounding was checked by search:** every figure each model gave was looked up in its source text. Only Qwen3.5-0.8B invented one.
- **Two filings show an input problem,** a Tesla 8-K and a Shopify 6-K. Their first 3,000 characters are SEC cover-page boilerplate, so every model saw a cover and no news (see change 3).
- **No Raspberry Pi was measured.** The published llama.cpp measurements on a Pi 5 (llama.cpp PR #17494; the research note in this brief's history) put a 2B-class model at **about 30–40 s per filing**, and a 1B model at 10–17 s. That runs in the background, once per filing, and is acceptable there. The kit runs on a Pi unchanged if a check is wanted.

### What each alternative runtime offers

Researched from each project's own documentation, 2026-10-20:
- **llama.cpp as a library** (crate `llama-cpp-2`, dynamic backends):
  - Metal on macOS by default;
  - Vulkan for NVIDIA, AMD and Intel graphics;
  - CPU variants chosen at runtime for the instructions the machine has;
  - GPU layers "auto" by default upstream.
  The model is loaded once and stays resident.
- **llamafile 0.10.x:**
  - Metal needs Xcode's command-line tools on first run;
  - other GPUs need `-ngl 999`;
  - it falls back to the CPU silently.
- **The rest:**
  - Ollama is about 1.5 GB to bundle;
  - ONNX Runtime GenAI has no Metal;
  - mistral.rs and candle have no Vulkan;
  - MLX runs only on a Mac;
  - MLC-LLM is unmaintained.

## Required changes

### 1. Fix what is broken now (one small PR, before the rest)

- **The GPU.**
  - Pass `-ngl 999` instead of `-ngl 0`, in both launch paths (`localmodel.rs:470` and the `sh` fallback).
  - Give `-c 4096`, so the window fits the prompt.
  - Raise the runner from the 2024 llamafile to 0.10.6, or keep it until change 4 replaces it. Say which.
  - *Test:* the command line the code builds carries `-ngl 999`, never `0`.
- **The process lives and dies with the server.**
  - It is started with `BAGHOLDER_EXIT_WITH` set to the server's own pid (#555's mechanism), so it exits when the server does.
  - Its port is taken only after the old one is confirmed free, and a second spawn is refused while one answers.
  - *Test:* a server stopped while its model runs leaves no model process. Two calls to bring the model up start one process.
- **A read counts only when the model answered.**
  - A timeout, an error or an empty reply leaves `enrich_reads` as it was (`filings.rs:1671`).
  - Ollama counts as ready only when `/api/tags` lists the model asked for (`localmodel.rs:196`).
  - *Test:* a POST hook that fails, or returns 404, changes no read count. Ollama without the model counts as not ready.

### 2. Name the filings the regulators already describe (no model)

- **EDGAR's submissions data:**
  - read `items`, `reportDate`, `acceptanceDateTime` and `size` (`edgar.rs:278`), which are thrown away today;
  - name an 8-K from its `items`, and a 10-K or 10-Q from its form plus `reportDate`, with no download;
  - parse forms 3, 4, 5, 144 and D, and 13D/G, from their XML (SEC technical specifications), mark the result final, and never call the model on them;
  - fix the 13D amendment called a 13G (`edgar.rs:396`).
- **SEDAR+:**
  - take the document type from the CSA Filing Inventory's list rather than from substring guesses;
  - take a material change report's own summary from its Item 4. Confirm the current form's wording first: the copy found was a rescinded one.
- **One table of form names,** where there are three today (`edgar.rs:202`, `formnames.rs:14`, `filings.rs:950`).
- *Tests:*
  - generated submissions rows give their titles with no download, and the model hook set to panic;
  - generated ownership XML across every transaction code;
  - every Inventory document type maps to a category;
  - every code is named the same on every surface.

### 3. The rest: one call to Qwen3.5-2B, on the press release, checked against it

- **The model:** Qwen3.5-2B, Q4_0 (Apache-2.0, 1.21 GB), pinned by its SHA-256 (`briefs/70/llm-bench/models.txt`).
  - It gave the best answers measured, and every figure it gave is in its filing.
  - Its licence asks nothing of a redistributor beyond Apache-2.0's notice.
  - Thinking is off (`enable_thinking: false`).
- **One call:** the filing text first, then the instruction. The output is held to a JSON schema with a title of at most 8 words and a summary of at most 20.
  - `cache_prompt` on;
  - greedy decoding;
  - at most 120 output tokens.
  This replaces `summarize` and `title_from_model` (`enrich.rs:294`, `:368`).
- **The right text:**
  - an EDGAR filing is read from its press-release exhibit, chosen by the index's document type (EX-99.1), not by file size (`edgar.rs:427`);
  - the SEC cover page is stripped before the 3,000 characters are taken.
- **Checked against the filing:**
  - every number, and every capitalised name, in the summary must appear in the filing's text;
  - a summary that fails is not shown, and the title alone stands.
  Small models can invent: one did, in this test.
- **The extracted text is kept with the filing,** and the model's step is versioned apart from it, so a new prompt or model re-reads no PDF (`ENRICH_VERSION`, `app.rs:20`).
- *Tests:*
  - the schema call over the eight sample filings in `briefs/70/llm-bench/samples`, with a model stand-in;
  - a summary carrying a number the text lacks is dropped;
  - an exhibit chosen over a larger XBRL file;
  - a version bump makes no source request.

### 4. The model inside the server: a plan first

Running llama.cpp in the server through `llama-cpp-2`, with dynamic backends (Metal, Vulkan, the CPU variants), loaded once, replaces the downloaded executable and its process altogether. It adds:
- a dependency with C++ in its build;
- Vulkan in CI;
- backend libraries shipped beside the binary.

That is a change to how the app is built and shipped, so it goes through the gate. The plan must state:
- the build and archive size per platform;
- how a GPU device is chosen safely. Vulkan can pick llvmpipe in a container, or a GPU too weak to help; use an allowlist, with a short micro-benchmark at start, falling back to the CPU;
- the cost against §16.

Changes 1 to 3 don't wait for it.

## Order

1. Change 1 now, in one PR.
2. Then change 2, which takes most filings off the model.
3. Then change 3.
4. Change 4's plan alongside them.

Each is measured with this brief's kit, before and after: one cold filing, one older filing, and a batch of ten.
