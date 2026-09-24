# Brief 04: options, trade marks, and what a blind check of the cases found

**Checked:** `svelte-migration` at `039613de`.

**How it was checked:**
1. 15 cases were re-derived by agents who never saw the answers or the engine.
2. The engine's code was traced in five areas (trade application, endings, trade marks, broker positions, case coverage), each trace checked by an independent skeptic.
3. The reviewer re-read every finding below that changes a recommendation.

**This replaces earlier versions of this brief**, which were written before the engine's code was read. In particular, "show the broker's per-contract position" is withdrawn, and the options "design" turned out to be already built.

## Already right: no change

**The four actions** (buy to open, sell to close, sell to open, buy to close), **the three endings**, and **every strategy** (long calls and puts; covered and naked calls; secured and naked puts) are implemented generically, as the stage 2 definitions say:
- `apply_trade` (`engine/src/ledger.rs:953-994`) keeps one net position per contract when no effect is stated.
- `apply_delivery` (`ledger.rs:1135-1180`) closes the shares held and opens the rest the other way. It values them at the stated cash, else strike × shares.

## 1. A stated effect that contradicts the position: fix before 3b

**Today:** a stated "open" against an opposite position doesn't close it. The engine opens the other direction beside it, in a round trip of its own, and taints the holding (`ledger.rs:961`, `:970-974`, `:982`, `:547-549`). The case `options.json:27-49` expects this. The result:
- **Two positions:** a long and a short of one contract in one account.
- **Later matching breaks.** `close` stops at the first lot of the other direction (`ledger.rs:630-633`), so a correctly labelled buy to close can't cover the short and goes beyond-held, and a buy with no effect adds to the long.
- **The equity series and the broker check** use net units (`ledger.rs:800-814`) and never read the taint: they show 0 with no gap.

**When it fires:** never on today's data. The import states no option effect (`book/src/import/mapping.rs:137-141`). It starts when 3b's Wealthsimple rows state their effects, so it must be fixed before 3b lands.

**The fix:**
- **Hold back what contradicts the position.** A transaction whose stated effect contradicts the position before it is not applied. That covers an open meeting the opposite direction, a close meeting the opposite direction, and an ending that meets the wrong side (an assignment against a long or nothing, an exercise against a short, an expiry row whose sign opposes the position).
- **Reuse the existing "waits" path**, the one `quantity-unstated` and `leg-unstated` use (`ledger.rs:835-881`): taint the holding and list the transaction in `unapplied`. The equity series then states the account's day from the broker, marked as the broker's (`equity.rs:126-130`, `:265-268`). No new mechanism is needed.
- **Keep beyond-held** for a close that meets nothing, as definition :75 says.
- **Strengthen the runner's invariant:** no book holds lots of both directions. `tests/cases.rs:493-496` checks only duplicate keys.
- **Write the rule** into the stage 2 definitions (The ledger, Options) in a sentence.

## 2. Same-instant ordering makes conflicts out of correct records: fix with §1

At one instant, a record stating "open" is applied before one stating "close" (`ledger.rs:465-474`; stage 2, "The order of transactions"). Example: long 1 held, then a sell to close 1 and a sell to open 1 at the same instant (or undated on the same day). The open goes first and meets the long. Today that raises a false conflict. Under §1's fix it would hold back the open and leave the book flat, which is wrong.

**The fix:** at one instant, a stated close that meets a position goes before a stated open in the other direction. An open still goes first only when a close at the same instant needs it (a short opened and covered at one instant). Add a case each way. §1 must not land without this.

## 3. A corporate event that continues a holding leaves a short behind

`apply_leg` counts the units held from long lots only (`ledger.rs:1259`), and `take` stops at the first lot that isn't long (`:695`). So when an event moves a holding to a new instrument, a short position stays under the old one, with no gap.

**The fix:** an event moves or scales short lots as it does long ones, or the holding waits. Add a case with a short through a split and through a continuation.

## 4. Several open round trips shown as one position

`build_positions` makes one position per account, instrument and direction from every lot of that direction, and keys it by the first lot's round trip (`engine/src/positions.rs:122-124`, `:193`). `SPEC.md` §2 defines a position that way, but stage 2 says a position's id is its round trip's trade id. When several round trips are open in one direction, the position takes the first one's id and journal. That happens with a deposited coin beside bought ones, or with shares delivered into an existing holding.

**The fix:** say in the definitions what such a position links to (all its open round trips, not the first alone), and add a case.

## 5. The importer and the engine disagree on ending rows

- **An expiry row with no quantity.** The engine and definition :77 treat it as a full close (`ledger.rs:873-875`). The import gives the same row a `quantity-not-stated` problem (`mapping.rs:293-295`), so the account's equity waits for nothing.
- **An assignment or exercise row with no quantity** taints only the contract (`ledger.rs:876-881`). The delivered shares neither move nor wait.

3b's Wealthsimple mapping replaces the import. Make it agree with the engine's definitions for these rows, and make a delivery with no quantity put the underlying on hold too. Add a case each.

## 6. Cases to add

Cases already there:
- **The short put assigned:** `options.json:227-250`. Restate it with the sale stated as a sell to open, and the received shares later sold.
- **The short call assigned** already holds the shares (`options.json:204`, `:217`). Rename it to say the call was covered.

New cases:
- **Options with stated effects, closed in parts**, one long and one short.
- **A long put exercised, with the shares held and without.** Also amend stage 2 :74, which allows a short to be opened by an assignment only; the code opens one for an exercise too (`ledger.rs:1162`, `:1175-1179`).
- **A naked call assigned with no shares held.** State the cash: the value is the stated cash, and strike × shares only without it.
- **A covered call of 2 contracts assigned with only 150 of 200 shares held.** Don't assert `cash_invariant`: the runner sums every fill's cash (`tests/cases.rs:179-185`, `:505`).
- **The conflict and ordering cases of §1 and §2, and the short through events of §3.**
- **Stage 2 :78:** say that shares already held are closed inside their own round trip, and only the remainder opens a round trip keyed by the delivery.

## 7. Trade marks

- **Eight marks no definition names:** `rolled`, `split`, `continued`, `deposited`, `transferred`, `from-event`, `assignment` and `no-expiry-record` (`ledger.rs:72-84`). Only `reward` is defined.
- **None reaches the page today.** The engine runs only in `compare-figures`, and the wire carries the old model's marks.
- **`039613de` already rules that each is defined in `SPEC.md` at the switch or kept off the page** (`docs/architecture.md:244`), but it leaves out `deposited`. Add it.
- **`basis-unknown`** is a gap in the engine, not a mark, but the page looks for it among the marks (`web/src/lib/Trades.svelte:77`). At the switch, the page reads the gap.

## 8. Run the blind check over every case

`039613de`'s check covered every case in two files and only the first case elsewhere (`tests/cases/README.md:17`), so it never reached the conflict case. Run it over every case in every file, one blind agent and one judge per file. That's a fan-out job for a workflow.
