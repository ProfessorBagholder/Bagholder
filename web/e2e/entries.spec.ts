import { expect, test } from '@playwright/test'
import { openWithStatus, ready, subUrl } from './helpers'

// SPEC §2, What you enter: an opening balance in Add trade, against units that arrived
// without a cost; an event's values on the trade page, beside the journal, while the
// event waits on them; and what was entered shows as entered by you. The made-up book
// has nothing waiting, so the document carries a stand-in: what the page does with it
// is under test here, and what the server does with an entry in its own tests
// (rust/crates/server/src/entries.rs).

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Doc = any

/** The first held share, and a stand-in for something about it that waits on the person. */
function waitingOn(m: Doc, what: 'cost-of-arrival' | 'event') {
  const p = m.positions.find((x: Doc) => x.kind === 'Shares')
  const w = { transaction: '01900000-0000-7000-8000-00000000abcd/trade', what, account: p.accountId, accountName: p.account, instrument: p.instrument, symbol: p.symbol, currency: p.currency, day: '2026-01-05', units: '40' }
  m.waiting = [w]
  return { p, w }
}

test('an opening balance is entered against the arrival it prices', async ({ page, request }) => {
  let w: Doc
  await openWithStatus(page, request, {}, '', (m) => ({ w } = waitingOn(m, 'cost-of-arrival')))
  await ready(page)
  let sent: Doc = null
  await page.route('**/api/entries', (route) => { sent = route.request().postDataJSON(); return route.fulfill({ json: { ok: true } }) })
  await page.getByRole('button', { name: 'Menu' }).click()
  await page.getByText('Add trade').click()
  // the switch is there while something waits on its cost
  await page.locator('.seg-opt', { hasText: 'Opening balance' }).click()
  await page.getByLabel('Arrival').selectOption(w.transaction)
  await expect(page.locator('#modalDlg')).toContainText(w.currency)
  await page.getByLabel('Cost').fill('1,234.56')
  await page.getByLabel('Acquired').fill('2020-05-01')
  await page.getByRole('button', { name: 'Add opening balance' }).click()
  await expect.poll(() => sent).toEqual({ entry: 'cost-of-arrival', arrival: w.transaction, cost: '1234.56', acquired: '2020-05-01' })
  await expect(page.getByText('Opening balance added')).toBeVisible()
})

test('an opening balance the server refuses says why and stays open', async ({ page, request }) => {
  await openWithStatus(page, request, {}, '', (m) => waitingOn(m, 'cost-of-arrival'))
  await ready(page)
  await page.getByRole('button', { name: 'Menu' }).click()
  await page.getByText('Add trade').click()
  await page.locator('.seg-opt', { hasText: 'Opening balance' }).click()
  // nothing chosen: said before anything is sent
  await page.getByRole('button', { name: 'Add opening balance' }).click()
  await expect(page.locator('#modalDlg .status-err')).toHaveText('The arrival, its cost and the day acquired are required.')
  const arrival = await page.getByLabel('Arrival').locator('option').nth(1).getAttribute('value')
  await page.getByLabel('Arrival').selectOption(arrival!)
  await page.getByLabel('Cost').fill('100')
  await page.getByLabel('Acquired').fill('2020-05-01')
  // the real server: the stand-in arrival is no transaction it holds
  await page.getByRole('button', { name: 'Add opening balance' }).click()
  await expect(page.locator('#modalDlg .status-err')).toContainText('no transaction')
  await expect(page.getByRole('heading', { name: 'Add trade' })).toBeVisible()
})

test('with nothing waiting on its cost, Add trade is the trade form alone', async ({ page }) => {
  await page.goto('/')
  await ready(page)
  await page.getByRole('button', { name: 'Menu' }).click()
  await page.getByText('Add trade').click()
  await expect(page.locator('#modalDlg .seg-opt', { hasText: 'Opening balance' })).toHaveCount(0)
})

test('an event waiting on a holding is entered beside its journal, as a return of capital or a spin-off', async ({ page, request }) => {
  let p: Doc, w: Doc
  await openWithStatus(page, request, {}, '', (m) => ({ p, w } = waitingOn(m, 'event')))
  await ready(page)
  const sent: Doc[] = []
  await page.route('**/api/entries', (route) => { sent.push(route.request().postDataJSON()); return route.fulfill({ json: { ok: true } }) })
  await page.goto('/#portfolio/' + encodeURIComponent(p.id))
  await expect(page).toHaveURL(subUrl('portfolio', p.id))
  await expect(page.getByText('Corporate event · ' + w.day)).toBeVisible()
  // a return of capital, a unit, entered with Enter
  await page.locator('.seg-opt', { hasText: 'Return of capital' }).click()
  await page.getByLabel('Capital returned a unit').fill('0.25')
  await page.keyboard.press('Enter')
  await expect.poll(() => sent[0]).toEqual({ entry: 'return-of-capital', distribution: w.transaction, perUnit: '0.25' })
  // a spin-off: the holding it came out of, and its share of that one's cost
  await page.locator('.seg-opt', { hasText: 'Spin-off' }).click()
  const parent = page.getByLabel('Parent')
  const choice = await parent.locator('option').nth(1).getAttribute('value')
  await parent.selectOption(choice!)
  await page.getByLabel('Share of cost').fill('0.3')
  await page.getByRole('button', { name: 'Enter' }).click()
  await expect.poll(() => sent[1]).toEqual({ entry: 'spin-off', event: w.transaction, parent: choice, children: [{ instrument: p.instrument, costShare: '0.3' }] })
})

test('a holding with nothing waiting has no event form', async ({ page, request }) => {
  const m = await (await request.get('/api/figures')).json()
  const p = m.positions.find((x: Doc) => x.kind === 'Shares')
  await page.goto('/#portfolio/' + encodeURIComponent(p.id))
  await ready(page)
  await expect(page.getByText('Corporate event')).toHaveCount(0)
})

test('a cost you entered shows as entered by you on its page', async ({ page, request }) => {
  let p: Doc
  await openWithStatus(page, request, {}, '', (m) => {
    p = m.positions.find((x: Doc) => x.kind === 'Shares')
    p.flags = [...p.flags, 'entered']
  })
  await ready(page)
  await page.goto('/#portfolio/' + encodeURIComponent(p.id))
  await expect(page.locator('#page')).toContainText('· Entered by you')
})
