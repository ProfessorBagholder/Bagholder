import { store, loadModel } from '../state.svelte'
import { computeVals, tick, type Ticket, type TicketAccount, type ValsCtx } from './vals'

// The order ticket's live state. Opened from a ⌘K row's Buy/Sell; the quote is
// polled while open; submit posts to /api/order. Draft is kept in localStorage so
// reopening the same symbol/side restores what was typed.

export const ticketStore = $state<{ t: Ticket | null }>({ t: null })

const POLL_MS = 5000
const DRAFT_KEYS = ['type', 'tif', 'qty', 'limit', 'stop', 'sl', 'tp'] as const
let seq = 0
let timer: ReturnType<typeof setTimeout> | undefined

// The accounts a ticket can trade in: open, self-directed, not managed/crypto.
export function ticketAccounts(): TicketAccount[] {
  return (store.model?.accounts ?? [])
    .filter((a) => a.status !== 'closed' && !/MANAGED|CRYPTO|PREDICTION/i.test(a.type))
    .map((a) => ({ id: a.id, name: a.name, margin: /MARGIN/i.test(a.type), marginAccountId: null, nav: a.nav }))
}
function nav(): number {
  return (store.model?.accounts ?? []).filter((a) => a.status !== 'closed' && a.nav != null).reduce((s, a) => s + (a.nav ?? 0), 0)
}
export function ctx(): ValsCtx {
  return { nav: nav(), accounts: ticketAccounts() }
}
export function vals() {
  return ticketStore.t ? computeVals(ticketStore.t, ctx()) : null
}

function loadDraft(symbol: string, side: string): Partial<Ticket> | null {
  try {
    const d = JSON.parse(localStorage.getItem('bh3.ticketDraft') || 'null')
    return d && d.symbol === symbol && d.side === side ? d : null
  } catch {
    return null
  }
}
function saveDraft(t: Ticket) {
  try {
    const d: Record<string, unknown> = { symbol: t.symbol, side: t.side }
    for (const k of DRAFT_KEYS) d[k] = t[k]
    localStorage.setItem('bh3.ticketDraft', JSON.stringify(d))
  } catch { /* ignore */ }
}
function dropDraft() { try { localStorage.removeItem('bh3.ticketDraft') } catch { /* ignore */ } }

export function openTicket(symbol: string, side: 'BUY' | 'SELL', exchange = '', securityId = '') {
  const accounts = ticketAccounts()
  const held = (store.model?.positions ?? []).find((p) => p.symbol === symbol && !p.short)
  let accountId = held && side === 'SELL' ? held.account : ''
  if (!accountId) accountId = (accounts.find((a) => a.margin) || accounts[0] || { id: '' }).id
  const t: Ticket = {
    step: 'form', symbol, securityId: securityId || held?.securityId || '', exchange,
    side, accountId, type: 'LIMIT', tif: 'DAY',
    qty: held && side === 'SELL' ? held.qty : 1, limit: null, stop: null,
    sl: { on: true, kind: 'stop', price: null, pct: null, priceUnit: 'amt', trail: null, unit: 'pct' },
    tp: { on: true, price: null, pct: null, unit: 'amt' },
    heldQty: held ? held.qty : null, text: {}, data: null, error: '', busy: false, submitError: '',
  }
  const d = loadDraft(symbol, side)
  if (d) for (const k of DRAFT_KEYS) if (d[k] != null) (t as unknown as Record<string, unknown>)[k] = (d as Record<string, unknown>)[k]
  ticketStore.t = t
  fetchQuote()
}

export function closeTicket(discard = false) {
  const t = ticketStore.t
  if (t) discard ? dropDraft() : saveDraft(t)
  ticketStore.t = null
  seq++
  clearTimeout(timer)
}

export async function fetchQuote() {
  const t = ticketStore.t
  if (!t) return
  const my = ++seq
  clearTimeout(timer)
  try {
    const qp = new URLSearchParams({ symbol: t.symbol, security: t.securityId || '', account: t.accountId || '', exchange: t.exchange || '' })
    const r = await fetch('/api/order/quote?' + qp.toString())
    const d = await r.json()
    const cur = ticketStore.t
    if (!cur || my !== seq) return
    if (d && d.ok) {
      cur.data = d
      cur.error = ''
      if (d.orderTypes?.length && !d.orderTypes.includes(cur.type)) cur.type = d.orderTypes.includes('LIMIT') ? 'LIMIT' : d.orderTypes[0]
    } else {
      cur.error = (d && d.error) || 'Not connected.'
    }
  } catch {
    const cur = ticketStore.t
    if (cur && my === seq) cur.error = 'Not connected.'
  }
  if (ticketStore.t && my === seq) timer = setTimeout(fetchQuote, POLL_MS)
}

export async function submit() {
  const t = ticketStore.t
  if (!t || t.busy) return
  const v = computeVals(t, ctx())
  const q = v.q
  const body = {
    symbol: t.symbol, securityId: q.securityId || t.securityId || '', accountId: t.accountId, side: t.side, type: t.type, tif: t.tif,
    quantity: v.qtyN,
    limitPrice: t.type === 'LIMIT' || t.type === 'STOP_LIMIT' ? v.limit : null,
    stopPrice: t.type === 'STOP' || t.type === 'STOP_LIMIT' ? v.stop : null,
    currency: q.currency || '',
    stopLoss: v.slOn ? { kind: t.sl.kind, price: v.slPrice, trail: v.trail, trailUnit: t.sl.unit } : null,
    takeProfit: v.tpOn ? { price: v.tpPrice } : null,
  }
  t.busy = true
  t.submitError = ''
  try {
    const r = await fetch('/api/order', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) })
    const d = await r.json()
    const cur = ticketStore.t
    if (!cur) return
    cur.busy = false
    if (!d || !d.ok) {
      cur.submitError = (d && d.error) || 'Could not submit the order.'
      return
    }
    closeTicket(true)
    loadModel()
  } catch {
    if (ticketStore.t) { ticketStore.t.busy = false; ticketStore.t.submitError = 'Could not submit the order.' }
  }
}

export { tick }
