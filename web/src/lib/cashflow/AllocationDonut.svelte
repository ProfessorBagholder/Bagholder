<script lang="ts">
  import { cad, pct } from '../fmt'

  let { items }: { items: { symbol: string; value: number }[] } = $props()

  const PALETTE = ['#3ecf8e', '#4b9fff', '#b78bff', '#f2a341', '#f0616d', '#3ec9c9', '#e26fb0', '#a3c644', '#8b93a7', '#6b7280']

  const slices = $derived.by(() => {
    const pos = items.filter((i) => i.value > 0).sort((a, b) => b.value - a.value)
    const total = pos.reduce((s, i) => s + i.value, 0) || 1
    let acc = 0
    return pos.map((i, idx) => {
      const share = i.value / total
      const s = { ...i, share, offset: acc, color: PALETTE[idx % PALETTE.length] }
      acc += share
      return s
    })
  })
  const total = $derived(slices.reduce((s, i) => s + i.value, 0))

  const R = 60
  const C = 2 * Math.PI * R
  let hover = $state<number | null>(null)
  const center = $derived(hover != null && slices[hover] ? slices[hover] : null)
</script>

<div class="card">
  <h5>Allocation</h5>
  <div class="body">
    <svg viewBox="0 0 160 160" class="donut">
      <g transform="rotate(-90 80 80)">
        {#each slices as s, i (s.symbol)}
          <circle
            cx="80" cy="80" r={R} fill="none"
            stroke={s.color} stroke-width={hover === i ? 22 : 18}
            stroke-dasharray="{s.share * C} {C}"
            stroke-dashoffset={-s.offset * C}
            role="presentation"
            onmouseenter={() => (hover = i)}
            onmouseleave={() => (hover = null)}
          />
        {/each}
      </g>
      <text x="80" y="74" text-anchor="middle" class="c-label">{center ? center.symbol : 'Projected'}</text>
      <text x="80" y="92" text-anchor="middle" class="c-value">{center ? cad(center.value) : cad(total)}</text>
      {#if center}<text x="80" y="106" text-anchor="middle" class="c-share">{pct(center.share)}</text>{/if}
    </svg>
    <div class="legend">
      {#each slices as s, i (s.symbol)}
        <div class="row" class:hi={hover === i} role="presentation" onmouseenter={() => (hover = i)} onmouseleave={() => (hover = null)}>
          <span class="dot" style="background: {s.color}"></span>
          <span class="sym">{s.symbol}</span>
          <span class="val">{cad(s.value)}</span>
          <span class="shr">{pct(s.share)}</span>
        </div>
      {/each}
    </div>
  </div>
</div>

<style>
  .card { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 16px 18px; height: 380px; display: flex; flex-direction: column; }
  .card h5 { margin: 0 0 10px; font-size: 14px; font-weight: 600; }
  .body { display: flex; gap: 16px; align-items: center; flex: 1; min-height: 0; }
  .donut { width: 200px; height: 200px; flex-shrink: 0; }
  .c-label { fill: #8b93a7; font-size: 9px; }
  .c-value { fill: #e6e9ef; font-size: 13px; font-weight: 600; }
  .c-share { fill: #8b93a7; font-size: 9px; }
  .legend { flex: 1; overflow-y: auto; max-height: 100%; display: grid; grid-template-columns: auto 1fr auto auto; gap: 3px 8px; align-content: center; font-size: 12px; font-variant-numeric: tabular-nums; }
  .row { display: contents; cursor: default; }
  .dot { width: 9px; height: 9px; border-radius: 2px; align-self: center; }
  .sym { font-weight: 500; }
  .val, .shr { text-align: right; color: #8b93a7; }
  .row.hi .sym, .row.hi .val, .row.hi .shr { color: #e6e9ef; }
</style>
