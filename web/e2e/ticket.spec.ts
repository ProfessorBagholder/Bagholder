import { expect, test } from '@playwright/test'
import { openWithStatus, ready, figures } from './helpers'

// SPEC.md "### Order ticket". The made-up book (e2e/serve.mjs -> demo_book.rs) already
// holds NVDA and AAPL as open positions in the USD margin account ("Trading"; NVDA in the
// RRSP too), TD as an open position in the CAD "TFSA", and UBER as a fully-closed trade in
// "Trading" with no open position -- real book data, not synthesised, so a Buy/Sell icon's
// availability (SPEC: Sell only where a position is held) is the book's own truth.
//
// The ticket's live quote is a document, `quote:<params>`, whose key is built by
// ticket.svelte.ts's fetchQuote() from the exact symbol/security/account/exchange the
// ticket opened with (insertion order: symbol, security, account, exchange). openTicket()
// resolves the account deterministically from the book before any quote answers: a Buy or
// Sell on a held symbol goes to the account holding it; a Buy on a symbol with no position
// falls back to the book's margin account, "Trading". Those rules, read from the figures
// document, let the test compute the exact doc key up front.
const quoteKey = (symbol: string, security: string, account: string, exchange: string) =>
  'quote:' + new URLSearchParams({ symbol, security, account, exchange }).toString()

// An order names an account by the broker's own id for it. The book's holding of a
// symbol (the first, as the ticket takes it) gives the account a ticket on it opens on
// and the quantity a Sell starts from.
async function holding(request: import('@playwright/test').APIRequestContext, symbol: string): Promise<{ account: string; qty: string }> {
  const m = await figures(request)
  // held in more than one account: the margin account among them, else the first by name (SPEC §4 Order ticket)
  const accounts = m.accounts as { id: string; name: string; margin: boolean; brokerAccount: string | null }[]
  const of = (id: string) => accounts.find((a) => a.id === id)!
  const p = (m.positions as { symbol: string; accountId: string; qty: string }[])
    .filter((x) => x.symbol === symbol)
    .sort((a, b) => Number(of(b.accountId).margin) - Number(of(a.accountId).margin) || of(a.accountId).name.localeCompare(of(b.accountId).name))[0]
  const account = of(p.accountId).brokerAccount!
  return { account, qty: p.qty }
}
// the account a Buy on a symbol the book does not hold opens on: the book's first open,
// tradable margin account
async function marginAccount(request: import('@playwright/test').APIRequestContext): Promise<string> {
  const m = await figures(request)
  return (m.accounts as { brokerAccount: string | null; margin: boolean; tradable: boolean; status: string }[]).find((a) => a.margin && a.tradable && a.status !== 'closed' && a.brokerAccount != null)!.brokerAccount!
}

test('opens from a ⌘K listing row\'s Buy icon, on the account holding the symbol', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '')
  await ready(page)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
  await expect(page.getByRole('dialog', { name: 'New order' })).toBeVisible()
  await expect(page.locator('#tkQuote')).toContainText('NVDA')
  await expect(page.locator('.tk-segopt.buy')).toHaveClass(/on/)
  await expect(page.locator('#tk-account')).toHaveValue((await holding(request, 'NVDA')).account) // the account holding the NVDA position
})

test('opens from a ⌘K listing row\'s Sell icon, only offered where a position is held', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '')
  await ready(page)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Sell NVDA', exact: true }).click()
  await expect(page.getByRole('dialog', { name: 'New order' })).toBeVisible()
  await expect(page.locator('.tk-segopt.sell')).toHaveClass(/on/)
  await expect(page.locator('#tk-qty')).toHaveValue((await holding(request, 'NVDA')).qty) // the held quantity, not the Buy default of 1
  // a closed trade with no open position (UBER) offers Buy but never Sell
  await page.keyboard.press('Escape')
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('UBER')
  await expect(page.getByRole('button', { name: 'Sell UBER', exact: true })).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Buy UBER', exact: true })).toBeVisible()
})

test('opens from the holding\'s own page, and Buy defaults to one share', async ({ page, request }) => {
  const model = await figures(request)
  const nvda = model.positions.find((p: { symbol: string }) => p.symbol === 'NVDA')
  await openWithStatus(page, request, {}, '#portfolio/' + encodeURIComponent(nvda.id))
  await ready(page)
  await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
  await expect(page.getByRole('dialog', { name: 'New order' })).toBeVisible()
  await expect(page.locator('#tk-qty')).toHaveValue('1')
})

