import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render } from '@testing-library/svelte'
import Options from './Options.svelte'
import type { Trade } from '../model'

// the card's document, as the stream delivers it: each watch kept, its holder written
// and its change said by the test
type Watch = { key: string; holder: { data: unknown }; changed?: () => void; stopped: boolean }
const watches: Watch[] = []
vi.mock(import('../live.svelte'), async (original) => ({
  ...(await original()),
  watchDoc: ((key: string, _params: unknown, holder: { data: unknown }, changed?: () => void) => {
    const w: Watch = { key, holder, changed, stopped: false }
    watches.push(w)
    return () => {
      w.stopped = true
    }
  }) as never,
}))

function deliver(data: unknown): void {
  const w = watches[watches.length - 1]
  w.holder.data = data
  w.changed?.()
}

// SPEC.md markets-123 to markets-129: the Options card as the server states it,
// every figure made up and the server's own.
const trade = { id: 'listing:KO@US', symbol: 'KO', exchange: 'NYSE', currency: 'USD', kind: 'Shares', listing: true } as unknown as Trade

const row = (occ: string, title: string, strike: string, cost: string | null, premium: string, y: string | null) => ({ occ, title, strike, cost, premium, yield: y })
const card = {
  ok: true,
  expiries: [
    {
      day: '2026-11-13',
      label: 'Nov 13',
      puts: { rows: [row('KO261113P00070000', 'KO Nov 13 70 put', '70', '7000', '161', '2.30'), row('KO261113P00060000', 'KO Nov 13 60 put', '60', '6000', '40', '0.67')], near: 0 },
      // no call of this expiry is bid on: the server lists none
      calls: { rows: [], near: null },
    },
    {
      day: '2026-12-11',
      label: 'Dec 11',
      puts: { rows: [row('KO261211P00066000', 'KO Dec 11 66 put', '66', '6600', '24', '0.36'), row('KO261211P00062500', 'KO Dec 11 62.50 put', '62.5', '6250', '11', '0.18')], near: 0 },
      calls: { rows: [row('KO261211C00066000', 'KO Dec 11 66 call', '66', '6842', '266', '3.89')], near: 0 },
    },
  ],
}
const contract = {
  ok: true,
  contract: {
    occ: 'KO261211P00066000',
    title: 'KO Dec 11 66 put',
    cost: '6600',
    premium: '24',
    chart: { points: [{ day: '2026-09-29', premium: '62' }, { day: '2026-09-30', premium: '61' }, { day: '2026-11-11', premium: '24' }], top: '64', middle: '44', bottom: '24', from: 'Sep 29' },
    chartFailed: null,
  },
}

function answering(answers: Record<string, unknown>) {
  const asked: string[] = []
  vi.stubGlobal('fetch', async (path: string) => {
    asked.push(String(path))
    const key = Object.keys(answers).find((k) => String(path).startsWith(k))
    return new Response(JSON.stringify(key ? answers[key] : { ok: false, error: 'not asked for' }))
  })
  return asked
}

const texts = (els: Element[]) => els.map((e) => e.textContent?.trim())

afterEach(() => {
  vi.unstubAllGlobals()
  watches.length = 0
})

