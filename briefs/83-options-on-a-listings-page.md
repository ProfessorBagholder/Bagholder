# 83. Options on a listing's page

Sent to the CTO 2026-11-11. The owner's go: "ok, you can hand it off to CTO to implement", on the mockup at https://claude.ai/artifact/9Yd49GPrHZKFxBzfKyYVSH (open it with the Artifact tool's read; build to it exactly).

## The owner's words (decisions.md, each with its date)

- "what i need to see instantly on any ticker i enter is how much it would cost to put one contract in play, how much premium i'd earn in the cleanest possible UI. stop adding all kinds of other things that I didn't ask for."
- "didn't ask for 'delta' or 'IV' or 'volume' or 'open int.'"; "no idea what 'standard' is in this context, another thign i did not ask for."
- "this thing has to be coherent, simple, concise, unambiguous, no made up jargon, no extra fluff"
- "'Cost to put in play' should just say 'Cost'"; "'Premium you'd earn' should just say 'Premium'"
- "add a column that says 'Yield' and is the percentage of the premium divided by the cost"
- "'per year' is hard to estimate reliably ... however the % gain in relative terms for each individual trade is iron clad"

**Nothing beyond the mockup appears anywhere: no extra column, word, note, label or helper sentence.** A wording question comes to me, never a guess.

## What it is

A card titled `Options` on a listing's page (`markets-15`): the page ⌘K opens for any ticker, under Markets for one not held and under Portfolio for a holding, as today. It sits directly under the price chart, before short interest and disclosures. It appears only for a US listing that has options; otherwise there is no card at all.

**The card:**
- `Puts · Calls` (Puts first).
- One button per expiry the chain lists, nearest first, labelled as the mockup does (`Dec 11`).
- A table with four columns, exactly: `Strike`, `Cost`, `Premium`, `Yield`. One row per strike the chain lists for that expiry and side, highest first. The list scrolls inside the card at 436 px (the News card's cap), opened with the strike nearest the price in view.

**The figures, for one contract:**
- **Premium** = the bid × 100.
- **Cost** = the strike × 100 for a put; 100 × the listing's current price for a call.
- **Yield** = Premium ÷ Cost, as a percent to two decimals.
- In dollars, whole (`$6,600`, `$49`, `0.74%`).
- No bid, or a bid of zero: Premium and Yield read `—`.
- A contract whose deliverable is not 100 shares (an adjusted contract, whose OCC root is not the listing's symbol): not listed.

**A row** turns the card into that contract, as the mockup does:
- The title: `KO Dec 11 66 put`.
- `Back`.
- Cost and Premium.
- A chart of the premium each day since the contract began trading. Past days are Yahoo's daily close for the contract's OCC symbol × 100; today's point is today's Premium. The axes are as the mockup draws them. A contract Yahoo has no bars for draws no chart.
- The alert row: `Tell me when the premium reaches $[ ]` and `Save`. Once saved, `Alert at $X` and `Remove`, with the level dashed across the chart.

**Alerts** are a new notification kind through the existing notifications (§2 Notifications), toggled in the menu's Notifications list like the others.
- The banner: `KO Dec 11 66 put: premium reached $90`. A click opens that contract on its listing's page.
- It tells once when the Premium reaches the level, and again only after it has fallen back below.
- An alert ends with its contract's expiry.

## Reads
- **Cboe's delayed chain** (market-data-9's source; the address answers 307 to `cdn-api.cboe.com`): read when the card shows, then every 15 minutes while it shows. For each listing with an alert, every 15 minutes in market hours, page or none. Nothing else, and no recording.
- **Yahoo's chart for a contract:** on opening it, on Yahoo's existing pace.
- Correct SPEC trades-64 ("No source keeps an option's own price history"): Yahoo does, for a contract that has traded.

## Not in
Delta, IV, volume, open interest; any per-month or per-year figure; a Markets card or ranking; history recording or backfill; Canadian-listed options; placing option orders; multi-leg; the phone apps.

## Two PRs, sent to me at opening
1. **The card and the contract view.** Tests:
   - Recorded chain and contract-chart captures, every figure made up.
   - The three formulas on puts and calls, with hand-worked cases.
   - No bid gives `—`; an adjusted contract is not listed; no card for a listing without options.
   - An agent tool for the chain and a contract, in the same change.
2. **Alerts.**
   - The crossing told once and again only after falling back; ended at expiry.
   - The 15-minute reads only for listings with an alert.
   - The banner's words exactly.

Plants for each. Copy the mockup's layout and words exactly, in the app's own tokens for every theme. Done means clicked through on the scratch copy with a made-up book, every step of the mockup.
