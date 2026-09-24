# Brief 05: proportionate effort

**Why:** a poller ran for over ten hours to settle a question about option chains. The instructions push one way: never guess, prove from real replies, research before building. Nothing pushes back with "and it must be worth what it costs". An agent following only the first half will buy certainty at any price.

Examples won't fix that, because the next case won't look like this one. A rule about cost will, backed by two checks that don't rely on the agent remembering it.

The poller came from the stage 3a plan itself. Research 4 says it is "settled from chains captured before and after a session's close on the same day, and across a weekend". That is a plan to wait on the world for hours, and the gate (brief 02) passed it. The gate now checks for this (§3).

## 1. The rule: at the top of `CLAUDE.md`, under the owner's rule on the old app

> **Proportionate effort.** Every step costs something: the owner's time and attention, time on the clock, tokens, and requests to outside services. "Never guess" means never *present* a guess as a fact; it does not mean buying certainty at any price. Before a step that is slow, unattended or open-ended, name exactly what you need to know, then take the first of these that settles it:
> 1. reason from what is already known;
> 2. read what exists: the code, the docs, recorded replies, data already stored;
> 3. make one cheap request;
> 4. change the design so the question no longer needs answering (for example, take the value from each reply as it comes, instead of learning how the source behaves).
>
> An experiment comes only after all four fail. One that waits on the outside world, runs unattended, or takes longer than fifteen minutes is proposed to the owner first, with what it costs and which of the four were tried. Nothing is left running that no one is waiting on, and a session never ends with a process still running.

## 2. In every plan

- **Each open question states how it will be answered and how long that takes.** An answer that needs waiting (a market session, a weekend, a source's next publication) goes to the top of the plan as an owner decision, with the cheaper routes that were tried and why they failed.
- **Add to the plan template's Handoff:** "Nothing left running." Before handing off, list every background process and every scheduled wake-up this session started, and stop each one or give the reason it must stay.

## 3. At the gate

The reviewer reads every research question and every "how it will be answered" for cost. A slow, unattended or open-ended method with no owner decision behind it is a required change. That applies to any plan, not only 3a.

## 4. A backstop the agent can't forget: a hook in the building session's own settings

Claude Code runs hooks before tool calls, and a hook can allow, deny or ask. Set one up in the local session's own settings (not committed: `.claude/` stays out of the repository) with the `update-config` skill, and test it:

- **Ask the owner** before a background command, unless it is one of the repository's own build and test commands (`cargo build`, `cargo test`, `npm run check`, `npm test`, `npm run build`, `npm run e2e`).
- **Ask the owner** before a command that waits in a loop (`sleep` inside `while`/`until`/`for`, `watch`), and before any scheduled wake-up or recurring job.
- **Deny** a single `sleep` longer than a minute.

That reaches the owner only in the rare case where the rule above has already been broken, and it stops that case before it runs.

## 5. Now

- Put §1 into `CLAUDE.md` and §2 into `PLAN.template.md`.
- Mark 3a's research 4 answered, from the replies already recorded or by the design route in §1 point 4, with no further captures.
- List anything still running, and stop it.
