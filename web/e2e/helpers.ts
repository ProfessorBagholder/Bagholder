import { expect, type APIRequestContext, type Page } from '@playwright/test'

/** The page has its model and has finished arriving: the shell alone is there before that. */
export async function ready(page: Page): Promise<void> {
  await expect(page.locator('#page > [data-arrived]')).toBeVisible()
}

/**
 * The figures document the page is sent, under `filters` (none by default): money,
 * quantities and prices as exact decimal text, a figure that waits as `{ gaps }`,
 * accounts and instruments by their ids (web/src/lib/generated/figures.ts).
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export async function figures(request: APIRequestContext, filters?: Record<string, unknown>): Promise<any> {
  const r = await request.get('/api/figures' + (filters ? '?filters=' + encodeURIComponent(JSON.stringify(filters)) : ''))
  expect(r.ok(), await r.text()).toBeTruthy()
  return r.json()
}

/**
 * The model document as the stream sends it: the figures, with the header's status
 * beside them (the server's own status, less the versions only the stream reads).
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export async function modelDoc(request: APIRequestContext, status: Record<string, unknown> = {}): Promise<any> {
  const model = await figures(request)
  const { dataVersion: _d, coreVersion: _c, ...stated } = await (await request.get('/api/status')).json()
  model.status = { ...stated, ...status }
  return model
}

/** A stream body that sends `model` as the whole view, then each of `docs` as its own document. */
export function streamBody(model: unknown, docs: Record<string, unknown> = {}, tail = ''): string {
  const others = Object.entries(docs).map(([doc, data]) => `event: snapshot\ndata: ${JSON.stringify({ doc, data })}\n\n`).join('')
  return `retry: 200\nevent: hello\ndata: {"id":1}\n\nevent: snapshot\ndata: ${JSON.stringify({ doc: 'model', data: model })}\n\n${others}${tail}`
}

/**
 * Open the page on the real book, with the header's status changed as given. The
 * server's own stream is stood in for by one snapshot: what a status the test cannot
 * bring about for real (an update on offer, a server of another protocol) looks like.
 * `docs` are documents (the orders, a listing's filings) sent the same way.
 */
export async function openWithStatus(
  page: Page,
  request: APIRequestContext,
  status: Record<string, unknown>,
  hash = '',
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  change: (model: any) => void = () => {},
  docs: Record<string, unknown> = {},
): Promise<void> {
  const model = await modelDoc(request, status)
  change(model)
  // a document is sent with every (re)connection, so one the page asks for later reaches it on the next
  const body = streamBody(model, docs)
  await page.route('**/api/events?*', (route) => route.fulfill({ status: 200, contentType: 'text/event-stream', body }))
  await page.goto('/' + hash)
}

// ---------------------------------------------------------------------------
// Local mirrors of the page's formatting (SPEC.md §3), so a figure on screen is
// checked exactly against the document's own decimal text, without importing app
// source into the tests (they never do). A Dec is formatted from its text by
// Intl.NumberFormat, which formats the exact value a string states, never a float.
// ---------------------------------------------------------------------------

/** A figure that waits: the gaps it waits on. */
export type Waits = { gaps: string[] }
/** Exact decimal text, a figure that waits, or nothing. */
export type Amount = string | Waits | null | undefined

export function waits(v: unknown): v is Waits {
  return typeof v === 'object' && v !== null && Array.isArray((v as Waits).gaps)
}

// the one word a waiting figure shows after its dash (SPEC.md §3, Missing)
const WAITS_FOR: Record<string, string> = {
  'rate-pending': 'rate', 'rate-missing': 'rate', 'rate-unpublished': 'rate', 'rate-not-held': 'rate',
  'multiplier-unstated': 'size', 'quantity-unstated': 'qty', 'leg-unstated': 'leg', 'price-unknown': 'price',
  'basis-unknown': 'cost', 'event-unknown': 'event', 'event-on-short': 'event', 'no-expiry-record': 'expiry',
  'adjustment-conflict': 'record', 'effect-conflict': 'record', 'beyond-held': 'record', 'record-problem': 'record',
  'currency-unstated': 'currency', 'value-unstated': 'value', 'payer-not-read': 'payer', 'no-distribution-yet': 'unpaid',
  'schedule-unstated': 'schedule', 'form-unstated': 'form', 'buying-power-unread': 'unread', arithmetic: 'overflow',
}
export function waiting(f: Waits): string {
  const g = f.gaps[0] ?? ''
  const w = WAITS_FOR[g] ?? WAITS_FOR[g.split(':')[0]] ?? ''
  return w ? '— ' + w : '—'
}

