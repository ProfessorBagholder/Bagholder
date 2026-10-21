import { expect, test } from '@playwright/test'
import { modelDoc, money0, standIn, streamBody } from './helpers'

// Debts held outside the broker (docs/plans/debts-held-outside-wealthsimple.md):
// repayments in each month's Cashflow bar, what is owed among the Net worth tab's
// liabilities, and the question for payments no rule matched. The made-up book
// holds no outside debt, so the documents are the real book's with the debts
// stood in. All figures are made up.

const loan = { id: 'loan-1', kind: 'AutoLoan', name: 'Car loan', institution: 'Omicron Trust', amount: '9000', asOf: '2026-10-01T12:00:00Z' }
const card = { id: 'card-1', kind: 'CreditCard', name: 'Visa', institution: 'Zeta Bank', amount: '830.50', asOf: null }

test('what is owed outside is a liability on the Net worth tab, each with its kind and institution, and net worth is after it', async ({ page, request }) => {
  const model = await modelDoc(request)
  const nw = model.portfolio.netWorth
  const owed = Number(nw.liabilitiesTotal) + 9830.5
  nw.liabilities = [...nw.liabilities, loan, card]
  nw.liabilitiesTotal = String(owed)
  nw.netWorth = String(Number(nw.assetsTotal) - owed)
  await standIn(page, streamBody(model))
  await page.goto('/#networth')
  const tile = (label: string) => page.locator('#page .kpi').filter({ has: page.locator('.lbl', { hasText: new RegExp('^' + label + '$') }) })
  await expect(tile('Liabilities').locator('.v')).toHaveText(money0(String(owed)))
  await expect(tile('Net worth').locator('.v')).toHaveText(money0(nw.netWorth))
  await expect(tile('Liabilities').locator('.s')).toHaveText(nw.liabilities.length + ' debts')
  const rows = page.locator('#page .card').filter({ has: page.locator('h5', { hasText: /^Liabilities$/ }) }).locator('tbody tr')
  await expect(rows.filter({ hasText: 'Car loan' })).toContainText('Auto loan · Omicron Trust')
  await expect(rows.filter({ hasText: 'Car loan' }).locator('td').last()).toHaveText(money0('9000'))
  await expect(rows.filter({ hasText: 'Visa' })).toContainText('Credit card · Zeta Bank')
  await expect(rows.filter({ hasText: 'Visa' }).locator('td').last()).toHaveText(money0('830.50'))
})

test('Net asset value says its accounts whatever is owed: what is owed is on the Net worth tab', async ({ page, request }) => {
  const model = await modelDoc(request)
  model.portfolio.netWorth.liabilities = [loan, card]
  await standIn(page, streamBody(model))
  await page.goto('/#portfolio')
  const tile = page.locator('#page .kpi').filter({ hasText: 'Net asset value' })
  await expect(tile.locator('.s')).toContainText(/accounts?, /)
  await expect(tile.locator('.s')).not.toContainText('owed')
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
  model.cashflow.asked = [{ key: 'debit:zeta bank', payee: 'ZETA BANK', count: 3, candidates: [card, { ...loan, id: 'loc-1', kind: 'LineOfCredit', name: 'Line of credit', institution: 'Zeta Bank' }] }]
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
