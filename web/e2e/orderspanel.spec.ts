import { expect, test, type APIRequestContext, type Page } from '@playwright/test'
import { openWithStatus, ready, modelDoc, streamBody } from './helpers'

// SPEC.md "### Orders". Fixture orders/brackets stand in for the doc the server would
// send for `orders` (orders.svelte.ts watches that exact key); e2e/orders.spec.ts already
// covers the page-scroll-lock and an order editor's Enter-saves behaviour, so this file
// covers the rest: the tabs, the badge, a stop order's dimmed Edit, cancel confirmation,
// a waiting bracket's leg rows versus an armed bracket's own card, bracket edit/remove/
// cancel, the draft card, and the orders document updating a card in place.

// cards as the server builds them (web/src/lib/generated/orders.ts): amounts as exact
// decimal text, each card's tab, date and whether it acts, each leg's word
const card = (over: Record<string, unknown>) => ({
  id: '', account: 'acct-tfsa', exchange: 'TSX-V', symbol: 'QNC', side: 'buy', kind: 'limit', tif: 'day',
  quantity: '1', limitPrice: null, stopPrice: null, state: 'pending', filled: '0', average: null, why: null,
  value: null, approx: false, tab: 'pending', at: '2026-09-18T14:31:00Z', live: true, editable: true, legs: [], ...over,
})
const leg = (over: Record<string, unknown>) => ({ key: 'sl', quantity: '1', level: '1', trailPct: null, trailAmount: null, filled: null, note: '', amount: '1', ...over })

const oPending = card({ id: 'o-1', quantity: '100', limitPrice: '1.75', tif: 'until-cancel', value: '175' })
const oStop = card({ id: 'o-2', symbol: 'NVDA', exchange: 'NASDAQ', account: 'acct-trading', side: 'sell', kind: 'stop', quantity: '10', stopPrice: '150', value: '1500', editable: false })
const oPartial = card({ id: 'o-3', symbol: 'AAPL', exchange: 'NASDAQ', account: 'acct-trading', quantity: '100', limitPrice: '160', state: 'partly-filled', filled: '40', average: '159.5', value: '16000' })
const oFilled = card({ id: 'o-4', symbol: 'TD', exchange: 'TSX', side: 'sell', kind: 'market', quantity: '50', state: 'filled', filled: '50', average: '81.2', value: '4060', tab: 'filled', live: false, editable: false })
const oRejected = card({ id: 'o-5', symbol: 'SHOP', exchange: 'TSX', quantity: '20', limitPrice: '100', state: 'rejected', why: 'Insufficient funds.', value: '2000', tab: 'cancelled', live: false, editable: false })

// an armed bracket: its entry has filled (a card on the Filled tab), the bracket is its own card
const oArmedEntry = card({ id: 'o-6', symbol: 'NVDA', exchange: 'NASDAQ', account: 'acct-trading', quantity: '25', limitPrice: '165.4', state: 'filled', filled: '25', average: '165.4', value: '4135', tab: 'filled', at: '2026-09-18T14:00:00Z', live: false, editable: false })
const bArmed = {
  id: 'b-1', account: 'acct-trading', exchange: 'NASDAQ', symbol: 'NVDA', tab: 'pending', at: '2026-09-19T09:00:00Z', live: true, value: '4135',
  legs: [leg({ quantity: '25', level: '157.13', amount: '3928.25' }), leg({ key: 'tp', quantity: '25', level: '181.94', amount: '4548.5' })],
  endWord: null, stopLevel: '157.13', trailPct: null, trailAmount: null, target: '181.94',
}

// a bracket still waiting for its entry: leg rows on the entry's own pending card, no card of its own
const oWaitingEntry = card({
  id: 'o-7', symbol: 'TD', exchange: 'TSX', quantity: '50', limitPrice: '80', value: '4000',
  legs: [leg({ quantity: '50', level: '76', amount: '3800' }), leg({ key: 'tp', quantity: '50', level: '84', amount: '4200' })],
})

