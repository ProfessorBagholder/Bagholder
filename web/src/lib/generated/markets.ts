// Generated from rust/crates/store/src/feeds.rs and the server's market documents. Do not
// edit: change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_market_types`.

import type { Dec } from '../dec'
import type { OkOr } from './common'
import type { Fill } from './figures'
import type { SymbolMatch } from './wire'

export type GaugeReading = { label: string, score: number, rating: string, };

export type GaugePart = { name: string, score: number, rating: string, };

export type GaugePoint = { date: string, score: number, };

export type Gauge = { index: string, source: string, score: number, rating: string, asOf: string, previous: Array<GaugeReading>, parts: Array<GaugePart>, series: Array<GaugePoint>, };

export type StoredGauge = { fetchedAt: string, readVersion: number, index: string, source: string, score: number, rating: string, asOf: string, previous: Array<GaugeReading>, parts: Array<GaugePart>, series: Array<GaugePoint>, };

export type FearDoc = { ok: true, gauge: StoredGauge | null, 
/**
 * A read of the publisher is in the air.
 */
reading: boolean, };

export type UniverseDoc = { 
/**
 * The source's failure, from its last read until it next answers.
 */
failed: string | null, };

export type NewsDoc = { reading: Array<string>, };

export type ShortMarket = "us" | "ca";

export type VolumeSpan = "day" | "period";

export type ShortReport = { date: string, shares: Dec, };

export type ShortsView = { symbol: string, exchange: string, market: ShortMarket, name: string, asOf: string, 
/**
 * The position sold short.
 */
shares: Dec | null, previous: Dec | null, previousOf: string, change: Dec | null, float: Dec | null, 
/**
 * The float not sold short: the float less the position.
 */
unshorted: Dec | null, 
/**
 * The position over the float, as a fraction.
 */
ofFloat: number | null, averageVolume: Dec | null, daysToCover: number | null, volumeOf: string, volumeSpan: VolumeSpan | null, shortVolume: Dec | null, totalVolume: Dec | null, 
/**
 * The shares traded that were not sold short.
 */
longVolume: Dec | null, 
/**
 * The short volume over the total, as a fraction.
 */
ofVolume: number | null, 
/**
 * The reports behind the position, oldest first; `None` where they were not read.
 */
series: Array<ShortReport> | null, fetchedAt: string, };

export type ShortsPayload = { ok: true, covered: boolean, shorts: ShortsView | null, };

export type ShortsFeedRow = { positionId: string | null, held: boolean, watched: boolean, symbol: string, exchange: string, market: ShortMarket, name: string, asOf: string, 
/**
 * The position sold short.
 */
shares: Dec | null, previous: Dec | null, previousOf: string, change: Dec | null, float: Dec | null, 
/**
 * The float not sold short: the float less the position.
 */
unshorted: Dec | null, 
/**
 * The position over the float, as a fraction.
 */
ofFloat: number | null, averageVolume: Dec | null, daysToCover: number | null, volumeOf: string, volumeSpan: VolumeSpan | null, shortVolume: Dec | null, totalVolume: Dec | null, 
/**
 * The shares traded that were not sold short.
 */
longVolume: Dec | null, 
/**
 * The short volume over the total, as a fraction.
 */
ofVolume: number | null, 
/**
 * The reports behind the position, oldest first; `None` where they were not read.
 */
series: Array<ShortReport> | null, fetchedAt: string, };

export type ShortsFeed = { ok: true, rows: Array<ShortsFeedRow>, reading: boolean, };

export type FearAnswer = FearDoc | OkOr;

export type ShortsAnswer = ShortsPayload | OkOr;

export type ListingAnswer = { ok: false, error: string, } | { ok: true, symbol: string, positionId: string, } | { ok: true, symbol: string, exchange: string, currency: string, kind: string, name: string, securityId: string, fills: Array<Fill>, 
/**
 * Its price for a glance, where its source gave one.
 */
price: Dec | null, 
/**
 * The day's change, as a fraction.
 */
percentChange: number | null, 
/**
 * Why there is no price, when its source did not answer.
 */
priceFailed: string | null, };

export type NewsSymbolAnswer = { ok: false, error: string, } | { ok: true, count: number, source: string, exchange: string, };

export type WatchlistAnswer = { ok: true, 
/**
 * The instrument followed or dropped.
 */
id: string, } | { ok: false, error: string, };

export type TilesAnswer = { ok: true, } | { ok: false, error: string, };

export type Listing = { symbol: string, exchange: string, currency: string, name: string, };

export type Fear = { index: string | null, };

export type ShortsQuery = { 
/**
 * carry the series too, for the listing's own page
 */
trend: boolean, symbol: string, exchange: string, currency: string, name: string, };

export type GlanceAnswer = { ok: true, price: Dec, 
/**
 * The day's change in points, where the source states it.
 */
change: Dec | null, 
/**
 * The day's change, as a fraction.
 */
percentChange: number | null, } | { ok: false, error: string, };

export type Search = { q: string, };

export type SymbolSearchAnswer = { ok: true, matches: Array<SymbolMatch>, } | { ok: false, error: string, matches: Array<SymbolMatch>, };

export type WatchlistBody = { id?: string, symbol: string, exchange: string, name?: string, currency?: string, securityId?: string, };

export type WatchlistRemove = { id: string, };

export type TilesSet = { tiles: Array<TileRef>, };

export type TileRef = { symbol: string, exchange: string, };