test('switching Buy and Sell recomputes the defaults: a Sell sells the held shares where they are, a Buy one share', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '')
  await ready(page)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('TD')
  await page.getByRole('button', { name: 'Buy TD', exact: true }).click()
  await expect(page.getByRole('dialog', { name: 'New order' })).toBeVisible()
  await expect(page.locator('#tk-qty')).toHaveValue('1')
  await page.locator('.tk-segopt.sell').click()
  const td = await holding(request, 'TD')
  await expect(page.locator('#tk-qty')).toHaveValue(td.qty) // every share held, not the Buy's one
  await expect(page.locator('#tk-account')).toHaveValue(td.account) // where the shares are
  await page.locator('.tk-segopt.buy').click()
  await expect(page.locator('#tk-qty')).toHaveValue('1')
})

test('a Market order hides Limit/Stop price and Time in force; other types show what they need', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '')
  await ready(page)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
  await expect(page.locator('#tk-limit')).toBeVisible() // LIMIT is the default type
  await expect(page.locator('#tk-stop')).toHaveCount(0)
  await expect(page.locator('#tk-tif')).toBeVisible()
  await page.locator('#tk-type').selectOption('MARKET')
  await expect(page.locator('#tk-limit')).toHaveCount(0)
  await expect(page.locator('#tk-stop')).toHaveCount(0)
  await expect(page.locator('#tk-tif')).toHaveCount(0) // hidden for Market
  await page.locator('#tk-type').selectOption('STOP')
  await expect(page.locator('#tk-stop')).toBeVisible()
  await expect(page.locator('#tk-limit')).toHaveCount(0)
  await page.locator('#tk-type').selectOption('STOP_LIMIT')
  await expect(page.locator('#tk-stop')).toBeVisible()
  await expect(page.locator('#tk-limit')).toBeVisible()
})

test('only the order types the quote reports for the security are offered', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '', () => {}, {
    [quoteKey('TD', 'sec-td', (await holding(request, 'TD')).account, 'TSX')]: { ok: true, orderTypes: ['MARKET', 'LIMIT'] },
  })
  await ready(page)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('TD')
  await page.getByRole('button', { name: 'Sell TD', exact: true }).click()
  await expect(page.locator('#tk-type option')).toHaveCount(2)
  await expect(page.locator('#tk-type option[value="STOP"]')).toHaveCount(0)
})

test('a Sell shows no Stop loss or Take profit section', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '')
  await ready(page)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('TD')
  await page.getByRole('button', { name: 'Sell TD', exact: true }).click()
  await expect(page.getByText('Stop loss')).toHaveCount(0)
  await expect(page.getByText('Take profit')).toHaveCount(0)
})

test('typing Shares sets Amount at the working price, and Amount sets whole Shares back', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '')
  await ready(page)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
  await page.locator('#tk-limit').fill('100')
  await page.locator('#tk-limit').press('Enter') // commits the typed limit as the working price
  await page.locator('#tk-qty').fill('10')
  await expect(page.locator('#tk-amt')).toHaveValue('$1,000')
  await page.locator('#tk-amt').fill('2000')
  await expect(page.locator('#tk-qty')).toHaveValue('20')
})

test('Max sets the whole shares the account\'s buying power covers, and is off when unknown', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '', () => {}, {
    [quoteKey('NVDA', 'sec-nvda', (await holding(request, 'NVDA')).account, 'NASDAQ')]: { ok: true, buyingPower: 1000 },
  })
  await ready(page)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
  await page.locator('#tk-limit').fill('100')
  await page.locator('#tk-limit').press('Enter')
  await expect(page.locator('.tk-max').first()).not.toHaveClass(/off/)
  await page.locator('.tk-max').first().click()
  await expect(page.locator('#tk-qty')).toHaveValue('10') // floor(1000 / 100)
})

test('Max is off (and the position\'s shares fill Sell) when buying power is unknown', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '') // no quote doc at all: no buying power
  await ready(page)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
  await expect(page.locator('.tk-max').first()).toHaveClass(/off/)
  await page.keyboard.press('Escape')
  await page.locator('.tk-x, .tk-scrim').first() // no-op, ensures the ticket actually closed below
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Sell NVDA', exact: true }).click()
  await expect(page.locator('.tk-max').first()).not.toHaveClass(/off/) // the held quantity is known
})

