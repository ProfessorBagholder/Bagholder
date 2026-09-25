// A listing the book does not hold opens the page a holding opens, with the listing
// standing in for the trade: no size, no cost and no P&L, since it has none, and the
// executions of the trades once closed on it marked on its chart. It is what every
// click on an unheld ticker leads to: the watchlist, the short-interest table, a
// heatmap tile, the symbol picker, a notification.
//
// What is known of a listing is asked for once, when its page first opens
// (`/api/listing`); its price, where the watchlist carries it, is the model's own and
// moves with it.

import { lookup } from './api'
import { bareSymbol } from './sym'
import { localDay } from './fmt'
import type { Fill, Model, Trade } from './model'

interface Known {
  symbol: string
  exchange: string
  currency: string
  name: string
  kind: string
  securityId: string
  fills?: Fill[]
  price?: number | null
  percentChange?: number | null
}

const LISTING_DAYS = 365
const known = $state<Record<string, Known>>({})
// asked once while the page is open; a failed lookup is asked again the next time the page opens
const listings = lookup('GET /api/listing')
const applied = new Set<string>()

export function listingId(o: { symbol?: string; exchange?: string | null }): string {
  return 'listing:' + bareSymbol(String(o.symbol || '')).toUpperCase() + '@' + String(o.exchange || '').toUpperCase()
}

export const isListingId = (id: string | null | undefined): boolean => String(id || '').startsWith('listing:')

function entry(id: string): Known {
  const [symbol, exchange = ''] = id.slice('listing:'.length).split('@')
  return (known[id] ??= { symbol, exchange, currency: '', name: '', kind: '', securityId: '' })
}

/** What a row already says of the listing it opens, kept so its page has a name at once. */
export function rememberListing(o: { symbol: string; exchange?: string | null; currency?: string; name?: string; kind?: string }): string {
  const id = listingId(o)
  const l = entry(id)
  if (!l.currency && o.currency) l.currency = o.currency
  if (!l.name && o.name) l.name = o.name
  if (!l.kind && o.kind) l.kind = o.kind
  return id
}

/** The listing standing in for a trade on the detail page. */
export function listingAsTrade(id: string, model: Model | null): Trade | null {
  if (!isListingId(id)) return null
  const l = entry(id)
  // the quote the model already keeps for a watched listing, which moves with it
  const w = model?.markets.watchlist.find((x) => listingId(x) === id)
  const watched = w && w.last != null
  const since = localDay(new Date(Date.now() - LISTING_DAYS * 86400000))
  return {
    ...l,
    id,
    listing: true,
    holding: false,
    status: 'listing',
    legs: [],
    tags: [],
    fills: l.fills,
    pnl: null,
    pnlPct: null,
    holdDays: null,
    entryDate: since,
    exitDate: null,
    last: watched ? w.last : (l.price ?? null),
    percentChange: watched ? w.percentChange : (l.percentChange ?? null),
    kind: l.kind || w?.kind || 'Shares',
    name: l.name || w?.name || '',
  } as unknown as Trade
}

/**
 * Ask what the server knows of a listing, once. `held` is called with the holding's
 * id when the book turns out to hold it (a row opened before the model caught up):
 * its page is the holding's.
 */
export async function loadListing(id: string, held: (positionId: string) => void): Promise<void> {
  if (!isListingId(id)) return
  const l = entry(id)
  const d = await listings.read({ query: { symbol: l.symbol, exchange: l.exchange, currency: l.currency, name: l.name } }, { key: id })
  if (!d.ok || 'error' in d) {
    l.fills = []
    return
  }
  if ('positionId' in d) return held(d.positionId)
  if (applied.has(id)) return
  applied.add(id)
  for (const k of ['exchange', 'currency', 'name', 'kind', 'securityId'] as const) {
    if (!l[k] && d[k]) l[k] = d[k] as string
  }
  l.fills = d.fills
  l.price = d.price ?? null
  l.percentChange = d.percentChange ?? null
}
