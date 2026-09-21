# Python data reconciliation

This contribution updates the Python reference only, with reusable data fixtures.
It ports missing data fixes from JUBUSINESS `67c22f9` onto upstream `f8965f6`,
including the corrections discovered after the first version of PR #232.
No account database, personal export, statement correction rule, credentials,
branding, styling, portfolio sleeves or deployment configuration is included.

## Compared with current upstream

| Area | Current upstream | This contribution |
| --- | --- | --- |
| Deposited crypto | `40298cb` already excludes unknown acquisition basis from scoring and separates deposited/bought portions | Preserves that separation; withholds unknown public P&L and basis-dependent filters/grades; custody quantities remain visible without claiming zero cost |
| Quotes and cached model | Has source-kind collision protection, pooled connections, cached FIFO books and quote-only refresh | Preserves these paths; broker-only shares also reject coin quotes; import, detail, FX and statement-rule changes invalidate the correct cache |
| Exact option and swap executions | Still maps summary activities without the private order-detail layer | Reads verified order legs; validates units, fees, identities and currencies; rejects incomplete swaps; stores each parent/leg group atomically |
| Option ordering and results | Same-day roll folding and inferred split behavior remain | Uses execution time and stable broker IDs; unrelated contracts retain their own results; unknown split ratios are not guessed from prices |
| Transfers and corporate actions | Does not have the private detail reader or inventory engine | Reads linked funding details and corporate child activities; carries quantity, original cost, dates and fees through transfers, rename chains, consolidations and listing conversions |
| Ticker suffixes | No dated transfer-detail reconciliation | Same broker ID and currency plus a suffix-only difference can link AAA to AAA.TO; unrelated renames are not inferred |
| Broker-only holdings | Builds positions from activity lots | Identified current share/ETF balances remain visible when destination acquisition history is absent; basis remains unknown |
| Complete-account CSV exports | Ordinary append/import path | Explicit, idempotent account/date replacement windows, mapping validation, option units, signed legs and preserved raw sync history |
| Manual asset movements | Aggregate feed valuation cannot establish the transferred securities | Optional validated statement evidence supplies exact quantities for exact events; rules persist through clear/resync and never supply invented prices or costs |
| Broker realized-return report | No equivalent private report reader | Validates complete CAD pagination and totals, retries inconsistent reads once and retains the last complete report as stale on failure; stays separate from FIFO |
| Read failures | No shared private DNS cooldown | Paces read-only broker/market requests and backs off DNS failure; order mutations bypass retries |

Upstream's language-folder restructuring, data-home isolation, test guards,
packaging, other features and existing crypto protections are retained.

## Execution and inventory semantics

`sync_details.py` reads order details using read-only Wealthsimple queries.
A summary can label DOGE while containing BTC units: it cannot establish a
DOGE sale price. Verified swaps create separate signed disposal/acquisition
legs, including outgoing-coin fees. Cash is not misclassified as a deposit.
Option prices are per share with the contract multiplier; exported prices per
contract are normalized. First sync backfills details; unresolved orders retry.

`ws_reconcile.py` reads linked in-kind transfers and corporate-action children.
Only unambiguous dated legs move inventory. DLR and DLR.U are the explicit
reciprocal listing pair; carrying cost is converted using historical FX.
A missing source lot or unresolved spin-off allocation remains unknown.
Option assignment/exercise closes the contract at zero; strike cash and
applicable delivery fees belong to the stock leg.

Broker-reported quantity is authoritative only after a successful balance read.
Security-ID and symbol fallback handle retired identities. Complete export
history remains authoritative for non-crypto quantities when a cached balance
disagrees, with `balanceMismatch` exposed. Mixed-direction books are not reduced
to a guessed single holding. Broker-only fallback excludes cash, derivatives,
closed accounts and unidentified listings.

Unknown basis is marked in the data rather than converted into profit. Public
trade P&L and unknown holding cost/average/unrealized fields are null. Such trades
are excluded from performance statistics and cannot pass profit/entry filters;
known and unknown portions stay separate. Raw executions remain inspectable.

## Complete-account import API

`POST /api/import` accepts `name`, CSV `text`, and `reconcile: true` for complete
Wealthsimple activity exports with `effective_date`. Ordinary legacy/statement
CSV import keeps its existing behavior. No menu or visual redesign is included.

The export must contain every activity type for its accounts and period. A file
filtered to one security or activity type is not a complete replacement source.
The replacement interval is the inclusive first/last date actually in the file;
other accounts and dates remain intact. Parsed export rows are retained apart
from the original broker records, and repeated imports do not double count.
Unparsed rows or ambiguous account mapping stop reconciliation before writes.
`accountMap` can supply custodian-account to broker-account mapping explicitly.

## Persistent statement-confirmed corrections

`store.save_statement_corrections(rules)` saves approved evidence under
`statement_transfer_corrections_v1`. Each rule names an exact manual broker
event, date, source/destination accounts, currency, evidence description and
complete security list (symbol, broker ID and positive quantity). No rule ships
with the code. Price, cost and profit fields are rejected.

Only matching source and destination web records activate a rule. The effective
snapshot overlays paired inventory movements; the database keeps the original
broker records and aggregate valuations. The ordinary inventory engine carries
existing cost/dates/fees. Missing source cost remains unknown. Changed records
block the rule; broker-supplied linked legs take precedence to avoid duplication.
The model and book API expose audit status under `statementCorrections`.

All clear-data options preserve this evidence configuration. A later web sync
recreates the matching broker records and reapplies the correction. Replace the
validated ruleset to amend evidence, or save `[]` to remove all corrections.
The clear/rebuild regression uses synthetic rules, never a customer statement.

## Broker reporting and presentation scope

`brokerPerformance` contains the broker-adjusted all-account/all-time CAD report,
its by-security/monthly breakdown and freshness state. It is suppressed when
trade filters apply. It does not overwrite Trading P&L: average-cost accounting,
return of capital, broker adjustments and FX may differ from journal FIFO.
`inventoryWarnings` and `statementCorrections` expose unresolved evidence.
This PR changes data/API behavior, not the page's layout or new report panels.

## Regression contracts and platform scope

The existing Python tests plus synthetic sync/import/statement cases cover
atomic rollback, partial reads, repeat sync, account clear/rebuild, stable broker
identities, suffix matching, cache invalidation and preserved upstream behavior.
Four additional shared JSON cases specify hand-checkable results:

- Transfer 10 shares bought at 20 with fee 2; sell at 30 with fee 3: realized 95.
- Consolidate 100 shares with total cost 200 into 10; sell at 30: realized 100.
- Transfer 10 shares without acquisition history then sell: public P&L null,
  zero scored trades, no invented 300 profit.
- Exercise a 10-strike call purchased for 100: option result -100 and 100 shares
  with carrying cost 1000; delivery cash is not option profit.

The JSON schema is `today`, `snapshot`, `market`, `filters`, `journal`, `expect`.
Files live in `tests/cases`; generator: `python/tests/make_cases.py`. Existing
option-roll and crypto fixtures also record corrected expectations. These are
reference contracts for reuse by Rust, Go, Swift and Kotlin, **not a claim those
implementations are updated or pass them**. Cross-port implementation/testing is
outside the contributor's expressly requested Python scope.

Private fix inventory reviewed: the earlier crypto/options/import commits through
`55d6b81`, then `80619b5` (broker-only holdings portion), `ab78fca`, `2820f50`
(read pacing portion), `14fc341`, `fa61565` (data warning portion), `9a2c5cb`,
`7a90f3b` (inventory/partial-trade data portion) and `67c22f9`. Presentation,
sleeve and unrelated performance changes were excluded.
