<script lang="ts">
  // The heatmap card (heatmapCardHtml) and, expanded, the full-page view
  // (heatmapFullHtml at #heatFull). The reference page reaches the full view by a
  // route; because this migration may not add routes, the expander opens it as a
  // fixed overlay instead — the same #heatFull markup, the same controls.
  import type { Markets } from '../model'
  import type { Tile } from './treemap'
  import { heatColor } from './treemap'
  import { ICONS } from '../icons'
  import Mseg from '../markets/Mseg.svelte'
  import Icon from '../markets/Icon.svelte'
  import HeatBox from './HeatBox.svelte'
  import { api } from '../markets/util'

  let { markets }: { markets: Markets } = $props()

  const MARKET_U: Record<string, string> = { ca: 'Canada', us: 'US', intl: 'International' }
  const HEAT_UNIVERSES = ['holdings', 'watchlist', 'both', 'ca', 'us', 'intl']
  const UNIVERSE_OPTS = [['holdings', 'Holdings'], ['watchlist', 'Watchlist'], ['both', 'Both'], '|', ['ca', 'Canada'], ['us', 'US'], ['intl', 'International']] as const
  const SIZE_OPTS = [['value', 'Market value'], ['equal', 'Equal']] as const
  const LEGEND = [-3, -2, -1, -0.2, 0.2, 1, 2, 3]

  function load(): { universe: string; size: string } {
    try {
      return { universe: 'holdings', size: 'value', ...JSON.parse(localStorage.getItem('bh2.heatmap') || '{}') }
    } catch {
      return { universe: 'holdings', size: 'value' }
    }
  }
  let heat = $state(load())
  function save() {
    try {
      localStorage.setItem('bh2.heatmap', JSON.stringify(heat))
    } catch {
      /* ignore */
    }
  }

  let full = $state(false)
  let cycling = $state(false)

  // ledger's heatTiles: the tiles for the chosen universe
  function heatTilesFor(h: { universe: string; size: string }): Tile[] {
    const held = (markets.holdings || []) as Tile[]
    const watched = (markets.watchlist || []).filter((w) => !w.positionId)
    let tiles: Tile[]
    if (MARKET_U[h.universe]) {
      tiles = ((markets.universes as Record<string, Tile[]>)[h.universe] || []).slice()
    } else {
      const minHeld = held.length ? Math.min(...held.map((t) => t.value)) : 1
      const w: Tile[] = watched.map((x) => ({ id: null, symbol: x.symbol, exchange: x.exchange, value: h.universe === 'both' ? minHeld : 1, percentChange: x.percentChange, sector: x.sector }))
      tiles = h.universe === 'holdings' ? held.slice() : h.universe === 'watchlist' ? w : held.concat(w)
    }
    if (h.size === 'equal') tiles = tiles.map((t) => ({ ...t, value: 1 }))
    return tiles
  }
  const tiles = $derived(heatTilesFor(heat))
  const emptyWord = $derived(heat.universe === 'watchlist' ? 'Nothing watched.' : MARKET_U[heat.universe] ? 'Not read yet.' : 'No open positions.')

  function pick(patch: Partial<typeof heat>) {
    if ('universe' in patch && cycling) cycling = false // a scope picked by hand ends the cycling
    heat = { ...heat, ...patch }
    save()
    // a market universe never read: ask for it now
    if (MARKET_U[heat.universe] && !((markets.universes as Record<string, unknown[]>)[heat.universe] || []).length) api('POST', '/api/markets/refresh', {})
  }

  // the slideshow: every 20s, the next scope with something to show
  $effect(() => {
    if (!cycling || !full) return
    const id = setInterval(() => {
      const i = Math.max(0, HEAT_UNIVERSES.indexOf(heat.universe))
      for (let k = 1; k <= HEAT_UNIVERSES.length; k++) {
        const u = HEAT_UNIVERSES[(i + k) % HEAT_UNIVERSES.length]
        const next = { ...heat, universe: u }
        if (MARKET_U[u] && !((markets.universes as Record<string, unknown[]>)[u] || []).length) api('POST', '/api/markets/refresh', {})
        if (heatTilesFor(next).length) {
          heat = next
          save()
          break
        }
      }
    }, 20000)
    return () => clearInterval(id)
  })

  function closeFull() {
    full = false
    cycling = false
  }

  // Escape leaves the full-page heatmap (the reference does this from its route).
  // Capture phase so it runs before App's global Escape cascade, and stop there.
  function onKeyCapture(e: KeyboardEvent) {
    if (full && e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      closeFull()
    }
  }
</script>

<svelte:window onkeydowncapture={onKeyCapture} />

{#snippet header(isFull: boolean)}
  <div style="display:flex;align-items:center;gap:14px{isFull ? '' : ';margin-bottom:12px'}">
    <h5>Heatmap</h5>
    <Mseg options={UNIVERSE_OPTS} cur={heat.universe} onpick={(u) => pick({ universe: u })} />
    <Mseg options={SIZE_OPTS} cur={heat.size} onpick={(s) => pick({ size: s })} />
    <span style="margin-left:auto;display:flex;align-items:center;gap:8px;font-size:11px;color:var(--ink55)">
      <span>−3%</span>
      {#each LEGEND as c (c)}<span style="width:16px;height:9px;border-radius:2px;background:{heatColor(c)}"></span>{/each}
      <span>+3%</span>
    </span>
    {#if isFull}
      <button class="heat-ghost" aria-label={cycling ? 'Stop cycling' : 'Cycle through the scopes'} onclick={() => (cycling = !cycling)}><Icon d={cycling ? ICONS.pause : ICONS.play} size={14} /></button>
      <button class="heat-ghost" aria-label="Back to Markets" onclick={closeFull}><Icon d={ICONS.x} size={14} /></button>
    {:else}
      <button class="heat-ghost" aria-label="Heatmap on its own" onclick={() => (full = true)}><Icon d={ICONS.arrowsOut} size={14} /></button>
    {/if}
  </div>
{/snippet}

<div class="card elev-sm" style="padding:14px 16px 16px">
  {@render header(false)}
  {#if tiles.length}
    <HeatBox {tiles} universe={heat.universe} boxStyle="position:relative;width:100%;height:430px" />
  {:else}
    <div class="muted empty" style="font-size:12px">{emptyWord}</div>
  {/if}
</div>

{#if full}
  <div class="heat-full-scrim">
    <div id="heatFull">
      {@render header(true)}
      {#if tiles.length}
        <HeatBox {tiles} universe={heat.universe} boxStyle="position:relative;flex:1;min-height:0" />
      {:else}
        <div class="muted empty" style="font-size:12px">{emptyWord}</div>
      {/if}
    </div>
  </div>
{/if}

<style>
  /* the reference page renders #heatFull as its own route; here it is an overlay
     over the app, on the page's own background */
  .heat-full-scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: var(--bg);
  }
</style>
