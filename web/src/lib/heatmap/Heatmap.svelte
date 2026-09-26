<script lang="ts">
  // The heatmap: Markets' card (heatmapCardHtml) and, with `alone`, the heatmap on its
  // own at `#heatmap/...` (heatmapFullHtml) -- the same header row, the same box, the
  // window to itself.
  import type { Markets } from '../model'
  import type { Tile } from './treemap'
  import { heatColor } from './treemap'
  import { ICONS } from '../icons'
  import Mseg from '../markets/Mseg.svelte'
  import Icon from '../markets/Icon.svelte'
  import HeatBox from './HeatBox.svelte'
  import type { UniverseDoc } from '../generated/markets'
  import { watchDoc } from '../live'
  import { go, goHash, rewrite, route } from '../router.svelte'
  import { heat, show, remember, heatHash, applyAddress, nextScope, marketsOnShow, MARKET_U, EVERY_SCOPE } from './heat.svelte'

  let { markets, alone = false }: { markets: Markets; alone?: boolean } = $props()

  const UNIVERSE_OPTS = [['holdings', 'Holdings'], ['watchlist', 'Watchlist'], ['both', 'Both'], '|', ['ca', 'Canada'], ['us', 'US'], ['intl', 'International']] as const
  const SIZE_OPTS = [['value', 'Market value'], ['equal', 'Equal']] as const
  const LEGEND = [-3, -2, -1, -0.2, 0.2, 1, 2, 3]

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

  // Each market universe on show is a document the server is told of: it reads the
  // universe when it has no rows or they are stale, and keeps it fresh while shown,
  // whichever way it came on show (a click, the remembered choice, the address, the
  // slideshow). Its rows arrive in the model; the document says only a failed read.
  const udocs = $state<Record<string, { data: UniverseDoc | null }>>({})
  $effect(() => {
    const stops = marketsOnShow(heat.universe, alone ? show.on : null).map((u) => {
      if (!udocs[u]) udocs[u] = { data: null }
      return watchDoc('universe:' + u, {}, udocs[u])
    })
    return () => stops.forEach((stop) => stop())
  })
  const emptyWord = $derived(heat.universe === 'watchlist' ? 'Nothing watched.' : MARKET_U[heat.universe] ? udocs[heat.universe]?.data?.failed || 'Not read yet.' : 'No open positions.')

  function pick(patch: Partial<typeof heat>) {
    if ('universe' in patch) show.on = null // a scope picked by hand ends the cycling
    Object.assign(heat, patch)
    remember()
    if (alone) rewrite(heatHash()) // the address stays true, without a history entry
  }

  // on its own, the address says what to show: a bookmark lands where the view was left
  $effect(() => {
    if (alone && route.heat) applyAddress(route.heat)
  })

  // The slideshow: after each dwell, the next scope with something to show. The dwell is
  // the reader's own figure, from the address or the play button -- a timer by nature.
  $effect(() => {
    const s = show.on
    if (!alone || !s) return
    const id = setInterval(() => {
      const next = nextScope(s.list, heat.universe, (u) => heatTilesFor({ ...heat, universe: u }).length > 0)
      if (next === heat.universe) return
      heat.universe = next
      remember()
    }, s.seconds * 1000)
    return () => clearInterval(id)
  })

  function toggleShow() {
    show.on = show.on ? null : { ...EVERY_SCOPE }
    rewrite(heatHash()) // a bookmark of a running show restarts it
  }
</script>

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
      <button class="heat-ghost" aria-label={show.on ? 'Stop cycling' : 'Cycle through the scopes'} onclick={toggleShow}><Icon d={show.on ? ICONS.pause : ICONS.play} size={14} /></button>
      <button class="heat-ghost" aria-label="Back to Markets" onclick={() => go('markets')}><Icon d={ICONS.x} size={14} /></button>
    {:else}
      <button class="heat-ghost" aria-label="Heatmap on its own" onclick={() => goHash(heatHash())}><Icon d={ICONS.arrowsOut} size={14} /></button>
    {/if}
  </div>
{/snippet}

{#if alone}
  <div id="heatFull">
    {@render header(true)}
    {#if tiles.length}
      <HeatBox {tiles} universe={heat.universe} boxStyle="position:relative;flex:1;min-height:0" />
    {:else}
      <div class="muted empty" style="font-size:12px">{emptyWord}</div>
    {/if}
  </div>
{:else}
  <div class="card elev-sm" style="padding:14px 16px 16px">
    {@render header(false)}
    {#if tiles.length}
      <HeatBox {tiles} universe={heat.universe} boxStyle="position:relative;width:100%;height:430px" />
    {:else}
      <div class="muted empty" style="font-size:12px">{emptyWord}</div>
    {/if}
  </div>
{/if}
