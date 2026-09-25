import { describe, expect, it } from 'vitest'

// No money arithmetic in the page (docs/plans/stage-3c-switch.md, §5): an amount,
// a quantity or a price arrives as exact decimal text (`Dec`), which the type
// checker refuses to do arithmetic on; what could get round it is turning one into
// a float. Every place the page makes a number is listed here with why it is not
// a figure being worked out; one added fails until it is argued for here.
const sources = import.meta.glob(['/src/**/*.svelte', '/src/**/*.ts', '!/src/**/*.test.ts', '!/src/lib/generated/**'], {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>

const MAKES_A_NUMBER = /Number\(|parseFloat\(|parseInt\(|as unknown as number|as number\b/g

const ALLOWED: Record<string, [number, string]> = {
  '/src/lib/dec.ts': [4, "`plot` (a chart's coordinate, never shown), `ticketNumber` (the order API's own numbers), and Intl's typing of the exact text it formats"],
  '/src/lib/live.ts': [1, "a stream message's number"],
  '/src/lib/router.svelte.ts': [1, "the heatmap address's seconds"],
  '/src/lib/Portfolio.svelte': [1, "the count in the server's `Other (n)`"],
  '/src/lib/cuttip.ts': [1, 'a CSS length'],
  '/src/lib/Dashboard.svelte': [1, 'a yearly return, a ratio the server states as a number'],
  '/src/lib/fmt.ts': [1, 'formatting a number the old wire still sends (the markets context, stage 5)'],
  '/src/lib/markets/util.ts': [3, 'formatting the markets context (stage 5), and a date\'s parts'],
  '/src/lib/trade/chart.ts': [2, "a chart bar's own time key"],
  '/src/lib/trade/discStore.svelte.ts': [1, "a disclosure's size in its words"],
  '/src/lib/trade/shorts.svelte.ts': [5, "a date's parts"],
  '/src/lib/trade/ShortInterest.svelte': [4, 'share counts the short-interest source states as numbers (stage 5)'],
  '/src/lib/actions/roll.ts': [2, "a rolling digit's position"],
  '/src/lib/actions/tradeChart.ts': [6, "a chart bar's values for the charting library"],
  '/src/lib/ticket/ticket.svelte.ts': [9, 'the order API takes numbers: the server\'s figures, the held units and the Max cross into them (`ticketNumber`)'],
  '/src/lib/ticket/OrderTicket.svelte': [1, "the units an amount bought, as the server stated them, into the field's number"],
  '/src/lib/ticket/vals.ts': [1, 'the number typed into a field, which the server reads as text'],
  '/src/lib/orders/OrdersPanel.svelte': [4, "the orders document's numbers (the old store's until stage 4 moves orders)"],
  '/src/lib/orders/orders.svelte.ts': [1, 'the number typed into an order edit (stage 4)'],
  '/src/lib/heatmap/treemap.ts': [1, "a tile's day change, a ratio (stage 5)"],
}

describe('the page does no money arithmetic', () => {
  it('makes a number only where one is argued for', () => {
    const found: Record<string, number> = {}
    for (const [path, text] of Object.entries(sources)) {
      const n = (text.match(MAKES_A_NUMBER) || []).length
      if (n) found[path] = n
    }
    expect(found).toEqual(Object.fromEntries(Object.entries(ALLOWED).map(([p, [n]]) => [p, n])))
  })
})