// a bracket cancelled by the person without either leg exiting
const oCancelledEntry = card({ id: 'o-8', symbol: 'ENB', exchange: 'TSX', quantity: '10', limitPrice: '50', state: 'filled', filled: '10', average: '50', value: '500', tab: 'filled', at: '2026-09-17T08:00:00Z', live: false, editable: false })
const bCancelled = {
  id: 'b-3', account: 'acct-tfsa', exchange: 'TSX', symbol: 'ENB', tab: 'cancelled', at: '2026-09-17T09:05:00Z', live: false, value: '500',
  legs: [leg({ quantity: '10', level: '47', amount: '470', note: 'Off' })],
  endWord: 'Cancelled', stopLevel: '47', trailPct: null, trailAmount: null, target: null,
}

const allOrders = { ok: true, live: false, refreshedAt: null, error: null, orders: [oPending, oStop, oPartial, oFilled, oRejected, oArmedEntry, oWaitingEntry, oCancelledEntry], brackets: [bArmed, bCancelled] }
const noOrders = { ok: true, live: false, refreshedAt: null, error: null, orders: [], brackets: [] }

async function openPanel(page: Page, request: APIRequestContext, orders: unknown = allOrders, status: Record<string, unknown> = { openOrders: 5 }): Promise<void> {
  await openWithStatus(page, request, status, '', () => {}, { orders })
  await ready(page)
  await page.getByRole('button', { name: 'Orders' }).click()
  await expect(page.getByRole('dialog', { name: 'Orders' })).toBeVisible()
}

test('Ctrl+O opens the panel, and pressed again closes it', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '', () => {}, { orders: allOrders })
  await ready(page)
  await page.keyboard.press('Control+o')
  await expect(page.getByRole('dialog', { name: 'Orders' })).toBeVisible()
  await page.keyboard.press('Control+o')
  await expect(page.getByRole('dialog', { name: 'Orders' })).toHaveCount(0)
})

test('the header names the pending count in the ink\'s tint, matching the panel', async ({ page, request }) => {
  await openPanel(page, request)
  await expect(page.locator('button[aria-label="Orders"] .od-badge')).toHaveText('5')
  await expect(page.locator('.od-head')).toContainText('Pending orders')
  const cards = page.locator('#odBody .od-card')
  await expect(cards).toHaveCount(5) // o-1, o-2 (stop), o-3 (partial), o-7 (a waiting bracket's entry), and the armed bracket
})

test('←/→ move between Pending, Filled and Cancelled, and the heading follows', async ({ page, request }) => {
  await openPanel(page, request)
  await expect(page.locator('.od-seg.on')).toHaveText('Pending')
  await page.keyboard.press('ArrowRight')
  await expect(page.locator('.od-seg.on')).toHaveText('Filled')
  await expect(page.locator('.od-head')).toContainText('Filled orders')
  await page.keyboard.press('ArrowRight')
  await expect(page.locator('.od-seg.on')).toHaveText('Cancelled')
  await expect(page.locator('.od-head')).toContainText('Cancelled and rejected')
  await page.keyboard.press('ArrowLeft')
  await expect(page.locator('.od-seg.on')).toHaveText('Filled')
})

test('a pending card\'s grammar: side, listing, value, quantity and how, when, Edit and Cancel', async ({ page, request }) => {
  await openPanel(page, request)
  const card = page.locator('.od-card', { hasText: 'TSX-V: QNC' })
  await expect(card.locator('.od-title')).toContainText('Buy')
  await expect(card.locator('.od-value')).toHaveText('$175.00')
  await expect(card.locator('.od-line')).toHaveText('100 at 1.75 limit · GTC')
  await expect(card.getByRole('button', { name: 'Edit' })).toBeEnabled()
  await expect(card.getByRole('button', { name: 'Cancel' })).toBeEnabled()
})

test('a stop order\'s Edit is dimmed -- Wealthsimple\'s modify carries no stop price', async ({ page, request }) => {
  await openPanel(page, request)
  const card = page.locator('.od-card', { hasText: 'NASDAQ: NVDA' }).filter({ hasText: 'stop' }).filter({ hasNotText: 'Bracket' })
  await expect(card.locator('.od-line')).toHaveText('10 at 150.00 stop · Day')
  await expect(card.getByRole('button', { name: 'Edit' })).toBeDisabled()
  await expect(card.getByRole('button', { name: 'Cancel' })).toBeEnabled()
})

