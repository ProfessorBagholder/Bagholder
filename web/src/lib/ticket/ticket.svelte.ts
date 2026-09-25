import { store } from '../state.svelte'
import type { Position } from '../model'
import { ui, flash } from '../ui.svelte'
import { watchDoc } from '../live'
import { symText } from '../sym'
import { px, qty as qtyFmt } from '../fmt'
import { view, previewRequest, plain, type Ticket, type TicketAccount, type ValsCtx } from './vals'
import type { Preview } from '../generated/orders'
import { call } from '../api'
import { ticketNumber } from '../dec'

// The order ticket's live state. Opened from a ⌘K row's Buy/Sell (or the trade
// detail); the quote is polled while open; submit posts to /api/order. A draft is
// kept in localStorage so reopening the same symbol/side restores what was typed,
// and it also surfaces as a card in the Orders panel until resumed or discarded.

export const ticketStore = $state<{ t: Ticket | null; preview: Preview | null; previewError: string }>({ t: null, preview: null, previewError: '' })

// The set-aside draft (legacy state.ticketDraft): the Orders panel reads it to
// show the Pending draft card with Resume / Discard.
export interface TicketDraft {
  symbol: string; side: 'BUY' | 'SELL'; exchange: string; at: string
  accountId: string; type: string; tif: string
  qty: number | null; limit: number | null; stop: number | null
  sl: Ticket['sl']; tp: Ticket['tp']; text: Record<string, string | null>
}
export const draftStore = $state<{ d: TicketDraft | null }>({ d: null })

const TK_DRAFT_KEYS = ['accountId', 'type', 'tif', 'qty', 'limit', 'stop', 'sl', 'tp', 'text'] as const


// what the book knows about the listing the broker names `securityId` (or, with none
// given, the one symbol it holds by that text): its kind and the open position
function tkLookup(symbol: string, securityId: string): { securityId: string; kind: string; position: Position | null } {
  const m = store.model
  const is = (x: { security: string; symbol: string }) => (securityId ? x.security === securityId : x.symbol === symbol)
  // held in more than one account: the margin account among them, else the first by name
  const accounts = m?.accounts ?? []
  const rank = (accountId: string) => {
    const a = accounts.find((x) => x.id === accountId)
    return [a?.margin ? 0 : 1, a?.name ?? ''] as const
  }
  const pos =
    (m?.positions ?? [])
      .filter(is)
      .sort((a, b) => {
        const [x, y] = [rank(a.accountId), rank(b.accountId)]
        return x[0] - y[0] || x[1].localeCompare(y[1])
      })[0] || null
  const t = pos || (m?.trades ?? []).find(is) || null
  return { securityId: t ? t.security : securityId, kind: t ? t.kind : '', position: pos }
}

// an account as an order names it: by the broker's own id for it
function brokerAccount(id: string): string {
  return (store.model?.accounts ?? []).find((a) => a.id === id)?.brokerAccount ?? ''
}

// The accounts a ticket can route to: the server's list once the quote has answered,
// the book's own list before that, joined so an account chosen from either is never lost.
export function ticketAccounts(): TicketAccount[] {
  const t = ticketStore.t
  const out: TicketAccount[] = (t && t.data && t.data.accounts ? t.data.accounts : []).slice()
  ;(store.model?.accounts ?? [])
    .filter((a) => a.status !== 'closed' && a.tradable && a.brokerAccount != null)
    .forEach((a) => {
      const id = a.brokerAccount as string
      if (!out.some((x) => x.id === id)) out.push({ id, name: a.name, margin: a.margin, marginAccountId: a.margin ? id : '' })
    })
  return out
}
// the accounts' value, for the order's share of it
function nav() {
  const n = store.model?.navTotal
  return n == null || typeof n !== 'string' ? null : n
}
export function ctx(): ValsCtx {
  return { nav: nav(), accounts: ticketAccounts() }
}
export function vals() {
  return ticketStore.t ? view(ticketStore.t, ctx(), ticketStore.preview) : null
}

// The ticket's figures, asked of the server whenever what it holds changes; an
// answer to something since changed is not shown.
let asked = 0
export async function refreshPreview(): Promise<void> {
  const t = ticketStore.t
  if (!t) return
  const n = ++asked
  const r = await call('POST /api/order/preview', { body: previewRequest(t, ctx()) })
  if (n !== asked || ticketStore.t !== t) return
  if ('error' in r && r.error) {
    ticketStore.previewError = r.error
    return
  }
  ticketStore.previewError = ''
  ticketStore.preview = r as Preview
}