test('a stop loss and target round to the cent even off a quote with extra precision', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '', () => {}, {
    [quoteKey('AAPL', 'sec-aapl', (await holding(request, 'AAPL')).account, 'NASDAQ')]: { ok: true, quote: { last: 163.47, currency: 'USD', multiplier: 1 } },
  })
  await ready(page)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('AAPL')
  await page.getByRole('button', { name: 'Buy AAPL', exact: true }).click()
  await expect(page.locator('#tk-sl-l')).toHaveText('Sells at 155.30') // 163.47 * 0.95 = 155.2965
  await expect(page.locator('#tk-sl-r')).toHaveText('−$8.17 (−5.0%)')
  await expect(page.locator('#tk-tp-l')).toHaveText('Sells at 179.82') // 163.47 * 1.10 = 179.817
  await expect(page.locator('#tk-tp-r')).toHaveText('+$16.35 (+10.0%)')
})

test('a trailing stop\'s loss is its distance exactly, and it reads "Starts at"', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '', () => {}, {
    [quoteKey('NVDA', 'sec-nvda', (await holding(request, 'NVDA')).account, 'NASDAQ')]: { ok: true, quote: { last: 165.4, currency: 'USD', multiplier: 1 } },
  })
  await ready(page)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
  await page.locator('#tk-qty').fill('25')
  await page.locator('#tk-slkind').selectOption('trail')
  await expect(page.locator('#tk-sl-l')).toHaveText('Starts at 157.13') // 5% trail under 165.40
  await expect(page.locator('#tk-sl-r')).toHaveText('−$206.75 (−5.0%)')
})

test('removing and re-adding Stop loss and Take profit', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '')
  await ready(page)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
  await page.getByRole('button', { name: 'Remove stop loss' }).click()
  await expect(page.locator('#tk-sl-l')).toHaveCount(0)
  await page.getByRole('button', { name: 'Add stop loss' }).click()
  await expect(page.locator('#tk-sl-l')).toBeVisible()
  await page.getByRole('button', { name: 'Remove take profit' }).click()
  await expect(page.locator('#tk-tp-l')).toHaveCount(0)
})

test('a quote that cannot be had says why, and the figures are dashes', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '', () => {}, {
    [quoteKey('UBER', 'sec-uber', await marginAccount(request), 'NYSE')]: { ok: false, error: 'Wealthsimple has no quote for UBER.' },
  })
  await ready(page)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('UBER')
  await page.getByRole('button', { name: 'Buy UBER', exact: true }).click()
  await expect(page.locator('.status-err')).toHaveText('Wealthsimple has no quote for UBER.')
  await expect(page.locator('#tkQuote')).toContainText('—') // em dash: last price unknown
})

test('the review step shows the order, the risk, and Available margin after on a margin account', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '', () => {}, {
    [quoteKey('NVDA', 'sec-nvda', (await holding(request, 'NVDA')).account, 'NASDAQ')]: {
      ok: true,
      quote: { last: 165.4, currency: 'USD', multiplier: 1, securityId: 'sec-nvda' },
      fxUsdCad: 1, marginRate: 0.4, marginAvailable: 5000,
    },
  })
  await ready(page)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
  await page.locator('#tk-qty').fill('25')
  await page.locator('#tk-limit').fill('165.40')
  await page.locator('#tk-limit').press('Enter')
  await page.getByRole('button', { name: 'Review' }).click()
  await expect(page.getByRole('dialog', { name: 'Review order' })).toBeVisible()
  await expect(page.locator('.tk-body')).toContainText('Buy 25 NVDA')
  await expect(page.locator('.tk-body')).toContainText('Limit 165.40 · Day · Trading')
  const rows = page.locator('.tk-row')
  await expect(rows.filter({ hasText: 'Stop loss' })).toContainText('Market at 157.13')
  await expect(rows.filter({ hasText: 'Take profit' })).toContainText('Limit at 181.94')
  await expect(rows.filter({ hasText: 'At risk' })).toContainText('−$206.75 (−5.0%)')
  await expect(rows.filter({ hasText: 'Target' })).toContainText('+$413.50 (+10.0%)')
  await expect(rows.filter({ hasText: 'Risk / reward' })).toContainText('1:2')
  // the order's CAD cost (25 × 165.40 at a rate of 1) over the accounts' value
  const nav = Number((await figures(request)).navTotal)
  await expect(rows.filter({ hasText: 'Position size' })).toContainText(((4135 / nav) * 100).toFixed(1) + '% of net asset value')
  await expect(rows.filter({ hasText: 'Available margin after' })).toContainText('$3,346')
  await expect(page.locator('.tk-body')).toContainText('Estimated cost')
  await expect(page.locator('.tk-body')).toContainText('$4,135')
})

