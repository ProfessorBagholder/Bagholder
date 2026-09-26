// The Orders panel (SPEC.md §4, Orders): the orders the app placed or Wealthsimple
// reports as pending, and the brackets that armed, watched while the panel is open.
// The server builds every card (rust/crates/server/src/orders): its tab, its date,
// whether it acts, its value and each leg's amount as exact decimal text, each leg's
// word. The page only writes them out in the card grammar; it works nothing out.

import { filters } from '../filters.svelte'
import { store } from '../state.svelte'
import { flash } from '../ui.svelte'
import { watchDoc } from '../live'
import { draftStore, type TicketDraft } from '../ticket/ticket.svelte'
import { px, money, qty as qtyFmt } from '../fmt'
import { plain } from '../ticket/vals'
import { symText } from '../sym'
import { call } from '../api'
import { cmp, sign, type Dec } from '../dec'

import type { BracketCard, Leg, OrderCard, OrdersDoc } from '../generated/orders'
export type { BracketCard, Leg, OrderCard, OrdersDoc } from '../generated/orders'

const MON = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

export type Tab = 'pending' | 'filled' | 'cancelled'

export const ordersStore = $state<{ data: OrdersDoc | null; error: string; loaded: boolean }>({ data: null, error: '', loaded: false })
export const panel = $state<{
  tab: Tab
  orderEdit: { id: string; error: string; qty: string | null; limit: string | null } | null
  bracketEdit: { id: string; error: string; sl: string | null; tp: string | null } | null
  busy: string
}>({ tab: 'pending', orderEdit: null, bracketEdit: null, busy: '' })

// The orders are shown only while the panel is open, so they are sent only then: the
// whole document when it opens, and after that each card's change as it happens (a
// fill, a cancel going through, a bracket arming) written into that card's object.
let stopWatching: (() => void) | undefined
export function openOrders(): void {
  panel.orderEdit = null
  panel.bracketEdit = null
  stopWatching?.()
  const holder = {
    get data() { return ordersStore.data },
    set data(v: OrdersDoc | null) {
      ordersStore.data = v
      ordersStore.loaded = true
      ordersStore.error = v && v.ok === false ? 'Could not load the orders.' : ''
    },
  }
  stopWatching = watchDoc<OrdersDoc>('orders', {}, holder)
}
export function closeOrders(): void {
  stopWatching?.()
  stopWatching = undefined
}

export function orderById(id: string): OrderCard | null { return (ordersStore.data?.orders ?? []).find((x) => x.id === id) || null }
export function bracketById(id: string): BracketCard | null { return (ordersStore.data?.brackets ?? []).find((x) => x.id === id) || null }

// --- the card grammar ---

const TIF: Record<string, string> = { day: 'Day', 'until-cancel': 'GTC' }

/** Whether the order takes a limit price: a limit or stop-limit order. */
export const hasLimit = (o: OrderCard): boolean => o.kind === 'limit' || o.kind === 'stop-limit'

// the quantity and how it works: `5 at 165.40 limit`, `5 at market`, `5 stop 1.60 · limit 1.55`
function terms(o: Pick<OrderCard, 'kind' | 'quantity' | 'limitPrice' | 'stopPrice'>): string {
  const q = qtyFmt(o.quantity)
  switch (o.kind) {
    case 'market': return q + ' at market'
    case 'stop': return q + ' at ' + px(o.stopPrice) + ' stop'
    case 'stop-limit': return q + ' stop ' + px(o.stopPrice) + ' · limit ' + px(o.limitPrice)
    default: return q + ' at ' + px(o.limitPrice) + ' limit'
  }
}

// how an order filled: `Filled 5 at 1.75`
function filledWords(quantity: Dec, average: Dec | null): string {
  return 'Filled ' + qtyFmt(quantity) + (average != null ? ' at ' + px(average) : '')
}

/**
 * Row two of an order's card: its terms and time in force (a market order's is its
 * day, not said: `5 at market`), `Cancelling` while its cancel is out; how it went
 * once filled.
 */
export function orderDetailLine(o: OrderCard): string {
  if (o.state === 'filled') return filledWords(sign(o.filled) > 0 ? o.filled : o.quantity, o.average)
  const tif = o.tif && o.kind !== 'market' ? TIF[o.tif] ?? '' : ''
  return terms(o) + (tif ? ' · ' + tif : '') + (o.state === 'cancelling' ? ' · Cancelling' : '')
}

