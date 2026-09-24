# Brief 05: think first, then look only for what's missing

**The failure:** acting before thinking. The agent reaches for a broad or mechanical move (scanning everything, trying things until one works, watching the world to see what happens) when it already has what it needs, or could get the one missing piece directly.

**The latest case:** a poller ran for over ten hours on an option-chain question. The chain's own timestamp was already known to be UTC (confirmed against the reply's `Last-Modified`, as the 3a plan records), so the question could have been worked out from that. Its method came from the stage 3a plan itself: research 4 says "settled from chains captured before and after a session's close on the same day, and across a weekend". The gate (brief 02) passed that.

Examples can't cover the next case. What covers it is a habit: write down what is known before doing anything to learn more.

## 1. The rule: at the top of `CLAUDE.md`, under the owner's rule on the old app

> **Think first; look only for what's missing.** Before you search, scan, measure, poll, run or wait, state what you already know about the problem and the exact gap that remains. Usually what you know already settles it: the code in front of you, the docs, the replies already recorded, what the owner has said. Act on that. When a gap remains, go straight to the one place that answers it. A broad or blind move (searching everything, trying until something works, watching to see what happens) means the problem isn't understood yet: stop and think instead. If there is no direct way to close the gap, say what is missing and ask.

## 2. In every plan

- **Each open question states two things:** what is already known that bears on it (with where it comes from), and the direct way the rest will be settled.
- **Watching is a last resort.** A question that can only be settled by watching the world over time goes to the top of the plan as an owner decision, with why reasoning and a direct lookup could not settle it.
- **Add to the plan template's Handoff:** "Nothing left running." List every background process and scheduled wake-up this session started, and stop each one or give the reason it must stay.

## 3. At the gate

The reviewer reads every open question for its method. A broad, blind or watching method, where what is already known or one direct lookup would do, is a required change. That applies to any plan.

## 4. A backstop in the building session's own settings

Claude Code runs hooks before tool calls, and a hook can allow, deny or ask. Set one up in the local session's own settings (not committed; `.claude/` stays out of the repository) with the `update-config` skill, and test it:
- **Ask the owner** before a background command, unless it is one of the repository's own build and test commands (`cargo build`, `cargo test`, `npm run check`, `npm test`, `npm run build`, `npm run e2e`).
- **Ask the owner** before a command that waits in a loop (`sleep` inside `while`/`until`/`for`, `watch`), and before any scheduled wake-up or recurring job.
- **Deny** a single `sleep` longer than a minute.

It catches only the worst form of the failure, the unattended one. The rule in §1 is what prevents the rest.

## 5. Now

- Put §1 into `CLAUDE.md` and §2 into `PLAN.template.md`.
- Settle 3a's research 4 from what is already recorded (the chain's own timestamp, the captures already taken) with no further captures, and write down the reasoning.
- List anything still running, and stop it.
