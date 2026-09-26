// SPEC §4 Markets, Fear & Greed, on the card itself: both meters are kept fresh while
// the card shows, a meter being read reads `Reading…`, and the word under the dial
// is in its band's colour.

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render } from '@testing-library/svelte'
import { flushSync } from 'svelte'
import type { FearDoc } from '../generated/markets'

// the stream is stood in for: what the card watches is handed straight to it
const watched = new Map<string, { data: FearDoc | null }>()
vi.mock(import('../live'), async (original) => ({
  ...(await original()),
  watchDoc: ((key: string, _params: unknown, holder: { data: FearDoc | null }) => {
    watched.set(key, holder)
    return () => watched.delete(key)
  }) as never,
}))

const { default: FearGreed } = await import('./FearGreed.svelte')

const INDEXES = ['stocks', 'crypto']

const doc = (index: string, score: number | null, reading = false): FearDoc =>
  ({
    ok: true,
    reading,
    gauge: score == null ? null : {
      index, source: 'publisher', score, rating: 'Word for ' + score, asOf: '2026-09-19', fetchedAt: '', readVersion: 1,
      previous: [], parts: [], series: [],
    },
  }) as unknown as FearDoc

function show(index: string) {
  localStorage.setItem('bh2.fear', index)
  const view = render(FearGreed)
  flushSync()
  return view
}

beforeEach(() => watched.clear())

describe('while the card shows', () => {
  it('both meters are watched, whichever is on show, and neither once it goes', () => {
    for (const index of INDEXES) {
      const view = show(index)
      expect([...watched.keys()].sort()).toEqual(INDEXES.map((ix) => 'fear:' + ix).sort())
      view.unmount()
      expect(watched.size).toBe(0)
    }
  })
})

describe('a meter with nothing held', () => {
  it('reads "Reading…" before the server has sent it and while its publisher is read; "did not answer" only after', () => {
    for (const index of INDEXES) {
      const { container, unmount } = show(index)
      expect(container.textContent).toContain('Reading…')
      watched.get('fear:' + index)!.data = doc(index, null, true)
      flushSync()
      expect(container.textContent).toContain('Reading…')
      expect(container.textContent).not.toContain('did not answer')
      watched.get('fear:' + index)!.data = doc(index, null, false)
      flushSync()
      expect(container.textContent).toContain('The index did not answer.')
      unmount()
    }
  })
})

describe('the score and its word under the dial', () => {
  it('are drawn in the same colour, the band\'s, across the whole scale', () => {
    for (let score = 0; score <= 100; score += 5) {
      const { container, unmount } = show('stocks')
      watched.get('fear:stocks')!.data = doc('stocks', score)
      flushSync()
      const num = container.querySelector('.tab') as HTMLElement
      const word = num.nextElementSibling as HTMLElement
      expect(word.textContent).toBe('Word for ' + score)
      expect(word.style.color).toBe(num.style.color)
      expect(word.style.color).not.toBe('')
      unmount()
    }
  })
})
