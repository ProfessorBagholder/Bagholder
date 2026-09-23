<script lang="ts">
  // Short interest, a faithful port of ledger.html's shortsSectionHtml and its
  // four building blocks (shortsVolumeCardHtml, shortsFloatCardHtml,
  // shortsRingCard, shortsHistoryHtml). Two ring cards side by side, then the
  // history line chart. Nothing is drawn where the regulator does not cover the
  // listing.
  import type { Trade } from '../model'
  import { qty } from '../fmt'
  import Donut, { type DonutItem } from '../Donut.svelte'
  import { ensureShorts, shortsKey, shortDay, shortSpan, shortsStore } from './shorts.svelte'

  let { trade }: { trade: Trade } = $props()

  const n2 = (v: number, dp: number) => Number(v).toLocaleString('en-US', { minimumFractionDigits: dp, maximumFractionDigits: dp })

  // Asked for when the card is shown, and again when the reader comes back to the tab
  // with a reading more than thirty minutes old (ensureShorts keeps a younger one). No
  // clock runs on an open card: the exchanges report short interest twice a month.
  $effect(() => {
    const t = trade
    ensureShorts(t)
    const back = () => {
      if (!document.hidden) ensureShorts(t)
    }
    document.addEventListener('visibilitychange', back)
    return () => document.removeEventListener('visibilitychange', back)
  })

  const rec = $derived(shortsStore[shortsKey(trade)])
  const s = $derived(rec && rec.ok && rec.covered ? rec.shorts : null)

  interface Ring {
    key: string
    title: string
    when: string
    pct: number | null
    parts: DonutItem[] | null
    footer: string
    missing: string
  }

  const volume = $derived.by<Ring | null>(() => {
    if (!s) return null
    const pct = s.volumePct ?? null
    const share = pct == null ? null : Math.max(0, Math.min(1, pct / 100))
    const rest = s.totalVolume != null && s.shortVolume != null ? Math.max(0, s.totalVolume - s.shortVolume) : null
    const parts: DonutItem[] | null =
      share == null
        ? null
        : [
            { label: 'Short volume', v: 0, share, count: s.shortVolume == null ? null : Math.round(s.shortVolume), color: 'var(--pie-1)' },
            { label: 'Shares traded', v: 0, share: 1 - share, count: rest == null ? null : Math.round(rest), color: 'rgba(var(--ink-rgb),.16)' },
          ]
    const cover = s.daysToCover == null ? '' : n2(s.daysToCover, 1) + ' days to cover' + (s.averageVolume ? ' at ' + qty(Math.round(s.averageVolume)) + ' a day' : '')
    return { key: 'sivol', title: 'Short volume', when: s.volumeOf ? shortSpan(s.volumeOf) : '', pct, parts, footer: cover, missing: 'No trading is reported for this listing.' }
  })

  const float = $derived.by<Ring | null>(() => {
    if (!s) return null
    const pct = s.ofFloat ?? null
    const share = pct == null || !s.float || s.shares == null ? null : Math.max(0, Math.min(1, s.shares / s.float))
    const parts: DonutItem[] | null =
      share == null
        ? null
        : [
            { label: 'Short interest', v: 0, share, count: Math.round(s.shares as number), color: 'var(--pie-1)' },
            { label: 'Float', v: 0, share: 1 - share, count: Math.round((s.float as number) - (s.shares as number)), color: 'rgba(var(--ink-rgb),.16)' },
          ]
    const change = s.change == null ? '' : (s.change > 0 ? '+' : '') + qty(s.change) + (s.previousOf ? ' since ' + shortDay(s.previousOf) : '')
    const held = s.shares == null ? '' : qty(s.shares) + ' shares short'
    return {
      key: 'sifloat',
      title: 'Short vs float',
      when: s.asOf ? 'as of ' + shortDay(s.asOf) : '',
      pct: share == null ? null : pct,
      parts,
      footer: [held, change].filter(Boolean).join(' · '),
      missing: 'No float is published for this listing.',
    }
  })

  // --- history line chart geometry (shortsHistoryHtml) ---
  const W = 880, H = 180, TOP = 8, BOT = H - 12
  const pts = $derived((s?.series || []).filter((p) => p && p.shares != null))
  const geo = $derived.by(() => {
    if (pts.length < 3) return null
    const lo = Math.min(...pts.map((p) => p.shares))
    const hi = Math.max(...pts.map((p) => p.shares))
    const span = hi - lo || 1
    const x = (i: number) => (i / (pts.length - 1)) * W
    const y = (v: number) => TOP + (1 - (v - lo) / span) * (BOT - TOP)
    const line = pts.map((p, i) => (i ? 'L' : 'M') + x(i).toFixed(1) + ' ' + y(p.shares).toFixed(1)).join(' ')
    const ticks: [number, number][] = [
      [hi, TOP],
      [lo + (span * 2) / 3, TOP + (BOT - TOP) / 3],
      [lo + span / 3, TOP + ((BOT - TOP) * 2) / 3],
      [lo, BOT],
    ]
    const ys = pts.map((p) => y(p.shares))
    const want = Math.min(6, pts.length)
    const marks: string[] = []
    for (let i = 0; i < want; i++) marks.push(shortDay(pts[Math.round((i * (pts.length - 1)) / Math.max(1, want - 1))].date))
    return { lo, hi, span, line, ticks, ys, marks }
  })

  let plotEl = $state<HTMLDivElement | null>(null)
  let svgEl = $state<SVGSVGElement | null>(null)
  let hi = $state(-1)

  function dimAfter(frac: number | null) {
    if (!svgEl) return
    const at = frac == null ? '' : (Math.max(0, Math.min(1, frac)) * 100).toFixed(2) + '%'
    const mask = frac == null ? '' : 'linear-gradient(to right, #000 ' + at + ', rgba(0,0,0,.28) ' + at + ')'
    svgEl.style.maskImage = mask
    ;(svgEl.style as any).webkitMaskImage = mask
  }
  function move(e: MouseEvent) {
    if (!geo || !plotEl) return
    const n = geo.ys.length
    const r = plotEl.getBoundingClientRect()
    if (!r.width) return
    hi = Math.max(0, Math.min(n - 1, Math.round(((e.clientX - r.left) / r.width) * (n - 1))))
    dimAfter(hi / Math.max(1, n - 1))
  }
  function leave() {
    hi = -1
    dimAfter(null)
  }
  const hx = $derived(geo && hi >= 0 ? (((hi / Math.max(1, geo.ys.length - 1)) * 100).toFixed(2) + '%') : '0%')
  const hy = $derived(geo && hi >= 0 ? ((geo.ys[hi] / H) * 100).toFixed(2) + '%' : '0%')
  const tipShift = $derived(geo && hi >= 0 ? (hi < geo.ys.length * 0.1 ? '0%' : hi > geo.ys.length * 0.9 ? '-100%' : '-50%') : '-50%')