test('a partly filled order carries a third line naming the fill so far', async ({ page, request }) => {
  await openPanel(page, request)
  const card = page.locator('.od-card', { hasText: 'NASDAQ: AAPL' })
  await expect(card.locator('.od-fill')).toHaveText('40 of 100 filled at 159.50')
})

test('Cancel asks first, naming the order; Keep order returns; Cancel order sends the id', async ({ page, request }) => {
  await openPanel(page, request)
  let sent: unknown = null
  await page.route('**/api/order/cancel', (route) => { sent = route.request().postDataJSON(); return route.fulfill({ json: { ok: true } }) })
  const card = page.locator('.od-card', { hasText: 'TSX-V: QNC' })
  await card.getByRole('button', { name: 'Cancel' }).click()
  await expect(page.getByRole('heading', { name: 'Cancel order' })).toBeVisible()
  await expect(page.locator('#confirmDlg')).toContainText('Buy 100 QNC at 1.75 limit')
  await page.getByRole('button', { name: 'Keep order' }).click()
  await expect(page.locator('#confirmDlg')).toHaveCount(0)
  expect(sent).toBeNull()
  await card.getByRole('button', { name: 'Cancel' }).click()
  await page.getByRole('button', { name: 'Cancel order' }).click()
  await expect.poll(() => sent).toEqual({ id: 'o-1' })
  await expect(page.locator('#syncline')).toContainText('Cancel sent · Buy 100 QNC at 1.75 limit')
})

test('Filled orders sit under their own heading, dated, with no Edit or Cancel', async ({ page, request }) => {
  await openPanel(page, request)
  await page.keyboard.press('ArrowRight')
  const card = page.locator('.od-card', { hasText: 'TSX: TD' })
  await expect(card.locator('.od-line')).toHaveText('Filled 50 at 81.20')
  await expect(card.getByRole('button', { name: 'Edit' })).toHaveCount(0)
  await expect(card.locator('.od-when')).not.toHaveText('')
})

test('a rejected order reads Rejected in red, with Wealthsimple\'s reason on its own line', async ({ page, request }) => {
  await openPanel(page, request)
  await page.keyboard.press('ArrowRight')
  await page.keyboard.press('ArrowRight')
  const card = page.locator('.od-card', { hasText: 'TSX: SHOP' })
  await expect(card.locator('.od-state')).toHaveText('Rejected')
  await expect(card.locator('.od-state')).toHaveClass(/neg/)
  await expect(card.locator('.od-fill')).toHaveText('Insufficient funds.')
})

test('a bracket still waiting for its fill is leg rows on the entry\'s own card, not a card of its own', async ({ page, request }) => {
  await openPanel(page, request)
  const cards = page.locator('#odBody .od-card')
  await expect(cards).toHaveCount(5) // no separate card for the waiting bracket
  const card = page.locator('.od-card', { hasText: 'TSX: TD' })
  const legs = card.locator('.od-leg')
  await expect(legs).toHaveCount(2)
  await expect(legs.filter({ hasText: 'Stop loss' })).toContainText('50 at 76.00')
  await expect(legs.filter({ hasText: 'Take profit' })).toContainText('50 at 84.00')
})

test('an armed bracket is its own card, the stop and target as leg rows, aligned under its value', async ({ page, request }) => {
  await openPanel(page, request)
  const card = page.locator('.od-card', { hasText: 'NASDAQ: NVDA' }).filter({ hasText: 'Bracket' })
  await expect(card.locator('.od-title')).toContainText('Bracket')
  await expect(card.locator('.od-value')).toHaveText('$4,135.00') // 25 * 165.40, what was paid for the shares
  const legs = card.locator('.od-leg')
  await expect(legs.filter({ hasText: 'Stop loss' })).toContainText('25 at 157.13')
  await expect(legs.filter({ hasText: 'Stop loss' }).locator('.od-leg-amt')).toHaveText('$3,928.25')
  await expect(legs.filter({ hasText: 'Take profit' })).toContainText('25 at 181.94')
  await expect(card.getByRole('button', { name: 'Edit' })).toBeVisible()
})

