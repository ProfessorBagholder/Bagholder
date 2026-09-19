<script lang="ts">
  import { pct } from './fmt'

  type Item = { label: string; value: number }
  let {
    items,
    centerLabel,
    centerTotal,
    cap = 0,
    showValue = false,
    valueFmt = (n: number) => String(n),
  }: {
    items: Item[]
    centerLabel: string
    centerTotal: string
    cap?: number
    showValue?: boolean
    valueFmt?: (n: number) => string
  } = $props()

  const PALETTE = ['#3ecf8e', '#4b9fff', '#b78bff', '#f2a341', '#f0616d', '#3ec9c9', '#e26fb0', '#a3c644', '#e8c34a', '#7c8aff', '#d98cff']
  const GREY = '#5b6474'

  const slices = $derived.by(() => {
    const pos = items.filter((i) => i.value > 0 && i.label !== 'Not classified')
    const nc = items.find((i) => i.label === 'Not classified' && i.value > 0)
    pos.sort((a, b) => b.value - a.value)
    let shown: Item[] = pos
    if (cap > 0 && pos.length > cap) {
      const head = pos.slice(0, cap)
      const restCount = pos.length - cap
      const restSum = pos.slice(cap).reduce((s, i) => s + i.value, 0)
      shown = [...head, { label: `Other (${restCount})`, value: restSum }]
    }
    const total = items.reduce((s, i) => s + (i.value > 0 ? i.value : 0), 0) || 1
    const out = shown.map((i, idx) => ({ ...i, share: i.value / total, color: i.label.startsWith('Other (') ? GREY : PALETTE[idx % PALETTE.length] }))
    if (nc) out.push({ label: 'Not classified', value: nc.value, share: nc.value / total, color: GREY })
    let acc = 0
    return out.map((s) => { const withOff = { ...s, offset: acc }; acc += s.share; return withOff })
  })

  const R = 60
  const C = 2 * Math.PI * R
  let hover = $state<number | null>(null)
  const active = $derived(hover != null && slices[hover] ? slices[hover] : null)
</script>

<div class="donut-wrap">
  <svg viewBox="0 0 160 160" class="donut">
    <g transform="rotate(-90 80 80)">
      {#each slices as s, i (i)}
        <circle
          cx="80" cy="80" r={R} fill="none"
          stroke={s.color} stroke-width={hover === i ? 22 : 18}
          stroke-dasharray="{s.share * C} {C}" stroke-dashoffset={-s.offset * C}
          opacity={hover == null || hover === i ? 1 : 0.35}
          role="presentation"
          onmouseenter={() => (hover = i)} onmouseleave={() => (hover = null)}
        />
      {/each}
    </g>
    <text x="80" y="74" text-anchor="middle" class="c-label">{active ? active.label : centerLabel}</text>
    <text x="80" y="92" text-anchor="middle" class="c-value">{active ? (showValue ? valueFmt(active.value) : pct(active.share)) : centerTotal}</text>
    {#if active}<text x="80" y="106" text-anchor="middle" class="c-share">{showValue ? pct(active.share) : ''}</text>{/if}
  </svg>
  <div class="legend">
    {#each slices as s, i (i)}
      <div class="row" class:hi={hover === i} class:dim={hover != null && hover !== i} role="presentation" onmouseenter={() => (hover = i)} onmouseleave={() => (hover = null)}>
        <span class="dot" style="background: {s.color}"></span>
        <span class="lbl">{s.label}</span>
        {#if showValue}<span class="val">{valueFmt(s.value)}</span>{/if}
        <span class="shr">{pct(s.share)}</span>
      </div>
    {/each}
  </div>
</div>

<style>
  .donut-wrap { display: flex; gap: 16px; align-items: center; flex: 1; min-height: 0; }
  .donut { width: 190px; height: 190px; flex-shrink: 0; }
  .c-label { fill: #8b93a7; font-size: 9px; }
  .c-value { fill: #e6e9ef; font-size: 12px; font-weight: 600; }
  .c-share { fill: #8b93a7; font-size: 9px; }
  .legend { flex: 1; overflow-y: auto; max-height: 100%; display: grid; grid-template-columns: auto 1fr auto auto; gap: 3px 8px; align-content: center; font-size: 12px; font-variant-numeric: tabular-nums; }
  .row { display: contents; cursor: default; }
  .dot { width: 9px; height: 9px; border-radius: 2px; align-self: center; }
  .lbl { font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .val, .shr { text-align: right; color: #8b93a7; }
  .row.hi .lbl, .row.hi .val, .row.hi .shr { color: #e6e9ef; }
  .row.dim { opacity: 0.5; }
</style>
