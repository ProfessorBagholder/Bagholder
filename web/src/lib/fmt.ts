import { abs, absBelow, digits, sign, waits, type Dec, type Fig, type Waits } from './dec'

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

// Times arrive as UTC instants and are shown in the viewer's own zone (SPEC:
// "Intraday bars and execution times are shown in the viewer's local time").
const pad2 = (n: number) => String(n).padStart(2, '0')

/** The viewer's calendar day, `YYYY-MM-DD`. */
export function localDay(d: Date = new Date()): string {
  return d.getFullYear() + '-' + pad2(d.getMonth() + 1) + '-' + pad2(d.getDate())
}

/** An instant as the viewer's day and `HH:MM`; a bare date (no time was
 *  recorded) keeps its day and has no time. */
export function localWhen(when: string | null, date: string): { day: string; time: string } {
  const t = when?.includes('T') ? Date.parse(when) : NaN
  if (!isFinite(t)) return { day: date, time: '' }
  const d = new Date(t)
  return { day: localDay(d), time: pad2(d.getHours()) + ':' + pad2(d.getMinutes()) }
}

export function esc(s: unknown): string {
  return String(s == null ? '' : s).replace(/[&<>"']/g, (c) => (({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }) as Record<string, string>)[c])
}

function n2(v: number, dp: number): string {
  return Number(v).toLocaleString('en-US', { minimumFractionDigits: dp, maximumFractionDigits: dp })
}

// What a figure waits on, as the one short word the page shows after its dash
// (SPEC.md §3, Missing): the engine's gap words, each a thing the figure needs.
const WAITS_FOR: Record<string, string> = {
  'rate-pending': 'rate', 'rate-missing': 'rate', 'rate-unpublished': 'rate', 'rate-not-held': 'rate',
  'multiplier-unstated': 'size', 'quantity-unstated': 'qty', 'leg-unstated': 'leg', 'price-unknown': 'price',
  'basis-unknown': 'cost', 'event-unknown': 'event', 'event-on-short': 'event', 'no-expiry-record': 'expiry',
  'adjustment-conflict': 'record', 'effect-conflict': 'record', 'beyond-held': 'record', 'record-problem': 'record',
  'currency-unstated': 'currency', 'value-unstated': 'value', 'payer-not-read': 'payer', 'no-distribution-yet': 'unpaid',
  'schedule-unstated': 'schedule', 'form-unstated': 'form', 'buying-power-unread': 'unread', arithmetic: 'overflow',
}

/** Every gap word the page has a word for (a test holds this to the engine's list). */
export const WAITS_WORDS = Object.keys(WAITS_FOR)

/** The dash and word a figure that waits is shown as. */
export function waiting(f: Waits): string {
  const g = f.gaps[0] ?? ''
  const w = WAITS_FOR[g] ?? WAITS_FOR[g.split(':')[0]] ?? ''
  return w ? '— ' + w : '—'
}

type Amount = Fig<Dec> | number | null | undefined

/** A total's second line: how many it left out because their figures wait (SPEC §1). */

/** A money amount: U+2212 minus, en-US grouping, never a currency label. Exact text is written as it states. */
export function money(v: Amount, _ccy?: string, dp?: number): string {
  if (v == null) return '—'
  if (waits(v)) return waiting(v)
  dp = dp == null ? 2 : dp
  if (typeof v === 'number') {
    if (!isFinite(v)) return '—'
    return (v < 0 ? '−' : '') + '$' + n2(Math.abs(v), dp)
  }
  return (sign(v) < 0 ? '−' : '') + '$' + digits(v, dp)
}
export function money0(v: Amount, ccy?: string): string {
  return money(v, ccy, 0)
}
export function signedMoney(v: Amount, ccy?: string, dp?: number): string {
  if (v == null) return '—'
  if (waits(v)) return waiting(v)
  const pos = typeof v === 'number' ? v >= 0 : sign(v) >= 0
  return (pos ? '+' : '') + money(v, ccy, dp)
}
export function pct(v: number | null | undefined | Waits, dp?: number): string {
  if (v == null) return '—'
  if (waits(v)) return waiting(v)
  if (!isFinite(v)) return '—'
  dp = dp == null ? 1 : dp
  return (v < 0 ? '−' : '+') + Math.abs(v * 100).toFixed(dp) + '%'
}
/** A difference of two percentages, signed, in percentage points: `+4.2 pts`. */
export function pts(v: number | null | undefined): string {
  if (v == null || !isFinite(v)) return '—'
  return (v < 0 ? '−' : '+') + Math.abs(v * 100).toFixed(1) + ' pts'
}
export function pctPlain(v: number | null | undefined | Waits, dp?: number): string {
  if (v == null) return '—'
  if (waits(v)) return waiting(v)
  if (!isFinite(v)) return '—'
  return (v * 100).toFixed(dp == null ? 1 : dp) + '%'
}
/** A quantity: whole units bare, a fraction under one to six places, else two. */
export function qty(v: Amount): string {
  if (v == null) return '—'
  if (waits(v)) return waiting(v)
  if (typeof v === 'number') {
    if (!isFinite(v)) return '—'
    const a = Math.abs(v)
    const dp = a % 1 ? (a < 1 ? 6 : 2) : 0
    return (v < 0 ? '−' : '') + n2(a, dp)
  }
  const whole = !v.includes('.') || /\.0*$/.test(v)
  const dp = whole ? 0 : absBelow(v, '1') ? 6 : 2
  return (sign(v) < 0 ? '−' : '') + digits(v, dp)
}
/** A price: two places; under a dollar a third only when it says something; under a cent five. */
export function px(v: Amount): string {
  if (v == null) return '—'
  if (waits(v)) return waiting(v)
  if (typeof v === 'number') {
    if (!isFinite(v)) return '—'
    const a = Math.abs(v)
    const dp = a === 0 ? 2 : a < 0.01 ? 5 : a < 1 ? (Math.round(a * 1000) % 10 === 0 ? 2 : 3) : 2
    return n2(v, dp)
  }
  const s = sign(v)
  const dp = s === 0 ? 2 : absBelow(v, '0.01') ? 5 : absBelow(v, '1') ? (digits(v, 3).endsWith('0') ? 2 : 3) : 2
  return (s < 0 ? '-' : '') + digits(v, dp)
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
/** A dollar amount in thousands, one place (`$18.3k`), else whole dollars: the chart labels. */
export function shortMoney(v: Amount): string {
  if (v == null) return '—'
  if (waits(v)) return waiting(v)
  if (typeof v === 'number') {
    const a = Math.abs(v)
    return (v < 0 ? '−$' : '$') + (a >= 1000 ? (a / 1000).toFixed(1) + 'k' : String(Math.round(a)))
  }
  const neg = sign(v) < 0
  const a = abs(v)
  // thousands by moving the point, exactly
  const [i, f = ''] = a.split('.')
  const k = (i.length > 3 ? i.slice(0, -3) + '.' + i.slice(-3) + f : null) as Dec | null
  return (neg ? '−$' : '$') + (k ? digits(k, 1).replace(/,/g, ',') + 'k' : digits(a, 0))
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
export function cls(v: Amount): string {
  const s = v == null || waits(v) ? 0 : typeof v === 'number' ? Math.sign(v) : sign(v)
  return s > 0 ? 'pos' : s < 0 ? 'neg' : ''
}
// the CSS colour var for a signed value; zero reads as positive, as the page does
export function color(v: Amount): string {
  const s = v == null || waits(v) ? null : typeof v === 'number' ? Math.sign(v) : sign(v)
  return s != null && s >= 0 ? 'var(--pos)' : 'var(--neg)'
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
