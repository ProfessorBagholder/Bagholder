// A match's price and day change, read for the glance and remembered for a minute
// (SPEC.md §4 Markets, Watchlist: a minute's memory, nothing stored): asked for again
// the next time it is wanted once that minute is up, the one shown standing until the
// new one lands. Reactive $state so the watchlist row and the add-row suggestion fill
// in when a quote lands.
import { bareSymbol } from '../sym'
import { request } from '../api'

export interface Quote {
  last: number | null
  percentChange: number | null
}

const MEMORY_MS = 60_000

export const sugQuotes = $state<Record<string, Quote>>({})
const readAt: Record<string, number> = {}
const pending: Record<string, boolean> = {}

export function sugKey(w: { symbol: string; exchange?: string }): string {
  return bareSymbol(w.symbol) + '@' + String(w.exchange || '').toUpperCase()
}

export function sugQuoteSchedule(rows: { symbol: string; exchange?: string; currency?: string; last?: unknown }[]): void {
  rows.forEach((w) => {
    const k = sugKey(w)
    if (w.last != null || pending[k] || (sugQuotes[k] && Date.now() - readAt[k] < MEMORY_MS)) return
    pending[k] = true
    request<{ ok: boolean; price?: number | null; percentChange?: number | null }>(
      'GET',
      '/api/symbols/quote?symbol=' + encodeURIComponent(w.symbol) + '&exchange=' + encodeURIComponent(w.exchange || '') + '&currency=' + encodeURIComponent(w.currency || ''),
    ).then((r) => {
      delete pending[k]
      if (r && r.ok && r.price != null) {
        readAt[k] = Date.now()
        sugQuotes[k] = { last: r.price, percentChange: r.percentChange ?? null }
      }
    })
  })
}
