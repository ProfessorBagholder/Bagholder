// The page's types.
//
// What the server sends as the model is generated from the Rust types that make it
// (`generated/wire.ts`, from rust/crates/model): a field renamed, added or made
// nullable there fails the type check here. This file gives those types the names
// the page uses, and holds by hand only what has no Rust type yet (the header's
// status, and the documents read on demand: short interest, filings, fear and greed).

import type * as wire from './generated/wire'

export type {
  Kind, Kpi, Annualized, Drawdown, YearRow, GradeBucket, Grades, BySymbolRow, QueueRow, MonthlyBar,
  CashflowTile, CashflowMonth, CashflowRow, Cashflow, Position, Portfolio, Allocation, ExposureSlice,
  Fill, Leg, MarketTile, MarketInstrument, WatchItem, NewsTag, NewsItem, Markets, Account, Options,
  Filters, Unmatched, OpenLot, TradeDetail,
} from './generated/wire'

export type {
  Regulator, FiledDocument, Filing, SourceStatus, FilingsDoc, FilingsPayload, FeedFiling, FilingsFeed, Enriched,
} from './generated/filings'

export type EquityPoint = wire.Point
export type Benchmark = wire.BenchmarkRef
export type Listing = wire.ListingInfo
export type Slice = wire.ExposureSlice

/** A tile of a heatmap: one of the book's holdings, or a constituent of a market universe. */
export type HeatHolding = wire.HeldTile | wire.UniverseTile

/** A paying holding, with the market value the page joins on from the holding of the same id. */
export type CashflowHolding = wire.CashflowHolding & { mv?: number | null }

/**
 * A trade as the page shows it. A holding or a market listing stands in for one on
 * the detail page, carrying the fields a trade does not have.
 */
export type Trade = wire.Trade & {
  holding?: boolean
  listing?: boolean
  avg?: number
  mv?: number
  cost?: number
  last?: number | null
  percentChange?: number | null
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
  updateError: string
  /** the listings the server's news pass has still to read (`*` is the market's feed) */
  newsReading: string[]
  protocol: string
  notify: Notify
  [k: string]: unknown
}

/** The model as the stream delivers it: the view, with the header's status beside it. */
export type Model = wire.View & { status: Status }

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

