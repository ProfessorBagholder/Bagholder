// Generated from the server's route table (`api_routes!`). Do not edit: change the
// route's declaration, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_routes_are_the_servers`.

import type { HistoryAnswer, HistoryQuery } from './chart'
import type { OkOr } from './common'
import type { Detail, Figures } from './figures'
import type { Document, EnrichAnswer, Filings, FilingsAnswer, FilingsFeed, Scope } from './filings'
import type { Fear, FearAnswer, GlanceAnswer, Listing, ListingAnswer, NewsSymbolAnswer, Search, ShortsAnswer, ShortsFeed, ShortsQuery, SymbolSearchAnswer, TilesAnswer, TilesSet, WatchlistAnswer, WatchlistBody } from './markets'
import type { Clear, ClearAnswer, EntryAnswer, EntryRequest, FiguresQuery, ImportReport, ImportRequest, JournalAnswer, JournalEntryRequest, TradeQuery, WatchRequest, WatchStatus } from './model_api'
import type { NotificationIds, NotificationsAnswer, NotificationsClearAnswer, NotificationsReadAnswer, NotificationsSeenAnswer, NotifySettingsAnswer, NotifySettingsPatch, NotifyTestAnswer } from './notifications'
import type { Adjust, Modify, Named, OrderActionAnswer, OrdersDoc, PlaceTicketAnswer, Preview, PreviewRequest, QuoteOf, RefreshAndOrders, Ticket, TicketQuote } from './orders'
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
  'POST /api/orders/refresh': { answer: RefreshAndOrders }
  'GET /api/order/quote': { query: QuoteOf; answer: TicketQuote }
  'POST /api/order/preview': { body: PreviewRequest; answer: Preview }
  'POST /api/order': { body: Ticket; answer: PlaceTicketAnswer }
  'GET /api/symbols/search': { query: Search; answer: SymbolSearchAnswer }
  'GET /api/symbols/quote': { query: Listing; answer: GlanceAnswer }
  'GET /api/listing': { query: Listing; answer: ListingAnswer }
  'GET /api/filings': { query: Filings; answer: FilingsAnswer }
  'GET /api/filings/feed': { query: Scope; answer: FilingsFeed }
  'GET /api/filings/enrich': { query: Document; answer: EnrichAnswer }
  'GET /api/news/symbol': { query: Listing; answer: NewsSymbolAnswer }
  'GET /api/fear': { query: Fear; answer: FearAnswer }
  'GET /api/shorts': { query: ShortsQuery; answer: ShortsAnswer }
  'GET /api/shorts/feed': { answer: ShortsFeed }
  'GET /api/history': { query: HistoryQuery; answer: HistoryAnswer }
  'POST /api/watchlist/add': { body: WatchlistBody; answer: WatchlistAnswer }
  'POST /api/watchlist/remove': { body: WatchlistBody; answer: WatchlistAnswer }
  'POST /api/tiles/set': { body: TilesSet; answer: TilesAnswer }
  'GET /api/status': { answer: StatusAnswer }
  'GET /api/figures': { query: FiguresQuery; answer: Figures }
  'GET /api/figures/detail': { query: TradeQuery; answer: Detail }
  'POST /api/data/clear': { body: Clear; answer: ClearAnswer }
  'POST /api/journal': { body: JournalEntryRequest; answer: JournalAnswer }
  'POST /api/entries': { body: EntryRequest; answer: EntryAnswer }
  'POST /api/import': { body: ImportRequest; answer: ImportReport }
  'POST /api/watch/clear': { answer: WatchStatus }
  'GET /api/watch': { answer: WatchStatus }
  'POST /api/watch': { body: WatchRequest; answer: WatchStatus }
  'POST /api/watch/scan': { answer: WatchStatus }
}
