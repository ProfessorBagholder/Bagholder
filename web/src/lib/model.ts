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

export interface Model {
  ok: boolean
  currency: string
  kpi: Kpi
  equity: { label: string; series: EquityPoint[]; annualized: number | null; drawdown: number | null }
  monthly: MonthlyBar[]
}
