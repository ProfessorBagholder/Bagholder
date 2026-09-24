# Brief 05: every action is the best course for the objective

**The failure:** actions that don't serve the objective. The agent reaches for something because it would produce information, or because it can, not because it is the best way to get the job done: a ten-hour poller, a `find /`, trying things until one works.

**The latest case:** the poller. The chain's own timestamp was already known to be UTC (confirmed against the reply's `Last-Modified`, as the 3a plan records), so the question could have been answered from that. The method came from the stage 3a plan itself: research 4 says "settled from chains captured before and after a session's close on the same day, and across a weekend". The gate (brief 02) passed that.

Examples can't cover the next case. One test covers all of them.

## 1. The rule: at the top of `CLAUDE.md`, under the owner's rule on the old app

> **Every action must be the best course of action for the objective.** Before you do anything (search, scan, run, measure, poll, build, wait), ask: is this actually the best way to achieve what I am trying to achieve? Not whether it would tell you something, not whether it is allowed, not whether it is cheap.
>
> Start from what you already know: the code in front of you, the docs, the replies already recorded, what the owner has said. Usually that settles it: act on it. When something is genuinely missing, the best course is the direct route to that one thing.
>
> A broad or blind move (searching everything, trying things until one works, watching to see what happens) is almost never the best course; it means the problem is not understood yet, so stop and think. If you cannot say why an action is the best course for the objective, don't do it.

## 2. In every plan

- **Each open question states three things:** the objective it serves, what is already known that bears on it (with where it comes from), and why the way chosen to settle it is the best course for that objective.
- **Add to the plan template's Handoff:** "Nothing left running." List every background process and scheduled wake-up this session started, and stop each one or give the reason it must stay.

## 3. At the gate

The reviewer checks every open question's method against its objective. A method that is not the best course for it is a required change, whoever approved it and whatever it costs. That applies to any plan.

## 4. A backstop in the building session's own settings

Claude Code runs hooks before tool calls, and a hook can allow, deny or ask. Set one up in the local session's own settings (not committed; `.claude/` stays out of the repository) with the `update-config` skill, and test it:
- **Ask** before a background command, unless it is one of the repository's own build and test commands (`cargo build`, `cargo test`, `npm run check`, `npm test`, `npm run build`, `npm run e2e`).
- **Ask** before a command that waits in a loop (`sleep` inside `while`/`until`/`for`, `watch`), and before any scheduled wake-up or recurring job.
- **Deny** a single `sleep` longer than a minute.

It catches only the unattended form of the failure. The rule in §1 is what prevents the rest.

## 5. Now

- Put §1 into `CLAUDE.md` and §2 into `PLAN.template.md`.
- Settle 3a's research 4 from what is already recorded (the chain's own timestamp, the captures already taken), and write down the reasoning.
- List anything still running, and stop it.