/** The order as a sentence, for the confirm dialog and the header: `Buy 100 QNC at 1.75 limit`. */
export function orderLine(o: Pick<OrderCard, 'side' | 'symbol' | 'kind' | 'quantity' | 'limitPrice' | 'stopPrice'>): string {
  const t = terms(o)
  const sp = t.indexOf(' ')
  return (o.side === 'sell' ? 'Sell ' : 'Buy ') + t.slice(0, sp) + ' ' + symText(o.symbol) + t.slice(sp)
}

/** The listing as the card's title names it: `NYSE: QNC`, the symbol alone with no exchange stored. */
export function listing(c: { exchange: string; symbol: string }): string {
  return (c.exchange ? c.exchange + ': ' : '') + symText(c.symbol)
}

/** The value at the right of an order's card: `≈` before a market order's guessed fill. */
export function orderValue(o: OrderCard): string {
  if (o.value == null) return ''
  return (o.approx ? '≈ ' : '') + money(o.value, '', 2)
}

/**
 * The third line, only for what the second cannot say: the fill so far on a pending
 * order partly filled, the reason on a rejected or failed one.
 */
export function orderFillLine(o: OrderCard): string {
  if (o.state === 'rejected' || o.state === 'failed') return o.why ?? ''
  if (o.tab === 'pending' && sign(o.filled) > 0 && cmp(o.filled, o.quantity) < 0)
    return qtyFmt(o.filled) + ' of ' + qtyFmt(o.quantity) + ' filled' + (o.average != null ? ' at ' + px(o.average) : '')
  return ''
}

/** Written and sent, Wealthsimple's answer not read yet (SPEC.md §4, Orders, Status). */
export const orderUnconfirmed = (o: OrderCard): boolean => o.state === 'sending' || o.state === 'unconfirmed'
export const SENT_WORD = 'Sent · not confirmed'

/** A finished order's state word at the right of its foot on the Cancelled tab, and whether it is red. */
export function orderEndWord(o: OrderCard): [string, boolean] {
  switch (o.state) {
    case 'cancelled': return ['Cancelled', false]
    case 'expired': return ['Expired', false]
    case 'rejected': return ['Rejected', true]
    case 'failed': return ['Failed', true]
    case 'dry': return ['Not sent', false]
    default: return ['', false]
  }
}

export function orderWhenWord(iso: string | undefined): string {
  const t = Date.parse(iso || '')
  if (!isFinite(t)) return '—'
  const d = new Date(t), now = new Date()
  const h = d.getHours() % 12 || 12, m = String(d.getMinutes()).padStart(2, '0'), ap = d.getHours() < 12 ? 'AM' : 'PM'
  const time = h + ':' + m + ' ' + ap
  if (d.toDateString() === now.toDateString()) return 'Today ' + time
  return MON[d.getMonth()] + ' ' + d.getDate() + (d.getFullYear() !== now.getFullYear() ? ' ' + d.getFullYear() : '') + ', ' + time
}

/** A leg row as a card shows it (SPEC.md §4, Orders, Bracket rows). */
export interface LegRow { key: string; label: string; tone: string; line: string; note: string; amount: string }
export function legRow(l: Leg): LegRow {
  const sl = l.key === 'sl'
  const trail = l.trailPct != null ? ' · trailing ' + plain(l.trailPct) + '%' : l.trailAmount != null ? ' · trailing ' + px(l.trailAmount) : ''
  return {
    key: l.key,
    label: sl ? 'Stop loss' : 'Take profit',
    tone: sl ? 'neg' : 'pos',
    line: l.filled ? filledWords(l.filled.quantity, l.filled.average) : qtyFmt(l.quantity) + ' at ' + px(l.level) + trail,
    note: l.note,
    amount: money(l.amount, '', 2),
  }
}

/** A bracket's editor: the stop leg's field (its label and starting text), the target's starting text. */
export function bracketEditor(b: BracketCard): { sl: { label: string; start: string; field: 'price' | 'trail'; cur: Dec } | null; tp: { start: string; cur: Dec } | null } {
  const sl = b.trailAmount != null ? { label: 'Trail', start: px(b.trailAmount), field: 'trail' as const, cur: b.trailAmount }
    : b.trailPct != null ? { label: 'Trail %', start: plain(b.trailPct), field: 'trail' as const, cur: b.trailPct }
    : b.stopLevel != null ? { label: 'Stop price', start: px(b.stopLevel), field: 'price' as const, cur: b.stopLevel }
    : null
  return { sl, tp: b.target != null ? { start: px(b.target), cur: b.target } : null }
}

