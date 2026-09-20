// A hand-written subset of the /api/model shape, for the Dashboard spike only.
// In the real migration these types are generated from the Rust structs via
// ts-rs so the client/server seam is compile-checked — see docs/frontend-backend-migration.md.

export interface EquityPoint {
  d: string // YYYY-MM-DD
  v: number // equity value
  dep: number // cumulative net deposits
}

export interface Kpi {
  realized: number
  count: number
  wins: number
  losses: number
  breakeven: number
  winRate: number | null
  profitFactor: number | null
  profitFactorInfinite: boolean
  expectancy: number | null
  avgWin: number
  avgLoss: number
  avgHold: number
  grossWin: number
  grossLoss: number
  fees: number
  openCount: number
}

// equity.annualized and equity.drawdown are objects, not scalars.
export interface Annualized {
  rate: number | null
  years: number
  count: number
  first: string
  last: string
}
export interface Drawdown {
  pct: number | null
  abs: number
  at: string
  peakAt: string
}
export interface YearRow {
  year: string
  r: number
  spR: number | null
  days: number
  from: string
  to: string
  flow: number
  endV: number
}
export interface GradeBucket {
  grade: string
  n: number
  pnl: number
  tradeIds: string[]
}
export interface Grades {
  buckets: GradeBucket[]
  ungraded: number
  graded: number
}
export interface BySymbolRow {
  symbol: string
  pnl: number
  n: number
  legs: number
  winRate: number
  avgHold: number
  tradeIds: string[]
}
export interface QueueRow {
  id: string
  symbol: string
  date: string
  pnl: number
  currency: string
  missing: string
}
export interface Benchmark {
  key: string
  label: string
}
export interface Notify {
  unread: number
  [k: string]: unknown
}
export interface Status {
  ok: boolean
  connected: boolean
  email: string
  version: string
  lastSync: string
  syncing: boolean
  syncStep: string
  error: string
  openOrders: number
  updateAvailable: boolean
  canUpdate: boolean
  latestVersion: string
  updateUrl: string
  updateBy: string
  updating: string
  protocol: string
  notify: Notify
  [k: string]: unknown
}

export interface MonthlyBar {
  key: string
  label: string
  value: number
  count: number
  tradeIds: string[]
}

// --- Cashflow tab ---
export interface CashflowTile {
  label: string
  total?: number
  perMonth?: number
  count?: number
  marginUsed?: number
  interestPerMonth?: number
  interestMonths?: number
  yield?: number
  projected?: number
  earned?: number
  book?: number
}

export interface CashflowMonth {
  key: string
  label: string
  value: number
  count: number
}

export interface CashflowHolding {
  id: string
  symbol: string
  account: string
  qty: number
  per: number | null
  freq: number
  freqVerified: boolean
  rateSource?: string
  priceSource?: string
  cost: number
  avg: number
  last: number
  ytd: number
  ttm: number
  all: number
  nextExDate: string
  nextPayDate: string
  exPast: boolean
  payPast: boolean
  yob: number // projected monthly income
  annual: number | null
  yoc: number | null
  currentYield: number | null
  // merged in from model.positions[].mv by matching id (not sent on the holding)
  mv?: number | null
}

export interface CashflowRow {
  id: string
  date: string
  time: string
  symbol: string
  name: string
  kind: string
  account: string
  qty: number | null
  per: number | null
  amount: number
  currency: string
  amountCad: number
}

export interface Cashflow {
  tiles: CashflowTile[]
  months: CashflowMonth[]
  holdings: CashflowHolding[]
  rows: CashflowRow[]
  other: CashflowRow[]
  total: number
  count: number
  skippedFilters?: string[]
  interest?: number
  withholding?: number
}

// --- Portfolio tab ---
export interface Position {
  id: string
  symbol: string
  name: string
  exchange: string
  kind: string
  account: string
  accountId: string
  currency: string
  securityId: string
  short: boolean
  qty: number
  avg: number
  last: number
  cost: number
  mv: number
  priceChange: number | null
  percentChange: number | null
  dayChange: number | null
  unreal: number
  unrealPct: number
  priceSource: string
}

export interface Slice {
  label?: string
  name?: string
  symbol?: string
  value: number
  share: number
}

export interface Portfolio {
  allocation: { id: string; symbol: string; account: string; value: number; share: number }[]
  sectors: { name: string; value: number; share: number }[]
  regions: { name: string; value: number; share: number }[]
  marketValue: number
  costBasis: number
  unrealized: number
  unrealizedPct: number
  positionCount: number
  accountCount: number
  nav: number | null
  navAccounts: number
  marginUsed: number
  marginUsedPct: number | null
  availableMargin: number | null
  availableMarginUnavailable: string[] | null
  hasMargin: boolean
  cash: number
  cashPct: number | null
  dayChange: number | null
  dayChangePct: number | null
}

// --- Trades tab ---
// One execution on the trade detail (present only when the model was fetched with
// &trade=<id>): the fills table and the chart markers read these.
export interface Fill {
  id: string
  when: string
  date: string
  time: string
  side: string
  sub: string
  qty: number
  price: number | null
  amount: number
  fees: number
  currency: string
  flags: string[]
}
export interface Leg {
  key: string
  qty: number
  entry: number
  exit: number
  entryDate: string
  exitDate: string
  pnl: number
  pnlCad: number
  fees: number
  buyActivityId: string
  sellActivityId: string
  flags: string[]
}

