// Which listing a word typed in the News box names (SPEC.md §4 Markets, News): a token
// that names a listing takes the chip; any other word stays a text search. Which listing
// is settled from what the app knows, in order: the listings the book holds, watches and
// has traded, then the exchanges' directories, by the ticker itself and never by a
// match on a name.

import { bareSymbol } from '../sym'

export interface Chip {
  symbol: string
  exchange: string
  name?: string
  currency?: string
}

interface Row {
  symbol?: string | null
  exchange?: string | null
  name?: string | null
  currency?: string | null
}

/** The text as a ticker would be written: one short word, upper-cased. Anything else is never a chip. */
export function tickerKey(text: string): string | null {
  const key = text.trim().toUpperCase()
  return /^[A-Z0-9.\-]{1,6}$/.test(key) ? key : null
}

const chipOf = (r: Row): Chip => ({
  symbol: bareSymbol(String(r.symbol || '')).toUpperCase(),
  exchange: String(r.exchange || '').toUpperCase(),
  name: r.name || '',
  currency: r.currency || '',
})

// a listing's own ticker, never a contract's (which is written with its terms after a space)
const names = (key: string) => (r: Row) => {
  const s = String(r.symbol || '')
  return s.length > 0 && s.indexOf(' ') < 0 && bareSymbol(s).toUpperCase() === key
}

/** The listing of the book's own rows (held, watched, traded) the ticker names, or none. */
export function bookListing(key: string, rows: Row[]): Chip | null {
  const hit = rows.find(names(key))
  return hit ? chipOf(hit) : null
}

/** The listing a directory's answer names by that very ticker, or none: a name match names nothing. */
export function directoryListing(key: string, matches: Row[]): Chip | null {
  const hit = matches.find(names(key))
  return hit ? chipOf(hit) : null
}
