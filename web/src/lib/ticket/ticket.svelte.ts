import { store, loadModel } from '../state.svelte'
import type { Position } from '../model'
import { ui } from '../ui.svelte'
import { symText } from '../sym'
import { px, qty as qtyFmt } from '../fmt'
import { computeVals, tick, plain, type Ticket, type TicketAccount, type ValsCtx } from './vals'

// The order ticket's live state. Opened from a ⌘K row's Buy/Sell (or the trade
// detail); the quote is polled while open; submit posts to /api/order. A draft is
// kept in localStorage so reopening the same symbol/side restores what was typed,
// and it also surfaces as a card in the Orders panel until resumed or discarded.

export const ticketStore = $state<{ t: Ticket | null }>({ t: null })

// The set-aside draft (legacy state.ticketDraft): the Orders panel reads it to
// show the Pending draft card with Resume / Discard.
export interface TicketDraft {
  symbol: string; side: 'BUY' | 'SELL'; exchange: string; at: string
  accountId: string; type: string; tif: string
  qty: number | null; limit: number | null; stop: number | null
  sl: Ticket['sl']; tp: Ticket['tp']; text: Record<string, string | null>
}
export const draftStore = $state<{ d: TicketDraft | null }>({ d: null })

const POLL_MS = 5000
const TK_DRAFT_KEYS = ['accountId', 'type', 'tif', 'qty', 'limit', 'stop', 'sl', 'tp', 'text'] as const
let seq = 0
let timer: ReturnType<typeof setTimeout> | undefined

function api(method: string, path: string, body?: unknown): Promise<{ ok?: boolean; error?: string; [k: string]: unknown }> {
  const opts: RequestInit = { method, headers: { 'X-Bagholder': '1' } }
  if (body !== undefined) {
    ;(opts.headers as Record<string, string>)['Content-Type'] = 'application/json'
    opts.body = JSON.stringify(body)
  }
  return fetch(path, opts).then((r) => r.json()).catch((e) => ({ ok: false, error: String(e) }))
}

// what the book knows about the symbol: its listing id, kind, and the open position
function tkLookup(symbol: string): { securityId: string; kind: string; position: Position | null } {
  const m = store.model
  const pos = (m?.positions ?? []).find((p) => p.symbol === symbol) || null
  const t = pos || (m?.trades ?? []).find((x) => x.symbol === symbol) || null
  return { securityId: t ? t.securityId || '' : '', kind: t ? t.kind : '', position: pos }
}