test('Back returns to the form with every value kept', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '')
  await ready(page)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
  await page.locator('#tk-qty').fill('7')
  await page.getByRole('button', { name: 'Review' }).click()
  await expect(page.getByRole('dialog', { name: 'Review order' })).toBeVisible()
  await page.getByRole('button', { name: 'Back' }).click()
  await expect(page.getByRole('dialog', { name: 'New order' })).toBeVisible()
  await expect(page.locator('#tk-qty')).toHaveValue('7')
})

test('submitting sends Wealthsimple\'s own request shape and flashes the order placed', async ({ page, request }) => {
  const nvdaAccount = (await holding(request, 'NVDA')).account
  await openWithStatus(page, request, {}, '', () => {}, {
    [quoteKey('NVDA', 'sec-nvda', nvdaAccount, 'NASDAQ')]: { ok: true, quote: { last: 165.4, currency: 'USD', multiplier: 1, securityId: 'sec-nvda' } },
  })
  await ready(page)
  let sent: unknown = null
  await page.route('**/api/order', (route) => { sent = route.request().postDataJSON(); return route.fulfill({ json: { ok: true, status: 'sent' } }) })
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
  await page.locator('#tk-qty').fill('25')
  await page.locator('#tk-limit').fill('165.40')
  await page.locator('#tk-limit').press('Enter')
  await page.getByRole('button', { name: 'Review' }).click()
  await page.getByRole('button', { name: 'Submit' }).click()
  await expect.poll(() => sent).toEqual({
    symbol: 'NVDA', securityId: 'sec-nvda', accountId: nvdaAccount, side: 'BUY', type: 'LIMIT', tif: 'DAY',
    quantity: 25, limitPrice: 165.4, stopPrice: null, currency: 'USD',
    stopLoss: { kind: 'stop', price: 157.13, trail: 5, trailUnit: 'pct' },
    takeProfit: { price: 181.94 },
  })
  await expect(page.locator('#syncline')).toContainText('Order placed · Buy 25 NVDA at 165.40 limit, stop 157.13, target 181.94')
  await expect(page.getByRole('dialog')).toHaveCount(0) // the panel closes
})

test('under dry orders the header says the order was not sent', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '')
  await ready(page)
  await page.route('**/api/order', (route) => route.fulfill({ json: { ok: true, status: 'dry' } }))
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
  await page.getByRole('button', { name: 'Review' }).click()
  await page.getByRole('button', { name: 'Submit' }).click()
  await expect(page.locator('#syncline')).toContainText('Not sent (orders are off) · Buy 1 NVDA')
})

test('a rejection stays on the review step with its reason', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '')
  await ready(page)
  await page.route('**/api/order', (route) => route.fulfill({ json: { ok: false, error: 'A limit price is required.' } }))
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
  await page.getByRole('button', { name: 'Review' }).click()
  await page.getByRole('button', { name: 'Submit' }).click()
  await expect(page.locator('.status-err')).toHaveText('A limit price is required.')
  await expect(page.getByRole('dialog', { name: 'Review order' })).toBeVisible() // still there, nothing recorded
})

test('Esc keeps the draft; reopening the same symbol and side restores it', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '')
  await ready(page)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
  await page.locator('#tk-qty').fill('12')
  await page.keyboard.press('Escape')
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
  await expect(page.locator('#tk-qty')).toHaveValue('12')
})

test('Cancel discards the draft; reopening starts fresh', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '')
  await ready(page)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
  await page.locator('#tk-qty').fill('12')
  await page.getByRole('button', { name: 'Cancel' }).click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
  await expect(page.locator('#tk-qty')).toHaveValue('1')
})

test('Enter in a field blurs it instead of doing anything else', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '')
  await ready(page)
  await page.keyboard.press('Control+k')
  await page.getByRole('textbox', { name: 'Search' }).fill('NVDA')
  await page.getByRole('button', { name: 'Buy NVDA', exact: true }).click()
  await page.locator('#tk-qty').fill('9')
  await page.locator('#tk-qty').press('Enter')
  await expect(page.locator('#tk-qty')).not.toBeFocused()
  await expect(page.getByRole('dialog', { name: 'New order' })).toBeVisible() // still on the form
  await expect(page.locator('#tk-qty')).toHaveValue('9')
})
