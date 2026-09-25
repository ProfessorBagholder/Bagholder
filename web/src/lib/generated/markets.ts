// Generated from rust/crates/store/src/feeds.rs and the server's market documents. Do not
// edit: change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_market_types`.

import type { OkOr } from './common'
import type { Fill } from './figures'
import type { MarketTile, SymbolMatch } from './wire'

export type GaugeReading = { label: string, score: number, rating: string, };

export type GaugePart = { name: string, score: number, rating: string, };

export type GaugePoint = { date: string, score: number, };

export type Gauge = { index: string, source: string, score: number, rating: string, asOf: string, previous: Array<GaugeReading>, parts: Array<GaugePart>, series: Array<GaugePoint>, };

export type StoredGauge = { fetchedAt: string, readVersion: number, index: string, source: string, score: number, rating: string, asOf: string, previous: Array<GaugeReading>, parts: Array<GaugePart>, series: Array<GaugePoint>, };

export type FearDoc = { ok: true, gauge: StoredGauge | null, };

export type ShortMarket = "us" | "ca";

export type VolumeSpan = "day" | "period";

export type ShortPoint = { date: string, shares: number, };

export type Shorts = { symbol: string, exchange: string, market: ShortMarket, name: string, asOf: string, shares: number | null, previous: number | null, previousOf: string, change: number | null, float: number | null, ofFloat: number | null, averageVolume: number | null, daysToCover: number | null, volumeOf: string, volumeSpan: VolumeSpan | null, shortVolume: number | null, totalVolume: number | null, volumePct: number | null, 
/**
 * The reports behind the position, oldest first; `None` where they were
 * not read.
 */
series: Array<ShortPoint> | null, };

export type StoredShorts = { fetchedAt: string, readVersion: number, symbol: string, exchange: string, market: ShortMarket, name: string, asOf: string, shares: number | null, previous: number | null, previousOf: string, change: number | null, float: number | null, ofFloat: number | null, averageVolume: number | null, daysToCover: number | null, volumeOf: string, volumeSpan: VolumeSpan | null, shortVolume: number | null, totalVolume: number | null, volumePct: number | null, 
/**
 * The reports behind the position, oldest first; `None` where they were
 * not read.
 */
series: Array<ShortPoint> | null, };

export type ShortsPayload = { ok: true, covered: boolean, shorts: StoredShorts | null, };

export type ShortsFeedRow = { positionId: string | null, held: boolean, watched: boolean, fetchedAt: string, readVersion: number, symbol: string, exchange: string, market: ShortMarket, name: string, asOf: string, shares: number | null, previous: number | null, previousOf: string, change: number | null, float: number | null, ofFloat: number | null, averageVolume: number | null, daysToCover: number | null, volumeOf: string, volumeSpan: VolumeSpan | null, shortVolume: number | null, totalVolume: number | null, volumePct: number | null, 
/**
 * The reports behind the position, oldest first; `None` where they were
 * not read.
 */
series: Array<ShortPoint> | null, };

export type ShortsFeed = { ok: true, rows: Array<ShortsFeedRow>, reading: boolean, };

export type FearAnswer = FearDoc | OkOr;

export type ShortsAnswer = ShortsPayload | OkOr;

export type ListingAnswer = { ok: false, error: string, } | { ok: true, symbol: string, positionId: string, } | { ok: true, symbol: string, exchange: string, currency: string, kind: string, name: string, securityId: string, fills: Array<Fill>, price: number | null, percentChange: number | null, };

export type NewsSymbolAnswer = { ok: false, error: string, } | { ok: true, count: number, source: string, exchange: string, };

export type WatchlistAnswer = { ok: true, watchlist: Array<WatchedListing>, } | { ok: false, error: string, };

export type TilesAnswer = { ok: true, tiles: Array<MarketTile>, } | { ok: false, error: string, };

export type Listing = { symbol: string, exchange: string, currency: string, name: string, };

export type Fear = { index: string | null, };

export type ShortsQuery = { 
/**
 * carry the series too, for the listing's own page
 */
trend: boolean, symbol: string, exchange: string, currency: string, name: string, };

export type GlanceAnswer = { ok: boolean, price: number | null, priceChange: number | null, percentChange: number | null, };

export type Search = { q: string, };

export type SymbolSearchAnswer = { ok: true, matches: Array<SymbolMatch>, } | { ok: false, error: string, matches: Array<SymbolMatch>, };

export type WatchlistBody = { symbol: string, exchange: string, name?: string, currency?: string, securityId?: string, };

export type TilesSet = { tiles: Array<TileRef>, };

export type WatchedListing = { symbol: string, exchange: string, name: string, currency: string, securityId: string, addedAt: string, };

export type TileRef = { symbol: string, exchange: string, };
