// SPEC §4 Disclosures, on the card itself: a column appears only when it carries a
// value, and a forced read shows the Reading state until it answers.

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render } from '@testing-library/svelte'
import { flushSync, tick } from 'svelte'
import type { FilingsDoc, Filing, Trade } from '../model'

// the stream is stood in for: what the card watches is handed straight to it
const holders: { data: FilingsDoc | null }[] = []
vi.mock(import('../live'), async (original) => ({
  ...(await original()),
  watchDoc: ((_key: string, _params: unknown, holder: { data: FilingsDoc | null }) => {
    holders.push(holder)
    return () => {}
  }) as never,
}))
// the forced read answers when the test says so
let answer: (a: Record<string, unknown>) => void = () => {}
const asked: unknown[] = []
vi.mock(import('../api'), async (original) => ({
  ...(await original()),
  call: ((_key: string, input: unknown) => {
    asked.push(input)
    return new Promise((r) => (answer = r))
  }) as never,
}))

const { default: Disclosures } = await import('./Disclosures.svelte')

let n = 0
const listing = (): Trade => ({ symbol: 'T' + ++n, name: 'Test Co', exchange: 'NASDAQ', currency: 'USD' }) as unknown as Trade

const row = (id: string, summary: string): Filing =>
  ({
    id, source: 'SEC', category: 'Financials', type: '10-Q', title: '', subject: 'Title ' + id, summary,
    date: '2026-09-0' + id.length, dateText: '', size: '', url: '', profileNo: '', issuer: '',
    enrichedAt: '', enrichVersion: 11, enrichFinal: true, fetchedAt: '',
  }) as unknown as Filing

const doc = (symbol: string, filings: Filing[]): FilingsDoc =>
  ({
    ok: true, symbol, available: true, sources: { SEC: { available: true, filer: true, matched: true } }, categories: [],
    fetchedAt: '2026-09-19T00:00:00Z', everRead: true, summaryStatus: 'ready', reading: [], filings,
  }) as unknown as FilingsDoc

function show(filings: Filing[]) {
  const t = listing()
  const view = render(Disclosures, { props: { trade: t } })
  flushSync()
  holders[holders.length - 1].data = doc(t.symbol, filings)
  flushSync()
  return view.container
}

beforeEach(() => {
  holders.length = 0
  asked.length = 0
})

describe('the Summary column', () => {
  it('is not drawn while no row has a summary', () => {
    const c = show([row('a', ''), row('bb', '')])
    expect(c.querySelectorAll('.dc-row')).toHaveLength(2)
    expect(c.querySelector('.dc-head')!.textContent).not.toContain('Summary')
    expect(c.querySelectorAll('.dc-sumcell')).toHaveLength(0)
  })

  it('appears the moment one row has a summary, on every row', () => {
    const c = show([row('a', ''), row('bb', '')])
    const h = holders[holders.length - 1]
    h.data = { ...h.data!, filings: [row('a', ''), row('bb', 'One sentence.')] }
    flushSync()
    expect(c.querySelector('.dc-head')!.textContent).toContain('Summary')
    expect(c.querySelectorAll('.dc-sumcell')).toHaveLength(2)
  })
})

describe('the re-read button', () => {
  it('shows the Reading state until the forced read answers, whatever the stream sends meanwhile', async () => {
    const c = show([row('a', 'One sentence.')])
    ;(c.querySelector('[aria-label="Re-read disclosures"]') as HTMLButtonElement).click()
    flushSync()
    expect(asked).toHaveLength(1)
    expect(c.textContent).toContain('Reading disclosures…')
    expect(c.querySelectorAll('.dc-row .dc-title')).toHaveLength(0)
    // the list changing on the stream does not end the Reading state
    const h = holders[holders.length - 1]
    h.data = { ...h.data!, filings: [row('a', 'One sentence.'), row('bb', '')] }
    flushSync()
    expect(c.textContent).toContain('Reading disclosures…')
    answer({ ok: true })
    await tick()
    await tick()
    flushSync()
    expect(c.textContent).not.toContain('Reading disclosures…')
    expect(c.querySelectorAll('.dc-row')).toHaveLength(2)
  })

  it('says so when the forced read is refused', async () => {
    const c = show([row('a', 'One sentence.')])
    ;(c.querySelector('[aria-label="Re-read disclosures"]') as HTMLButtonElement).click()
    flushSync()
    answer({ ok: false, error: 'store unavailable' })
    await tick()
    await tick()
    flushSync()
    expect(c.textContent).toContain('Could not read disclosures: store unavailable')
  })
})
