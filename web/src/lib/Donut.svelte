<script lang="ts">
  // One donut for Allocation, Sectors, Regions and the Cashflow pie: the slices,
  // the centre reading the label and total (or a hovered slice), and the legend
  // as an aligned grid. Ported from ledger.html donutPieces/donutHover, with the
  // hover expressed as reactive state instead of DOM mutation.
  import { money0, pctPlain, qty, leftOut } from './fmt'
  import { symText } from './sym'
  import type { Dec, Fig } from './dec'

  export interface DonutItem {
    label: string
    /** What the slice is worth: exact, as the server states it, or a size the page was given. */
    v: Fig<Dec> | number
    share: number
    color: string
    count?: number | null
  }
  let {
    items,
    total = null,
    totalLeftOut = 0,
    centreLabel,
    centreText = '',
    side = 'l',
    size = undefined,
    symbols = false,
    legendFirst = false,
  }: {
    items: DonutItem[]
    total?: Fig<Dec> | number | null
    /** How many the total left out because their figures wait. */
    totalLeftOut?: number
    centreLabel: string
    centreText?: string
    side?: 'l' | 'r' | 'row'
    size?: number | string
    symbols?: boolean
    // Render the legend before the ring (the exposure card's Sectors donut puts
    // its legend at the far-left grid column, ring to its right).
    legendFirst?: boolean
  } = $props()

  const R = 44, W = 14, C = 60, SIZE = typeof size === 'number' ? size : 264

  const arcs = $derived.by(() => {
    let acc = 0
    return items.map((x) => {
      const a0 = acc * 2 * Math.PI - Math.PI / 2
      const a1 = (acc + x.share) * 2 * Math.PI - Math.PI / 2
      acc += x.share
      const full = x.share >= 0.9999
      const large = x.share > 0.5 ? 1 : 0
      const d = full
        ? ''
        : 'M' + (C + R * Math.cos(a0)).toFixed(2) + ' ' + (C + R * Math.sin(a0)).toFixed(2) + ' A' + R + ' ' + R + ' 0 ' + large + ' 1 ' + (C + R * Math.cos(a1)).toFixed(2) + ' ' + (C + R * Math.sin(a1)).toFixed(2)
      return { full, d, color: x.color }
    })
  })

  let hover = $state<number>(-1)
  const label = (x: DonutItem) => (symbols ? symText(x.label) : x.label)

  const ringWidth = typeof size === 'string' ? size : size ? SIZE + 'px' : 'max(200px, min(' + SIZE + 'px, 100% - 140px))'
</script>

{#if legendFirst}{@render legend()}{@render ring()}{:else}{@render ring()}{@render legend()}{/if}

{#snippet ring()}
<!-- ring -->
<div style="position:relative;flex:none;width:{ringWidth};aspect-ratio:1/1">
  <svg viewBox="0 0 120 120" style="width:100%;height:100%;display:block">
    {#each arcs as a, i (i)}
      {#if a.full}
        <circle cx={C} cy={C} r={R} fill="none" stroke={a.color} stroke-width={W} style="opacity:{hover < 0 || hover === i ? 1 : 0.35}" role="presentation" onmouseenter={() => (hover = i)} onmouseleave={() => (hover = -1)} />
      {:else}
        <path d={a.d} fill="none" stroke={a.color} stroke-width={W} style="cursor:default;opacity:{hover < 0 || hover === i ? 1 : 0.35}" role="presentation" onmouseenter={() => (hover = i)} onmouseleave={() => (hover = -1)} />
      {/if}
    {/each}
  </svg>
  <div style="position:absolute;inset:21%;display:flex;flex-direction:column;align-items:center;justify-content:center;text-align:center;pointer-events:none">
    {#if hover >= 0 && items[hover]}
      {@const x = items[hover]}
      {#if total == null}
        <div class="lbl">{label(x)}</div>
        <div class="tab" style="font-size:17px;font-weight:500;margin-top:2px">{#if x.count == null}{pctPlain(x.share)}{:else}<span style="font-size:15px;letter-spacing:-.01em">{qty(x.count)}</span>{/if}</div>
        {#if x.count != null}<div class="muted" style="font-size:11px;margin-top:3px">{pctPlain(x.share)}</div>{/if}
      {:else}
        <div class="lbl">{label(x)}</div>
        <div class="tab" style="font-size:17px;font-weight:500;margin-top:2px">{money0(x.v)}</div>
        <div class="muted" style="font-size:11px;margin-top:2px">{pctPlain(x.share)}</div>
      {/if}
    {:else}
      <div class="lbl">{centreLabel}</div>
      <div class="tab" style="font-size:17px;font-weight:500;margin-top:2px">{total == null ? centreText : money0(total)}</div>
      {#if totalLeftOut}<div class="muted" style="font-size:11px;margin-top:2px">{leftOut(totalLeftOut)}</div>{/if}
    {/if}
  </div>
</div>
{/snippet}

{#snippet legend()}
<!-- legend -->
{#if side === 'row'}
  <div style="display:flex;flex-wrap:wrap;justify-content:center;gap:8px 24px;font-size:12.5px">
    {#each items as x, i (i)}
      <span style="display:inline-flex;align-items:center;gap:8px;white-space:nowrap;opacity:{hover < 0 || hover === i ? 1 : 0.45}" role="presentation" onmouseenter={() => (hover = i)} onmouseleave={() => (hover = -1)}>
        <span style="width:9px;height:9px;border-radius:2px;background:{x.color}"></span>
        <span style="font-weight:500">{label(x)}</span>
        <span class="tab" style="color:var(--ink55)">{pctPlain(x.share)}</span>
      </span>
    {/each}
  </div>
{:else}
  <div class="scroll" style="flex:0 1 auto;min-width:0;min-height:0;max-height:100%;justify-self:{side === 'r' ? 'end' : 'start'};display:grid;grid-template-columns:auto minmax(0,1fr) auto;column-gap:12px;row-gap:9px;align-content:center;align-items:center;font-size:12.5px">
    {#each items as x, i (i)}
      <span style="display:contents;opacity:{hover < 0 || hover === i ? 1 : 0.45}" role="presentation" onmouseenter={() => (hover = i)} onmouseleave={() => (hover = -1)}>
        {#if side === 'r'}
          <span class="tab" style="text-align:left;white-space:nowrap;opacity:{hover < 0 || hover === i ? 1 : 0.45}">{pctPlain(x.share)}</span>
          <span style="font-weight:500;font-variant-numeric:normal;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;text-align:right;opacity:{hover < 0 || hover === i ? 1 : 0.45}">{label(x)}</span>
          <span style="width:9px;height:9px;border-radius:2px;background:{x.color};opacity:{hover < 0 || hover === i ? 1 : 0.45}"></span>
        {:else}
          <span style="width:9px;height:9px;border-radius:2px;background:{x.color};opacity:{hover < 0 || hover === i ? 1 : 0.45}"></span>
          <span style="font-weight:500;font-variant-numeric:normal;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;opacity:{hover < 0 || hover === i ? 1 : 0.45}">{label(x)}</span>
          <span class="tab" style="text-align:right;white-space:nowrap;opacity:{hover < 0 || hover === i ? 1 : 0.45}">{pctPlain(x.share)}</span>
        {/if}
      </span>
    {/each}
  </div>
{/if}
{/snippet}
