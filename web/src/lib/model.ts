// The page's types.
//
// The figures the server sends are generated from the Rust types that make them
// (`generated/figures.ts`, from rust/crates/server/src/wire): money, quantities and
// prices are exact decimal text (`Dec`, `./dec`), a figure that may wait is a `Fig`,
// a ratio is a number. This file gives those types the names the page uses, and
// holds by hand only what has no Rust type yet.

import type * as figures from './generated/figures'
import type * as wire from './generated/wire'
import type { Dec, Fig } from './dec'

export type {
  Kpi, Annualized, Drawdown, YearRow, GradeBucket, Grades, BySymbolRow, QueueRow, MonthlyBar, Partial,
  CashflowTile, CashflowMonth, CashflowRow, CashflowHolding, Cashflow, Position, Portfolio, Slice,
  Fill, Detail, Account, Options, AccountOption, InstrumentOption, Point as EquityPoint, BenchmarkRef as Benchmark, Equity,
  BookDoc, DashboardDoc, PositionsDoc, TradesDoc, CashflowDoc, TradeDoc, ExposureDoc, MarketsDoc, Waiting,
  MarketTile, WatchItem, DirectoryEntry, HeatTile, HeatBlock, HeatCounts, HeatmapDoc, NewsTag, Filed, Headline, ChipKinds, HeadlinesDoc,
} from './generated/figures'

export type { Kind } from './generated/wire'

export type {
  Regulator, FiledDocument, Filing, SourceStatus, FilingsDoc, FilingsPayload, FeedFiling, FilingsFeed, Enriched,
} from './generated/filings'

export type { Status, NotifyStatus, NotifySettings } from './generated/status'

export type {
  ShortMarket, VolumeSpan, ShortReport, ShortsView, ShortsPayload, ShortsFeedRow, ShortsFeed,
} from './generated/markets'

export type { Dec, Fig }

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

/**
 * A match from /api/symbols/search (watchlist add row, news/shorts lookup), with the
 * price a watchlist row already carries when the add row shows it beside a holding.
 */
export type SymbolMatch = wire.SymbolMatch & {
  last?: Fig<Dec> | null
  /** The day's change, as a fraction. */
  percentChange?: number | null
}