test('editing a bracket opens both legs, and Save sends only the leg that changed', async ({ page, request }) => {
  await openPanel(page, request)
  let sent: unknown[] = []
  await page.route('**/api/bracket/adjust', (route) => { sent.push(route.request().postDataJSON()); return route.fulfill({ json: { ok: true } }) })
  const card = page.locator('.od-card', { hasText: 'NASDAQ: NVDA' }).filter({ hasText: 'Bracket' })
  await card.getByRole('button', { name: 'Edit' }).click()
  await expect(page.locator('#od-sl')).toHaveValue('157.13')
  await expect(page.locator('#od-tp')).toHaveValue('181.94')
  await page.locator('#od-sl').fill('150')
  await card.getByRole('button', { name: 'Save' }).click()
  await expect.poll(() => sent).toEqual([{ id: 'b-1', leg: 'sl', price: '150', trail: null }])
  await expect(page.locator('#syncline')).toContainText('Bracket changed · NVDA')
})

test('Remove stop loss on a bracket sends the leg\'s removal and flashes it', async ({ page, request }) => {
  await openPanel(page, request)
  let sent: unknown = null
  await page.route('**/api/bracket/adjust', (route) => { sent = route.request().postDataJSON(); return route.fulfill({ json: { ok: true } }) })
  const card = page.locator('.od-card', { hasText: 'NASDAQ: NVDA' }).filter({ hasText: 'Bracket' })
  await card.getByRole('button', { name: 'Edit' }).click()
  await page.getByRole('button', { name: 'Remove stop loss' }).click()
  await expect.poll(() => sent).toEqual({ id: 'b-1', leg: 'sl', price: null, trail: null, remove: true })
  await expect(page.locator('#syncline')).toContainText('Stop loss removed · NVDA')
})

test('Cancel on a bracket asks once, naming both legs, then cancels it at Wealthsimple', async ({ page, request }) => {
  await openPanel(page, request)
  let sent: unknown = null
  await page.route('**/api/bracket/cancel', (route) => { sent = route.request().postDataJSON(); return route.fulfill({ json: { ok: true } }) })
  const card = page.locator('.od-card', { hasText: 'NASDAQ: NVDA' }).filter({ hasText: 'Bracket' })
  await card.getByRole('button', { name: 'Cancel' }).click()
  await expect(page.getByRole('heading', { name: 'Cancel bracket' })).toBeVisible()
  await expect(page.locator('#confirmDlg')).toContainText('Stop loss')
  await expect(page.locator('#confirmDlg')).toContainText('Take profit')
  await page.getByRole('button', { name: 'Keep bracket' }).click()
  expect(sent).toBeNull()
  await card.getByRole('button', { name: 'Cancel' }).click()
  await page.getByRole('button', { name: 'Cancel bracket' }).click()
  await expect.poll(() => sent).toEqual({ id: 'b-1' })
  await expect(page.locator('#syncline')).toContainText('Bracket cancelled · NVDA')
})

test('a bracket cancelled without either leg exiting reads Off on the leg and Cancelled on the card', async ({ page, request }) => {
  await openPanel(page, request)
  await page.keyboard.press('ArrowRight')
  await page.keyboard.press('ArrowRight')
  const card = page.locator('.od-card', { hasText: 'TSX: ENB' }).filter({ hasText: 'Bracket' })
  await expect(card.locator('.od-state')).toHaveText('Cancelled')
  await expect(card.locator('.od-leg').filter({ hasText: 'Stop loss' })).toContainText('Off')
})

test('Esc closes an open editor first, then the panel', async ({ page, request }) => {
  await openPanel(page, request)
  const card = page.locator('.od-card', { hasText: 'TSX-V: QNC' })
  await card.getByRole('button', { name: 'Edit' }).click()
  await expect(page.locator('#od-qty')).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(page.locator('#od-qty')).toHaveCount(0)
  await expect(page.getByRole('dialog', { name: 'Orders' })).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(page.getByRole('dialog', { name: 'Orders' })).toHaveCount(0)
})

