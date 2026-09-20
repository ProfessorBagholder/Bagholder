// Presentation formatting only. The server sends raw numbers; the client formats
// them. No money math here — just display.

export function cad(n: number | null, dp = 0): string {
  if (n == null || Number.isNaN(n)) return '—'
  const sign = n < 0 ? '-' : ''
  return sign + '$' + Math.abs(n).toLocaleString('en-CA', { minimumFractionDigits: dp, maximumFractionDigits: dp })
}

export function money(n: number | null, currency: string, dp = 2): string {
  if (n == null || Number.isNaN(n)) return '—'
  const sign = n < 0 ? '-' : ''
  const body = Math.abs(n).toLocaleString('en-CA', { minimumFractionDigits: dp, maximumFractionDigits: dp })
  return sign + '$' + body + (currency && currency !== 'CAD' ? ' ' + currency : '')
}

export function pct(fraction: number | null, dp = 1): string {
  if (fraction == null || Number.isNaN(fraction)) return '—'
  return (fraction * 100).toFixed(dp) + '%'
}

export function num(n: number | null, dp = 2): string {
  if (n == null || Number.isNaN(n)) return '—'
  return n.toLocaleString('en-CA', { minimumFractionDigits: dp, maximumFractionDigits: dp })
}

// A price: two decimals at or above $1, three below, so a sub-dollar quote
// (0.135) is not rounded to $0.14.
export function price(n: number | null): string {
  if (n == null || Number.isNaN(n)) return '—'
  const sign = n < 0 ? '-' : ''
  return sign + '$' + Math.abs(n).toFixed(Math.abs(n) < 1 ? 3 : 2)
}

// A percent value the server already scaled to percent units (e.g. -3.57 → "-3.57%"),
// as opposed to pct() which scales a 0..1 fraction.
export function pctRaw(n: number | null, dp = 2): string {
  if (n == null || Number.isNaN(n)) return '—'
  return (n >= 0 ? '+' : '') + n.toFixed(dp) + '%'
}

// Distribution per-unit amounts are shown to the cent or finer as declared.
export function per(n: number | null): string {
  if (n == null || Number.isNaN(n)) return '—'
  return '$' + n.toFixed(n < 1 ? 4 : 2).replace(/0+$/, '').replace(/\.$/, '')
}
