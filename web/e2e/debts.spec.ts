import { expect, test } from '@playwright/test'
import { modelDoc, money0, standIn, streamBody } from './helpers'

// Debts held outside the broker (docs/plans/debts-held-outside-wealthsimple.md):
// repayments in each month's Cashflow bar, what is owed under Net asset value, and
// the question for payments no rule matched. The made-up book holds no outside
// debt, so the documents are the real book's with the debts stood in.

const loan = { id: 'loan-1', name: 'Car loan', institution: 'Omicron Trust', amount: '9000', asOf: '2026-10-01T12:00:00Z' }
const card = { id: 'card-1', name: 'Visa', institution: 'Zeta Bank', amount: '830.50', asOf: null }

test('what is owed and the net after it sit under Net asset value, each debt with when it was stated on hover', async ({ page, request }) => {
  const model = await modelDoc(request)
  const nav = model.portfolio.nav as string
  model.portfolio.owed = { total: '9830.50', net: String(Number(nav) - 9830.5), debts: [loan, card] }
  await standIn(page, streamBody(model))
  await page.goto('/#portfolio')
  const tile = page.locator('#page .kpi').filter({ hasText: 'Net asset value' })
  await expect(tile.locator('.s')).toHaveText(money0('9830.50') + ' owed · ' + money0(String(Number(nav) - 9830.5)) + ' net')
  await tile.hover()
  await expect(tile.locator('.tip')).toContainText('Car loan · Omicron Trust')
  await expect(tile.locator('.tip')).toContainText('As of 1 Oct')
  await expect(tile.locator('.tip')).toContainText('As of an unstated date')
})

test('with nothing owed, or under one account, Net asset value says its accounts as before', async ({ page, request }) => {
  const model = await modelDoc(request)
  model.portfolio.owed = null
  await standIn(page, streamBody(model))
  await page.goto('/#portfolio')
  await expect(page.locator('#page .kpi').filter({ hasText: 'Net asset value' }).locator('.s')).toContainText(/accounts?, /)
})

test("a month's repayments are their own part of the bar, in the legend and the hover, and Net cashflow takes them off", async ({ page, request }) => {
  const model = await modelDoc(request)
  const months = model.cashflow.months as { key: string; value: string; interest: string; repayments: string; net: string }[]
  const m = months[months.length - 1]
  m.repayments = '450'
  m.net = String(Number(m.value) - Number(m.interest) - 450)
  await standIn(page, streamBody(model))
  await page.goto('/#cashflow')
  await expect(page.getByText('Repayments', { exact: true }).first()).toBeVisible()
  await page.locator('#cfPlot .bar-col').last().hover()
  const tip = page.locator('#cfPlot .tip')
  await expect(tip).toContainText('Repayments')
  await expect(tip).toContainText('−' + money0('450'))
})

test('a payee no rule matched is asked once, with a choice of each debt it may be, and the answer is sent', async ({ page, request }) => {
  const model = await modelDoc(request)
  model.cashflow.asked = [{ key: 'debit:zeta bank', payee: 'ZETA BANK', count: 3, candidates: [card, { ...loan, id: 'loc-1', name: 'Line of credit', institution: 'Zeta Bank' }] }]
  await standIn(page, streamBody(model))
  let sent: unknown = null
  await page.route('**/api/debts/answer', async (route) => {
    sent = route.request().postDataJSON()
    await route.fulfill({ status: 200, contentType: 'application/json', body: '{"ok":true}' })
  })
  await page.goto('/#cashflow')
  const ask = page.locator('.asked')
  await expect(ask).toContainText('3 payments to ZETA BANK: which debt did they repay?')
  const choices = page.getByRole('group', { name: 'Debt repaid by payments to ZETA BANK' })
  await expect(choices.getByRole('button')).toHaveText(['Visa · Zeta Bank', 'Line of credit · Zeta Bank', 'Not a debt'])
  await choices.getByRole('button', { name: 'Line of credit · Zeta Bank' }).click()
  await expect.poll(() => sent).toEqual({ payee: 'debit:zeta bank', debt: 'loc-1' })
  await choices.getByRole('button', { name: 'Not a debt' }).click()
  await expect.poll(() => sent).toEqual({ payee: 'debit:zeta bank', debt: null })
})
