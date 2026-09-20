// A symbol as the app shows it everywhere: the bare ticker, a contract's
// underlying bared too (QNC.TO 20NOV26 3.00 CALL reads QNC 20NOV26 3.00 CALL).
// Ported verbatim from ledger.html.
export const bareSymbol = (s: unknown): string => String(s || '').toUpperCase().replace(/\.(TO|V|CN|NE)$/, '')
export const symText = (s: unknown): string => {
  const str = String(s || '')
  const m = str.match(/^(\S+)(.*)$/s)
  return m ? bareSymbol(m[1]) + m[2] : str
}
