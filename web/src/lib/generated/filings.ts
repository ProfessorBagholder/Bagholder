// Generated from rust/crates/store/src/feeds.rs and the server's filings documents. Do not
// edit: change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_filing_types`.

export type Regulator = "SEDAR+" | "SEC";

export type FiledDocument = { id: string, source: Regulator, category: string, profileNo: string, issuer: string, 
/**
 * The form or document type.
 */
type: string, title: string, date: string, dateText: string, size: string, url: string, };

export type Filing = { subject: string, summary: string, enrichedAt: string, 
/**
 * The version of the logic that read it; `None` when never read.
 */
enrichVersion: number | null, 
/**
 * Read for good: a form read from its own boxes is not read again.
 */
enrichFinal: boolean, fetchedAt: string, id: string, source: Regulator, category: string, profileNo: string, issuer: string, 
/**
 * The form or document type.
 */
type: string, title: string, date: string, dateText: string, size: string, url: string, };

export type SourceStatus = { available: boolean, matched: boolean, filer: boolean, error: string, };

export type FilingsDoc = { ok: true, symbol: string, available: boolean, sources: { [key in string]: SourceStatus }, categories: Array<string>, fetchedAt: string, everRead: boolean, summaryStatus: string, reading: Array<string>, filings: Array<Filing>, };

export type FilingsPayload = { ok: true, symbol: string, available: boolean, sources: { [key in string]: SourceStatus }, categories: Array<string>, profileNo: string, fetchedAt: string, refreshed: boolean, sourceUnavailable: boolean, filings: Array<Filing>, };

export type FeedFiling = { symbol: string, exchange: string, subject: string, summary: string, enrichedAt: string, 
/**
 * The version of the logic that read it; `None` when never read.
 */
enrichVersion: number | null, 
/**
 * Read for good: a form read from its own boxes is not read again.
 */
enrichFinal: boolean, fetchedAt: string, id: string, source: Regulator, category: string, profileNo: string, issuer: string, 
/**
 * The form or document type.
 */
type: string, title: string, date: string, dateText: string, size: string, url: string, };

export type FilingsFeed = { ok: true, scope: string, filings: Array<FeedFiling>, reading: boolean, };

export type Enriched = { ok: true, id: string, subject: string, summary: string, summaryAvailable: boolean, summaryStatus: string, };
