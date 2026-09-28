// A listing the book does not hold opens the page a holding opens, with the listing
// standing in for the trade: no size, no cost and no P&L, since it has none, and the
// executions of the trades once closed on it marked on its chart. It is what every
// click on an unheld ticker leads to: the watchlist, the short-interest table, a
// heatmap tile, the symbol picker, a notification.
//
// What is known of a listing is asked for once, when its page first opens
// (`/api/listing`); its price, where the watchlist carries it, is the model's own and
// moves with it.

import { heldOnceRead, read } from './reads.svelte'
import { flash } from './ui.svelte'
import { bareSymbol } from './sym'
import { localDay } from './fmt'
import type { Dec } from './dec'
import type { Fill, MarketsDoc, Trade } from './model'

interface Known {
  symbol: string
  exchange: string
  currency: string
  name: string
  kind: string
  securityId: string
  fills?: Fill[]
  price?: Dec | null
  /** The day's change, as a fraction. */
  percentChange?: number | null
}

const LISTING_DAYS = 365
const known = $state<Record<string, Known>>({})
// asked once while the page is open, what was held drawn first; a failed lookup is asked again the next time the page opens
const applied = new Set<string>()

export function listingId(o: { symbol?: string; exchange?: string | null }): string {
  return 'listing:' + bareSymbol(String(o.symbol || '')).toUpperCase() + '@' + String(o.exchange || '').toUpperCase()
}

export const isListingId = (id: string | null | undefined): boolean => String(id || '').startsWith('listing:')

function entry(id: string): Known {
  // the store's own (reactive) object, so what is written to it reaches the page
  if (!known[id]) known[id] = parsed(id)
  return known[id]
}
/** What the address itself says of a listing: its symbol and venue. */
function parsed(id: string): Known {
  const [symbol, exchange = ''] = id.slice('listing:'.length).split('@')
  return { symbol, exchange, currency: '', name: '', kind: '', securityId: '' }
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
export function listingAsTrade(id: string, markets: MarketsDoc | null): Trade | null {
  if (!isListingId(id)) return null
  // read only: a page drawing a listing writes nothing while it draws
  const l = known[id] ?? parsed(id)
  // the quote the model already keeps for a watched listing, which moves with it
  const w = markets?.watchlist.find((x) => listingId(x) === id)
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
export async function loadListing(id: string, heldBy: (positionId: string) => void): Promise<void> {
  if (!isListingId(id)) return
  const l = entry(id)
  const q = { query: { symbol: l.symbol, exchange: l.exchange, currency: l.currency, name: l.name } }
  // what was last known of it is taken at once: a listing the book turned out to hold opens its holding's page
  const had = await heldOnceRead('GET /api/listing', q, { key: id })
  if (had?.ok) {
    if ('positionId' in had) return heldBy(had.positionId)
    take(l, had)
  }
  const d = await read('GET /api/listing', q, { key: id })
  if (!d.ok) {
    // said in the header; the page shows the listing without executions rather than waiting for ever
    flash('Could not read ' + l.symbol + ': ' + d.error, 'err')
    l.fills = []
    return
  }
  if ('positionId' in d) return heldBy(d.positionId)
  if (applied.has(id)) return
  applied.add(id)
  take(l, d)
}

/** What the server knows of a listing, written where it differs from what is shown. */
function take(l: Known, d: { exchange?: string; currency?: string; name?: string; kind?: string; securityId?: string; fills: Fill[]; price?: Dec | null; percentChange?: number | null }): void {
  for (const k of ['exchange', 'currency', 'name', 'kind', 'securityId'] as const) {
    if (!l[k] && d[k]) l[k] = d[k] as string
  }
  if (JSON.stringify(l.fills) !== JSON.stringify(d.fills)) l.fills = d.fills
  if (l.price !== (d.price ?? null)) l.price = d.price ?? null
  if (l.percentChange !== (d.percentChange ?? null)) l.percentChange = d.percentChange ?? null
}
