<script lang="ts">
  // The Fear & Greed card (fearCardHtml): the dial, the "where it stood" and
  // "what it is made of" reading rows, and the history line with its hover. The
  // gauge is read from /api/fear?index=…, cached per index, and a slow response is
  // guarded by a request id so it cannot overwrite a newer selection.
  import type { FearDoc } from '../generated/markets'
  import { relTime } from '../fmt'
  import { n2, shortDay } from './util'
  import { watchDoc } from '../live'
  import { store } from '../state.svelte'
  import Mseg from './Mseg.svelte'

  const INDEX_OPTS = [['stocks', 'Stocks'], ['crypto', 'Crypto']] as const
  const FEAR_BANDS: [number, number, string, string][] = [
    [0, 25, 'Extreme fear', 'var(--heat-n4)'],
    [25, 45, 'Fear', 'var(--heat-n3)'],
    [45, 56, 'Neutral', 'rgba(var(--ink-rgb),.22)'],
    [56, 76, 'Greed', 'var(--heat-p3)'],
    [76, 100, 'Extreme greed', 'var(--heat-p4)'],
  ]
  function fearBand(score: number | null) {
    return score == null ? null : FEAR_BANDS.find((b) => score < b[1]) || FEAR_BANDS[FEAR_BANDS.length - 1]
  }
  function fearInk(score: number | null) {
    const b = fearBand(score)
    return !b || b[2] === 'Neutral' ? 'var(--ink75)' : b[3]
  }

  let fearIndex = $state((() => { try { return localStorage.getItem('bh2.fear') || 'stocks' } catch { return 'stocks' } })())
  // Both meters are sent while this card is shown, the one on show and the one a
  // click away: what is held at once, and the fresh reading when the server has it.
  // The server reads a publisher only while some page shows its meter, so watching
  // both keeps both fresh, and turning to the other never draws an old reading first.
  const docs = $state<Record<string, { data: FearDoc | null }>>(Object.fromEntries(INDEX_OPTS.map(([ix]) => [ix, { data: null }])))
  $effect(() => {
    const stops = INDEX_OPTS.map(([ix]) => watchDoc('fear:' + ix, {}, docs[ix]))
    return () => stops.forEach((stop) => stop())
  })
  function pickIndex(ix: string) {
    fearIndex = ix
    try { localStorage.setItem('bh2.fear', ix) } catch { /* ignore */ }
  }

  // nothing held yet: `Reading…` until the server has sent the meter and while its
  // publisher is being read; only a read that has answered with nothing is said so
  const held = $derived({ loading: !docs[fearIndex]?.data || !!docs[fearIndex]?.data?.reading, rec: docs[fearIndex]?.data?.gauge ?? null })
  const g = $derived(held.rec)
  const today = $derived(String((store.model as unknown as { today?: string } | null)?.today || ''))
  const when = $derived.by(() => {
    if (!g || !g.asOf) return ''
    return String(g.asOf).slice(0, 10) === today ? relTime(g.asOf) : shortDay(String(g.asOf).slice(0, 10))
  })
  const parts = $derived(g?.parts || [])

  // --- the dial ---
  const W = 280, H = 150, cx = 140, cy = 140, r = 104
  function at(v: number, rad: number): [number, number] {
    const a = ((180 - Math.max(0, Math.min(100, v)) * 1.8) * Math.PI) / 180
    return [cx + rad * Math.cos(a), cy - rad * Math.sin(a)]
  }
  function fearArc(from: number, to: number): string {
    const a = ((180 - from * 1.8) * Math.PI) / 180
    const b = ((180 - to * 1.8) * Math.PI) / 180
    const x1 = cx + r * Math.cos(a), y1 = cy - r * Math.sin(a)
    const x2 = cx + r * Math.cos(b), y2 = cy - r * Math.sin(b)
    return 'M' + x1.toFixed(1) + ' ' + y1.toFixed(1) + ' A' + r + ' ' + r + ' 0 0 1 ' + x2.toFixed(1) + ' ' + y2.toFixed(1)
  }
  const needle = $derived.by(() => {
    if (!g || g.score == null) return null
    const [tx, ty] = at(g.score, 16)
    const [nx, ny] = at(g.score, 88)
    return { tx, ty, nx, ny }
  })

  // --- the history ---
  const HW = 880, HH = 96, TOP = 6, BOT = HH - 6
  const hpts = $derived((g?.series || []).filter((p) => p && p.score != null))
  const hy = (v: number) => TOP + (1 - v / 100) * (BOT - TOP)
  const hx = (i: number) => (i / Math.max(1, hpts.length - 1)) * HW
  const histLine = $derived(hpts.map((p, i) => (i ? 'L' : 'M') + hx(i).toFixed(1) + ' ' + hy(p.score).toFixed(1)).join(' '))
  function fearDay(iso: string): string {
    const day = String(iso || '')
    return day.slice(0, 4) === today.slice(0, 4) ? shortDay(day) : shortDay(day) + ' ' + day.slice(0, 4)
  }

  // history hover (ledger's lineLayer/lineHover, local to this chart)
  let plot = $state<HTMLElement | null>(null)
  let hi = $state<number | null>(null)
  function onMove(e: MouseEvent) {
    if (!plot || !hpts.length) return
    const rect = plot.getBoundingClientRect()
    if (!rect.width) return
    hi = Math.max(0, Math.min(hpts.length - 1, Math.round(((e.clientX - rect.left) / rect.width) * (hpts.length - 1))))
  }
  function onLeave() { hi = null }
  const hoverRead = $derived.by(() => {
    if (hi == null || !hpts[hi]) return null
    const p = hpts[hi]
    const xPct = (hi / Math.max(1, hpts.length - 1)) * 100
    return {
      xPct,
      topPct: (hy(p.score) / HH) * 100,
      value: n2(p.score, 0),
      color: fearInk(p.score),
      label: (fearBand(p.score) || ['', '', ''])[2] + ' · ' + fearDay(p.date),
      shift: hi < hpts.length * 0.1 ? '0%' : hi > hpts.length * 0.9 ? '-100%' : '-50%',
    }
  })
