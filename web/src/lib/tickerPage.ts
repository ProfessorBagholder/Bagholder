/**
 * Where a ticker chosen in the filter's search goes (`SPEC.md` filters-18): its
 * holding's page where the book holds the instrument, the listing's own page under
 * Markets otherwise, and `wait` while the holdings are not read yet. What is not
 * known is never taken as not held: a held ticker chosen before the holdings
 * arrived, or while a reconnect or a renewed sign-in held them up, opened under
 * Markets without its executions (the owner, 2026-11-06).
 */
export function tickerPage(instrument: string, holdings: readonly { instrument: string; id: string }[] | null | undefined): 'wait' | 'markets' | { holding: string } {
  // a row a directory gave names no instrument of the book: never held
  if (!instrument) return 'markets'
  if (!holdings) return 'wait'
  const held = holdings.find((p) => p.instrument === instrument)
  return held ? { holding: held.id } : 'markets'
}
