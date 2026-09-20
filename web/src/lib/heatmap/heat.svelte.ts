// What the heatmap shows -- its scope and sizing, remembered in the browser -- and, on
// its own, the slideshow. One state for the card and for the heatmap on its own, so the
// address, the card and the page agree.
import { HEAT_UNIVERSES, type HeatAddress } from '../router.svelte'

export const MARKET_U: Record<string, string> = { ca: 'Canada', us: 'US', intl: 'International' }

function remembered(): { universe: string; size: string } {
  try {
    return { universe: 'holdings', size: 'value', ...JSON.parse(localStorage.getItem('bh2.heatmap') || '{}') }
  } catch {
    return { universe: 'holdings', size: 'value' }
  }
}

export const heat = $state(remembered())
/** The slideshow, when one runs: the scopes it goes through and how long it stays on each. */
export const show = $state<{ on: { list: string[]; seconds: number } | null }>({ on: null })

export function remember(): void {
  try {
    localStorage.setItem('bh2.heatmap', JSON.stringify(heat))
  } catch {
    /* a browser that keeps nothing still shows the choice */
  }
}

/** The address of the heatmap on its own as it now stands. */
export function heatHash(): string {
  const s = show.on
  return '#heatmap/' + (s ? s.list.join(',') : heat.universe) + '/' + heat.size + (s ? '/' + s.seconds : '')
}

/** What an address asks for becomes the heatmap's own choice. */
export function applyAddress(a: HeatAddress): void {
  show.on = a.seconds ? { list: a.list, seconds: a.seconds } : null
  const universe = a.list.length ? (a.list.includes(heat.universe) ? heat.universe : a.list[0]) : heat.universe
  const size = a.size ?? heat.size
  if (universe === heat.universe && size === heat.size) return
  heat.universe = universe
  heat.size = size
  remember()
}

/**
 * The slideshow's next scope: the next in the list with something to show. `has` says
 * whether a scope has tiles; `unread` is told of each market never read on the way, so
 * it can be asked for.
 */
export function nextScope(list: string[], current: string, has: (u: string) => boolean, unread: (u: string) => void = () => {}): string {
  const i = Math.max(0, list.indexOf(current))
  for (let k = 1; k <= list.length; k++) {
    const u = list[(i + k) % list.length]
    if (has(u)) return u
    if (MARKET_U[u]) unread(u)
  }
  return current
}

export const EVERY_SCOPE = { list: HEAT_UNIVERSES.slice(), seconds: 20 }
