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
  winRate: number
  profitFactor: number
  profitFactorInfinite: boolean
  expectancy: number
  avgWin: number
  avgLoss: number
  avgHold: number
  openCount: number
}

export interface MonthlyBar {
  key: string
  label: string
  value: number
  count: number
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
  annual: number
  yoc: number
  currentYield: number
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
}

// --- Portfolio tab ---
export interface Position {
  id: string
  symbol: string
  name: string
  exchange: string
  kind: string
  account: string
  currency: string
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
  nav: number
  marginUsed: number
  marginUsedPct: number
  availableMargin: number | null
  availableMarginUnavailable: string | null
  hasMargin: boolean
  cash: number
  cashPct: number
  dayChange: number | null
  dayChangePct: number | null
}

// --- Trades tab ---
export interface Trade {
  id: string
  status: string
  symbol: string
  name: string
  account: string
  exchange: string
  kind: string
  currency: string
  qty: number
  entry: number
  exit: number
  entryDate: string
  exitDate: string
  holdDays: number
  pnl: number
  pnlPct: number
  grade: string | null
  tags: string[]
  flags: string[]
}

export interface Model {
  ok: boolean
  currency: string
  kpi: Kpi
  equity: { label: string; series: EquityPoint[]; annualized: number | null; drawdown: number | null }
  monthly: MonthlyBar[]
  cashflow: Cashflow
  positions: Position[]
  portfolio: Portfolio
  trades: Trade[]
}
