// Generated from rust/crates/market/src/stored/feeds.rs and the server's market documents. Do not
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
failed: string | null, 
/**
 * The source is being read this moment: what a heatmap with no rows yet says.
 */
reading: boolean, };

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

export type SymbolSearchAnswer = { ok: true, matches: Array<SymbolMatch>, unread: Array<string>, };

export type Resolve = { symbol: string, };

export type ResolvedListing = { 
/**
 * The listing (`SYMBOL@VENUE`).
 */
listing: string, symbol: string, name: string, exchange: string, currency: string, held: boolean, };

export type ResolveAnswer = { ok: true, listings: Array<ResolvedListing>, unread: Array<string>, } | { ok: false, error: string, };

export type WatchlistBody = { id?: string, symbol: string, exchange: string, name?: string, currency?: string, securityId?: string, };

export type WatchlistRemove = { id: string, };

export type TilesSet = { tiles: Array<TileRef>, };

export type TileRef = { symbol: string, exchange: string, };

export type OptionContractQuery = { 
/**
 * the contract's OCC symbol, as the card names it
 */
occ: string, symbol: string, exchange: string, currency: string, name: string, };

export type OptionRow = { 
/**
 * Its OCC symbol, as Cboe states it (`KO261211P00066000`).
 */
occ: string, 
/**
 * As the card titles it opened (`KO Dec 11 66 put`, `KO Jan 15 '27 66 put`).
 */
title: string, strike: Dec, 
/**
 * What one contract puts in play: the strike × 100 for a put, 100 × the
 * listing's current price for a call; none for a call while the listing's
 * price is not read.
 */
cost: Dec | null, 
/**
 * The bid × 100; none where Cboe states no bid, or a bid of zero.
 */
premium: Dec | null, 
/**
 * The Premium over the Cost, in percent, to two decimals.
 */
yield: Dec | null, };

export type OptionSide = { rows: Array<OptionRow>, 
/**
 * The row whose strike is nearest the listing's current price, which the
 * card opens in view; none while the price is not read.
 */
near: number | null, };

export type OptionExpiry = { 
/**
 * Its day, `YYYY-MM-DD`.
 */
day: string, 
/**
 * As its button says it: `Dec 11`, and `Jan 15 '27` in a later year.
 */
label: string, puts: OptionSide, calls: OptionSide, };

export type OptionsAnswer = { ok: true, 
/**
 * Nearest first. None for a listing with no options: the page then
 * shows no card.
 */
expiries: Array<OptionExpiry>, } | OkOr;

export type PremiumPoint = { 
/**
 * `YYYY-MM-DD`; the last point is today's.
 */
day: string, premium: Dec, };

export type PremiumChart = { 
/**
 * Oldest first; the last is today's Premium.
 */
points: Array<PremiumPoint>, 
/**
 * The axis's top, middle and bottom lines, in dollars.
 */
top: Dec, middle: Dec, bottom: Dec, 
/**
 * The first day, as the axis says it (`Nov 3`).
 */
from: string, };

export type OptionContract = { occ: string, 
/**
 * `KO Dec 11 66 put`.
 */
title: string, cost: Dec | null, premium: Dec | null, 
/**
 * None where Yahoo has no bars for it.
 */
chart: PremiumChart | null, 
/**
 * Why its chart was not read, where it was not.
 */
chartFailed: string | null, };

export type OptionContractAnswer = { ok: true, contract: OptionContract, } | OkOr;

export type TaxWait = { 
/**
 * `slip`, `record-date`, `rate`, `slip-differs`, `moved-in-cost`, `moved-out`,
 * `period-open`, `account-plan`, `corporate-event`, `units`, `earlier-year`,
 * `calendar` or `arithmetic` (`acb::Missing`).
 */
code: string, day: string | null, transaction: string | null, account: string | null, currency: string | null, year: number | null, 
/**
 * The slip less the figures, for `slip-differs`.
 */
amount: Dec | null, };

export type TaxGain = { 
/**
 * `sale` or `below-zero`.
 */
kind: string, day: string, transaction: string | null, units: Dec, proceeds: Dec, cost: Dec, outlays: Dec, gain: Dec, denied: Dec, };

export type TaxYear = { year: number, complete: boolean, waiting: Array<TaxWait>, gains: Array<TaxGain>, };

export type TaxRow = { day: string, 
/**
 * `bought`, `sold`, `moved-in`, `return-of-capital`, `non-cash`, `below-zero`,
 * `loss-added` or `split`.
 */
event: string, units: Dec, cad: Dec, unitsAfter: Dec, costAfter: Dec, rate: Dec | null, 
/**
 * `book`, `entered`, `fund`, `breakdown` or `slip`.
 */
source: string, 
/**
 * The row's transaction, for `book` and `entered`; whose figure, for `fund`.
 */
sourceDetail: string | null, };

export type TaxHolding = { 
/**
 * The book's id for the fund.
 */
fund: string, symbol: string, 
/**
 * Its venue's market identifier code, where known.
 */
venue: string | null, name: string | null, 
/**
 * `canada` or `united-states`.
 */
listed: string, units: Dec, cost: Dec, 
/**
 * The cost per unit, to the fourth place, where units are held.
 */
perUnit: Dec | null, completeThrough: number | null, years: Array<TaxYear>, rows: Array<TaxRow>, };

export type TaxNote = { 
/**
 * `slip-without-text`, `slip-refused`, `fund-unmatched`, `fund-ambiguous`,
 * `combines` or `venue-unknown`.
 */
code: string, account: string | null, year: number | null, 
/**
 * The slip's kind (`t3`, `t5`, `t5008`).
 */
kind: string | null, 
/**
 * The fund as the slip names it.
 */
name: string | null, 
/**
 * How many funds a T3 combines.
 */
funds: number | null, 
/**
 * The book's id for the fund, for `venue-unknown`.
 */
fund: string | null, };

export type TaxSale = { year: number, account: string, 
/**
 * The security as the slip names it.
 */
security: string, day: string, units: Dec, 
/**
 * Box 21.
 */
proceeds: Dec, 
/**
 * Box 20, where the slip states one.
 */
slipCost: Dec | null, 
/**
 * What box 20 must say: box 21 less the ledger's gain; none unless matched.
 */
enter: Dec | null, 
/**
 * `right` (the slip's box 20 is what it must say), `change`, `unmatched` (no
 * sale of the ledger's is it) or `ambiguous` (more than one could be).
 */
state: string, 
/**
 * The fund of the ledger's sale it is.
 */
fund: string | null, };

export type TaxDoc = { holdings: Array<TaxHolding>, 
/**
 * The years complete for every holding that has them, oldest first.
 */
ready: Array<number>, notes: Array<TaxNote>, sales: Array<TaxSale>, };

export type TaxAnswer = { ok: true, tax: TaxDoc, } | OkOr;
