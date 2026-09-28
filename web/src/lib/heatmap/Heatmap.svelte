<script lang="ts">
  // The heatmap: Markets' card (heatmapCardHtml) and, with `alone`, the heatmap on its
  // own at `#heatmap/...` (heatmapFullHtml) -- the same header row, the same box, the
  // window to itself.
  import type { HeatCounts } from '../model'
  import { heatColor } from './treemap'
  import { heatmap as doc, use, filtered } from '../subs.svelte'
  import { ICONS } from '../icons'
  import Mseg from '../markets/Mseg.svelte'
  import Icon from '../markets/Icon.svelte'
  import HeatBox from './HeatBox.svelte'
  import type { UniverseDoc } from '../generated/markets'
  import { watchDoc } from '../live.svelte'
  import { go, goHash, rewrite, route } from '../router.svelte'
  import { heat, show, remember, heatHash, applyAddress, nextScope, marketsOnShow, MARKET_U, EVERY_SCOPE } from './heat.svelte'

  let { alone = false }: { alone?: boolean } = $props()

  const UNIVERSE_OPTS = [['holdings', 'Holdings'], ['watchlist', 'Watchlist'], ['both', 'Both'], '|', ['ca', 'Canada'], ['us', 'US'], ['intl', 'International']] as const
  const SIZE_OPTS = [['value', 'Market value'], ['equal', 'Equal']] as const
  const LEGEND = [-0.03, -0.02, -0.01, -0.002, 0.002, 0.01, 0.02, 0.03]

  // The heatmap the server builds for the universe and sizing shown: its sector
  // blocks, each with its value and change, the tiles in each with the small ones
  // folded into `Other (N)`, and how many tiles each universe has.
  use('heatmap', doc, () => ({ ...filtered(), universe: heat.universe, size: heat.size }))
  // what was shown stands until the new universe's arrives, so its tiles travel
  const blocks = $derived(doc.data?.blocks ?? [])

  // Each market universe on show is a document the server is told of: it reads the
  // universe when it has no rows or they are stale, and keeps it fresh while shown,
  // whichever way it came on show (a click, the remembered choice, the address, the
  // slideshow). Its rows arrive in the model; the document says only a failed read.
  const udocs = $state<Record<string, { data: UniverseDoc | null }>>({})
  $effect.pre(() => {
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
      const counts = doc.data?.counts
      const next = nextScope(s.list, heat.universe, (u) => !!counts && (counts[u as keyof HeatCounts] ?? 0) > 0)
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
    {#if blocks.length}
      <HeatBox {blocks} universe={heat.universe} boxStyle="position:relative;flex:1;min-height:0" />
    {:else}
      <div class="muted empty" style="font-size:12px">{emptyWord}</div>
    {/if}
  </div>
{:else}
  <div class="card elev-sm" style="padding:14px 16px 16px">
    {@render header(false)}
    {#if blocks.length}
      <HeatBox {blocks} universe={heat.universe} boxStyle="position:relative;width:100%;height:430px" />
    {:else}
      <div class="muted empty" style="font-size:12px">{emptyWord}</div>
    {/if}
  </div>
{/if}
