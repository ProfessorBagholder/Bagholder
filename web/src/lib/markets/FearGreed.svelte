<script lang="ts">
  import { onMount } from 'svelte'
  import type { FearGauge } from '../model'
  import { equityChart } from '../actions/equityChart'

  const BANDS: [number, number, string, string][] = [
    [0, 25, 'Extreme fear', '#d4586f'],
    [25, 45, 'Fear', '#a8455a'],
    [45, 56, 'Neutral', '#3a4152'],
    [56, 76, 'Greed', '#2f9e6b'],
    [76, 100, 'Extreme greed', '#4fc98d'],
  ]
  const bandColor = (score: number) => (BANDS.find(([lo, hi]) => score >= lo && score < hi) ?? BANDS[4])[3]

  let index = $state<'stocks' | 'crypto'>('stocks')
  let gauge = $state<FearGauge | null>(null)
  let loading = $state(true)

  // Guard against a stale response overwriting a newer one: only the latest
  // request applies. Without this, a slow initial stocks fetch can land after
  // a crypto toggle and clobber it.
  let reqId = 0
  async function load(idx: string) {
    const my = ++reqId
    loading = true
    try {
      const r = await fetch('/api/fear?index=' + idx)
      const d = await r.json()
      if (my !== reqId) return
      gauge = d.ok ? d.gauge : null
    } catch {
      if (my === reqId) gauge = null
    }
    if (my === reqId) loading = false
  }
  function pick(idx: 'stocks' | 'crypto') {
    if (idx === index && gauge) return
    index = idx
    load(idx)
  }
  onMount(() => load('stocks'))

  const cx = 110, cy = 110, R = 85
  const pt = (score: number, r: number) => {
    const a = Math.PI * (1 - score / 100)
    return [cx + r * Math.cos(a), cy - r * Math.sin(a)]
  }
  const arc = (s0: number, s1: number, r: number) => {
    const [x0, y0] = pt(s0, r), [x1, y1] = pt(s1, r)
    return `M ${x0} ${y0} A ${r} ${r} 0 0 1 ${x1} ${y1}`
  }
  const needle = $derived(gauge ? pt(gauge.score, R - 20) : [cx, cy - (R - 20)])
  const seriesPoints = $derived(gauge ? gauge.series.map((p) => ({ d: p.date, v: p.score, dep: 0 })) : [])
</script>

<div class="card">
  <div class="head">
    <h5>Fear &amp; Greed</h5>
    <div class="seg">
      <button class:on={index === 'stocks'} onclick={() => pick('stocks')}>Stocks</button>
      <button class:on={index === 'crypto'} onclick={() => pick('crypto')}>Crypto</button>
    </div>
    {#if gauge}<span class="src">{gauge.source} · {gauge.asOf.slice(0, 10)}</span>{/if}
  </div>

  {#if loading}
    <p class="msg">Reading…</p>
  {:else if !gauge}
    <p class="msg">No reading available.</p>
  {:else}
    <div class="body">
      <div class="dial">
        <svg viewBox="0 0 220 130">
          {#each BANDS as [s0, s1, , color]}
            <path d={arc(s0, s1, R)} fill="none" stroke={color} stroke-width="14" />
          {/each}
          <line x1={cx} y1={cy} x2={needle[0]} y2={needle[1]} stroke={bandColor(gauge.score)} stroke-width="3" stroke-linecap="round" />
          <circle cx={cx} cy={cy} r="5" fill={bandColor(gauge.score)} />
        </svg>
        <div class="score" style="color: {bandColor(gauge.score)}">{Math.round(gauge.score)}</div>
        <div class="rating" style="color: {bandColor(gauge.score)}">{gauge.rating}</div>
      </div>

      <div class="cols">
        <div class="col">
          <h6>Where it stood</h6>
          {#each gauge.previous as p (p.label)}
            <div class="row"><span>{p.label}</span><b style="color: {bandColor(p.score)}">{Math.round(p.score)}</b><span class="rt">{p.rating}</span></div>
          {/each}
        </div>
        {#if gauge.parts.length}
          <div class="col">
            <h6>What it is made of</h6>
            {#each gauge.parts as p (p.name)}
              <div class="row"><span>{p.name}</span><b style="color: {bandColor(p.score)}">{Math.round(p.score)}</b><span class="rt">{p.rating}</span></div>
            {/each}
          </div>
        {/if}
      </div>
    </div>

    {#if seriesPoints.length >= 10}
      {#key index}<div class="foot" use:equityChart={seriesPoints}></div>{/key}
    {/if}
  {/if}
</div>

<style>
  .card { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 14px 16px; }
  .head { display: flex; align-items: center; gap: 12px; margin-bottom: 8px; }
  .head h5 { margin: 0; font-size: 14px; font-weight: 600; }
  .seg { display: inline-flex; background: #1c2230; border-radius: 7px; padding: 2px; }
  .seg button { background: none; border: 0; color: #8b93a7; font: inherit; font-size: 11px; padding: 3px 10px; border-radius: 5px; cursor: pointer; }
  .seg button.on { background: #2a3242; color: #e6e9ef; }
  .src { margin-left: auto; color: #8b93a7; font-size: 11px; }
  .msg { color: #8b93a7; font-size: 13px; }
  .body { display: flex; gap: 20px; align-items: center; flex-wrap: wrap; }
  .dial { position: relative; width: 220px; text-align: center; }
  .dial svg { width: 220px; height: 130px; }
  .score { font-size: 30px; font-weight: 700; margin-top: -10px; }
  .rating { font-size: 13px; }
  .cols { display: flex; gap: 24px; flex: 1; min-width: 260px; }
  .col { flex: 1; }
  .col h6 { margin: 0 0 6px; color: #8b93a7; font-size: 11px; text-transform: uppercase; letter-spacing: 0.03em; font-weight: 500; }
  .row { display: grid; grid-template-columns: 1fr auto auto; gap: 10px; font-size: 12px; padding: 2px 0; align-items: baseline; }
  .row span { color: #c4cbd8; } .row .rt { color: #8b93a7; font-size: 11px; text-align: right; min-width: 70px; }
  .row b { font-variant-numeric: tabular-nums; }
  .foot { width: 100%; height: 120px; margin-top: 12px; }
</style>
