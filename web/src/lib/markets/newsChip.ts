// The News box's chip (SPEC.md §4 Markets, News): a token that names a listing takes
// it; any other word stays a text search. Which listing is the resolver's
// (`/api/listings`): one listing takes the chip, two or more are offered by venue.

import { listingKey } from '../listingKey'

export interface Chip {
  symbol: string
  exchange: string
  name?: string
  currency?: string
}

/** The text as a ticker would be written: one short word, upper-cased. Anything else is never a chip. */
export function tickerKey(text: string): string | null {
  const key = text.trim().toUpperCase()
  return /^[A-Z0-9.\-]{1,6}$/.test(key) ? key : null
}

/** The chip's listing key; '' for one that names no venue, which no listing is. */
export const chipKey = (c: Chip): string => listingKey(c.symbol, c.exchange) ?? ''
