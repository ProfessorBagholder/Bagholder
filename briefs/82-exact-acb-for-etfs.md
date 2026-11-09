# 82. Exact ACB for ETFs, applied to the Wealthsimple Tax return

Sent to the CTO 2026-11-09. The owner's go: "yes", to the design below.

## The owner's words (decisions.md, each with its date)

- "remember we are talking specifically about COVERED CALL ETFs."
- "Bagholder has absolute access to the API. It can pull literally anything including any PDFs or CSVs it needs. So pushing that on the user is unacceptable."
- "estimates? wtf? you have all the exact values ... are you trying to tell me you cannot produce PRECISE CORRECt ACB tracking like adjustedcostbase.ca and such services?"
- "individuals file one tax return per year, typically in April-May, after all the documents are available."
- "ok then how can you build this in, the right way, so that it's the most easiest and simplest to actually use when tax time comes?"
- "We also use WS for filing the tax return."

## What it is

T3s, T5s and T5008s already reach the return through Auto-fill. The one thing wrong is the cost on the T5008s, and no slip exists for a gain from ACB going below zero. So Bagholder keeps an exact ACB ledger for each ETF held outside registered accounts, and at tax time applies the right numbers to the person's Wealthsimple Tax return with one reviewed click. The person runs Auto-fill, clicks, checks and files.

No estimates anywhere. No waiting states. Nothing the person has to fetch, import or type, except the rare case in rule 9.

**In:** every ETF in a non-registered account (built for covered-call funds; a plain ETF is the same calculation, so no list of funds is needed).

**Not in:** individual stocks; limited partnerships (T5013); joint-account shares; a spouse's or another broker's accounts; a gain preview on the order ticket; a permanent tab; filing the return; any change to Book, to Trades' P&L or to any figure on the page today.

## Step 0: survey, no PR. Report to me by message

1. How Wealthsimple exposes tax documents: T3, T5, T5008, every year, every account; list and fetch; PDF or data. For a T5008: which amounts and currency boxes 20 and 21 carry, and whether proceeds are gross or net of the commission.
2. Wealthsimple Tax, which sits inside the same login: the calls its page makes to read a return, list its T5008s, set boxes 20 and 21, and add a Capital Gains line, and whether Bagholder's session reaches them. Read the calls from the page's own code. **No writes to anything real.**
3. What each covered-call issuer publishes per distribution on its own site, for the issuers we already read (Harvest, Hamilton, Evolve, BMO, Global X Canada, Purpose, Ninepoint) and any others that fund covered-call ETFs. Hamilton publishes annual per-unit figures only and says the characterization is "only for the entire year and not with each distribution".

Never put a real slip, a real figure or anything from the owner's return in the repository or a message; captures have every amount and date made up. If a permission check refuses a read, tell me what it blocked and leave it.

## Five PRs, in this order, one PR each, sent to me at opening, plants in each

### 1. The slips
Pull T3, T5 and T5008 for every year held, read them (`pdf-extract` is in the workspace if they are PDFs), store them. Wealthsimple's load: past years once; for the year just ended, once a day from February until that year's slips are all in, then none. Nothing during the rest of the year.

### 2. The fund figures
Per-unit return of capital and non-cash distribution for each distribution of each year, from the first of these that has it:
1. the fund company's own published per-distribution figures;
2. the fund's annual per-unit figures, applied to each distribution in proportion to its cash per unit.

The CDS form (per record date) would sit between them. See PR 6 below.

### 3. The ledger, in `engine`, pure and derived on every read (no migration, the current read wins)

