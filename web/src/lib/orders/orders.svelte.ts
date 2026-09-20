// Orders and brackets the app placed or Wealthsimple reports as pending, polled
// while the panel is open. Ported from ledger.html's orders panel: the tabs, the
// card/leg/foot builders, the in-place editors, cancel and adjust.

import { filters } from '../filters.svelte'
import { ui, flash } from '../ui.svelte'
import { watchDoc } from '../live'
import { draftStore } from '../ticket/ticket.svelte'
import { px, money, qty as qtyFmt } from '../fmt'
import { plain } from '../ticket/vals'
import { symText } from '../sym'
import { request } from '../api'

const MON = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

export interface Order {
  id: string; createdAt: string; updatedAt?: string; accountId?: string; account: string; symbol: string; currency: string
  side: string; type: string; quantity: number; limitPrice: number | null; stopPrice: number | null
  tif: string; status: string; filledQty: number | null; avgFill: number | null; role: string; exchange: string
  parentId?: string; error?: string; securityId?: string
}
export interface Bracket {
  id: string; orderId: string; symbol: string; quantity: number
  slKind: string; slPrice: number | null; slTrail: number | null; slTrailUnit: string; slOrderId?: string; slMode?: string
  tpPrice: number | null; tpOrderId?: string
  status: string; outcome: string; error?: string; attempts?: number
  createdAt?: string; armedAt?: string; updatedAt?: string
}
export interface OrdersResp { ok: boolean; orders: Order[]; brackets: Bracket[]; live?: boolean; refreshedAt?: string }

export const ordersStore = $state<{ data: OrdersResp | null; error: string; loaded: boolean }>({ data: null, error: '', loaded: false })
export const panel = $state<{
  tab: 'pending' | 'filled' | 'cancelled'
  orderEdit: { id: string; error: string; qty: string | null; limit: string | null } | null
  bracketEdit: { id: string; error: string; sl: string | null; tp: string | null } | null
  busy: string
}>({ tab: 'pending', orderEdit: null, bracketEdit: null, busy: '' })


export const ORDER_LIVE: Record<string, number> = { sent: 1, pending: 1, cancelling: 1 }
export const BRACKET_LIVE: Record<string, number> = { waiting: 1, armed: 1, firing: 1, target_placed: 1, stopping: 1, closing: 1 }

// The orders are shown only while the panel is open, so they are sent only then: the
// whole list when it opens, and after that each order's change as it happens (a fill,
// a cancel going through, a bracket arming) written into that order's object. The
// server reads them back from Wealthsimple closely while the panel is open or an
// order is live; nothing here asks again.
let stopWatching: (() => void) | undefined
export function openOrders(): void {
  panel.orderEdit = null
  panel.bracketEdit = null
  stopWatching?.()
  const holder = {
    get data() { return ordersStore.data },
    set data(v: OrdersResp | null) {
      ordersStore.data = v
      ordersStore.loaded = true
      ordersStore.error = v && v.ok === false ? 'Could not load the orders.' : ''
    },
  }
  stopWatching = watchDoc<OrdersResp>('orders', {}, holder)
}
export function closeOrders(): void {
  stopWatching?.()
  stopWatching = undefined
}

