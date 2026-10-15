import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { OrderListing } from '../generated/orders'

// the ticket's quote is a live document; no server answers here
vi.mock(import('../live.svelte'), async (original) => ({ ...(await original()), watchDoc: () => () => {} }))
// the server's answers, each held until the test lets it go
const answers: ((v: unknown) => void)[] = []
vi.mock(import('../api'), async (original) => ({ ...(await original()), call: (() => new Promise((ok) => answers.push(ok))) as never }))

// Two listings that share a ticker, each its own by Wealthsimple's id (brief 34, section 1).
const tsx: OrderListing = { securityId: 'sec-s-tsx', symbol: 'RY', exchange: 'TSX', currency: 'CAD' }
const nyse: OrderListing = { securityId: 'sec-s-nyse', symbol: 'RY', exchange: 'NYSE', currency: 'USD' }

describe('the ticket draft belongs to its listing', () => {
  beforeEach(() => {
    localStorage.clear()
    vi.resetModules()
  })

  it("a draft closed on one listing never fills the other listing's ticket with the same ticker", async () => {
    const tk = await import('./ticket.svelte')
    tk.openTicket(tsx, 'BUY')
    tk.ticketStore.t!.qty = 7
    tk.ticketStore.t!.limit = 12.5
    tk.closeTicket()
    expect(tk.draftStore.d?.listing.securityId).toBe('sec-s-tsx')
    tk.openTicket(nyse, 'BUY')
    expect(tk.ticketStore.t!.listing).toEqual(nyse)
    expect([tk.ticketStore.t!.qty, tk.ticketStore.t!.limit]).toEqual([1, null])
    tk.closeTicket(true)
    tk.openTicket(tsx, 'BUY')
    expect([tk.ticketStore.t!.qty, tk.ticketStore.t!.limit]).toEqual([1, null])
  })

  it('a draft reopens on its own listing, and Resume opens that listing', async () => {
    const tk = await import('./ticket.svelte')
    tk.openTicket(tsx, 'BUY')
    tk.ticketStore.t!.qty = 7
    tk.closeTicket()
    tk.resumeDraft()
    expect(tk.ticketStore.t!.listing).toEqual(tsx)
    expect(tk.ticketStore.t!.qty).toBe(7)
  })

  it('a draft saved before drafts named their listing is dropped, never matched by its ticker', async () => {
    localStorage.setItem('bh2.ticketDraft', JSON.stringify({ symbol: 'RY', side: 'BUY', exchange: 'TSX', at: '2026-10-01T00:00:00Z', qty: 7 }))
    const tk = await import('./ticket.svelte')
    expect(tk.draftStore.d).toBeNull()
    expect(localStorage.getItem('bh2.ticketDraft')).toBeNull()
  })

  it('a listing answered after another ticket opened never replaces it', async () => {
    const tk = await import('./ticket.svelte')
    const asked = tk.ticketOn({ kind: 'venue', symbol: 'RY', venue: 'NYSE' }, 'BUY')
    tk.openTicket(tsx, 'BUY')
    tk.ticketStore.t!.qty = 5
    answers.shift()!({ ok: true, listing: nyse })
    await asked
    expect(tk.ticketStore.t!.listing).toEqual(tsx)
    expect(tk.ticketStore.t!.qty).toBe(5)
  })

  it('a Sell from a holding page sells that holding where it is, matched by its id alone', async () => {
    const subs = await import('../subs.svelte')
    subs.book.data = { accounts: [{ id: 'acct-1', brokerAccount: 'A1', name: 'TFSA', margin: false, tradable: true, status: 'open' }, { id: 'acct-2', brokerAccount: 'A2', name: 'Trading', margin: true, tradable: true, status: 'open' }], options: { instruments: [] } } as never
    // the holding's broker id has not reached the page yet
    subs.positions.data = { positions: [{ id: 'h-tsx', security: '', symbol: 'RY', accountId: 'acct-1', qty: '150' }] } as never
    const tk = await import('./ticket.svelte')
    tk.openTicket(tsx, 'SELL', 'h-tsx')
    expect([tk.ticketStore.t!.accountId, tk.ticketStore.t!.qty]).toEqual(['A1', 150])
    subs.book.data = null
    subs.positions.data = null
  })
})
