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
import { tell } from './ui.svelte'
import { bareSymbol } from './sym'
import { keyParts, listingKey, venueName } from './listingKey'
import { localDay } from './fmt'
import type { Dec } from './dec'
import type { Fill, MarketsDoc, Trade } from './model'
import type { ResolvedListing } from './generated/markets'

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

/**
 * A listing's page address: `listing:` and its key (`SYMBOL@VENUE`), so two listings
 * of one ticker are two pages and one listing written two ways is one. A row that
 * names no venue is `listing:SYMBOL`, which no listing is: its page offers every
 * listing of the symbol to pick from (`/api/listings`).
 */
export function listingId(o: { symbol?: string; exchange?: string | null; listing?: string | null }): string {
  if (o.listing) return 'listing:' + o.listing
  const key = listingKey(bareSymbol(String(o.symbol || '')), o.exchange)
  return 'listing:' + (key ?? bareSymbol(String(o.symbol || '')).toUpperCase())
}

export const isListingId = (id: string | null | undefined): boolean => String(id || '').startsWith('listing:')

/**
 * The address as the app writes it: one an older link wrote with the venue in words
 * (`listing:TD@TSX`) is the same listing's key (`listing:TD@XTSE`); one naming a venue
 * no listing is keyed by, or none, names only its symbol.
 */
export function canonicalListingId(id: string): string {
  if (!isListingId(id)) return id
  const { symbol, venue } = keyParts(id.slice('listing:'.length))
  return listingId({ symbol, exchange: venue })
}

/** Whether the address names a listing, not only a symbol. */
export const namesListing = (id: string): boolean => id.slice('listing:'.length).includes('@')

function entry(id: string): Known {
  // the store's own (reactive) object, so what is written to it reaches the page
  if (!known[id]) known[id] = parsed(id)
  return known[id]
}
/**
 * The venue of the listing's own address, as the row that opened it wrote it where
 * one did, else as its key names it; empty where the address names none: an order is
 * placed only on a listing named exactly, so the venue is never taken from anything
 * found by the ticker (brief 36, 2).
 */
export function listingVenue(id: string): string {
  if (!namesListing(id)) return ''
  return known[id]?.exchange || parsed(id).exchange
}

/** What the address itself says of a listing: its symbol and the venue its key names, by its name. */
function parsed(id: string): Known {
  const { symbol, venue } = keyParts(id.slice('listing:'.length))
  return { symbol, exchange: namesListing(id) ? venueName(venue) : '', currency: '', name: '', kind: '', securityId: '' }
}

/** What a row already says of the listing it opens, kept so its page has a name at once. */
export function rememberListing(o: { symbol: string; exchange?: string | null; listing?: string | null; currency?: string; name?: string; kind?: string }): string {
  const id = listingId(o)
  const l = entry(id)
  // the venue in the row's own words, which the page shows
  if (namesListing(id) && o.exchange && l.exchange !== o.exchange) l.exchange = o.exchange
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
  const w = namesListing(id) ? markets?.watchlist.find((x) => listingId(x) === id) : undefined
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
  // an address naming only a symbol is no listing: it is resolved (`resolveListing`), never read by its ticker
  if (!isListingId(id) || !namesListing(id)) return
  const l = entry(id)
  const q = { query: { symbol: l.symbol, exchange: l.exchange, currency: l.currency, name: l.name } }
  // what was last known of it is taken at once: a listing the book turned out to hold opens its holding's page
  const had = await heldOnceRead('GET /api/listing', q, { key: id })
  // a holding found by the ticker alone is not this page's: only an address that
  // names its venue opens the holding's page (brief 34, U17)
  const named = namesListing(id)
  if (had?.ok) {
    if ('positionId' in had) {
      if (named) return heldBy(had.positionId)
    } else take(id, l, had)
  }
  const d = await read('GET /api/listing', q, { key: id })
  if (!d.ok) {
    // said in the header; the page shows the listing without executions rather than waiting for ever
    tell('Could not read ' + l.symbol + ': ' + d.error, 'err')
    l.fills = []
    return
  }
  if ('positionId' in d) {
    if (named) heldBy(d.positionId)
    return
  }
  if (applied.has(id)) return
  applied.add(id)
  take(id, l, d)
}

/**
 * What the server knows of a listing, written where it differs from what is shown.
 * A page whose address names no venue takes no venue and no broker id from it: the
 * server can only have found those by the ticker.
 */
function take(id: string, l: Known, d: { exchange?: string; currency?: string; name?: string; kind?: string; securityId?: string; fills: Fill[]; price?: Dec | null; percentChange?: number | null }): void {
  const keys = namesListing(id) ? (['exchange', 'currency', 'name', 'kind', 'securityId'] as const) : (['currency', 'name', 'kind'] as const)
  for (const k of keys) {
    // the venue as the server names it in words, over the code the address carries
    if (d[k] && (!l[k] || (k === 'exchange' && l[k] !== d[k]))) l[k] = d[k] as string
  }
  if (JSON.stringify(l.fills) !== JSON.stringify(d.fills)) l.fills = d.fills
  if (l.price !== (d.price ?? null)) l.price = d.price ?? null
  if (l.percentChange !== (d.percentChange ?? null)) l.percentChange = d.percentChange ?? null
}

/** The listings an address naming only a symbol may be, once asked, or why they could not be read. */
export const listingChoices = $state<Record<string, { listings: ResolvedListing[]; error: string }>>({})

/**
 * Resolve an address that names only a symbol: every listing of it, each with its
 * venue. Exactly one, where every directory answered, is that listing's page
 * (`opened` with its address); two or more, or one beside a directory that did not
 * answer, are offered as rows (`listingChoices`) with the failure, and none is
 * chosen for the person.
 */
export async function resolveListing(id: string, opened: (listingPage: string) => void): Promise<void> {
  if (!isListingId(id) || namesListing(id)) return
  const symbol = parsed(id).symbol
  const r = await read('GET /api/listings', { query: { symbol } })
  const listings: ResolvedListing[] = r.ok ? r.listings : []
  const error = r.ok ? r.unread.join(' ') : r.error
  if (listings.length === 1 && !error) return opened(rememberListing(listings[0]))
  listingChoices[id] = { listings, error }
}