// --- pure formatting, ported verbatim ---
export function orderPrice(o: Order): string {
  if (o.type === 'MARKET') return '—'
  if (o.type === 'STOP') return px(o.stopPrice)
  if (o.type === 'STOP_LIMIT') return 'Stop ' + px(o.stopPrice) + ' · ' + px(o.limitPrice)
  return px(o.limitPrice)
}
export function orderTypeWord(t: string): string {
  return ({ MARKET: 'Market', LIMIT: 'Limit', STOP: 'Stop', STOP_LIMIT: 'Stop limit' } as Record<string, string>)[t] || (t ? t.charAt(0) + t.slice(1).toLowerCase().replace(/_/g, ' ') : '—')
}
export function orderById(id: string): Order | null { return (ordersStore.data?.orders ?? []).find((x) => x.id === id) || null }
export function bracketOf(o: Order): Bracket | null { return (ordersStore.data?.brackets ?? []).find((b) => b.orderId === o.id) || null }
export function orderLine(o: Order): string {
  return (o.side === 'SELL' ? 'Sell ' : 'Buy ') + qtyFmt(o.quantity) + ' ' + symText(o.symbol) + ' at ' + (o.type === 'MARKET' ? 'market' : orderPrice(o) + ' ' + orderTypeWord(o.type).toLowerCase())
}
// `165.40 · Day`; a filled order names its fill instead of its type
export function orderDetailLine(o: Order): string {
  if (o.status === 'filled') return 'Filled ' + qtyFmt(o.filledQty || o.quantity) + (o.avgFill ? ' at ' + px(o.avgFill) : '')
  const tif = o.tif === 'DAY' ? 'Day' : o.tif === 'UNTIL_CANCEL' ? 'GTC' : ''
  const how = o.type === 'MARKET' ? 'at market' : o.type === 'STOP_LIMIT' ? 'stop ' + px(o.stopPrice) + ' · limit ' + px(o.limitPrice) : 'at ' + orderPrice(o) + ' ' + orderTypeWord(o.type).toLowerCase()
  return qtyFmt(o.quantity) + ' ' + how + (tif ? ' · ' + tif : '')
}
// the card's title text (side span is separate): exchange and symbol
export function orderTitle(o: Order): string { return (o.exchange ? o.exchange + ': ' : '') + symText(o.symbol) }
export function orderFillLine(o: Order): string {
  const filled = o.filledQty || 0
  if (o.status === 'rejected' || o.status === 'failed') return o.error || ''
  if (o.status !== 'filled' && filled && filled < o.quantity) return qtyFmt(filled) + ' of ' + qtyFmt(o.quantity) + ' filled' + (o.avgFill ? ' at ' + px(o.avgFill) : '')
  return ''
}
export function orderPill(o: Order): [string, string] {
  const filled = o.filledQty || 0
  switch (o.status) {
    case 'pending': return filled && filled < o.quantity ? ['Partially filled', 'accent'] : ['Pending', 'accent']
    case 'sent': return ['Pending', 'accent']
    case 'cancelling': return ['Cancelling', 'accent']
    case 'filled': return ['Filled', 'pos']
    case 'cancelled': return ['Cancelled', '']
    case 'expired': return ['Expired', '']
    case 'rejected': return ['Rejected', 'neg']
    case 'failed': return ['Failed', 'neg']
    case 'dry': return ['Not sent', '']
    default: return [o.status || '—', '']
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
// contracts × 100; shares × 1
export function orderMultiplier(o: Order): number { return /\s\d{2}[A-Z]{3}\d{2}\s[\d.]+\s(CALL|PUT)$/.test(o.symbol || '') ? 100 : 1 }
export function orderValue(o: Order): string {
  const mult = orderMultiplier(o)
  if (o.status === 'filled' && o.avgFill) return money((o.filledQty || o.quantity) * o.avgFill * mult, '', 2)
  const price = o.type === 'MARKET' ? o.avgFill : o.type === 'STOP' ? o.stopPrice : o.limitPrice
  if (!(price != null && price > 0)) return ''
  return (o.type === 'MARKET' ? '≈ ' : '') + money(o.quantity * price * mult, '', 2)
}
export function bracketEndWord(b: Bracket): [string, string] {
  if (b.status === 'failed') return ['Failed', 'neg']
  return b.outcome === 'cancelled by the user' || b.outcome === 'both legs removed' ? ['Cancelled', ''] : ['Off', '']
}
export function bracketExited(b: Bracket): boolean { return b.status === 'done' && (b.outcome === 'stopped' || b.outcome === 'target') }

export interface Leg { key: string; label: string; tone: string; line: string; amount: string; note: string }
export function bracketLegs(b: Bracket): Leg[] {
  const legs: Leg[] = []
  const all = ordersStore.data?.orders ?? []
  const entry = orderById(b.orderId)
  const mult = entry ? orderMultiplier(entry) : 1
  const stopOrder = b.slOrderId ? orderById(b.slOrderId) : null, tpOrder = b.tpOrderId ? orderById(b.tpOrderId) : null
  const done = b.status === 'done', off = b.status === 'cancelled' || b.status === 'failed'
  const cancelling = (role: string) => all.some((o) => o.parentId === b.orderId && o.role === role && o.status === 'cancelling')
  const amount = (price: number | null) => (price && b.quantity ? money(b.quantity * price * mult, '', 2) : '')
  const exited = (role: string) => all.find((o) => o.parentId === b.orderId && o.role === role && o.status === 'filled') || null
  const went = (o: Order) => ({ line: 'Filled ' + qtyFmt(o.filledQty || b.quantity) + (o.avgFill ? ' at ' + px(o.avgFill) : ''), amount: o.avgFill ? money((o.filledQty || b.quantity) * o.avgFill * mult, '', 2) : '' })
  if (b.slKind) {
    let line = qtyFmt(b.quantity) + ' at ' + px(b.slPrice) + (b.slKind === 'trail' ? ' · trailing ' + (b.slTrailUnit === 'amt' ? px(b.slTrail) : plain(b.slTrail) + '%') : '')
    let amt = amount(b.slPrice)
    let note = ''
    if (b.status === 'armed' || b.status === 'firing') note = b.slMode === 'watched' ? '' : stopOrder && stopOrder.status === 'cancelling' ? 'Cancelling' : stopOrder && ORDER_LIVE[stopOrder.status] ? '' : b.attempts ? 'Retrying · ' + (b.error || '') : 'Placing'
    else if (b.status === 'stopping') note = b.attempts ? 'Retrying · ' + (b.error || '') : 'Placing'
    else if (b.status === 'closing') note = b.outcome === 'stopped' ? 'Filled' : stopOrder || cancelling('stop') ? 'Cancelling' : 'Cancelled'
    else if (done) { const f = b.outcome === 'stopped' ? exited('stop') : null; if (f) { const w = went(f); line = w.line; amt = w.amount } else note = b.outcome === 'stopped' ? 'Filled' : 'Cancelled' }
    else if (off) note = b.status === 'failed' ? 'Failed' : 'Off'
    legs.push({ key: 'sl', label: 'Stop loss', tone: 'neg', line, amount: amt, note })
  }
  if (b.tpPrice) {
    let line = qtyFmt(b.quantity) + ' at ' + px(b.tpPrice)
    let amt = amount(b.tpPrice)
    let note = ''
    if (b.status === 'armed' || b.status === 'firing') note = b.status === 'firing' && b.attempts ? 'Retrying · ' + (b.error || '') : ''
    else if (b.status === 'target_placed') note = tpOrder && tpOrder.status === 'filled' ? 'Filled' : b.tpOrderId ? '' : b.attempts ? 'Retrying · ' + (b.error || '') : 'Placing'
    else if (b.status === 'stopping') note = 'Cancelling'
    else if (b.status === 'closing') note = b.outcome === 'target' ? 'Filled' : cancelling('target') ? 'Cancelling' : 'Cancelled'
    else if (done) { const f = b.outcome === 'target' ? exited('target') : null; if (f) { const w = went(f); line = w.line; amt = w.amount } else note = b.outcome === 'target' ? 'Filled' : 'Cancelled' }
    else if (off) note = b.status === 'failed' ? 'Failed' : 'Off'
    legs.push({ key: 'tp', label: 'Take profit', tone: 'pos', line, amount: amt, note })
  }
  return legs
}

// the accounts in scope, from the page's filter
function ordersScope(): string[] { return filters.lists.account || [] }
export function inOrdersScope(account: string): boolean { const s = ordersScope(); return !s.length || s.indexOf(account) >= 0 }
export function ordersScopeLabel(): string { const s = ordersScope(); return s.length ? s.join(', ') : 'All Accounts' }

// --- actions ---
export function editOrder(id: string) { panel.orderEdit = { id, error: '', qty: null, limit: null }; panel.bracketEdit = null }
export function cancelOrderEdit() { panel.orderEdit = null }
export function editBracket(id: string) { panel.bracketEdit = { id, error: '', sl: null, tp: null }; panel.orderEdit = null }
export function cancelBracketEdit() { panel.bracketEdit = null }

function parseField(v: string | null): number | null { if (v == null) return null; const n = parseFloat(v.replace(/[^0-9.\-]/g, '')); return isNaN(n) ? null : n }

export async function orderEditSave(id: string) {
  const o = orderById(id), e = panel.orderEdit
  if (!o || !e) return
  const quantity = e.qty != null ? parseField(e.qty) : o.quantity
  const limitPrice = e.limit != null ? parseField(e.limit) : (o.type === 'LIMIT' || o.type === 'STOP_LIMIT' ? o.limitPrice : null)
  const hasLimit = o.type === 'LIMIT' || o.type === 'STOP_LIMIT'
  if (!(quantity != null && quantity > 0)) { e.error = 'Shares must be more than zero.'; return }
  if (hasLimit && !(limitPrice != null && limitPrice > 0)) { e.error = 'A limit price is required.'; return }
  panel.busy = 'orders'; e.error = ''
  const r = await request('POST', '/api/order/modify', { id, quantity, limitPrice })
  panel.busy = ''
  if (!r || !r.ok) { if (panel.orderEdit) panel.orderEdit.error = (r && (r.error as string)) || 'Could not change the order.'; return }
  panel.orderEdit = null
  flash('Order changed · ' + orderLine({ ...o, quantity, limitPrice: limitPrice != null ? limitPrice : o.limitPrice }), 'ok', 10000)
}

export async function bracketEditSave(id: string) {
  const e = panel.bracketEdit
  const b = (ordersStore.data?.brackets ?? []).find((x) => x.id === id)
  if (!b || !e) return
  const calls: Record<string, unknown>[] = []
  if (b.slKind) {
    const v = e.sl != null ? parseField(e.sl) : (b.slKind === 'trail' ? b.slTrail : b.slPrice)
    if (!(v != null && v > 0)) { e.error = b.slKind === 'trail' ? 'A trail is required.' : 'A stop price is required.'; return }
    const cur = b.slKind === 'trail' ? b.slTrail : b.slPrice
    if (v !== cur) calls.push(b.slKind === 'trail' ? { id, leg: 'sl', trail: v } : { id, leg: 'sl', price: v })
  }
  if (b.tpPrice) {
    const v = e.tp != null ? parseField(e.tp) : b.tpPrice
    if (!(v != null && v > 0)) { e.error = 'A limit price is required.'; return }
    if (v !== b.tpPrice) calls.push({ id, leg: 'tp', price: v })
  }
  if (!calls.length) { panel.bracketEdit = null; return }
  panel.busy = 'orders'; e.error = ''
  for (const call of calls) {
    const r = await request('POST', '/api/bracket/adjust', call)
    if (!r || !r.ok) { panel.busy = ''; if (panel.bracketEdit) panel.bracketEdit.error = (r && (r.error as string)) || 'Could not change the bracket.'; return }
  }
  panel.busy = ''; panel.bracketEdit = null
  flash('Bracket changed · ' + b.symbol, 'ok', 10000)
}

export async function bracketRemove(id: string, leg: string) {
  const b = (ordersStore.data?.brackets ?? []).find((x) => x.id === id)
  if (!b) return
  panel.busy = 'orders'
  const r = await request('POST', '/api/bracket/adjust', { id, leg, remove: true })
  panel.busy = ''
  if (!r || !r.ok) { if (panel.bracketEdit) panel.bracketEdit.error = (r && (r.error as string)) || 'Could not remove the leg.'; return }
  panel.bracketEdit = null
  flash((leg === 'sl' ? 'Stop loss removed · ' : 'Take profit removed · ') + b.symbol, 'ok', 10000)
}

export async function cancelOrderNow(id: string) {
  const o = orderById(id)
  const r = await request('POST', '/api/order/cancel', { id })
  flash(r && r.ok ? 'Cancel sent · ' + (o ? orderLine(o) : '') : (r && (r.error as string)) || 'Could not cancel the order.', r && r.ok ? 'ok' : 'err', r && r.ok ? 10000 : 6000)
}
export async function cancelBracketNow(id: string) {
  const b = (ordersStore.data?.brackets ?? []).find((x) => x.id === id)
  const r = await request('POST', '/api/bracket/cancel', { id })
  flash(r && r.ok ? 'Bracket cancelled · ' + (b ? b.symbol : '') : (r && (r.error as string)) || 'Could not cancel the bracket.', r && r.ok ? 'ok' : 'err', r && r.ok ? 10000 : 6000)
}

export { draftStore }
