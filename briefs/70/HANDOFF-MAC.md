# Handoff: run the filings-model benchmark on the Mac

For an Architect session running on the owner's Mac. This is the same test the server ran (brief 70), so the two can be compared: one engine build (llama.cpp b11750), six models, eight public company press releases (`llm-bench/samples/`, taken from SEC EDGAR).

## What it measures

- **Speed:** the input read and the output written, in tokens a second, each with the GPU on (`-ngl 99`, Metal) and off (`-ngl 0`, CPU only).
- **Quality:** for each filing, one call that returns a title (at most 8 words) and a summary (one sentence, at most 20 words) as JSON, with the GPU on. It records the time, the tokens, and the answers.

## Rules while it runs

- Don't touch the owner's Bagholder: not its checkout, its data folder, its running app or its port 8765.
- The script works in a `mktemp -d` folder and deletes it when it ends, whatever way it ends. It starts one `llama-server` at a time on 127.0.0.1:18080 and stops it before the next.
- Nothing is installed. It downloads about 4.5 GB over the run, one model at a time, and deletes each model after its test.
- When it's done, check that nothing is left behind: `pgrep -af "llama-server|llama-bench"` must print nothing.

## Steps

```bash
git -C ~/dev/Bagholder fetch -q origin architecture-briefs
git -C ~/dev/Bagholder worktree add --detach /tmp/bh-bench origin/architecture-briefs
cd /tmp/bh-bench/briefs/70/llm-bench && ./bench.sh
```

It writes `results-<hostname>.md` in that folder. The run takes about 20–40 minutes on Apple Silicon. Keep the Mac awake and plugged in, and don't run other heavy work during it.

Then commit the results file to the briefs branch and remove the worktree:

```bash
cd /tmp/bh-bench && git add briefs/70/llm-bench/results-*.md \
  && git -c user.name=Claude -c user.email=noreply@anthropic.com commit -m "Brief 70: benchmark results from the Mac" \
  && git push origin HEAD:refs/heads/architecture-briefs
git -C ~/dev/Bagholder worktree remove --force /tmp/bh-bench
```

The results hold only the machine's chip name and memory size, the timings, and the models' titles and summaries of public filings. There is no personal data.

If a step fails, report the exact error and stop. Don't work around it.
