// Generated from rust/crates/model (wire.rs and the types it names). Do not edit:
// change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-model --test types`.

export type Kind = "Shares" | "Options" | "Crypto" | "Futures";

export type Side = "BUY" | "SELL";

export type Direction = "LONG" | "SHORT";

export type ExitSide = "SELL" | "COVER";

export type Mark = "fill" | "quote";

export type Payment = "Dividend" | "Interest" | "Withholding tax" | "Interest charge";

export type RateSource = "declared" | "payments" | "";

export type Priced = "close" | "fill";

export type Op = ">" | "<";

export type Range = { op: Op, 
/**
 * Nothing: the range is off.
 */
v: number | null, };

export type Lists = { account: Array<string>, symbol: Array<string>, grade: Array<string>, tag: Array<string>, kind: Array<string>, exchange: Array<string>, side: Array<string>, result: Array<string>, };

export type Ranges = { price: Range, hold: Range, pnl: Range, qty: Range, };

export type Filters = { lists: Lists, ranges: Ranges, preset: string, years: Array<string>, from: string, to: string, search: string, benchmark: string, };

export type Leg = { 
/**
 * What a saved group names this member by.
 */
key: string, qty: number, entry: number, exit: number, entryDate: string, exitDate: string, pnl: number, pnlCad: number, fees: number, buyActivityId: string, sellActivityId: string, flags: Array<string>, };

export type Fill = { id: string, when: string, date: string, time: string, 
/**
 * `BUY`, `SELL`, or nothing when the row does not say.
 */
side: "BUY" | "SELL" | "", 
/**
 * Under a trade, what the fill did in it (`BUY TO OPEN`, `SELL (close +
 * open)`); under a holding, the broker's own sub-type.
 */
sub: string, 
/**
 * Signed: negative for a sale.
 */
qty: number, price: number, amount: number, fees: number, currency: string, flags: Array<string>, };

export type Tally = { qty: number, avg: number, fills: number, };

export type Trade = { id: string, status: string, 
/**
 * Grouped by hand, so not regrouped by the model.
 */
locked: boolean, symbol: string, underlying: string, name: string, exchange: string, kind: Kind, currency: string, account: string, accountId: string, securityId: string, side: ExitSide, openDirection: Direction, qty: number, mult: number, entry: number, exit: number, entryDate: string, exitDate: string, entryWhen: string, exitWhen: string, holdDays: number, pnl: number, pnlCad: number, fees: number, feesCad: number, 
/**
 * Nothing when there is no basis to measure against.
 */
pnlPct: number | null, 
/**
 * Sent only for the trade whose page is open.
 */
legs?: Array<Leg>, legCount: number, 
/**
 * Sent only for the trade whose page is open.
 */
fills?: Array<Fill>, opened: Tally, closed: Tally, netCash: number, flags: Array<string>, grade: string, thesis: string, tags: Array<string>, };

export type OpenLot = { opened: string, qty: number, price: number, basis: number, held: number, flags: Array<string>, activityId: string, };

export type Position = { 
/**
 * The round trip that opened it, which is the id the trade will have when
 * it closes, so the two share a journal entry.
 */
id: string, symbol: string, underlying: string, name: string, exchange: string, kind: Kind, account: string, accountId: string, currency: string, securityId: string, short: boolean, qty: number, mult: number, avg: number, cost: number, fees: number, last: number, priceSource: Mark, priceChange: number | null, percentChange: number | null, 
/**
 * The day's move on the whole position, in its own currency.
 */
dayChange: number | null, mv: number, unreal: number, unrealPct: number | null, 
/**
 * Days held, weighted by quantity.
 */
held: number, opened: string, 
/**
 * What Wealthsimple says the account holds, where it says.
 */
wsQty: number | null, rt: string | null, lots: Array<OpenLot>, 
/**
 * Sent only for the holding whose page is open.
 */
fills?: Array<Fill>, grade: string, thesis: string, tags: Array<string>, 
/**
 * Its share of the book's cost.
 */
alloc: number, };

export type TradeDetail = { id: string, legs: Array<Leg>, fills: Array<Fill>, };

export type Kpi = { realized: number, count: number, wins: number, losses: number, breakeven: number, winRate: number | null, grossWin: number, grossLoss: number, 
/**
 * Nothing when there are wins and no losses: see `profit_factor_infinite`.
 */
profitFactor: number | null, profitFactorInfinite: boolean, expectancy: number | null, avgWin: number, avgLoss: number, fees: number, avgHold: number | null, openCount: number, };

export type BySymbolRow = { 
/**
 * The underlying, so a chain of contracts sits under the name it is written on.
 */
symbol: string, pnl: number, n: number, legs: number, winRate: number, avgHold: number, tradeIds: Array<string>, };

export type MonthlyBar = { 
/**
 * `YYYY-MM`.
 */
key: string, label: string, value: number, count: number, tradeIds: Array<string>, };

export type GradeBucket = { grade: string, n: number, pnl: number, tradeIds: Array<string>, };

export type Grades = { buckets: Array<GradeBucket>, ungraded: number, graded: number, };

export type QueueRow = { id: string, symbol: string, date: string, pnl: number, currency: string, missing: string, };

export type Point = { d: string, v: number, 
/**
 * Net deposits to date; nothing when the record does not carry them.
 */
dep: number | null, };

export type YearRow = { year: string, r: number, days: number, from: string, to: string, 
/**
 * Net deposits over the year; nothing when the record does not carry them.
 */
flow: number | null, endV: number | null, spR: number | null, };

export type Annualized = { rate: number | null, years: number, count: number, first: string, last: string, };

export type Drawdown = { pct: number | null, abs: number | null, at: string, peakAt: string, };