1. **What is one holding.** One person's units of one fund across all their non-registered accounts, at average cost, in CAD. Trades' per-account first-in first-out lots are not touched.
2. **Cost and proceeds in CAD,** each on its own settlement date: T+1 from 2024-05-27 (Canada) and 2024-05-28 (US), T+2 before. Bank of Canada rate of that date. A fill the broker settled in CAD costs the CAD that actually moved. Commissions add to cost and are an outlay on a sale. The tax year of a sale is its settlement date's year.
3. **Each Canadian-listed fund's distribution** applied to the units held on its record date: return of capital lowers ACB, a non-cash distribution raises it. On one date: return of capital, then non-cash, then the reinvestment purchase.
4. **The slip wins.** Each year's return-of-capital total per fund equals the T3's box 42 in dollars (CRA's sign: positive lowers, negative raises). A difference of rounding goes on that year's last distribution. A difference beyond a cent per distribution row is flagged, never smoothed, and that year is not complete. A corporate-class fund on a T5: the same rule against its slip.
5. **ACB below zero:** a capital gain that day, ACB then zero, later purchases build from zero. This is a normal path for these funds and is first-class in the tests.
6. **Superficial loss** (61 days around the sale's settlement, identical units acquired in it and held at its end, by the person in any account, registered included): loss denied in proportion to min(sold, acquired, held at end) ÷ sold. Rebought in a non-registered account, it adds to ACB. Rebought in a registered account, it is gone for good.
7. **US-listed funds:** distributions never touch ACB, whatever the fund labels them. Income and foreign tax are what the T5 says. One plain sentence on the holding says so.
8. **Each year has a state:** complete once every slip and fund figure is in and rule 4 passes. The holding's line says "complete through <year>".
9. **The one thing it may ask:** units moved in from another broker at a cost Wealthsimple states wrongly or not at all. It uses `what-you-enter-3` as it stands. An entered correction is a ledger row marked "Entered by you". `what-you-enter-9`'s return-of-capital entry is that same row: reuse it, and say in the PR how it reads in each place.

Tests: case files in the engine's form, from CRA's own worked examples (T4037: identical properties, ACB below zero, foreign currency) and adjustedcostbase.ca's partial superficial-loss example, plus a made-up covered-call book over several years with monthly distributions, reinvestment and a part sale. Hold the ledger to a hand-worked result, not to itself.

### 4. The page
- One ACB line on an ETF holding in a non-registered account, nothing else on Portfolio.
- A **Tax** panel opened from the header menu as Agents is, and from one new notification kind when a year becomes complete. A year appears there only once complete. Copy the nearest screen; run the settled side-by-side check.
- Headline: `2025 is ready: 3 numbers to change on your return`. Then only what must change:
  - each T5008 whose box 20 (or 21, for a slip in USD) needs changing: fund, sale date, units, what the slip says, what to enter, copy button (`pages-9`);
  - a line for each gain from ACB below zero, under Capital gains, publicly traded shares;
  - a denied loss is just the right number with a one-line reason;
  - `Nothing to do: T3s and T5s come in through Auto-fill. Checked, all match.` and `9 other T5008s are already right.`
  - a switch for one combined line per fund.
- Name things by slip and box, so it serves any tax software.
- Any number opens its ledger: date, event, units, CAD change, ACB after, per unit, rate used, source.
- One download for an accountant: summary and every ledger, in one printable file. No heavy new dependency.
- One line, once: these are records, not tax advice.
- An agent tool for all of it.
- Rows on the phone per §8.

### 5. Apply to my return
`Apply to my 2025 return` on the panel. Same review shape as the order ticket: the exact changes, then submit.
- Needs Auto-fill run first; if the return has no T5008s yet, the button says so and links there.
- Re-read the return immediately before writing.
- Match each T5008 by fund, quantity and proceeds. Change only boxes 20 and 21 on matches, and add the below-zero lines, idempotently: applying twice changes nothing.
- Read back and state what landed. Anything unmatched is listed, not touched.
- Never files. Links to Wealthsimple Tax to look it over.
- Proven against a stand-in first. The last proof is on a separate blank tax profile, never a real return, and touches the owner's account, so ask me before that step.
- If step 0 shows the return can't be written, say so and this PR doesn't happen; the copy list stands.

### 6. The CDS form: last, and held
Each person's copy reading the per-record-date form only for funds that person holds, kept on their machine, nothing bundled or shared, between sources 1 and 2 above. The site's terms license personal use and bar a database or inclusion in another product without TMX's written consent. **Build it only if I say the owner has settled that.** No CDS content in the repository in any form.

## How it is checked
Done means the owner's job works end to end on the scratch copy with a made-up book: notification, panel, numbers, ledger, apply against the stand-in. Report real elapsed times.
