<script lang="ts">
  import type { Markets } from '../model'
  import { heatmapLayout, heatColor, type Tile } from './treemap'

  let { markets }: { markets: Markets } = $props()

  const MARKET_U: Record<string, string> = { ca: 'Canada', us: 'US', intl: 'International' }
  const UNIVERSES: [string, string][] = [['holdings', 'Holdings'], ['watchlist', 'Watchlist'], ['both', 'Both'], ['ca', 'Canada'], ['us', 'US'], ['intl', 'International']]

  // Per-viewer convenience, remembered in localStorage (wrapped — it can throw).
  function load(): { universe: string; size: string } {
    try {
      return { universe: 'holdings', size: 'value', ...JSON.parse(localStorage.getItem('bh3.heat') || '{}') }
    } catch { return { universe: 'holdings', size: 'value' } }
  }
  let heat = $state(load())
  function save() { try { localStorage.setItem('bh3.heat', JSON.stringify(heat)) } catch { /* ignore */ } }

  const tiles = $derived.by<Tile[]>(() => {
    const held = markets.holdings || []
    const watched = (markets.watchlist || []).filter((w) => !w.positionId)
    let out: Tile[]
    if (MARKET_U[heat.universe]) {
      out = ((markets.universes as Record<string, Tile[]>)[heat.universe] || []).slice()
    } else {
      const minHeld = held.length ? Math.min(...held.map((t) => t.value)) : 1
      const w: Tile[] = watched.map((x) => ({ id: null, symbol: x.symbol, exchange: x.exchange, value: heat.universe === 'both' ? minHeld : 1, percentChange: x.percentChange, sector: x.sector }))
      out = heat.universe === 'holdings' ? (held as Tile[]).slice() : heat.universe === 'watchlist' ? w : (held as Tile[]).concat(w)
    }
    if (heat.size === 'equal') out = out.map((t) => ({ ...t, value: 1 }))
    return out
  })

  let W = $state(0)
  let H = $state(0)
  const layout = $derived(W > 0 && H > 0 && tiles.length ? heatmapLayout(tiles, W, H) : { blocks: [], cells: [] })

  let hover = $state<number | null>(null)
  const emptyWord = $derived(heat.universe === 'watchlist' ? 'Nothing watched.' : MARKET_U[heat.universe] ? 'Not read yet.' : 'No open positions.')

  function seg(u: string) { heat = { ...heat, universe: u }; save() }
  function sizeSeg(s: string) { heat = { ...heat, size: s }; save() }
  const pctText = (c: number | null) => (c == null ? '' : (c >= 0 ? '+' : '') + c.toFixed(2) + '%')
</script>

<div class="card">
  <div class="head">
    <h5>Heatmap</h5>
    <div class="seg">
      {#each UNIVERSES as [u, label] (u)}
        <button class:on={heat.universe === u} onclick={() => seg(u)}>{label}</button>
      {/each}
    </div>
    <div class="seg">
      <button class:on={heat.size === 'value'} onclick={() => sizeSeg('value')}>Market value</button>
      <button class:on={heat.size === 'equal'} onclick={() => sizeSeg('equal')}>Equal</button>
    </div>
    <div class="legend">
      <span>−3%</span>
      {#each [-3, -2, -1, -0.2, 0.2, 1, 2, 3] as c}<i style="background: {heatColor(c)}"></i>{/each}
      <span>+3%</span>
    </div>
  </div>

  <div class="box" bind:clientWidth={W} bind:clientHeight={H}>
    {#if !tiles.length}
      <div class="empty">{emptyWord}</div>
    {:else}
      {#each layout.blocks as b (b.label)}
        <div class="block" style="left:{b.x}px; top:{b.y}px; width:{b.w}px; height:{b.h}px"></div>
        <div class="blabel" style="left:{b.x + 6}px; top:{b.y + 3}px; max-width:{b.w - 12}px">{b.label}</div>
      {/each}
      {#each layout.cells as c, i (i)}
        <div
          class="cell"
          class:big={c.big}
          style="left:{c.x}px; top:{c.y}px; width:{c.w}px; height:{c.h}px; background:{heatColor(c.percentChange)}"
          role="presentation"
          onmouseenter={() => (hover = i)}
          onmouseleave={() => (hover = null)}
        >
          {#if c.mid || c.big}
            <div class="csym">{c.symbol}</div>
            <div class="cpct">{pctText(c.percentChange)}</div>
          {/if}
        </div>
      {/each}
      {#if hover != null && layout.cells[hover]}
        {@const c = layout.cells[hover]}
        <div class="tip" style="left:{c.x + c.w / 2}px; top:{c.y}px">
          <b>{c.symbol}</b> {pctText(c.percentChange)}
        </div>
      {/if}
    {/if}
  </div>
</div>

<style>
  .card { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 14px 16px; }
  .head { display: flex; align-items: center; gap: 14px; margin-bottom: 12px; flex-wrap: wrap; }
  .head h5 { margin: 0; font-size: 14px; font-weight: 600; }
  .seg { display: inline-flex; background: #1c2230; border-radius: 7px; padding: 2px; }
  .seg button { background: none; border: 0; color: #8b93a7; font: inherit; font-size: 11px; padding: 3px 9px; border-radius: 5px; cursor: pointer; }
  .seg button.on { background: #2a3242; color: #e6e9ef; }
  .legend { margin-left: auto; display: flex; align-items: center; gap: 3px; font-size: 11px; color: #8b93a7; }
  .legend i { width: 16px; height: 9px; border-radius: 2px; display: inline-block; }
  .box { position: relative; width: 100%; height: 430px; }
  .empty { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; color: #8b93a7; font-size: 12px; }
  .block { position: absolute; border: 1px solid #0b0e14; border-radius: 4px; }
  .blabel { position: absolute; color: #c4cbd8; font-size: 11px; font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; pointer-events: none; }
  .cell { position: absolute; border: 1px solid #0b0e14; border-radius: 3px; overflow: hidden; display: flex; flex-direction: column; align-items: center; justify-content: center; color: #fff; }
  .csym { font-size: 12px; font-weight: 600; }
  .big .csym { font-size: 16px; }
  .cpct { font-size: 10px; opacity: 0.9; font-variant-numeric: tabular-nums; }
  .cell:hover { filter: brightness(1.12); }
  .tip { position: absolute; transform: translate(-50%, -110%); background: #0b0e14; border: 1px solid #2a3242; border-radius: 6px; padding: 4px 8px; font-size: 11px; white-space: nowrap; pointer-events: none; z-index: 5; }
</style>
