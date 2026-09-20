// Presentation formatting, ported verbatim from ledger.html's utilities so the
// Svelte build formats every figure exactly as the reference page does. The
// server sends raw numbers; the client formats them. No money math here.
//
// Conventions that matter and were previously gotten wrong:
//  - money() uses the U+2212 minus, en-US grouping, and NEVER labels currency
//    (aggregates are CAD, shown unlabelled; per-instrument figures are already
//    in the instrument's currency). The `ccy` argument is accepted for call-site
//    parity with the original and deliberately ignored.
//  - pct() is SIGNED (+/−); pctPlain() is the unsigned variant. They take a
//    fraction (0.035 → "3.5%"), not percent units.

const MON = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

export function esc(s: unknown): string {
  return String(s == null ? '' : s).replace(/[&<>"']/g, (c) => (({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }) as Record<string, string>)[c])
}

function n2(v: number, dp: number): string {
  return Number(v).toLocaleString('en-US', { minimumFractionDigits: dp, maximumFractionDigits: dp })
}

export function money(v: number | null | undefined, _ccy?: string, dp?: number): string {
  if (v == null || !isFinite(v)) return '—'
  dp = dp == null ? 2 : dp
  const sign = v < 0 ? '−' : ''
  return sign + '$' + n2(Math.abs(v), dp)
}
export function money0(v: number | null | undefined, ccy?: string): string {
  return money(v, ccy, 0)
}
export function signedMoney(v: number | null | undefined, ccy?: string, dp?: number): string {
  if (v == null || !isFinite(v)) return '—'
  return (v >= 0 ? '+' : '') + money(v, ccy, dp)
}
export function pct(v: number | null | undefined, dp?: number): string {
  if (v == null || !isFinite(v)) return '—'
  dp = dp == null ? 1 : dp
  return (v < 0 ? '−' : '+') + Math.abs(v * 100).toFixed(dp) + '%'
}
export function pctPlain(v: number | null | undefined, dp?: number): string {
  if (v == null || !isFinite(v)) return '—'
  return (v * 100).toFixed(dp == null ? 1 : dp) + '%'
}
export function qty(v: number | null | undefined): string {
  if (v == null || !isFinite(v)) return '—'
  const a = Math.abs(v)
  const dp = a % 1 ? (a < 1 ? 6 : 2) : 0
  return (v < 0 ? '−' : '') + n2(a, dp)
}
export function px(v: number | null | undefined): string {
  if (v == null || !isFinite(v)) return '—'
  const a = Math.abs(v)
  // under a dollar a third decimal only when it says something: 0.625, but 0.54
  const dp = a === 0 ? 2 : a < 0.01 ? 5 : a < 1 ? (Math.round(a * 1000) % 10 === 0 ? 2 : 3) : 2
  return n2(v, dp)
}
export function hold(d: number | null | undefined): string {
  return d == null ? '—' : Math.round(d).toLocaleString('en-US') + 'd'
}
export function stamp(iso: string | null | undefined): string {
  return iso ? MON[+iso.slice(5, 7) - 1] + " '" + iso.slice(2, 4) : ''
}
export function stampDay(iso: string | null | undefined): string {
  return iso ? +iso.slice(8, 10) + ' ' + MON[+iso.slice(5, 7) - 1] + " '" + iso.slice(2, 4) : ''
}
export function shortMoney(v: number): string {
  const a = Math.abs(v)
  return (v < 0 ? '−$' : '$') + (a >= 1000 ? (a / 1000).toFixed(1) + 'k' : String(Math.round(a)))
}
export function relTime(iso: string | null | undefined): string {
  if (!iso) return ''
  const t = Date.parse(iso)
  if (!isFinite(t)) return ''
  const m = Math.max(0, Math.round((Date.now() - t) / 60000))
  if (m < 1) return 'just now'
  if (m < 60) return m + ' min ago'
  const h = Math.round(m / 60)
  if (h < 48) return h + ' h ago'
  return Math.round(h / 24) + ' d ago'
}
// class name for a signed value: pos above zero, neg below, neither at zero
export function cls(v: number | null | undefined): string {
  return v != null && v > 0 ? 'pos' : v != null && v < 0 ? 'neg' : ''
}
// the CSS colour var for a signed value; zero reads as positive, as the page does
export function color(v: number | null | undefined): string {
  return v != null && v >= 0 ? 'var(--pos)' : 'var(--neg)'
}

// ---- deprecated shims (remove as each screen is rebuilt onto the faithful API) ----
/** @deprecated aggregates use money(); this exists only until callers migrate. */
export function cad(n: number | null, dp = 0): string {
  return money(n, undefined, dp)
}
/** @deprecated use px() */
export function price(n: number | null): string {
  return px(n)
}
/** @deprecated use n2 via qty()/px(); kept for legacy callers. */
export function num(n: number | null, dp = 2): string {
  if (n == null || Number.isNaN(n)) return '—'
  return n2(n, dp)
}
/** @deprecated the server no longer sends pre-scaled percent units to callers that matter; use pct(). */
export function pctRaw(n: number | null, dp = 2): string {
  if (n == null || Number.isNaN(n)) return '—'
  return (n >= 0 ? '+' : '−') + Math.abs(n).toFixed(dp) + '%'
}
/** @deprecated distribution per-unit amount */
export function per(n: number | null): string {
  if (n == null || Number.isNaN(n)) return '—'
  return '$' + n.toFixed(n < 1 ? 4 : 2).replace(/0+$/, '').replace(/\.$/, '')
}
