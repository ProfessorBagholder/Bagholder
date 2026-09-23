// Generated from rust/crates/store/src/feeds.rs and the server's notify module. Do not
// edit: change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_notifications_types`.

import type { NotifyStatus } from './status'

export type NotificationExtra = { symbol: string, exchange: string, at: string, url: string, doc: string, source: string, };

export type Notification = { id: number, at: string, kind: string, key: string, title: string, body: string, extra: NotificationExtra, seenAt: string, readAt: string, };

export type NotifySettingsPatch = { fills?: boolean, problems?: boolean, connection?: boolean, updates?: boolean, releasesHeld?: boolean, releasesWatched?: boolean, releasesAll?: boolean, disclosuresHeld?: boolean, disclosuresWatched?: boolean, disclosuresAll?: boolean, };

export type NotificationIds = { ids: Array<number> | null, };

export type NotificationsAnswer = { ok: boolean, settings: NotifyStatus, kinds: Array<string>, rows: Array<Notification>, unread: number, };

export type NotifySettingsAnswer = { ok: boolean, settings: NotifyStatus, };

export type NotifyTestAnswer = { ok: boolean, id: number, };

export type NotificationsReadAnswer = { ok: boolean, read: number, };

export type NotificationsSeenAnswer = { ok: boolean, seen: number, };

export type NotificationsClearAnswer = { ok: boolean, cleared: number, };