</script>

{#snippet ringCard(r: Ring)}
  <div class="card elev-sm" style="padding:16px 18px 12px;display:flex;flex-direction:column;min-height:0">
    <div style="display:flex;align-items:baseline;gap:12px;margin-bottom:10px"><h5>{r.title}</h5><span style="margin-left:auto;font-size:11px;color:var(--ink55)">{r.when}</span></div>
    {#if r.pct != null && r.parts}
      <div style="display:flex;flex-direction:column-reverse;align-items:center;gap:14px;flex:1;min-height:0;justify-content:center">
        <Donut items={r.parts} total={null} centreLabel="Short" centreText={n2(r.pct, 2) + '%'} side="row" size={210} />
      </div>
    {:else}
      <div class="muted empty" style="flex:1;font-size:12px">{r.missing}</div>
    {/if}
    {#if r.footer}<div class="tab" style="text-align:center;padding-top:10px;font-size:11.5px;color:var(--ink55)">{r.footer}</div>{/if}
  </div>
{/snippet}

{#if s && volume && float}
  <div style="display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr);gap:14px;align-items:stretch">
    {@render ringCard(volume)}
    {@render ringCard(float)}
  </div>
  {#if geo}
    <div class="card elev-sm" style="padding:16px 18px 12px">
      <div style="display:flex;align-items:baseline;gap:14px;margin-bottom:8px"><h5>Short interest over time</h5><span style="margin-left:auto;font-size:11px;color:var(--ink55)">every report since {shortDay(pts[0].date)}</span></div>
      <div style="display:flex;gap:10px">
        <div class="tab" style="position:relative;width:72px;height:{H}px;flex:none;font-size:10px;color:var(--ink55)">
          {#each geo.ticks as t (t[1])}
            <span style="position:absolute;left:0;top:{((t[1] / H) * 100).toFixed(1)}%;transform:translateY(-50%)">{qty(Math.round(t[0]))}</span>
          {/each}
        </div>
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div bind:this={plotEl} data-hover="sihist" style="position:relative;flex:1;min-width:0" onmousemove={move} onmouseleave={leave}>
          <div class="xline" hidden={hi < 0} style="left:{hx}"></div>
          <div class="line-dot" hidden={hi < 0} style="left:{hx};top:{hy};background:var(--accent)"></div>
          <div class="tip" hidden={hi < 0} style="left:{hx};transform:translateX({tipShift})">
            <div class="tv">{hi >= 0 ? qty(Math.round(pts[hi].shares)) : ''}</div>
            <div class="tl">{hi >= 0 ? shortDay(pts[hi].date) : ''}</div>
          </div>
          <svg bind:this={svgEl} viewBox="0 0 {W} {H}" preserveAspectRatio="none" style="width:100%;height:{H}px;display:block">
            <defs><linearGradient id="bhSi" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" style="stop-color:var(--accent);stop-opacity:var(--area)" /><stop offset="100%" style="stop-color:var(--accent);stop-opacity:0" /></linearGradient></defs>
            {#each geo.ticks as t (t[1])}
              <line x1="0" y1={t[1].toFixed(1)} x2={W} y2={t[1].toFixed(1)} style="stroke:var(--{t[1] === BOT ? 'hair' : 'grid'})" />
            {/each}
            <path d="{geo.line} L{W} {BOT} L0 {BOT} Z" fill="url(#bhSi)" />
            <path d={geo.line} fill="none" style="stroke:var(--accent)" stroke-width="2" stroke-linejoin="round" vector-effect="non-scaling-stroke" />
          </svg>
        </div>
      </div>
      <div style="display:flex;justify-content:space-between;font-size:10px;color:var(--ink55);padding:6px 0 0 82px">
        {#each geo.marks as m, i (i)}<span>{m}</span>{/each}
      </div>
    </div>
  {/if}
{/if}
