// Generated from rust/crates/store/src/feeds.rs and the server's market documents. Do not
// edit: change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_market_types`.

export type GaugeReading = { label: string, score: number, rating: string, };

export type GaugePart = { name: string, score: number, rating: string, };

export type GaugePoint = { date: string, score: number, };

export type Gauge = { index: string, source: string, score: number, rating: string, asOf: string, previous: Array<GaugeReading>, parts: Array<GaugePart>, series: Array<GaugePoint>, };

export type StoredGauge = { fetchedAt: string, readVersion: number, index: string, source: string, score: number, rating: string, asOf: string, previous: Array<GaugeReading>, parts: Array<GaugePart>, series: Array<GaugePoint>, };

export type FearDoc = { ok: true, gauge: StoredGauge | null, };