test('a closed-without-sending draft is an amber card above Pending, with Resume and Discard', async ({ page, request }) => {
  const draft = {
    symbol: 'QNC', side: 'BUY', exchange: 'TSX-V', at: '2026-09-19T16:00:00Z',
    accountId: 'acct-tfsa', type: 'LIMIT', tif: 'DAY', qty: 1, limit: 1.72, stop: null,
    sl: { on: true, kind: 'stop', price: 1.6, pct: null, priceUnit: 'amt', trail: null, unit: 'pct' },
    tp: { on: false, price: null, pct: null, unit: 'amt' },
    text: {},
    // the server's figures for what the draft holds, as the ticket last had them
    preview: { entry: '1.72', limit: '1.72', quantity: '1', notional: '1.72', stopLossOn: true, takeProfitOn: false, trailing: false, stopLossPrice: '1.6', stopLossValue: '1.6', takeProfitPrice: null, takeProfitValue: null },
  }
  await page.addInitScript((d) => localStorage.setItem('bh2.ticketDraft', JSON.stringify(d)), draft)
  await openPanel(page, request, noOrders, { openOrders: 0 })
  const card = page.locator('.od-card.draft')
  await expect(card.locator('.od-draft')).toHaveText('Draft')
  await expect(card.locator('.od-title')).toContainText('TSX-V: QNC')
  await expect(card.locator('.od-value')).toHaveText('$1.72')
  await expect(card.locator('.od-line')).toHaveText('Buy 1 at 1.72 limit · Day')
  await expect(card.locator('.od-leg')).toHaveCount(1)
  await expect(card.locator('.od-leg')).toContainText('1 at 1.60')
  await expect(card.locator('.od-leg-amt')).toHaveText('$1.60')
  await card.getByRole('button', { name: 'Resume' }).click()
  await expect(page.getByRole('dialog', { name: 'Orders' })).toHaveCount(0)
  await expect(page.getByRole('dialog', { name: 'New order' })).toBeVisible()
  await expect(page.locator('#tk-limit')).toHaveValue('1.72')
})

test('Discard removes the draft card', async ({ page, request }) => {
  const draft = {
    symbol: 'QNC', side: 'BUY', exchange: 'TSX-V', at: '2026-09-19T16:00:00Z',
    accountId: 'acct-tfsa', type: 'LIMIT', tif: 'DAY', qty: 1, limit: 1.72, stop: null,
    sl: { on: false, kind: 'stop', price: null, pct: null, priceUnit: 'amt', trail: null, unit: 'pct' },
    tp: { on: false, price: null, pct: null, unit: 'amt' },
    text: {},
  }
  await page.addInitScript((d) => localStorage.setItem('bh2.ticketDraft', JSON.stringify(d)), draft)
  await openPanel(page, request, noOrders, { openOrders: 0 })
  await expect(page.locator('.od-card.draft')).toBeVisible()
  await page.locator('.od-card.draft').getByRole('button', { name: 'Discard' }).click()
  await expect(page.locator('.od-card.draft')).toHaveCount(0)
  await expect(page.locator('#odBody .dim')).toHaveText('No pending orders.')
})

