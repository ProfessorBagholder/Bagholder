// Generated from the server's status and notify modules. Do not edit: change the Rust type,
// then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_status_types`.

export type NotifySettings = { fills: boolean, problems: boolean, connection: boolean, updates: boolean, releasesHeld: boolean, releasesWatched: boolean, releasesAll: boolean, disclosuresHeld: boolean, disclosuresWatched: boolean, disclosuresAll: boolean, };

export type NotifyStatus = { native: string, unread: number, fills: boolean, problems: boolean, connection: boolean, updates: boolean, releasesHeld: boolean, releasesWatched: boolean, releasesAll: boolean, disclosuresHeld: boolean, disclosuresWatched: boolean, disclosuresAll: boolean, };

export type Status = { ok: true, connected: boolean, email: string, lastSync: string, activityCount: number, accountCount: number, capturing: boolean, syncing: boolean, listingsFilling: boolean, syncStep: string, error: string, summaryReady: boolean, protocol: string, startedAt: string, version: string, latestVersion: string, updateAvailable: boolean, updateUrl: string, canUpdate: boolean, updateBy: string, 
/**
 * `true` while `BAGHOLDER_LOGIN_VIEW` asks for the sign-in window shown
 * as a page rather than streamed frames.
 */
loginView: boolean, ordersLive: boolean, openOrders: number, updating: string, updateError: string, notify: NotifyStatus, newsReading: Array<string>, };