describe('the Options card', () => {
  it("watches its listing's document, is drawn for none until it says the listing has options, and lets it go", async () => {
    const { container, unmount } = render(Options, { trade })
    expect(watches.map((w) => w.key)).toEqual(['options:symbol=KO&exchange=NYSE&currency=USD'])
    expect(container.textContent?.trim()).toBe('')
    deliver({ ok: true, expiries: [] })
    await new Promise((r) => setTimeout(r, 0))
    expect(container.textContent?.trim()).toBe('')
    unmount()
    expect(watches[0].stopped).toBe(true)
  })

  it('keeps its watch and its card when the page hands it an equal listing, and watches anew for another', async () => {
    const { findByTestId, rerender } = render(Options, { trade })
    deliver(card)
    await findByTestId('options-card')
    // the same address opened again: an equal listing, a new object
    await rerender({ trade: { ...trade } as Trade })
    expect(watches.length).toBe(1)
    expect(watches[0].stopped).toBe(false)
    await findByTestId('options-card')
    // another listing: the first let go, its own watched
    await rerender({ trade: { ...trade, symbol: 'PEP' } as Trade })
    expect(watches.map((w) => [w.key, w.stopped])).toEqual([
      ['options:symbol=KO&exchange=NYSE&currency=USD', true],
      ['options:symbol=PEP&exchange=NYSE&currency=USD', false],
    ])
  })

  it('shows Puts first, each expiry nearest first, and a row a strike with its Cost, Premium and Yield', async () => {
    const { findByTestId, getByRole, getAllByRole } = render(Options, { trade })
    deliver(card)
    const el = await findByTestId('options-card')
    expect(getByRole('heading', { name: 'Options' })).toBeTruthy()
    expect(getByRole('button', { name: 'Puts' }).getAttribute('aria-pressed')).toBe('true')
    expect(getByRole('button', { name: 'Calls' }).getAttribute('aria-pressed')).toBe('false')
    expect(texts(Array.from(getByRole('group', { name: 'Expiry' }).querySelectorAll('button')))).toEqual(['Nov 13', 'Dec 11'])
    expect(texts(Array.from(el.querySelectorAll('.oc-head span')))).toEqual(['Strike', 'Cost', 'Premium', 'Yield'])
    const rows = () => Array.from(el.querySelectorAll('.oc-row')).map((r) => texts(Array.from(r.querySelectorAll('span'))))
    expect(rows()).toEqual([['70', '$7,000', '$161', '2.30%'], ['60', '$6,000', '$40', '0.67%']])
    await fireEvent.click(getAllByRole('button', { name: 'Dec 11' })[0])
    expect(rows()).toEqual([['66', '$6,600', '$24', '0.36%'], ['62.50', '$6,250', '$11', '0.18%']])
    await fireEvent.click(getByRole('button', { name: 'Calls' }))
    expect(rows()).toEqual([['66', '$6,842', '$266', '3.89%']])
    // an expiry with no call bid on has no button on Calls (SPEC.md markets-125)
    expect(texts(Array.from(getByRole('group', { name: 'Expiry' }).querySelectorAll('button')))).toEqual(['Dec 11'])
    // the chain read again: the row shows what it says now
    const again = structuredClone(card)
    again.expiries[1].calls.rows[0] = row('KO261211C00066000', 'KO Dec 11 66 call', '66', '6842', '270', '3.95')
    deliver(again)
    await new Promise((r) => setTimeout(r, 0))
    expect(rows()).toEqual([['66', '$6,842', '$270', '3.95%']])
  })

  it('opens a row as its contract, its premium each day charted, and goes back', async () => {
    const asked = answering({ '/api/options/contract?': contract })
    const { findByTestId, getByRole, findByRole, queryByRole } = render(Options, { trade })
    deliver(card)
    const el = await findByTestId('options-card')
    await fireEvent.click(getByRole('button', { name: 'Dec 11' }))
    await fireEvent.click(el.querySelectorAll('.oc-row')[0])
    expect(getByRole('heading', { name: 'KO Dec 11 66 put' })).toBeTruthy()
    expect(texts(Array.from(el.querySelectorAll('.oc-figs > div')))).toEqual(['Cost$6,600', 'Premium$24'])
    const chart = await findByRole('img', { name: 'Premium each day from Sep 29 to today' })
    expect(texts(Array.from(chart.querySelectorAll('text')))).toEqual(['$64', '$44', '$24', 'Sep 29', 'Today'])
    expect(chart.querySelector('polyline')?.getAttribute('points')).toBe('0,16.5 300,19.8 600,140')
    expect(asked.some((p) => p.startsWith('/api/options/contract?') && p.includes('occ=KO261211P00066000'))).toBe(true)
    await fireEvent.click(getByRole('button', { name: 'Back' }))
    expect(queryByRole('heading', { name: 'KO Dec 11 66 put' })).toBeNull()
    expect(getByRole('heading', { name: 'Options' })).toBeTruthy()
  })

  it('draws no chart for a contract Yahoo has no bars for', async () => {
    answering({ '/api/options/contract?': { ...contract, contract: { ...contract.contract, chart: null } } })
    const { findByTestId, queryByRole, findByRole } = render(Options, { trade })
    deliver(card)
    const el = await findByTestId('options-card')
    await fireEvent.click(el.querySelectorAll('.oc-row')[0])
    await findByRole('heading', { name: 'KO Nov 13 70 put' })
    await new Promise((r) => setTimeout(r, 0))
    expect(queryByRole('img')).toBeNull()
  })
})