test('while the limit sell at the target rests, the stop row reads Watching; once the stop is hit, Placing', async ({ page, request }) => {
  // the server's words for the legs: Watching on the stop while the target's limit sell rests
  const resting = { ...bArmed, legs: [{ ...bArmed.legs[0], note: 'Watching' }, bArmed.legs[1]] }
  const model = await modelDoc(request)
  const patch = `event: patch\ndata: ${JSON.stringify({ doc: 'orders', ops: [
    ['set', ['brackets', { k: 'id', v: 'b-1' }, 'legs', { k: 'key', v: 'sl' }, 'note'], 'Placing'],
    ['set', ['brackets', { k: 'id', v: 'b-1' }, 'legs', { k: 'key', v: 'tp' }, 'note'], 'Cancelling'],
  ] })}\n\n`
  const docs = { orders: { ...noOrders, live: true, orders: [oArmedEntry], brackets: [resting] } }
  await page.route('**/api/events?*', (route) => route.fulfill({ status: 200, contentType: 'text/event-stream', body: streamBody(model, docs) }))
  await page.goto('/')
  await ready(page)
  await page.getByRole('button', { name: 'Orders' }).click()
  const stopRow = page.locator('.od-card', { hasText: 'Bracket' }).locator('.od-leg').filter({ hasText: 'Stop loss' })
  await expect(stopRow.locator('.od-leg-state')).toHaveText('Watching')
  await expect(page.locator('.od-card', { hasText: 'Bracket' }).locator('.od-leg').filter({ hasText: 'Take profit' }).locator('.od-leg-state')).toHaveCount(0)
  // the stop level reached: the limit sell's cancel is out and the market sell follows
  await page.unroute('**/api/events?*')
  await page.route('**/api/events?*', (route) => route.fulfill({ status: 200, contentType: 'text/event-stream', body: streamBody(model, docs, patch) }))
  await page.reload()
  await ready(page)
  await page.getByRole('button', { name: 'Orders' }).click()
  await expect(page.locator('.od-card', { hasText: 'Bracket' }).locator('.od-leg').filter({ hasText: 'Stop loss' }).locator('.od-leg-state')).toHaveText('Placing')
  await expect(page.locator('.od-card', { hasText: 'Bracket' }).locator('.od-leg').filter({ hasText: 'Take profit' }).locator('.od-leg-state')).toHaveText('Cancelling')
})

test('a bracket that ended without exiting reads Off on both legs, and its foot says how it ended', async ({ page, request }) => {
  const off = (l: Record<string, unknown>) => ({ ...l, note: 'Off' })
  const ended = { ...bArmed, tab: 'cancelled', live: false, legs: bArmed.legs.map(off) }
  const byUser = { ...ended, id: 'b-u', at: '2026-09-19T10:00:00Z', endWord: 'Cancelled' }
  const sold = { ...ended, id: 'b-s', exchange: 'TSX', symbol: 'TD', at: '2026-09-19T11:00:00Z', endWord: 'Off' }
  await openPanel(page, request, { ...noOrders, live: true, brackets: [byUser, sold] }, { openOrders: 0 })
  await page.keyboard.press('ArrowRight')
  await page.keyboard.press('ArrowRight')
  const user = page.locator('.od-card', { hasText: 'NASDAQ: NVDA' })
  await expect(user.locator('.od-leg-state')).toHaveText(['Off', 'Off'])
  await expect(user.locator('.od-state')).toHaveText('Cancelled')
  await expect(user.getByRole('button', { name: 'Edit' })).toHaveCount(0)
  const offCard = page.locator('.od-card', { hasText: 'TSX: TD' })
  await expect(offCard.locator('.od-leg-state')).toHaveText(['Off', 'Off'])
  await expect(offCard.locator('.od-state')).toHaveText('Off')
})

test('a bracket that exited stands on the Filled tab, the exit\'s fill on its leg and Cancelled on the other', async ({ page, request }) => {
  const exited = {
    ...bArmed, tab: 'filled', live: false, at: '2026-09-19T12:00:00Z',
    legs: [{ ...bArmed.legs[0], filled: { quantity: '25', average: '157.02' }, amount: '3925.5' }, { ...bArmed.legs[1], note: 'Cancelled' }],
  }
  await openPanel(page, request, { ...noOrders, live: true, brackets: [exited] }, { openOrders: 0 })
  await page.keyboard.press('ArrowRight')
  const c = page.locator('.od-card', { hasText: 'Bracket' })
  const stop = c.locator('.od-leg').filter({ hasText: 'Stop loss' })
  await expect(stop.locator('.od-leg-value')).toHaveText('Filled 25 at 157.02')
  await expect(stop.locator('.od-leg-amt')).toHaveText('$3,925.50')
  await expect(stop.locator('.od-leg-state')).toHaveCount(0)
  await expect(c.locator('.od-leg').filter({ hasText: 'Take profit' }).locator('.od-leg-state')).toHaveText('Cancelled')
  await expect(c.locator('.od-state')).toHaveCount(0)
})

