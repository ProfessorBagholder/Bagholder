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

export interface Model {
  ok: boolean
  currency: string
  kpi: Kpi
  equity: { label: string; series: EquityPoint[]; annualized: number | null; drawdown: number | null }
  monthly: MonthlyBar[]
  cashflow: Cashflow
}
