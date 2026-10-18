// A listing's key, `SYMBOL@VENUE`, made as the server makes it
// (`bagholder_core::listing`): the venue is the market identifier code in Canada,
// `US` for any US exchange, `OTC` over the counter, `CRYPTO` for a coin, `INDEX` for a directory
// instrument and `OCC` for an option contract. Two listings that share a ticker
// are two keys; a symbol with no venue is no key. The table is the server's own
// (`generated/venues.ts`), and `listingKey.test.ts` holds this function to the
// keys the server makes of the same generated inputs.

import { CANADA_MICS, CRYPTO, INDEX, OCC, OTC, OTC_MICS, SPACES, US, US_MICS, VENUE_NAMES, VENUE_SUFFIXES, VENUE_WORDS } from './generated/venues'

const asciiUpper = (s: string): string => s.replace(/[a-z]/g, (c) => c.toUpperCase())

// space as the server reads it (its own table): every space is in the Basic
// Multilingual Plane, so a code unit is a character here
const space = new Set(SPACES)
function trimSpace(s: string): string {
  let a = 0
  let b = s.length
  while (a < b && space.has(s[a])) a++
  while (b > a && space.has(s[b - 1])) b--
  return s.slice(a, b)
}
const dropSpace = (s: string): string => Array.from(s).filter((c) => !space.has(c)).join('')

/** The market identifier code of a venue written in words or as a code; '' for none known. */
export function micOf(words: string): string {
  const w = asciiUpper(trimSpace(words))
  return VENUE_WORDS.find(([, ws]) => ws.includes(w))?.[0] ?? ''
}

/** The venue a listing on `mic` is keyed under: the code in Canada, `US` on a US exchange, `OTC` over the counter. */
function listingVenue(mic: string): string {
  if (US_MICS.includes(mic)) return US
  if (OTC_MICS.includes(mic)) return OTC
  if (CANADA_MICS.includes(mic)) return mic
  return ''
}

/** A symbol in the one spelling a key holds. */
export function spelling(symbol: string): string {
  let s = asciiUpper(trimSpace(symbol))
  const suffix = VENUE_SUFFIXES.find((x) => s.endsWith(x))
  if (suffix) s = s.slice(0, -suffix.length)
  return dropSpace(s).replace(/[-/]/g, '.')
}

/** The key of `symbol` on the venue `venue` names; null where it names none. */
export function listingKey(symbol: string | null | undefined, venue: string | null | undefined): string | null {
  const sym = String(symbol ?? '')
  const v = asciiUpper(trimSpace(String(venue ?? '')))
  const keyed = (s: string, at: string): string | null => (s ? s + '@' + at : null)
  if (v === US) return keyed(spelling(sym), US)
  if (v === OTC) return keyed(spelling(sym), OTC)
  if (v === CRYPTO) return keyed(spelling(sym), CRYPTO)
  if (v === INDEX) return keyed(asciiUpper(trimSpace(sym)), INDEX)
  if (v === OCC) return keyed(asciiUpper(dropSpace(sym)), OCC)
  const at = listingVenue(micOf(v))
  return at ? keyed(spelling(sym), at) : null
}

/** A key's own parts. */
export function keyParts(key: string): { symbol: string; venue: string } {
  const at = key.lastIndexOf('@')
  return at < 0 ? { symbol: key, venue: '' } : { symbol: key.slice(0, at), venue: key.slice(at + 1) }
}

/** The name a person reads a key's venue under (`XTSE` is `TSX`), the server's own table. */
export function venueName(venue: string): string {
  return VENUE_NAMES.find(([v]) => v === venue)?.[1] ?? venue
}
