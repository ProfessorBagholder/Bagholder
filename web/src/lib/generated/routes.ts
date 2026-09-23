// Generated from the server's route table (`api_routes!`). Do not edit: change the
// route's declaration, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_routes_are_the_servers`.

import type { Appended, BookAppend, ImportReport, WatchStatus } from './book'
import type { HistoryAnswer } from './chart'
import type { OkOr } from './common'
import type { Document, EnrichAnswer, Filings, FilingsAnswer, FilingsFeed, Scope } from './filings'
import type { Fear, FearAnswer, GlanceAnswer, Listing, ShortsAnswer, ShortsFeed, ShortsQuery } from './markets'
import type { Clear, DataSummary, Groups, GroupsAnswer, Import, JournalAnswer, JournalEntryRequest, Notes, NotesAnswer, TradeAnswer, TradeQuery } from './model_api'
import type { NotificationIds, NotificationsAnswer, NotificationsClearAnswer, NotificationsReadAnswer, NotificationsSeenAnswer, NotifySettingsAnswer, NotifySettingsPatch, NotifyTestAnswer } from './notifications'
import type { Adjust, Modify, Named, OrderActionAnswer, OrdersDoc, RefreshAndOrders } from './orders'
import type { CancelLoginAnswer, Capture, LoginInput, RefreshAnswer, StartLoginAnswer, SyncAnswer } from './session'
import type { StatusAnswer } from './status'

export interface Routes {
  'GET /api/notifications': { answer: NotificationsAnswer }
  'POST /api/notifications/settings': { body: NotifySettingsPatch; answer: NotifySettingsAnswer }
  'POST /api/notifications/test': { answer: NotifyTestAnswer }
  'POST /api/notifications/read': { body: NotificationIds; answer: NotificationsReadAnswer }
  'POST /api/notifications/seen': { body: NotificationIds; answer: NotificationsSeenAnswer }
  'POST /api/notifications/clear': { answer: NotificationsClearAnswer }
  'POST /api/login/start': { answer: StartLoginAnswer }
  'POST /api/login/cancel': { answer: CancelLoginAnswer }
  'POST /api/login/input': { body: LoginInput; answer: OkOr }
  'POST /api/capture': { body: Capture; answer: OkOr }
  'POST /api/refresh': { answer: RefreshAnswer }
  'POST /api/sync': { answer: SyncAnswer }
  'POST /api/disconnect': { answer: OkOr }
  'POST /api/update': { answer: OkOr }
  'GET /api/orders': { answer: OrdersDoc }
  'POST /api/order/cancel': { body: Named; answer: OrderActionAnswer }
  'POST /api/order/modify': { body: Modify; answer: OrderActionAnswer }
  'POST /api/bracket/adjust': { body: Adjust; answer: OrderActionAnswer }
  'POST /api/bracket/cancel': { body: Named; answer: OrderActionAnswer }
  'POST /api/book/append': { body: BookAppend; answer: Appended }
  'POST /api/orders/refresh': { answer: RefreshAndOrders }
  'GET /api/symbols/quote': { query: Listing; answer: GlanceAnswer }
  'GET /api/filings': { query: Filings; answer: FilingsAnswer }
  'GET /api/filings/feed': { query: Scope; answer: FilingsFeed }
  'GET /api/filings/enrich': { query: Document; answer: EnrichAnswer }
  'GET /api/fear': { query: Fear; answer: FearAnswer }
  'GET /api/shorts': { query: ShortsQuery; answer: ShortsAnswer }
  'GET /api/shorts/feed': { answer: ShortsFeed }
  'GET /api/history': { answer: HistoryAnswer }
  'POST /api/markets/refresh': { answer: OkOr }
  'GET /api/status': { answer: StatusAnswer }
  'GET /api/trade': { query: TradeQuery; answer: TradeAnswer }
  'GET /api/data': { answer: DataSummary }
  'POST /api/data/clear': { body: Clear; answer: DataSummary }
  'POST /api/journal': { body: JournalEntryRequest; answer: JournalAnswer }
  'POST /api/groups': { body: Groups; answer: GroupsAnswer }
  'POST /api/notes': { body: Notes; answer: NotesAnswer }
  'POST /api/import': { body: Import; answer: ImportReport }
  'POST /api/watch/clear': { answer: WatchStatus }
}