// Max: on a Buy the whole shares the account's buying power covers at the working
// price (the server's), on a Sell the position's shares.
export function maxQty(): number | null {
  const t = ticketStore.t
  if (!t) return null
  if (t.side !== 'BUY') return t.heldQty != null && t.heldQty > 0 ? t.heldQty : null
  return ticketNumber(ticketStore.preview?.maxQuantity)
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
  const info = tkLookup(symbol, securityId)
  const accounts = ticketAccounts()
  const held = info.position
  const heldIn = held ? brokerAccount(held.accountId) : ''
  let accountId = held && side === 'SELL' ? heldIn : '' // a sell goes where the shares are
  if (!accountId) accountId = tkRememberedAccount(accounts) || heldIn
  if (!accountId) accountId = (accounts.find((a) => a.margin) || accounts[0] || { id: '' }).id || ''
  ui.menuOpen = false
  const t: Ticket = {
    step: 'form', symbol, securityId: securityId || info.securityId, exchange,
    side: side === 'SELL' ? 'SELL' : 'BUY', accountId, type: 'LIMIT', tif: 'DAY',
    qty: held && side === 'SELL' ? ticketNumber(held.qty) : 1, limit: null, stop: null,
    sl: { on: true, kind: 'stop', price: null, pct: null, priceUnit: 'amt', trail: null, unit: 'pct' },
    tp: { on: true, price: null, pct: null, unit: 'amt' },
    heldQty: held ? ticketNumber(held.qty) : null, text: {}, data: null, error: '', busy: false, submitError: '',
  }
  const d = draftStore.d // what was typed before the ticket closed
  if (d && d.symbol === symbol && d.side === t.side) for (const k of TK_DRAFT_KEYS) if ((d as unknown as Record<string, unknown>)[k] != null) (t as unknown as Record<string, unknown>)[k] = (d as unknown as Record<string, unknown>)[k]
  ticketStore.t = t
  ticketStore.preview = null
  fetchQuote()
}

// the cross, Escape and the scrim keep the draft; Cancel and a sent order discard it
export function closeTicket(discard = false) {
  const t = ticketStore.t
  if (t) { if (discard) dropDraft(); else saveDraft(t) }
  ticketStore.t = null
  stopQuote?.()
  stopQuote = undefined
}

// The open ticket's quote. Wealthsimple pushes nothing, so the server asks it again
// every few seconds while a ticket shows it (rust docs.rs), and sends here only what
// moved -- the bid, the ask, the last -- written into the quote already shown. Called
// when the ticket opens and when its account changes (the quote is per account).
let stopQuote: (() => void) | undefined
export function fetchQuote() {
  const t = ticketStore.t
  stopQuote?.()
  stopQuote = undefined
  if (!t) return
  const qp = new URLSearchParams({ symbol: t.symbol, security: t.securityId || '', account: t.accountId || '', exchange: t.exchange || '' })
  const holder = {
    get data() { return ticketStore.t === t ? t.data : null },
    set data(v: typeof t.data) { if (ticketStore.t === t) t.data = v },
  }
  stopQuote = watchDoc('quote:' + qp.toString(), {}, holder, () => {
    const cur = ticketStore.t
    const r = cur?.data as ({ ok?: boolean; error?: string; accounts?: TicketAccount[]; orderTypes?: string[] } | null | undefined)
    if (!cur || cur !== t || !r) return
    if (r.ok === false) {
      cur.error = r.error || 'Quote failed.'
      return
    }
    cur.error = ''
    const accts = r.accounts || []
    if (!cur.accountId && accts.length) {
      cur.accountId = (accts.find((a) => a.margin) || accts[0]).id
      fetchQuote() // the quote is the account's: watch that one
      return
    }
    const ot = r.orderTypes || []
    if (ot.length && ot.indexOf(cur.type) < 0) cur.type = ot.indexOf('LIMIT') >= 0 ? 'LIMIT' : ot[0]
  })
}

function notice(v: ReturnType<typeof view>, sent: boolean) {
  const t = ticketStore.t!
  const head = sent ? 'Order placed · ' : 'Not sent (orders are off) · '
  return head + (v.buy ? 'Buy ' : 'Sell ') + qtyFmt(v.qty) + ' ' + symText(v.q.symbol || t.symbol) + ' at ' + (t.type === 'MARKET' ? 'market' : px(v.entry) + ' ' + v.typeWord.toLowerCase()) +
    (v.slOn ? ', stop ' + px(v.slPrice) : '') + (v.tpOn ? ', target ' + px(v.tpPrice) : '')
}

export async function submit() {
  const t = ticketStore.t
  if (!t || t.busy) return
  const v = view(t, ctx(), ticketStore.preview)
  const q = v.q
  // the order API takes numbers: the server's figures cross into them here
  const body = {
    symbol: t.symbol, securityId: q.securityId || t.securityId || '', accountId: t.accountId, side: t.side, type: t.type, tif: t.tif,
    quantity: ticketNumber(v.qty),
    limitPrice: t.type === 'LIMIT' || t.type === 'STOP_LIMIT' ? ticketNumber(v.limit) : null,
    stopPrice: t.type === 'STOP' || t.type === 'STOP_LIMIT' ? ticketNumber(v.stop) : null,
    currency: q.currency || '',
    stopLoss: v.slOn ? { kind: t.sl.kind, price: ticketNumber(v.slPrice), trail: ticketNumber(v.trail), trailUnit: t.sl.unit } : null,
    takeProfit: v.tpOn ? { price: ticketNumber(v.tpPrice) } : null,
  }
  t.busy = true
  t.submitError = ''
  const r = await call('POST /api/order', { body })
  const cur = ticketStore.t
  if (!cur) return
  cur.busy = false
  if (!r || !r.ok) {
    cur.submitError = (r && (r.error as string)) || 'Could not submit the order.'
    return
  }
  flash(notice(v, r.status === 'sent'), 'ok', 10000) // longer than the other notices: it names the whole order
  closeTicket(true)
}

// Resume a set-aside draft (from the Orders panel): reopen its ticket.
export function resumeDraft() {
  const d = draftStore.d
  if (!d) return
  openTicket(d.symbol, d.side, d.exchange, '')
}
export function discardDraft() { dropDraft() }

export { plain }
