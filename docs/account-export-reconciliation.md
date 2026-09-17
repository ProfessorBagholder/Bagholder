# Complete account export reconciliation

The Python server accepts complete Wealthsimple activity CSVs with
`effective_date` through `POST /api/import` with `reconcile: true`, `name`, and
`text`. Select exports containing every activity type for the chosen accounts
and period; filtered files cannot establish a complete replacement window.
The existing ordinary import continues to handle legacy/statement formats.
This PR does not add a menu action or change the page's appearance.

Reconciliation retains the original synced database rows. Each export replaces
the effective activity history only for its mapped account and the inclusive
first/last dates present in the file. Other accounts and dates remain intact.
Repeated imports replace the same interval without accumulating duplicates.
The HTTP import accepts `reconcile: true`; an optional `accountMap` maps custodian
account numbers to broker account IDs when automatic matching is ambiguous.
Automatic mapping uses broker balance identifiers, then unambiguous historical
trade matches. Unknown account mappings stop the import before any writes.

Corporate actions use signed, dated event legs rather than permanent ticker
aliases. Unambiguous rename, listing-swap and split pairs move FIFO lots and
their carrying costs without realizing a sale. Chains such as OLD → MID → NEW
are supported, and buying OLD again later creates a separate holding. Internal
account transfers move lots to the destination account. A single subdivision
delta adjusts units while preserving cost. Ambiguous pairs do not borrow cost
from an unrelated ticker; missing source basis is reported.

DLR and DLR.U are the only built-in reciprocal pair. Either leg of a standalone
listing-swap record can establish the other, and an explicit two-leg event is
applied once. Synced DLR `JOURNAL_SHARES` records also move into the reciprocal
listing. Cross-currency carrying costs use the historical FX cache.

Option symbols are normalized from OCC to the existing contract format. Export
prices are per contract, so they are divided by 100 for the model's per-share
premium representation. Explicit LONG/SHORT directions and expiry signs bypass
the feed's missing-leg inference. Exported option rolls retain each contract's
own cost instead of folding earlier realized results into a new contract.
Crypto movements are combined by coin, converting USD valuations to CAD using
the supplied FX rate or historical cache. Transfer valuations are not treated
as cash deposits, sales or original purchase costs.

Reconciled export quantities drive non-crypto positions when cached broker
balances disagree; the disagreement is exposed as `balanceMismatch` in the model response. Crypto still uses the broker's quantity because exports
can omit in-kind transfer fees. Missing transfer/distribution basis is exposed
through `basisWarnings`; portfolio cost and unrealized totals are withheld when
current basis is incomplete. This does not establish tax cost or verify all
historical performance calculations.

The implementation targets the Python server/web importer. Raw export rows are
not a promise of equivalent interpretation by independent mobile model ports.

Validation includes synthetic rename chains, ticker reuse, ambiguous pairs,
forward/reverse DLR conversions, explicit versus synthesized legs, split deltas,
account transfers, option units and direction, crypto currency normalization,
stale balances, mapping rejection, preserved raw sync data and idempotent
reimport. Personal CSVs are used only for private integration checks and are
not checked into the repository.

## Execution order and realized option results

The Python model orders dated executions by timestamp before side, including
raw synced trades. Side priority is only a tie-breaker. Ordinary contracts keep
their execution prices and realized results: another contract on the same
underlying and day is not evidence of a roll. The former same-day folding
heuristic could redistribute results, mutate IDs while tracking removed rows,
and inflate reported profits. It is removed for both synced and exported rows.
Explicit incomplete multileg feed inference remains separate; read-only order-detail enrichment supplies verified executions when the API provides them. Native model ports have not been updated by this Python-server correction;
changed shared fixtures identify the expected new results and require native
parity before a cross-platform release.

CryptoSwap export rows retain separate signed disposal/acquisition quantities.
Both valuations are converted to CAD together, the disposal realizes the
outgoing coin's FIFO result, and the acquisition establishes the incoming
coin's basis. Their cash amounts cancel; they are not deposits or withdrawals.
Raw SWAP_MARKET_ORDER rows with only one quantity are excluded from fills and
reported as missing swap legs in the model response. In
particular, the feed can label DOGE while carrying BTC units: dividing its CAD
amount by those units must not create a DOGE sale at the BTC price.

Closed trades that include missing original transfer basis now publish null
entry/P&L fields . They remain visible with executions, but
are excluded from win/loss, monthly, symbol and grade performance statistics.
They also cannot pass a winner/loser/breakeven filter or export a fabricated
profit. This does not reconstruct the original acquisition cost.

## Upstream comparison and integration

Ported onto upstream `e48a605` from the private fork's data commits:
`b9a79e1`, `51a0423`, `2811802`, `20b3331`, `e133a55`, `620147c`,
`09a1ec6`, `8e456e0`, and `55d6b81`. These fixes were absent from that base.

Upstream already has crypto transfer-out-at-cost handling, quote-source
collision protection, pooled SQLite connections, cached matched books, and
wildcard Python-module packaging. Those newer paths are preserved or adapted:
custody movements now also flag unknown incoming basis; transactions use the
connection pool's rollback semantics; import/detail/FX changes invalidate the
matched book; quote-only refreshes retain authoritative balance reconciliation.
No Dockerfile change is needed for the new modules.

Order-detail sync is read-only at Wealthsimple. First sync backfills details;
unresolved orders retry on subsequent syncs. Verified legs replace summary
interpretation atomically and retain stored identities. Incomplete details are
excluded rather than assigned inferred profits. There are no CSV inputs in
that sync path, and no production clear/resync is required.

This contribution contains no branding, layout, colours, portfolio sleeves,
deployment settings, credentials, database snapshots or personal exports.
Native Swift/Kotlin models are a separate implementation and are not ported
here. This PR is a desktop/Python integration draft pending that parity review.
