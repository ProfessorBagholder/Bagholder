// A match's price and day change, read once for the glance and remembered while
// the page lives (ledger's _sugQuotes / sugQuoteSchedule). Reactive $state so the
// watchlist row and the add-row suggestion fill in when a quote lands.
import { bareSymbol } from '../sym'
import { request } from '../api'

export interface Quote {
  last: number | null
  percentChange: number | null
}

export const sugQuotes = $state<Record<string, Quote>>({})
const pending: Record<string, boolean> = {}

export function sugKey(w: { symbol: string; exchange?: string }): string {
  return bareSymbol(w.symbol) + '@' + String(w.exchange || '').toUpperCase()
}

export function sugQuoteSchedule(rows: { symbol: string; exchange?: string; currency?: string; last?: number | null }[]): void {
  rows.forEach((w) => {
    const k = sugKey(w)
    if (w.last != null || sugQuotes[k] || pending[k]) return
    pending[k] = true
    request<{ ok: boolean; price?: number | null; percentChange?: number | null }>(
      'GET',
      '/api/symbols/quote?symbol=' + encodeURIComponent(w.symbol) + '&exchange=' + encodeURIComponent(w.exchange || '') + '&currency=' + encodeURIComponent(w.currency || ''),
    ).then((r) => {
      delete pending[k]
      if (r && r.ok && r.price != null) sugQuotes[k] = { last: r.price, percentChange: r.percentChange ?? null }
    })
  })
}
