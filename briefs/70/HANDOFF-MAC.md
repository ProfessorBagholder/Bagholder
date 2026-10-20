# Handoff: run the filings-model benchmark on another machine

For an Architect session on another machine, for example the owner's Mac. It runs the test brief 70 ran: one engine build (llama.cpp b11750), the models in `llm-bench/models.txt`, and eight public company press releases (`llm-bench/samples/`, from SEC EDGAR). Speed is measured with the GPU on and off. Quality is one JSON call per filing for a title and a summary.

## It runs inside your sandbox as it is

Don't change the owner's settings, add exclusions, or switch permission modes for this. If a step is refused, stop and report the exact error.

- **Folders:** everything is written under `$TMPDIR`. That covers the clone, the downloads and the results. Nothing goes in `/tmp` itself or in the owner's checkout.
- **Network hosts the run needs:**
  - `github.com`, `api.github.com`, `*.githubusercontent.com` (the engine), which are usually allowed already;
  - `huggingface.co`, `*.hf.co`, `cas-bridge.xethub.hf.co` (the models), which are usually not.

  Allow the Hugging Face hosts for the one `./bench.sh` command when you run it, and for nothing else.
- **Local:** it starts one `llama-server` at a time on 127.0.0.1:18080 and stops it before the next.
- **Leave alone:** the owner's Bagholder (its checkout, data folder, running app and port 8765).

## Steps

```bash
git clone -q --depth 1 --branch architecture-briefs https://github.com/ProfessorBagholder/Bagholder.git "$TMPDIR/bh-bench"
"$TMPDIR/bh-bench/briefs/70/llm-bench/bench.sh"
```

Run it by its path, as above, and never `cd` into the clone: a session's folder follows its shell, and the sandbox then protects the clone, so it can't be deleted afterwards. It writes `results-<hostname>.md` beside the script, in `$TMPDIR/bh-bench/briefs/70/llm-bench/`: about 20–40 minutes on Apple Silicon. Keep the machine awake and plugged in.

## When it's done

1. **Check nothing is left running:** `curl -s -o /dev/null 127.0.0.1:18080/health; echo $?` must print `7` (nothing listening).
2. **Send the results file's whole text to the Architect** in a message. No push is needed; the Architect commits it. It holds only the machine's chip and memory, the timings, and the models' titles and summaries of public filings.
3. **Delete the clone:** `rm -rf "$TMPDIR/bh-bench"`.
