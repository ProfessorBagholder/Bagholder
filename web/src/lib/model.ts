// The page's types.
//
// What the server sends as the model is generated from the Rust types that make it
// (`generated/wire.ts`, from rust/crates/model): a field renamed, added or made
// nullable there fails the type check here. This file gives those types the names
// the page uses, and holds by hand only what has no Rust type yet (the header's
// status, and the documents read on demand: short interest, filings, fear and greed).

import type * as wire from './generated/wire'
import type { Status } from './generated/status'

export type {
  Kind, Kpi, Annualized, Drawdown, YearRow, GradeBucket, Grades, BySymbolRow, QueueRow, MonthlyBar,
  CashflowTile, CashflowMonth, CashflowRow, Cashflow, Position, Portfolio, Allocation, ExposureSlice,
  Fill, Leg, MarketTile, MarketInstrument, WatchItem, NewsTag, NewsItem, Markets, Account, Options,
  Filters, Unmatched, OpenLot, TradeDetail,
} from './generated/wire'

export type {
  Regulator, FiledDocument, Filing, SourceStatus, FilingsDoc, FilingsPayload, FeedFiling, FilingsFeed, Enriched,
} from './generated/filings'

export type { Status, NotifyStatus, NotifySettings } from './generated/status'

export type {
  ShortMarket, VolumeSpan, ShortPoint, Shorts, StoredShorts, ShortsPayload, ShortsFeedRow, ShortsFeed,
} from './generated/markets'

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

/** The model as the stream delivers it: the view, with the header's status beside it. */
export type Model = wire.View & { status: Status }

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