/** The cards of a tab in the accounts in scope, newest first together. */
export type Card = { kind: 'order'; at: string; o: OrderCard } | { kind: 'bracket'; at: string; b: BracketCard }
export function tabCards(doc: OrdersDoc, tab: Tab): Card[] {
  const cards: Card[] = [
    ...doc.orders.filter((o) => o.tab === tab && inOrdersScope(o.account)).map((o) => ({ kind: 'order' as const, at: o.at, o })),
    ...doc.brackets.filter((b) => b.tab === tab && inOrdersScope(b.account)).map((b) => ({ kind: 'bracket' as const, at: b.at, b })),
  ]
  const when = (c: Card) => Date.parse(c.at) || 0
  return cards.sort((a, c) => when(c) - when(a))
}

/**
 * The draft card (SPEC.md §4, Orders, Pending): its words from what the draft holds,
 * its amount and its legs' levels and amounts the server's, from the preview the
 * ticket last had for it. A draft with no preview shows no figure.
 */
export function draftCard(d: TicketDraft): { d: TicketDraft; line: string; legs: { label: string; tone: string; value: string; amount: string }[]; value: string } {
  const p = d.preview ?? null
  const buy = d.side !== 'SELL', market = d.type === 'MARKET'
  const qty = p ? p.quantity : d.qty
  const tif = d.tif === 'DAY' ? 'Day' : 'GTC'
  const how = market ? 'at market'
    : d.type === 'STOP_LIMIT' ? 'stop ' + px(p ? p.stop : d.stop) + ' · limit ' + px(p ? p.limit : d.limit)
    : 'at ' + px(p ? p.entry : d.type === 'STOP' ? d.stop : d.limit) + ' ' + (d.type === 'STOP' ? 'stop' : 'limit')
  const line = (buy ? 'Buy ' : 'Sell ') + qtyFmt(qty) + ' ' + how + (market ? '' : ' · ' + tif)
  const legs: { label: string; tone: string; value: string; amount: string }[] = []
  if (p && p.stopLossOn && p.stopLossPrice != null) {
    const trail = p.trailing ? ' · trailing ' + (d.sl.unit === 'pct' ? plain(p.trail) + '%' : px(p.trail)) : ''
    legs.push({ label: 'Stop loss', tone: 'neg', value: qtyFmt(p.quantity) + ' at ' + px(p.stopLossPrice) + trail, amount: p.stopLossValue != null ? money(p.stopLossValue, '', 2) : '' })
  }
  if (p && p.takeProfitOn && p.takeProfitPrice != null) {
    legs.push({ label: 'Take profit', tone: 'pos', value: qtyFmt(p.quantity) + ' at ' + px(p.takeProfitPrice), amount: p.takeProfitValue != null ? money(p.takeProfitValue, '', 2) : '' })
  }
  // the order's amount where the draft holds a price: none for a market order
  return { d, line, legs, value: !market && p && p.notional != null ? money(p.notional, '', 2) : '' }
}

// the accounts in scope, from the page's filter, which names each by its id
function ordersScope() { const on = filters.lists.account; return (store.model?.accounts ?? []).filter((a) => on.includes(a.id)) }
// a card names its account by the broker's id for it
export function inOrdersScope(brokerAccount: string): boolean { return !filters.lists.account.length || ordersScope().some((a) => a.brokerAccount === brokerAccount) }
export function ordersScopeLabel(): string { const s = ordersScope(); return s.length ? s.map((a) => a.name).join(', ') : 'All Accounts' }

// --- actions ---
export function editOrder(id: string) { panel.orderEdit = { id, error: '', qty: null, limit: null }; panel.bracketEdit = null }
export function cancelOrderEdit() { panel.orderEdit = null }
export function editBracket(id: string) { panel.bracketEdit = { id, error: '', sl: null, tp: null }; panel.orderEdit = null }
export function cancelBracketEdit() { panel.bracketEdit = null }

