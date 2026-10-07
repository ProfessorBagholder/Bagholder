// Generated from the server's status and notify modules. Do not edit: change the Rust type,
// then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_status_types`.

import type { ImportReport } from './model_api'

export type NotifySettings = { fills: boolean, problems: boolean, connection: boolean, updates: boolean, releasesHeld: boolean, releasesWatched: boolean, releasesAll: boolean, disclosuresHeld: boolean, disclosuresWatched: boolean, disclosuresAll: boolean, };

export type NotifyStatus = { native: string, unread: number, fills: boolean, problems: boolean, connection: boolean, updates: boolean, releasesHeld: boolean, releasesWatched: boolean, releasesAll: boolean, disclosuresHeld: boolean, disclosuresWatched: boolean, disclosuresAll: boolean, };

export type Importing = { 
/**
 * The import's own id, the one `POST /api/import` answered with.
 */
id: string, file: string, 
/**
 * Bytes received of the file, and its size where the request states it.
 */
received: number, size: number | null, 
/**
 * Bytes of the file read through once it has arrived, to check every line
 * reads and to count its rows.
 */
checked: number, 
/**
 * Rows kept so far, of the file's rows once they are counted.
 */
rows: number, total: number | null, };

export type Imported = { id: string, file: string, report: ImportReport | null, error: string | null, 
/**
 * A page has shown it (`POST /api/import/told`): no page says it again.
 */
told: boolean, };

export type ModelDownload = { received: number, size: number, };

export type Status = { ok: true, connected: boolean, email: string, lastSync: string, activityCount: number, accountCount: number, capturing: boolean, syncing: boolean, listingsFilling: boolean, syncStep: string, error: string, summaryReady: boolean, protocol: string, startedAt: string, version: string, latestVersion: string, updateAvailable: boolean, updateUrl: string, canUpdate: boolean, updateBy: string, 
/**
 * `true` while `BAGHOLDER_LOGIN_VIEW` asks for the sign-in window shown
 * as a page rather than streamed frames.
 */
loginView: boolean, ordersLive: boolean, openOrders: number, updating: string, updateError: string, notify: NotifyStatus, 
/**
 * The import running now and how far it has come, for the import window.
 */
importing: Importing | null, 
/**
 * The last import that ended: what it did, or why nothing of it was kept.
 */
imported: Imported | null, 
/**
 * The language model's file while it downloads, and how far it has come.
 */
modelDownload: ModelDownload | null, };

export type StatusAnswer = { dataVersion: string, coreVersion: string, ok: true, connected: boolean, email: string, lastSync: string, activityCount: number, accountCount: number, capturing: boolean, syncing: boolean, listingsFilling: boolean, syncStep: string, error: string, summaryReady: boolean, protocol: string, startedAt: string, version: string, latestVersion: string, updateAvailable: boolean, updateUrl: string, canUpdate: boolean, updateBy: string, 
/**
 * `true` while `BAGHOLDER_LOGIN_VIEW` asks for the sign-in window shown
 * as a page rather than streamed frames.
 */
loginView: boolean, ordersLive: boolean, openOrders: number, updating: string, updateError: string, notify: NotifyStatus, 
/**
 * The import running now and how far it has come, for the import window.
 */
importing: Importing | null, 
/**
 * The last import that ended: what it did, or why nothing of it was kept.
 */
imported: Imported | null, 
/**
 * The language model's file while it downloads, and how far it has come.
 */
modelDownload: ModelDownload | null, };