// The accounts a ticket can route to: the server's list once the quote has answered,
// the book's own list before that, joined so an account chosen from either is never lost.
export function ticketAccounts(): TicketAccount[] {
  const t = ticketStore.t
  const out: TicketAccount[] = (t && t.data && t.data.accounts ? t.data.accounts : []).slice()
  ;(store.model?.accounts ?? [])
    .filter((a) => a.status !== 'closed' && /^SELF_DIRECTED/.test(a.type) && !/CRYPTO|PREDICTIONS|MANAGED/.test(a.type))
    .forEach((a) => {
      if (!out.some((x) => x.id === a.id)) out.push({ id: a.id, name: a.name, type: a.type, margin: /MARGIN/.test(a.type), marginAccountId: /MARGIN/.test(a.type) ? a.id : '' })
    })
  return out
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

// Max: on a Buy the whole shares the account's buying power covers at the working
// price, on a Sell the position's shares.
export function maxQty(): number | null {
  const t = ticketStore.t
  if (!t) return null
  const v = computeVals(t, ctx())
  const d = t.data || {}
  if (!v.buy) return t.heldQty != null && t.heldQty > 0 ? t.heldQty : null
  return d.buyingPower != null && v.entry != null && v.entry > 0 ? Math.max(0, Math.floor(d.buyingPower / (v.entry * v.mult))) : null
}

// the account picked last time, when it is still one the ticket offers
function tkRememberedAccount(accounts: TicketAccount[]): string {
  try {
    const id = localStorage.getItem('bh2.ticketAccount')
    if (id && accounts.some((a) => a.id === id)) return id
  } catch { /* ignore */ }
  return ''
}
function saveDraft(t: Ticket) {
  const d = { symbol: t.symbol, side: t.side, exchange: t.exchange, at: new Date().toISOString() } as Record<string, unknown>
  for (const k of TK_DRAFT_KEYS) d[k] = (t as unknown as Record<string, unknown>)[k]
  draftStore.d = d as unknown as TicketDraft
  try { localStorage.setItem('bh2.ticketDraft', JSON.stringify(d)) } catch { /* ignore */ }
}
export function dropDraft() {
  draftStore.d = null
  try { localStorage.removeItem('bh2.ticketDraft') } catch { /* ignore */ }
}
function loadDraftFromStorage() {
  try { draftStore.d = JSON.parse(localStorage.getItem('bh2.ticketDraft') || 'null') } catch { draftStore.d = null }
}
loadDraftFromStorage()

// Signature preserved: openTicket(symbol, side, exchange='', securityId='') — the
// command palette and the trade detail wire their Buy/Sell to exactly this.
export function openTicket(symbol: string, side: 'BUY' | 'SELL', exchange = '', securityId = '') {
  const info = tkLookup(symbol)
  const accounts = ticketAccounts()
  const held = info.position
  let accountId = held && side === 'SELL' ? held.accountId : '' // a sell goes where the shares are
  if (!accountId) accountId = tkRememberedAccount(accounts) || (held ? held.accountId : '')
  if (!accountId) accountId = (accounts.find((a) => a.margin) || accounts[0] || { id: '' }).id || ''
  ui.menuOpen = false
  const t: Ticket = {
    step: 'form', symbol, securityId: securityId || info.securityId, exchange,
    side: side === 'SELL' ? 'SELL' : 'BUY', accountId, type: 'LIMIT', tif: 'DAY',
    qty: held && side === 'SELL' ? held.qty : 1, limit: null, stop: null,
    sl: { on: true, kind: 'stop', price: null, pct: null, priceUnit: 'amt', trail: null, unit: 'pct' },
    tp: { on: true, price: null, pct: null, unit: 'amt' },
    heldQty: held ? held.qty : null, text: {}, data: null, error: '', busy: false, submitError: '',
  }
  const d = draftStore.d // what was typed before the ticket closed
  if (d && d.symbol === symbol && d.side === t.side) for (const k of TK_DRAFT_KEYS) if ((d as unknown as Record<string, unknown>)[k] != null) (t as unknown as Record<string, unknown>)[k] = (d as unknown as Record<string, unknown>)[k]
  ticketStore.t = t
  fetchQuote()
}

// the cross, Escape and the scrim keep the draft; Cancel and a sent order discard it
export function closeTicket(discard = false) {
  const t = ticketStore.t
  if (t) { if (discard) dropDraft(); else saveDraft(t) }
  ticketStore.t = null
  seq++
  clearTimeout(timer)
}

export async function fetchQuote() {
  const t = ticketStore.t
  if (!t) return
  const my = ++seq
  clearTimeout(timer)
  const qp = new URLSearchParams({ symbol: t.symbol, security: t.securityId || '', account: t.accountId || '', exchange: t.exchange || '' })
  const r = await api('GET', '/api/order/quote?' + qp.toString())
  const cur = ticketStore.t
  if (!cur || my !== seq) return
  if (r && r.ok) {
    cur.data = r as typeof cur.data
    cur.error = ''
    const accts = (r.accounts as TicketAccount[]) || []
    if (!cur.accountId && accts.length) cur.accountId = (accts.find((a) => a.margin) || accts[0]).id
    const ot = (r.orderTypes as string[]) || []
    if (ot.length && ot.indexOf(cur.type) < 0) cur.type = ot.indexOf('LIMIT') >= 0 ? 'LIMIT' : ot[0]
  } else {
    cur.error = (r && (r.error as string)) || 'Quote failed.'
  }
  if (ticketStore.t && my === seq) timer = setTimeout(fetchQuote, POLL_MS)
}

function notice(v: ReturnType<typeof computeVals>, sent: boolean) {
  const t = ticketStore.t!
  const head = sent ? 'Order placed · ' : 'Not sent (orders are off) · '
  return head + (v.buy ? 'Buy ' : 'Sell ') + qtyFmt(v.qtyN) + ' ' + symText(v.q.symbol || t.symbol) + ' at ' + (t.type === 'MARKET' ? 'market' : px(v.entry) + ' ' + v.typeWord.toLowerCase()) +
    (v.slOn ? ', stop ' + px(v.slPrice) : '') + (v.tpOn ? ', target ' + px(v.tpPrice) : '')
}
function flash(msg: string, ms: number) {
  ui.notice = msg
  ui.noticeKind = 'ok'
  setTimeout(() => { if (ui.notice === msg) { ui.notice = ''; ui.noticeKind = '' } }, ms)
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
  const r = await api('POST', '/api/order', body)
  const cur = ticketStore.t
  if (!cur) return
  cur.busy = false
  if (!r || !r.ok) {
    cur.submitError = (r && (r.error as string)) || 'Could not submit the order.'
    return
  }
  flash(notice(v, r.status === 'sent'), 10000) // longer than the other notices: it names the whole order
  closeTicket(true)
  loadModel()
}

// Resume a set-aside draft (from the Orders panel): reopen its ticket.
export function resumeDraft() {
  const d = draftStore.d
  if (!d) return
  openTicket(d.symbol, d.side, d.exchange, '')
}
export function discardDraft() { dropDraft() }

export { tick, plain }