const DEC_TEXT = /^\d+(\.\d+)?$/
/**
 * What was typed into an editor's box as the decimal text the server reads: the
 * formatting a figure is shown with (`$`, grouping commas, spaces) taken off. Text
 * that is not a positive decimal then is none.
 */
export function typedDec(v: string): Dec | null {
  const t = v.replace(/[$,\s]/g, '')
  return DEC_TEXT.test(t) && sign(t as Dec) > 0 ? (t as Dec) : null
}

export async function orderEditSave(id: string) {
  const o = orderById(id), e = panel.orderEdit
  if (!o || !e) return
  const quantity = e.qty != null ? typedDec(e.qty) : o.quantity
  const limitPrice = !hasLimit(o) ? null : e.limit != null ? typedDec(e.limit) : o.limitPrice
  if (quantity == null) { e.error = 'Shares must be more than zero.'; return }
  if (hasLimit(o) && limitPrice == null) { e.error = 'A limit price is required.'; return }
  panel.busy = 'orders'; e.error = ''
  const r = await call('POST /api/order/modify', { body: { id, quantity, limitPrice } })
  panel.busy = ''
  if (!r || !r.ok) { if (panel.orderEdit) panel.orderEdit.error = (r && r.error) || 'Could not change the order.'; return }
  panel.orderEdit = null
  flash('Order changed · ' + orderLine({ ...o, quantity, limitPrice: limitPrice ?? o.limitPrice }), 'ok', 10000)
}

export async function bracketEditSave(id: string) {
  const e = panel.bracketEdit
  const b = bracketById(id)
  if (!b || !e) return
  const ed = bracketEditor(b)
  const calls: { id: string; leg: string; price: Dec | null; trail: Dec | null }[] = []
  if (ed.sl) {
    const v = e.sl != null ? typedDec(e.sl) : ed.sl.cur
    if (v == null) { e.error = ed.sl.field === 'trail' ? 'A trail is required.' : 'A stop price is required.'; return }
    if (cmp(v, ed.sl.cur) !== 0) calls.push(ed.sl.field === 'trail' ? { id, leg: 'sl', price: null, trail: v } : { id, leg: 'sl', price: v, trail: null })
  }
  if (ed.tp) {
    const v = e.tp != null ? typedDec(e.tp) : ed.tp.cur
    if (v == null) { e.error = 'A limit price is required.'; return }
    if (cmp(v, ed.tp.cur) !== 0) calls.push({ id, leg: 'tp', price: v, trail: null })
  }
  if (!calls.length) { panel.bracketEdit = null; return }
  panel.busy = 'orders'; e.error = ''
  for (const body of calls) {
    const r = await call('POST /api/bracket/adjust', { body })
    if (!r || !r.ok) { panel.busy = ''; if (panel.bracketEdit) panel.bracketEdit.error = (r && r.error) || 'Could not change the bracket.'; return }
  }
  panel.busy = ''; panel.bracketEdit = null
  flash('Bracket changed · ' + symText(b.symbol), 'ok', 10000)
}

export async function bracketRemove(id: string, leg: string) {
  const b = bracketById(id)
  if (!b) return
  panel.busy = 'orders'
  const r = await call('POST /api/bracket/adjust', { body: { id, leg, price: null, trail: null, remove: true } })
  panel.busy = ''
  if (!r || !r.ok) { if (panel.bracketEdit) panel.bracketEdit.error = (r && r.error) || 'Could not remove the leg.'; return }
  panel.bracketEdit = null
  flash((leg === 'sl' ? 'Stop loss removed · ' : 'Take profit removed · ') + symText(b.symbol), 'ok', 10000)
}

export async function cancelOrderNow(id: string) {
  const o = orderById(id)
  const r = await call('POST /api/order/cancel', { body: { id } })
  flash(r && r.ok ? 'Cancel sent · ' + (o ? orderLine(o) : '') : (r && r.error) || 'Could not cancel the order.', r && r.ok ? 'ok' : 'err', r && r.ok ? 10000 : 6000)
}
export async function cancelBracketNow(id: string) {
  const b = bracketById(id)
  const r = await call('POST /api/bracket/cancel', { body: { id } })
  flash(r && r.ok ? 'Bracket cancelled · ' + (b ? symText(b.symbol) : '') : (r && r.error) || 'Could not cancel the bracket.', r && r.ok ? 'ok' : 'err', r && r.ok ? 10000 : 6000)
}

export { draftStore }
