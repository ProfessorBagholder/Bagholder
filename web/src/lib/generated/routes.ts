// Generated from the server's route table (`api_routes!`). Do not edit: change the
// route's declaration, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_routes_are_the_servers`.

import type { NotificationIds, NotificationsAnswer, NotifySettingsAnswer, NotifySettingsPatch, NotifyTestAnswer, NotificationsReadAnswer, NotificationsSeenAnswer, NotificationsClearAnswer } from './notifications'

export interface Routes {
  'GET /api/notifications': { answer: NotificationsAnswer }
  'POST /api/notifications/settings': { body: NotifySettingsPatch; answer: NotifySettingsAnswer }
  'POST /api/notifications/test': { answer: NotifyTestAnswer }
  'POST /api/notifications/read': { body: NotificationIds; answer: NotificationsReadAnswer }
  'POST /api/notifications/seen': { body: NotificationIds; answer: NotificationsSeenAnswer }
  'POST /api/notifications/clear': { answer: NotificationsClearAnswer }
}