</script>

<div class="card elev-sm" style="padding:14px 16px 12px">
  <div style="display:flex;align-items:center;gap:14px;min-height:28px;margin-bottom:8px">
    <h5>Fear &amp; Greed</h5>
    <Mseg options={INDEX_OPTS} cur={fearIndex} onpick={pickIndex} />
    {#if g}<span style="margin-left:auto;font-size:11px;color:var(--ink55)">{g.source || ''}{when ? ' · ' + when : ''}</span>{/if}
  </div>

  {#if !g}
    <div class="muted" style="font-size:12px">{held.loading ? 'Reading…' : 'The index did not answer.'}</div>
  {:else}
    <div style="display:grid;grid-template-columns:280px minmax(0,1fr){parts.length ? ' minmax(0,1.15fr)' : ''};gap:26px;align-items:start">
      <!-- dial -->
      <div style="width:{W}px;max-width:100%">
        <svg viewBox="0 0 {W} {H}" style="width:100%;height:auto;display:block" aria-hidden="true">
          {#each FEAR_BANDS as b (b[0])}
            <path d={fearArc(b[0] + (b[0] ? 0.8 : 0), b[1] - (b[1] < 100 ? 0.8 : 0))} fill="none" stroke={b[3]} stroke-width="15" />
          {/each}
          {#if needle}
            <line x1={needle.tx.toFixed(1)} y1={needle.ty.toFixed(1)} x2={needle.nx.toFixed(1)} y2={needle.ny.toFixed(1)} stroke="var(--ink)" stroke-width="2.5" stroke-linecap="round" />
            <circle cx={cx} cy={cy} r="6" fill="var(--ink)" /><circle cx={cx} cy={cy} r="2.5" fill="var(--card)" />
          {/if}
        </svg>
        <div style="display:flex;justify-content:space-between;font-size:10px;color:var(--ink55);padding:2px 4px 0"><span>Extreme fear</span><span>Extreme greed</span></div>
        <div style="text-align:center;margin-top:10px">
          <div class="tab" style="font-size:34px;font-weight:500;line-height:1;color:{fearInk(g.score)}">{g.score == null ? '—' : n2(g.score, 0)}</div>
          <div style="font-size:12.5px;margin-top:5px;color:{fearInk(g.score)}">{g.rating || ''}</div>
        </div>
      </div>

      <!-- where it stood -->
      {#if (g.previous || []).length}
        <div style="min-width:0;max-width:460px">
          <div class="lbl" style="margin-bottom:8px">Where it stood</div>
          {#each g.previous as row (row.label)}
            <div style="display:flex;align-items:baseline;gap:10px;padding:5px 0">
              <span style="font-size:12.5px;color:var(--ink75);overflow:hidden;text-overflow:ellipsis;white-space:nowrap">{row.label}</span>
              <span class="tab" style="margin-left:auto;font-size:12.5px">{row.score == null ? '—' : n2(row.score, 0)}</span>
              <span style="font-size:11px;width:82px;text-align:right;color:{fearInk(row.score)}">{row.rating || ''}</span>
            </div>
          {/each}
        </div>
      {/if}

      <!-- what it is made of -->
      {#if parts.length}
        <div style="min-width:0;max-width:460px">
          <div class="lbl" style="margin-bottom:8px">What it is made of</div>
          {#each parts as row (row.name)}
            <div style="display:flex;align-items:baseline;gap:10px;padding:5px 0">
              <span style="font-size:12.5px;color:var(--ink75);overflow:hidden;text-overflow:ellipsis;white-space:nowrap">{row.name}</span>
              <span class="tab" style="margin-left:auto;font-size:12.5px">{row.score == null ? '—' : n2(row.score, 0)}</span>
              <span style="font-size:11px;width:82px;text-align:right;color:{fearInk(row.score)}">{row.rating || ''}</span>
            </div>
          {/each}
        </div>
      {/if}
    </div>

    {#if hpts.length >= 3}
      <div style="margin-top:12px">
        <div bind:this={plot} style="position:relative" role="presentation" onmousemove={onMove} onmouseleave={onLeave}>
          {#if hoverRead}
            <div class="xline" style="left:{hoverRead.xPct.toFixed(2)}%"></div>
            <div class="line-dot" style="left:{hoverRead.xPct.toFixed(2)}%;top:{hoverRead.topPct.toFixed(2)}%;background:{hoverRead.color}"></div>
            <div class="tip" style="left:{hoverRead.xPct.toFixed(2)}%;transform:translateX({hoverRead.shift})"><div class="tv" style="color:{hoverRead.color}">{hoverRead.value}</div><div class="tl">{hoverRead.label}</div></div>
          {/if}
          <svg viewBox="0 0 {HW} {HH}" preserveAspectRatio="none" style="width:100%;height:{HH}px;display:block">
            <defs><linearGradient id="bhFg" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" style="stop-color:var(--accent);stop-opacity:var(--area)" /><stop offset="100%" style="stop-color:var(--accent);stop-opacity:0" /></linearGradient></defs>
            {#each [25, 50, 75] as v (v)}<line x1="0" y1={hy(v).toFixed(1)} x2={HW} y2={hy(v).toFixed(1)} style="stroke:var(--grid)" />{/each}
            <line x1="0" y1={BOT} x2={HW} y2={BOT} style="stroke:var(--hair)" />
            <path d={histLine + ' L' + HW + ' ' + BOT + ' L0 ' + BOT + ' Z'} fill="url(#bhFg)" />
            <path d={histLine} fill="none" style="stroke:var(--accent)" stroke-width="1.6" stroke-linejoin="round" vector-effect="non-scaling-stroke" />
          </svg>
        </div>
        <div style="display:flex;justify-content:space-between;font-size:10px;color:var(--ink55);padding-top:4px"><span>{fearDay(hpts[0].date)}</span><span>{fearDay(hpts[hpts.length - 1].date)}</span></div>
      </div>
    {/if}
  {/if}
</div>