test('after Cancel is accepted the card reads Cancelling on its second line, with no Edit or Cancel, until the next read', async ({ page, request }) => {
  const model = await modelDoc(request)
  const patch = `event: patch\ndata: ${JSON.stringify({ doc: 'orders', ops: [
    ['set', ['orders', { k: 'id', v: 'o-1' }, 'state'], 'cancelling'],
    ['set', ['orders', { k: 'id', v: 'o-1' }, 'live'], false],
  ] })}\n\n`
  const body = streamBody(model, { orders: { ...noOrders, live: true, orders: [oPending] } }, patch)
  await page.route('**/api/events?*', (route) => route.fulfill({ status: 200, contentType: 'text/event-stream', body }))
  await page.goto('/')
  await ready(page)
  await page.getByRole('button', { name: 'Orders' }).click()
  const card = page.locator('.od-card', { hasText: 'TSX-V: QNC' })
  await expect(card.locator('.od-line')).toHaveText('100 at 1.75 limit · GTC · Cancelling')
  await expect(card.getByRole('button', { name: 'Edit' })).toHaveCount(0)
  await expect(card.getByRole('button', { name: 'Cancel' })).toHaveCount(0)
})

test('an order Wealthsimple has not answered for is a Pending card reading Sent · not confirmed, with nothing to act on yet', async ({ page, request }) => {
  for (const state of ['sending', 'unconfirmed']) {
    const unanswered = { ...oPending, state, live: false, editable: false }
    await openPanel(page, request, { ...noOrders, live: true, orders: [unanswered] }, { openOrders: 1 })
    const c = page.locator('.od-card', { hasText: 'TSX-V: QNC' })
    await expect(c.locator('.od-line')).toHaveText('100 at 1.75 limit · GTC')
    await expect(c.locator('.od-state')).toHaveText('Sent · not confirmed')
    await expect(c.getByRole('button', { name: 'Cancel' })).toHaveCount(0)
    await expect(page.locator('button[aria-label="Orders"] .od-badge')).toHaveText('1')
    await page.unroute('**/api/events?*')
  }
})

test('a market order\'s value is its guessed fill, marked ≈; a value that waits reads the dash and its word', async ({ page, request }) => {
  const market = card({ id: 'o-m', symbol: 'TD', exchange: 'TSX', kind: 'market', quantity: '5', value: '406', approx: true })
  const waiting = card({ id: 'o-w', symbol: 'AAPL', exchange: 'NASDAQ', quantity: '1', limitPrice: '3', value: { gaps: ['multiplier-unstated'] } })
  await openPanel(page, request, { ...noOrders, live: true, orders: [market, waiting] }, { openOrders: 2 })
  await expect(page.locator('.od-card', { hasText: 'TSX: TD' }).locator('.od-value')).toHaveText('≈ $406.00')
  await expect(page.locator('.od-card', { hasText: 'TSX: TD' }).locator('.od-line')).toHaveText('5 at market')
  await expect(page.locator('.od-card', { hasText: 'NASDAQ: AAPL' }).locator('.od-value')).toHaveText('— size')
})

test('the orders document updates a card in place, with the panel open', async ({ page, request }) => {
  // o-3 (AAPL) stays pending and stays a card; only its third line -- the fill so
  // far -- should move, proving the document patches that one card rather than the
  // panel refetching or rebuilding its list.
  const model = await modelDoc(request)
  const patch = `event: patch\ndata: ${JSON.stringify({ doc: 'orders', ops: [
    ['set', ['orders', { k: 'id', v: 'o-3' }, 'filled'], '70'],
    ['set', ['orders', { k: 'id', v: 'o-3' }, 'average'], '160.1'],
  ] })}\n\n`
  const body = streamBody(model, { orders: allOrders }, patch)
  await page.route('**/api/events?*', (route) => route.fulfill({ status: 200, contentType: 'text/event-stream', body }))
  await page.goto('/')
  await ready(page)
  await page.getByRole('button', { name: 'Orders' }).click()
  const cards = page.locator('#odBody .od-card')
  await expect(cards).toHaveCount(5) // still the same five Pending cards, none added or removed
  const card = page.locator('.od-card', { hasText: 'NASDAQ: AAPL' })
  await expect(card.locator('.od-fill')).toHaveText('70 of 100 filled at 160.10')
  await expect(card.getByRole('button', { name: 'Edit' })).toBeEnabled() // still a live, editable order
})