export type EquityBlock = { label: string, series: Array<Point>, drawdown: Drawdown, annualized: Annualized, };

export type BenchmarkRef = { key: string, label: string, };

export type Allocation = { id: string, symbol: string, account: string, value: number, share: number, };

export type ExposureSlice = { name: string, value: number, share: number, };

export type Portfolio = { allocation: Array<Allocation>, sectors: Array<ExposureSlice>, regions: Array<ExposureSlice>, marketValue: number, costBasis: number, unrealized: number, unrealizedPct: number | null, positionCount: number, accountCount: number, nav: number | null, navAccounts: number, marginUsed: number, 
/**
 * By currency, to the cent.
 */
marginUsedBy: Record<string, number>, marginUsedPct: number | null, availableMargin: number | null, 
/**
 * The margin accounts whose buying power Wealthsimple did not give.
 */
availableMarginUnavailable: Array<string>, hasMargin: boolean, cash: number, cashPct: number | null, dayChange: number | null, dayChangePct: number | null, };

export type PositionsSummary = { count: number, book: number, mv: number, unreal: number, };

export type Account = { id: string, name: string, type: string, currency: string, status: string, nav: number | null, };

export type CashflowRow = { id: string, date: string, time: string, 
/**
 * The ticker; `Cash` for interest; `—` when the row names nothing.
 */
symbol: string, name: string, kind: Payment, account: string, accountId: string, 
/**
 * Shares paid on; nothing when the row does not say.
 */
qty: number | null, 
/**
 * Paid per share; nothing when the row does not say.
 */
per: number | null, amount: number, currency: string, amountCad: number, };

export type CashflowTile = { label: string, total: number, perMonth: number, count: number, } | { label: string, marginUsed: number, interestPerMonth: number, interestMonths: number, } | { label: string, yield: number | null, projected: number, earned: number, book: number, };

export type CashflowMonth = { key: string, label: string, value: number, count: number, };

export type CashflowHolding = { id: string, symbol: string, account: string, qty: number, per: number | null, 
/**
 * Payments a year, read from the record, never assumed.
 */
freq: number | null, freqVerified: boolean, rateSource: RateSource, cost: number, avg: number, last: number, priceSource: Priced, ytd: number, ttm: number, all: number, nextExDate: string, nextPayDate: string, exPast: boolean, payPast: boolean, 
/**
 * Projected income a payment.
 */
yob: number | null, annual: number | null, yoc: number | null, currentYield: number | null, };

export type Cashflow = { tiles: Array<CashflowTile>, months: Array<CashflowMonth>, holdings: Array<CashflowHolding>, rows: Array<CashflowRow>, other: Array<CashflowRow>, total: number, count: number, 
/**
 * The filters in force that the cashflow does not read.
 */
skippedFilters: Array<string>, interest: number, withholding: number, };

export type HeldTile = { id: string, symbol: string, exchange: string, value: number, percentChange: number | null, sector: string, };

export type UniverseTile = { 
/**
 * A universe's tile is no holding: always nothing.
 */
id: string | null, symbol: string, name: string, value: number, percentChange: number | null, sector: string, country: string, };

export type WatchItem = { symbol: string, exchange: string, name: string, currency: string, last: number | null, priceChange: number | null, percentChange: number | null, sector: string, kind: string, positionId: string | null, };

export type NewsTag = { symbol: string, exchange: string, held: boolean, watched: boolean, percentChange: number | null, positionId: string | null, };

export type NewsItem = { id: string, headline: string, source: string, url: string, publishedAt: string, 
/**
 * From the market's own feed rather than a listing's.
 */
market: boolean, tags: Array<NewsTag>, 
/**
 * `story`, or `release` for a company's own.
 */
kind: string, };

export type MarketTile = { symbol: string, exchange: string, label: string, name: string, kind: string, last: number | null, change: number | null, percentChange: number | null, decimals: number, 
/**
 * A contract quoted as 100 minus a rate carries that rate beside its price, and
 * the rate's move with it: both keys, or neither (`implied`).
 */
rate?: number, 
/**
 * Present with `rate`; nothing inside when the price has not moved.
 */
rateChange?: number | null, };

export type MarketInstrument = { symbol: string, label: string, name: string, exchange: string, kind: string, aliases: Array<string>, };

export type Markets = { holdings: Array<HeldTile>, watchlist: Array<WatchItem>, news: Array<NewsItem>, universes: Record<string, Array<UniverseTile>>, tiles: Array<MarketTile>, instruments: Array<MarketInstrument>, };

export type ListingInfo = { name: string, exchange: string, kind: Kind, currency: string, };

export type Options = { accounts: Array<string>, symbols: Array<string>, listings: Record<string, ListingInfo>, tags: Array<string>, exchanges: Array<string>, kinds: Array<Kind>, grades: Array<string>, sides: [string, string], results: [string, string, string], years: Array<string>, };

export type MarketDates = { fxLast: string, benchmarkLast: string, };

export type Unmatched = { symbol: string, currency: string, side: Side, quantity: number, price: number, date: string, description: string, accountId: string, account: string, activityId: string, };

export type View = { ok: boolean, today: string, syncedAt: string, currency: string, market: MarketDates, filters: Filters, options: Options, kpi: Kpi, equity: EquityBlock, years: Array<YearRow>, benchmark: BenchmarkRef, monthly: Array<MonthlyBar>, bySymbol: Array<BySymbolRow>, grades: Grades, queue: Array<QueueRow>, trades: Array<Trade>, tradeCount: number, tradeTotal: number, positions: Array<Position>, positionsSummary: PositionsSummary, portfolio: Portfolio, markets: Markets, cashflow: Cashflow, unmatched: Array<Unmatched>, accounts: Array<Account>, activityCount: number, };
