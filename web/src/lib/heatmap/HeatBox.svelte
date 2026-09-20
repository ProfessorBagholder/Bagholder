<script lang="ts">
  // The treemap itself: sector blocks and symbol tiles laid out by the squarified
  // algorithm and placed with absolute left/top/width/height. Nodes are keyed by
  // sector label and by symbol so that when the universe or sizer changes the same
  // node travels to its new slot (the .heat-tile/.heat-blk CSS transitions animate
  // it) instead of being recreated; a genuinely new tile gets .bh-new for its
  // arrival keyframe, but never on the first draw.
  import type { Tile } from './treemap'
  import { heatmapLayout, heatColor } from './treemap'
  import { signedPct } from '../markets/util'
  import { bareSymbol } from '../sym'
  import { goSub } from '../router.svelte'
  import { rememberListing } from '../listing.svelte'

  let { tiles, universe, boxStyle }: { tiles: Tile[]; universe: string; boxStyle: string } = $props()

  const MARKET_U: Record<string, string> = { ca: 'Canada', us: 'US', intl: 'International' }

  let W = $state(0)
  let H = $state(0)
  const layout = $derived(W > 0 && H > 0 && tiles.length ? heatmapLayout(tiles, W, H) : { blocks: [], cells: [] })

  // arrival tracking, mirroring mountHeatmap's `had`/`fresh`
  let seen = new Set<string>()
  let had = false
  let fresh = $state(new Set<string>())
  $effect(() => {
    const keys = new Set(layout.cells.map((c) => c.symbol))
    if (had) fresh = new Set([...keys].filter((k) => !seen.has(k)))
    seen = keys
    if (keys.size) had = true
  })

  const px1 = (n: number) => n.toFixed(1) + 'px'

  // a market universe's tiles carry no venue of their own: the universe is the venue
  function heatListing(c: Tile) {
    const venue = c.exchange || (universe === 'ca' ? 'TSX' : '')
    return {
      symbol: bareSymbol(c.symbol),
      exchange: venue,
      currency: c.currency || (universe === 'ca' ? 'CAD' : MARKET_U[universe] ? 'USD' : ''),
      name: c.name || '',
    }
  }
  function openCell(c: Tile & { other?: boolean }) {
    if (c.id) goSub('portfolio', c.id)
    else if (c.symbol && !c.other) {
      const o = heatListing(c)
      goSub('markets', rememberListing(o))
    }
  }
  const opens = (c: Tile & { other?: boolean }) => !!c.id || (!!c.symbol && !c.other)
  const blkColor = (chg: number | null) => (chg == null ? 'var(--ink55)' : chg >= 0 ? 'var(--pos)' : 'var(--neg)')
</script>

<div id="heatBox" style={boxStyle} bind:clientWidth={W} bind:clientHeight={H}>
  {#each layout.blocks as b (b.label)}
    <div class="heat-blk" style="left:{px1(b.x)};top:{px1(b.y)};width:{px1(b.w)};height:{px1(b.h)}">
      <div class="heat-hd"><span class="heat-hl">{b.label}</span><span class="num" style="margin-left:auto;font-weight:400;color:{blkColor(b.chg)}">{signedPct(b.chg)}</span></div>
    </div>
  {/each}
  {#each layout.cells as c (c.symbol)}
    <div
      class="heat-tile"
      class:go={opens(c)}
      class:bh-new={fresh.has(c.symbol)}
      style="left:{px1(c.x + 1)};top:{px1(c.y + 1)};width:{px1(Math.max(0, c.w - 2))};height:{px1(Math.max(0, c.h - 2))};background:{heatColor(c.percentChange)}"
      data-sym={c.other ? c.symbol : bareSymbol(c.symbol)}
      data-tip-sub={c.name || ''}
      role={opens(c) ? 'button' : undefined}
      tabindex={opens(c) ? -1 : undefined}
      onclick={() => openCell(c)}
      onkeydown={(e) => { if (opens(c) && (e.key === 'Enter' || e.key === ' ')) openCell(c) }}
    >
      <div class="heat-sym" style="font-size:{c.big ? 16 : c.mid ? 13 : 11}px">{c.other ? c.symbol : bareSymbol(c.symbol)}</div>
      {#if c.mid}<div class="heat-chg" style="font-size:{c.big ? 12 : 11}px">{signedPct(c.percentChange)}</div>{/if}
    </div>
  {/each}
</div>
