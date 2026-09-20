// Hash router as reactive state. The active tab is derived from location.hash;
// components read `route.tab` and Svelte re-renders the affected parts only —
// no whole-page redraw, no manual dispatch table.

export const TABS = ['dashboard', 'trades', 'portfolio', 'markets', 'cashflow'] as const
export type Tab = (typeof TABS)[number]

export const TAB_LABEL: Record<Tab, string> = {
  dashboard: 'Dashboard',
  trades: 'Trades',
  portfolio: 'Portfolio',
  markets: 'Markets',
  cashflow: 'Cashflow',
}

// An address an older version wrote, still bookmarked.
const ALIAS: Record<string, Tab> = { positions: 'portfolio' }

export function tabFromHash(hash: string): Tab {
  const h = hash.replace(/^#\/?/, '').split('/')[0]
  if (h === 'heatmap') return 'markets' // the heatmap on its own is Markets' card, alone
  return (TABS as readonly string[]).includes(h) ? (h as Tab) : (ALIAS[h] ?? 'dashboard')
}

// The heatmap on its own, `#heatmap/<scopes>/<size>/<seconds>`: what its address asks
// for. A list of scopes with a dwell of five seconds to an hour is a slideshow.
export const HEAT_UNIVERSES = ['holdings', 'watchlist', 'both', 'ca', 'us', 'intl']
export const HEAT_SIZES = ['value', 'equal']
export interface HeatAddress {
  list: string[]
  size: string | null
  seconds: number | null
}
export function heatFromHash(hash: string): HeatAddress | null {
  const parts = decodeURIComponent(hash.replace(/^#\/?/, '')).split('/')
  if (parts[0] !== 'heatmap') return null
  const list = (parts[1] || '').split(',').filter((u) => HEAT_UNIVERSES.includes(u))
  const secs = parseInt(parts[3], 10)
  return {
    list,
    size: HEAT_SIZES.includes(parts[2]) ? parts[2] : null,
    seconds: list.length > 1 && secs >= 5 && secs <= 3600 ? secs : null,
  }
}

// The second path segment — a drill-down id (e.g. a selected trade), or null.
export function subFromHash(hash: string): string | null {
  const parts = hash.replace(/^#\/?/, '').split('/')
  if (parts[0] === 'heatmap') return null
  return parts.length > 1 && parts[1] ? decodeURIComponent(parts.slice(1).join('/')) : null
}

const initHash = typeof location !== 'undefined' ? location.hash : ''
export const route = $state<{ tab: Tab; sub: string | null; heat: HeatAddress | null }>({ tab: tabFromHash(initHash), sub: subFromHash(initHash), heat: heatFromHash(initHash) })

// Where the list was scrolled to when a row of it was opened: Back returns there,
// and a page newly opened starts at its top.
let listScrollY = 0

// The route follows the address. It is read at once wherever the page itself changes
// the address (`go`, `goSub`), since the browser only announces a change of hash some
// time after it happens; the announcement still covers Back, Forward and an address typed.
function follow(): void {
  const was = { tab: route.tab, sub: route.sub }
  const heat = heatFromHash(location.hash)
  if (JSON.stringify(heat) !== JSON.stringify(route.heat)) route.heat = heat
  route.tab = tabFromHash(location.hash)
  route.sub = subFromHash(location.hash)
  if (route.tab === was.tab && route.sub === was.sub) return
  const opened = !!route.sub && route.sub !== was.sub
  const backToList = !route.sub && !!was.sub && route.tab === was.tab
  // the page under the old address is still what the window shows
  if (opened && !was.sub) listScrollY = window.scrollY
  if (opened || backToList) {
    const y = opened ? 0 : listScrollY
    // once the page under the new address has been drawn
    requestAnimationFrame(() => window.scrollTo(0, y))
  }
}

// Wire hash changes to the store; returns a teardown for onMount.
export function startRouter(): () => void {
  follow()
  window.addEventListener('hashchange', follow)
  return () => window.removeEventListener('hashchange', follow)
}

/** Leave the open row for its list without adding to the history: Back should not return to it. */
export function leaveSub(): void {
  if (!route.sub) return
  history.replaceState(null, '', '#' + route.tab)
  route.sub = null
}

/** Make the address say what the page now shows, without a history entry (the heatmap's controls, a slideshow started or stopped). */
export function rewrite(hash: string): void {
  history.replaceState(null, '', hash)
  follow()
}

/** Go to an address the page writes itself. */
export function goHash(hash: string): void {
  location.hash = hash
  follow()
}

export function go(tab: Tab): void {
  location.hash = tab
  follow()
}

export function goSub(tab: Tab, sub: string): void {
  location.hash = tab + '/' + encodeURIComponent(sub)
  follow()
}