const DEC = /^-?\d+(\.\d+)?$/
function dec(d: string): string {
  if (!DEC.test(d)) throw new Error('not a decimal: ' + JSON.stringify(d))
  return d
}
const isZero = (d: string) => /^-?0+(\.0*)?$/.test(d)
/** -1, 0 or 1, exactly. */
export function sign(d: string): -1 | 0 | 1 {
  dec(d)
  if (isZero(d)) return 0
  return d.startsWith('-') ? -1 : 1
}
const absText = (d: string) => (d.startsWith('-') ? d.slice(1) : d)
/** `|d|` with `min`..`max` decimals and en-US grouping, from the exact text. */
export function digits(d: string, min: number, max = min): string {
  return new Intl.NumberFormat('en-US', { minimumFractionDigits: min, maximumFractionDigits: max }).format(absText(dec(d)) as unknown as number)
}
/** Whether `|d|` is below `limit`, exactly. */
function absBelow(d: string, limit: string): boolean {
  return cmp(absText(d), limit) < 0
}
/** The order of two decimals, exactly (never through a float). */
export function cmp(a: string, b: string): number {
  const sa = sign(a), sb = sign(b)
  if (sa !== sb) return sa < sb ? -1 : 1
  const [ai, af = ''] = absText(a).split('.')
  const [bi, bf = ''] = absText(b).split('.')
  const i1 = ai.replace(/^0+(?=\d)/, ''), i2 = bi.replace(/^0+(?=\d)/, '')
  const n = Math.max(af.length, bf.length)
  const f1 = af.padEnd(n, '0'), f2 = bf.padEnd(n, '0')
  const m = i1.length !== i2.length ? (i1.length < i2.length ? -1 : 1) : i1 !== i2 ? (i1 < i2 ? -1 : 1) : f1 === f2 ? 0 : f1 < f2 ? -1 : 1
  return sa < 0 ? -m : m
}

/** Money: `$`, grouping, `−` for a negative, never a currency label. */
export function money(v: Amount, dp = 2): string {
  if (v == null) return '—'
  if (waits(v)) return waiting(v)
  return (sign(v) < 0 ? '−' : '') + '$' + digits(v, dp)
}
export const money0 = (v: Amount) => money(v, 0)
/** Signed money: a leading `+` when zero or positive. */
export function signedMoney(v: Amount, dp = 2): string {
  if (v == null) return '—'
  if (waits(v)) return waiting(v)
  return (sign(v) >= 0 ? '+' : '') + money(v, dp)
}
/** Quantity: whole units bare, under one six places, else two. */
export function qty(v: Amount): string {
  if (v == null) return '—'
  if (waits(v)) return waiting(v)
  const whole = !v.includes('.') || /\.0*$/.test(v)
  const dp = whole ? 0 : absBelow(v, '1') ? 6 : 2
  return (sign(v) < 0 ? '−' : '') + digits(v, dp)
}
/** Price: two places; under a dollar a third only when it is not zero; five under a cent. */
export function px(v: Amount): string {
  if (v == null) return '—'
  if (waits(v)) return waiting(v)
  const s = sign(v)
  const dp = s === 0 ? 2 : absBelow(v, '0.01') ? 5 : absBelow(v, '1') ? (digits(v, 3).endsWith('0') ? 2 : 3) : 2
  return (s < 0 ? '-' : '') + digits(v, dp)
}
/** A distribution per unit: `$`, four places under $1, else two. */
export function perUnit(v: Amount): string {
  if (v == null) return '—'
  if (waits(v)) return waiting(v)
  return money(v, absBelow(v, '1') ? 4 : 2)
}
/** A ratio as a signed percentage (P&L). */
export function pct(v: number | null | undefined | Waits, dp = 1): string {
  if (v == null) return '—'
  if (waits(v)) return waiting(v)
  return (v < 0 ? '−' : '+') + Math.abs(v * 100).toFixed(dp) + '%'
}
/** A ratio as an unsigned percentage (a rate). */
export function pctPlain(v: number | null | undefined | Waits, dp = 1): string {
  if (v == null) return '—'
  if (waits(v)) return waiting(v)
  return (v * 100).toFixed(dp) + '%'
}
export const hold = (d: number | null | undefined) => (d == null ? '—' : Math.round(d).toLocaleString('en-US') + 'd')
/** A total's second line: how many it left out because their figures wait. */

const MON = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']
export const stamp = (iso: string) => MON[+iso.slice(5, 7) - 1] + " '" + iso.slice(2, 4)
export const stampDay = (iso: string) => +iso.slice(8, 10) + ' ' + stamp(iso)

/** The bare ticker (SPEC.md §3, Symbol): a listing's suffix dropped, a contract's underlying's too. */
export const bareSymbol = (s: string) => s.toUpperCase().replace(/\.(TO|V|CN|NE)$/, '')
export const symText = (s: string) => {
  const m = s.match(/^(\S+)(.*)$/s)
  return m ? bareSymbol(m[1]) + m[2] : s
}

/** A hash address of a sub page, as the page writes it: ids can hold ':' and '@', which stay readable (SPEC.md §4 Markets). */
export function subUrl(tab: string, id: string): RegExp {
  const readable = encodeURIComponent(id).replace(/%3A/gi, ':').replace(/%40/gi, '@')
  return new RegExp('#' + tab + '/' + readable.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') + '$')
}

/**
 * The Trades list in its default order (SPEC.md "### Trades"): newest activity first, an
 * open trade by its latest fill and a closed one by its close (`lastDate`), ties in the
 * document's order.
 */
export function tradesInListOrder<T extends { lastDate: string }>(trades: T[]): T[] {
  return trades.map((t, i) => ({ t, i })).sort((a, b) => (a.t.lastDate < b.t.lastDate ? 1 : a.t.lastDate > b.t.lastDate ? -1 : a.i - b.i)).map((x) => x.t)
}
