// The page's types.
//
// The figures the server sends are generated from the Rust types that make them
// (`generated/figures.ts`, from rust/crates/server/src/wire): money, quantities and
// prices are exact decimal text (`Dec`, `./dec`), a figure that may wait is a `Fig`.
// The market around the book keeps its earlier types (`generated/wire.ts`) until its
// readers move (stage 5). This file gives those types the names the page uses, and
// holds by hand only what has no Rust type yet.

import type * as figures from './generated/figures'
import type * as wire from './generated/wire'
import type { Status } from './generated/status'
import type { Dec, Fig } from './dec'

export type {
  Kpi, Annualized, Drawdown, YearRow, GradeBucket, Grades, BySymbolRow, QueueRow, MonthlyBar, Partial,
  CashflowTile, CashflowMonth, CashflowRow, CashflowHolding, Cashflow, Position, Portfolio, Slice,
  Fill, Detail, Account, Options, AccountOption, InstrumentOption, Point as EquityPoint, BenchmarkRef as Benchmark, Equity,
} from './generated/figures'

export type { Kind, MarketTile, MarketInstrument, WatchItem, NewsTag, NewsItem, Markets, ExposureSlice } from './generated/wire'

export type {
  Regulator, FiledDocument, Filing, SourceStatus, FilingsDoc, FilingsPayload, FeedFiling, FilingsFeed, Enriched,
} from './generated/filings'

export type { Status, NotifyStatus, NotifySettings } from './generated/status'

export type {
  ShortMarket, VolumeSpan, ShortPoint, Shorts, StoredShorts, ShortsPayload, ShortsFeedRow, ShortsFeed,
} from './generated/markets'

export type { Dec, Fig }

/** A tile of a heatmap: one of the book's holdings, or a constituent of a market universe. */
export type HeatHolding = wire.HeldTile | wire.UniverseTile

/**
 * A trade as the page shows it. A holding or a market listing stands in for one on
 * the detail page, carrying the fields a trade does not have.
 */
export type Trade = figures.Trade & {
  holding?: boolean
  listing?: boolean
  /** The trade whose journal a holding shows: the round trip that opened it. */
  journal?: string
  /** A listing's own: the fills of the trades once closed on it. */
  fills?: figures.Fill[]
  avg?: Fig<Dec>
  mv?: Fig<Dec>
  cost?: Fig<Dec>
  last?: Fig<Dec> | null
  percentChange?: number | null
  held?: Fig<number>
}

/** The model as the stream delivers it: the figures, with the header's status beside them. */
export type Model = figures.Figures & { status: Status }

/**
 * A match from /api/symbols/search (watchlist add row, news/shorts lookup), with the
 * price a watchlist row already carries when the add row shows it beside a holding.
 */
export type SymbolMatch = wire.SymbolMatch & {
  last?: Fig<Dec> | number | null
  percentChange?: number | null
}
