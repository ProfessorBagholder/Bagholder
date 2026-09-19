// Presentation formatting only. The server sends raw numbers; the client formats
// them. No money math here — just display.

export function cad(n: number, dp = 0): string {
  if (n == null || Number.isNaN(n)) return '—'
  const sign = n < 0 ? '-' : ''
  return sign + '$' + Math.abs(n).toLocaleString('en-CA', { minimumFractionDigits: dp, maximumFractionDigits: dp })
}

export function money(n: number, currency: string, dp = 2): string {
  if (n == null || Number.isNaN(n)) return '—'
  const sign = n < 0 ? '-' : ''
  const body = Math.abs(n).toLocaleString('en-CA', { minimumFractionDigits: dp, maximumFractionDigits: dp })
  return sign + '$' + body + (currency && currency !== 'CAD' ? ' ' + currency : '')
}

export function pct(fraction: number, dp = 1): string {
  if (fraction == null || Number.isNaN(fraction)) return '—'
  return (fraction * 100).toFixed(dp) + '%'
}

export function num(n: number, dp = 2): string {
  if (n == null || Number.isNaN(n)) return '—'
  return n.toLocaleString('en-CA', { minimumFractionDigits: dp, maximumFractionDigits: dp })
}

// Distribution per-unit amounts are shown to the cent or finer as declared.
export function per(n: number | null): string {
  if (n == null || Number.isNaN(n)) return '—'
  return '$' + n.toFixed(n < 1 ? 4 : 2).replace(/0+$/, '').replace(/\.$/, '')
}