export interface Trade {
  id: string
  status: string
  symbol: string
  name: string
  account: string
  exchange: string
  kind: string
  currency: string
  securityId?: string
  side: string
  qty: number
  entry: number
  exit: number
  entryDate: string
  exitDate: string
  holdDays: number
  pnl: number
  pnlCad: number
  pnlPct: number
  fees: number
  grade: string
  thesis: string
  tags: string[]
  flags: string[]
  underlying?: string
  // present only when the model carries this trade's detail (&trade=<id>)
  fills?: Fill[]
  legs?: Leg[]
  // synthetic fields when a holding or a market listing stands in for a trade
  holding?: boolean
  listing?: boolean
  avg?: number
  mv?: number
  cost?: number
  last?: number | null
  percentChange?: number | null
}

// /api/shorts payload: { ok, covered, shorts }
export interface Shorts {
  asOf?: string
  shares?: number | null
  previous?: number | null
  previousOf?: string
  change?: number | null
  float?: number | null
  ofFloat?: number | null
  averageVolume?: number | null
  daysToCover?: number | null
  volumeOf?: string
  shortVolume?: number | null
  totalVolume?: number | null
  volumePct?: number | null
  series?: { date: string; shares: number }[]
}
export interface ShortsResp {
  ok: boolean
  covered?: boolean
  shorts?: Shorts
}

export interface Filing {
  id: string
  source: string
  category: string
  type: string
  title: string
  subject: string
  summary: string
  summaryStatus?: string
  /** nothing a further reading would add: the row is named as it will stay */
  enrichFinal?: boolean
  date: string
  dateText: string
  size: string
  url: string
}
export interface FilingsSource {
  available?: boolean
  filer?: boolean
  matched?: boolean
  // set when the source was tried and could not be reached (an outage, a maintenance page)
  error?: string
}
export interface FilingsPayload {
  ok: boolean
  filings?: Filing[]
  sources?: Record<string, FilingsSource>
  fetchedAt?: string
  /** false until the sources have been asked once for this listing */
  everRead?: boolean
  /** the documents the server's pass has still to read; the first is being read now */
  reading?: string[]
  /** the local model that writes the sentences: ready, starting, downloading, off */
  summaryStatus?: string
  error?: string
}

// --- Markets tab ---
export interface MarketTile {
  symbol: string
  label: string
  name: string
  exchange: string
  kind: string
  last: number | null
  change: number | null
  percentChange: number | null
  decimals: number
  rateOf?: number | null
  // a contract quoted as 100 minus a rate carries the implied rate and its move
  rate?: number | null
  rateChange?: number | null
}

// An entry in the market-instruments directory the tile picker searches.
export interface MarketInstrument {
  symbol: string
  label: string
  name: string
  exchange: string
  kind: string
  aliases?: string[]
}

export interface HeatHolding {
  id: string | null
  symbol: string
  exchange?: string
  value: number
  percentChange: number | null
  sector: string
  // universe tiles (ca/us/intl) also carry a name and a country
  name?: string
  country?: string
  currency?: string
}

export interface WatchItem {
  symbol: string
  exchange: string
  name: string
  currency: string
  last: number | null
  priceChange: number | null
  percentChange: number | null
  sector: string
  positionId?: string | null
  kind?: string
}

export interface NewsTag {
  symbol: string
  exchange: string
  held: boolean
  watched: boolean
  percentChange: number | null
  positionId: string | null
}
export interface NewsItem {
  id: string
  headline: string
  source: string
  url: string
  publishedAt: string
  market: boolean
  tags: NewsTag[]
  kind: string
}

export interface FearReading { label: string; score: number; rating: string }
export interface FearPart { name: string; score: number; rating: string }
export interface FearGauge {
  index: string
  source: string
  score: number
  rating: string
  asOf: string
  previous: FearReading[]
  parts: FearPart[]
  series: { date: string; score: number }[]
}

// A ranked short-interest row from /api/shorts/feed (and a single /api/shorts hit).
export interface ShortsFeedRow {
  symbol: string
  exchange?: string
  name?: string
  held?: boolean
  watched?: boolean
  positionId?: string | null
  shares: number | null
  ofFloat: number | null
  daysToCover: number | null
  volumePct: number | null
  asOf: string
}

// A match from /api/symbols/search (watchlist add row, news/shorts lookup).
export interface SymbolMatch {
  symbol: string
  exchange: string
  name: string
  currency: string
  kind?: string
  last?: number | null
  percentChange?: number | null
}

export interface Markets {
  holdings: HeatHolding[]
  watchlist: WatchItem[]
  universes: { ca?: HeatHolding[]; us?: HeatHolding[]; intl?: HeatHolding[] }
  tiles: MarketTile[]
  instruments: MarketInstrument[]
  news: NewsItem[]
}

export interface Account {
  id: string
  name: string
  type: string
  currency: string
  status: string
  nav: number | null
}

// One entry per symbol in options.symbols: the listing's name/exchange/kind/ccy,
// used by the filter popover's symbol picker (name, exchange, option detection).
export interface Listing {
  name?: string
  exchange?: string
  kind?: string
  currency?: string
}

export interface Options {
  accounts: string[]
  symbols: string[]
  listings: Record<string, Listing>
  tags: string[]
  exchanges: string[]
  kinds: string[]
  grades: string[]
  sides: string[]
  results: string[]
  years: string[]
}

export interface Model {
  ok: boolean
  currency: string
  options: Options
  kpi: Kpi
  equity: { label: string; series: EquityPoint[]; annualized: Annualized; drawdown: Drawdown }
  years: YearRow[]
  monthly: MonthlyBar[]
  grades: Grades
  bySymbol: BySymbolRow[]
  queue: QueueRow[]
  benchmark: Benchmark
  cashflow: Cashflow
  positions: Position[]
  portfolio: Portfolio
  trades: Trade[]
  markets: Markets
  accounts: Account[]
  status: Status
}
