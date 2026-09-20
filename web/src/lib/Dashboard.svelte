<script lang="ts">
  import { roll } from './actions/roll'
  import type { Model, EquityPoint } from './model'
  import { money, money0, pct, pctPlain, cls, color, stamp, stampDay, hold, shortMoney } from './fmt'
  import { symText } from './sym'
  import { sort, toggleSort, sortRows } from './sort.svelte'
  import { setBenchmark, setFilters } from './state.svelte'
  import { filters } from './filters.svelte'
  import { goSub, go } from './router.svelte'

  let { model }: { model: Model } = $props()

  const BENCHMARKS: [string, string][] = [
    ['SP500', 'S&P 500'],
    ['TSX', 'S&P/TSX'],
    ['TSX60', 'TSX 60'],
  ]
  const benchPref = $derived(BENCHMARKS.some((b) => b[0] === filters.benchmark) ? filters.benchmark : 'SP500')

  // ---- KPI row ----
  const k = $derived(model.kpi)
  const ann = $derived(model.equity.annualized)
  const dd = $derived(model.equity.drawdown)
  const pf = $derived(k.profitFactorInfinite ? '∞' : k.profitFactor == null ? '—' : k.profitFactor.toFixed(2))

  // ---- equity geometry (eqGeom / niceMax) ----
  function niceMax(peak: number): number {
    if (!(peak > 0)) return 1
    const step = Math.pow(10, Math.max(0, String(Math.round(peak / 3)).length - 2))
    return Math.max(step * 3, Math.ceil(peak / (step * 3)) * step * 3)
  }
  function eqGeom(series: EquityPoint[]) {
    const vals = series.map((p) => p.v)
    const peak = Math.max.apply(null, vals.concat([0]))
    const max = niceMax(peak)
    const W = 880, H = 260, pad = 8, top = 6
    const n = Math.max(1, vals.length - 1)
    const pts = vals.map((v, i) => [(i / n) * W, H - pad - (v / max) * (H - pad - top)] as [number, number])
    const path = pts.map((p, i) => (i ? 'L' : 'M') + p[0].toFixed(1) + ' ' + p[1].toFixed(1)).join(' ')
    return { max, path, area: path + ' L880 252 L0 252 Z', pts }
  }
  const eqSeries = $derived(model.equity.series || [])
  const g = $derived(eqSeries.length ? eqGeom(eqSeries) : null)
  const eqTick = (v: number) => '$' + Math.round(v).toLocaleString('en-US')
  const eqAxis = $derived.by(() => {
    const s = eqSeries
    if (!s.length) return [] as string[]
    const want = Math.min(6, s.length)
    const out: string[] = []
    for (let i = 0; i < want; i++) out.push(stamp(s[Math.round((i * (s.length - 1)) / Math.max(1, want - 1))].d))
    return [...new Set(out)]
  })

  // equity hover — crosshair, dot, tip, and the dim-after-cursor mask
  let eqPlot = $state<HTMLElement>()
  let eqHover = $state<{ x: string; y: string; tv: string; tl: string; tx: string } | null>(null)
  function onEqMove(e: MouseEvent) {
    const s = eqSeries
    if (!s.length || !eqPlot || !g) return
    const r = eqPlot.getBoundingClientRect()
    const i = Math.max(0, Math.min(s.length - 1, Math.round(((e.clientX - r.left) / r.width) * (s.length - 1))))
    const frac = i / Math.max(1, s.length - 1)
    eqHover = {
      x: (frac * 100).toFixed(2) + '%',
      y: ((g.pts[i][1] / 260) * 100).toFixed(2) + '%',
      tv: money0(s[i].v),
      tl: stampDay(s[i].d),
      tx: i < s.length * 0.1 ? '0%' : i > s.length * 0.9 ? '-100%' : '-50%',
    }
    const at = (Math.max(0, Math.min(1, frac)) * 100).toFixed(2) + '%'
    const mask = 'linear-gradient(to right, #000 ' + at + ', rgba(0,0,0,.28) ' + at + ')'
    const svg = eqPlot.querySelector('svg') as SVGElement | null
    if (svg) {
      svg.style.maskImage = mask
      ;(svg.style as unknown as { webkitMaskImage: string }).webkitMaskImage = mask
    }
  }
  function onEqLeave() {
    eqHover = null
    const svg = eqPlot?.querySelector('svg') as SVGElement | null
    if (svg) {
      svg.style.maskImage = ''
      ;(svg.style as unknown as { webkitMaskImage: string }).webkitMaskImage = ''
    }
  }

  // ---- annualized returns (years) ----
  const years = $derived((model.years || []).slice().reverse())
  const yearScale = $derived(
    Math.max.apply(
      null,
      (model.years || []).map((x) => Math.abs(x.r)).concat((model.years || []).map((x) => Math.abs(x.spR || 0))).concat([0.01]),
    ),
  )
  const bench = $derived(model.benchmark?.label || 'S&P 500')
  const yearsNote = $derived.by(() => {
    const comparable = (model.years || []).filter((x) => x.spR != null)
    const beat = comparable.filter((x) => x.spR != null && x.r > (x.spR as number)).length
    if (!(model.years || []).length) return 'No NAV history yet.'
    return 'Outperformed ' + bench + ' in ' + beat + ' of ' + comparable.length + (comparable.length === 1 ? ' year.' : ' years.')
  })

  // ---- monthly P&L geometry ----
  const ms = $derived(model.monthly || [])
  const mo = $derived.by(() => {
    const posMax = Math.max.apply(null, ms.map((x) => Math.max(0, x.value)).concat([0]))
    const negMax = Math.max.apply(null, ms.map((x) => Math.max(0, -x.value)).concat([0]))
    const total = posMax + negMax || 1
    let base = negMax > 0 ? Math.min(0.93, Math.max(0.35, posMax / total)) : 0.93
    if (!(posMax > 0) && negMax > 0) base = 0.07
    const posH = base * 100, negH = (1 - base) * 100
    const ticks: [number, number][] = []
    if (posMax > 0) {
      ticks.push([0, posMax])
      ticks.push([posH / 2, posMax / 2])
    }
    ticks.push([posH, 0])
    if (negMax > 0) ticks.push([100, -negMax])
    const want = Math.min(6, ms.length)
    const axis: string[] = []
    for (let i = 0; i < want; i++) axis.push(ms[Math.round((i * (ms.length - 1)) / Math.max(1, want - 1))].label)
    return { posMax, negMax, posH, negH, ticks, axis: [...new Set(axis)] }
  })
  let moHover = $state<{ x: string; tv: string; color: string; tl: string; tx: string } | null>(null)
  function onMoEnter(i: number) {
    const b = ms[i]
    if (!b) return
    const n = ms.length
    moHover = {
      x: (((i + 0.5) / n) * 100).toFixed(2) + '%',
      tv: money0(b.value),
      color: color(b.value),
      tl: b.label + ' · ' + b.count + (b.count === 1 ? ' trade' : ' trades'),
      tx: i < 2 ? '-10%' : i > n - 3 ? '-90%' : '-50%',
    }
  }
  const moTick = (v: number) => (v === 0 ? '$0' : (v > 0 ? '+' : '−') + '$' + Math.round(Math.abs(v)).toLocaleString('en-US'))

  // ---- grades ----
  const grades = $derived(model.grades)
  const gradePeak = $derived(Math.max.apply(null, (grades?.buckets || []).map((b) => Math.abs(b.pnl)).concat([1])))

  // ---- by symbol ----
  const bySymCols: { key: string; label: string; align?: string }[] = [
    { key: 'symbol', label: 'Symbol' },
    { key: 'pnl', label: 'P&L', align: 'right' },
    { key: 'n', label: 'Trades', align: 'right' },
    { key: 'winRate', label: 'Win rate', align: 'right' },
    { key: 'avgHold', label: 'Avg hold', align: 'right' },
  ]
  const bySymRows = $derived(
    sortRows(model.bySymbol || [], sort.bySymbol.key, sort.bySymbol.dir, (r, key) => (r as unknown as Record<string, unknown>)[key]),
  )

  // ---- queue ----
  const queue = $derived(model.queue || [])

  // ---- drill-downs ----
  function openTrade(id: string) {
    goSub('trades', id)
  }
  function symbolOpen(row: { symbol: string; tradeIds: string[] }) {
    if (row.tradeIds.length === 1) return openTrade(row.tradeIds[0])
    go('trades')
    setFilters({ lists: { ...filters.lists, symbol: [row.symbol] } })
  }
  function monthOpen(i: number) {
    const b = ms[i]
    if (!b) return
    if (b.tradeIds && b.tradeIds.length === 1) return openTrade(b.tradeIds[0])
    const y = +b.key.slice(0, 4), m = +b.key.slice(5, 7)
    const last = new Date(y, m, 0).getDate()
    go('trades')
    setFilters({ preset: 'all', years: [], from: b.key + '-01', to: b.key + '-' + String(last).padStart(2, '0') })
  }
  function gradeOpen(grade: string) {
    const b = grades.buckets.find((x) => x.grade === grade)
    if (!b || !b.n) return
    if (b.n === 1) return openTrade(b.tradeIds[0])
    go('trades')
    setFilters({ lists: { ...filters.lists, grade: [grade] } })
  }
</script>

<div style="padding:20px;display:flex;flex-direction:column;gap:14px">
  <!-- KPI row -->
  <div style="display:grid;grid-template-columns:repeat(6,minmax(0,1fr));gap:14px">
    <div class="card elev-sm kpi"><div class="lbl">Realized P&amp;L</div><div class="v {cls(k.realized)}" use:roll={money(k.realized)}></div><div class="s">{k.count}{k.count === 1 ? ' trade' : ' trades'}</div></div>
    <div class="card elev-sm kpi"><div class="lbl">Win rate</div><div class="v" use:roll={k.winRate == null ? '—' : pctPlain(k.winRate)}></div><div class="s">{k.wins} W · {k.losses} L{k.breakeven ? ' · ' + k.breakeven + ' BE' : ''}</div></div>
    <div class="card elev-sm kpi"><div class="lbl">Profit factor</div><div class="v" use:roll={pf}></div><div class="s">W {money0(k.grossWin)} · L {money0(k.grossLoss)}</div></div>
    <div class="card elev-sm kpi"><div class="lbl">Expectancy</div><div class="v" use:roll={k.expectancy == null ? '—' : money(k.expectancy)}></div><div class="s">Avg W {money0(k.avgWin)} · L {money0(k.avgLoss)}</div></div>
    <div class="card elev-sm kpi"><div class="lbl">Max drawdown</div><div class="v {dd.pct == null ? '' : 'neg'}" use:roll={dd.pct == null ? '—' : '−' + Math.abs(dd.pct * 100).toFixed(1) + '%'}></div><div class="s">{dd.pct == null ? 'No NAV history' : '−$' + Math.abs(Math.round(dd.abs ?? 0)).toLocaleString('en-US') + (dd.at ? ' · ' + stamp(dd.at) : '')}</div></div>
    <div class="card elev-sm kpi"><div class="lbl">Avg annualized</div><div class="v {ann.rate == null ? '' : cls(ann.rate)}" use:roll={ann.rate == null ? '—' : pct(ann.rate)}></div><div class="s">{ann.rate == null ? 'No NAV history' : 'Over ' + ann.count + (ann.count === 1 ? ' year' : ' years')}</div></div>
  </div>

  <!-- equity + annualized returns -->
  <div style="display:grid;grid-template-columns:calc((100% - 70px) / 6 * 4 + 42px) minmax(0,1fr);gap:14px">
    <!-- equity curve -->
    <div class="card elev-sm" style="padding:16px 18px 12px">
      <div style="display:flex;align-items:baseline;gap:14px;margin-bottom:8px"><h5>Equity curve</h5></div>
      {#if !g}
        <div class="empty muted" style="padding:40px 0">No NAV history yet. Sync to load it.</div>
      {:else}
        <div style="display:flex;gap:10px">
          <div class="tab" style="position:relative;width:56px;height:260px;flex:none;font-size:10px;color:var(--ink55)">
            {#each [[2.3, g.max], [33.8, (g.max * 2) / 3], [65.4, g.max / 3], [96.9, 0]] as t (t[0])}
              <span style="position:absolute;left:0;top:{t[0]}%;transform:translateY(-50%)">{eqTick(t[1])}</span>
            {/each}
          </div>
          <div bind:this={eqPlot} style="position:relative;flex:1;min-width:0" role="presentation" onmousemove={onEqMove} onmouseleave={onEqLeave}>
            <div class="xline" hidden={!eqHover} style="left:{eqHover?.x ?? '0'}"></div>
            <div style="position:absolute;width:7px;height:7px;margin:-4px 0 0 -3px;border-radius:50%;background:var(--pos);box-shadow:0 0 0 2px var(--surface);pointer-events:none" hidden={!eqHover} style:left={eqHover?.x} style:top={eqHover?.y}></div>
            <div class="tip" hidden={!eqHover} style="left:{eqHover?.x ?? '0'};transform:translateX({eqHover?.tx ?? '-50%'})"><div class="tv pos">{eqHover?.tv ?? ''}</div><div class="tl">{eqHover?.tl ?? ''}</div></div>
            <svg viewBox="0 0 880 260" preserveAspectRatio="none" style="width:100%;height:260px;display:block">
              <defs><linearGradient id="bhEq" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" style="stop-color:var(--pos);stop-opacity:var(--area)" /><stop offset="100%" style="stop-color:var(--pos);stop-opacity:0" /></linearGradient></defs>
              <line x1="0" y1="6" x2="880" y2="6" style="stroke:var(--grid)" /><line x1="0" y1="88" x2="880" y2="88" style="stroke:var(--grid)" /><line x1="0" y1="170" x2="880" y2="170" style="stroke:var(--grid)" /><line x1="0" y1="252" x2="880" y2="252" style="stroke:var(--hair)" />
              <path d={g.area} fill="url(#bhEq)" /><path d={g.path} fill="none" style="stroke:var(--pos)" stroke-width="2" stroke-linejoin="round" vector-effect="non-scaling-stroke" />
            </svg>
          </div>
        </div>
        <div style="display:flex;justify-content:space-between;font-size:10px;color:var(--ink55);padding:6px 0 0 66px">
          {#each eqAxis as a (a)}<span>{a}</span>{/each}
        </div>
      {/if}
    </div>

    <!-- annualized returns -->
    <div class="card elev-sm" style="padding:16px 18px 12px;display:flex;flex-direction:column;min-height:0;contain:size">
      <div style="display:flex;align-items:baseline;justify-content:space-between;gap:8px;margin-bottom:2px">
        <h5>Annualized returns</h5>
        <div style="display:flex;gap:4px">
          {#each BENCHMARKS as b (b[0])}
            <button class="pill" class:on={b[0] === benchPref} style="padding:2px 8px;font-size:11px;width:auto" onclick={() => setBenchmark(b[0])}>{b[1]}</button>
          {/each}
        </div>
      </div>
      <div class="muted" style="font-size:11px;margin-bottom:14px">Vs {bench}</div>
      <div class="scroll" style="flex:1;min-height:0;display:flex;flex-direction:column;gap:14px;padding-right:2px">
        {#if years.length}
          {#each years as y (y.year)}
            <div>
              <div style="display:flex;justify-content:space-between;font-size:12px;margin-bottom:5px">
                <span class="tab">{y.year}</span>
                <span class="tab" style="color:{color(y.r)}">{pct(y.r)}<span class="muted"> / {y.spR == null ? '—' : pct(y.spR)}</span></span>
              </div>
              <div style="display:flex;flex-direction:column;gap:3px">
                <div style="height:7px;width:{Math.max(2, (Math.abs(y.r) / yearScale) * 100).toFixed(0)}%;background:{color(y.r)};border-radius:2px"></div>
                <div style="height:7px;width:{Math.max(2, (Math.abs(y.spR || 0) / yearScale) * 100).toFixed(0)}%;background:var(--mixed);border-radius:2px"></div>
              </div>
            </div>
          {/each}
        {:else}
          <div class="muted" style="font-size:12px">No complete years yet.</div>
        {/if}
      </div>
      <div class="rule-t muted" style="margin-top:auto;padding-top:12px;font-size:10px">{yearsNote}</div>
    </div>
  </div>

  <!-- monthly + grade -->
  <div style="display:grid;grid-template-columns:calc((100% - 70px) / 6 * 4 + 42px) minmax(0,1fr);gap:14px">
    <!-- monthly P&L -->
    <div class="card elev-sm" style="padding:16px 18px 12px;display:flex;flex-direction:column">
      <div style="display:flex;align-items:baseline;gap:12px"><h5>Monthly P&amp;L</h5></div>
      <div style="display:flex;gap:10px;flex:1;min-height:150px;margin-top:12px">
        <div class="tab" style="position:relative;width:56px;flex:none;font-size:10px;color:var(--ink55)">
          {#each mo.ticks as t (t[0])}<span style="position:absolute;left:0;top:{t[0].toFixed(1)}%;transform:translateY(-50%)">{moTick(t[1])}</span>{/each}
        </div>
        <div style="position:relative;flex:1;min-width:0;display:flex;gap:5px;align-items:stretch" role="presentation" onmouseleave={() => (moHover = null)}>
          <div class="xline" hidden={!moHover} style="left:{moHover?.x ?? '0'}"></div>
          <div class="tip" hidden={!moHover} style="left:{moHover?.x ?? '0'};transform:translateX({moHover?.tx ?? '-50%'})"><div class="tv" style="color:{moHover?.color ?? ''}">{moHover?.tv ?? ''}</div><div class="tl">{moHover?.tl ?? ''}</div></div>
          {#if mo.posMax > 0}<div style="position:absolute;left:0;right:0;top:{(mo.posH / 2).toFixed(1)}%;height:1px;background:var(--grid)"></div>{/if}
          <div style="position:absolute;left:0;right:0;top:{mo.posH.toFixed(1)}%;height:1px;background:var(--hair)"></div>
          {#if ms.length}
            {#each ms as b, i (b.key)}
              {@const h = b.value >= 0 ? (mo.posMax ? Math.max(1.5, (b.value / mo.posMax) * mo.posH) : 0) : mo.negMax ? Math.max(1.5, (-b.value / mo.negMax) * mo.negH) : 0}
              <div class="bar-col" role="presentation" onmouseenter={() => onMoEnter(i)} onclick={() => monthOpen(i)}>
                {#if b.value >= 0}
                  <div style="position:absolute;left:0;right:0;bottom:{(100 - mo.posH).toFixed(1)}%;height:{h.toFixed(2)}%;background:var(--pos);border-radius:2px 2px 0 0"></div>
                {:else}
                  <div style="position:absolute;left:0;right:0;top:{mo.posH.toFixed(1)}%;height:{h.toFixed(2)}%;background:var(--neg);border-radius:0 0 2px 2px"></div>
                {/if}
              </div>
            {/each}
          {:else}
            <div class="muted empty" style="flex:1;font-size:12px">No closed trades in this range.</div>
          {/if}
        </div>
      </div>
      <div style="display:flex;justify-content:space-between;font-size:10px;color:var(--ink55);padding-left:66px;margin-top:6px">
        {#each mo.axis as a (a)}<span>{a}</span>{/each}
      </div>
    </div>

    <!-- grade vs P&L -->
    <div class="card elev-sm" style="padding:16px 18px 12px;display:flex;flex-direction:column">
      <h5>Grade vs P&amp;L</h5>
      <div class="muted" style="font-size:11px;margin-bottom:14px">Realized P&amp;L by the grade you gave the trade</div>
      {#if !grades.graded}
        <div style="flex:1;min-height:120px;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:6px;text-align:center">
          <div style="font-size:13px">No trades graded yet</div>
          <div class="dim" style="font-size:11px">{k.count} closed trades to review</div>
          <button class="btn btn-secondary" style="font-size:12px;margin-top:2px" onclick={() => go('trades')}>Open Trades</button>
        </div>
      {:else}
        <div style="position:relative;flex:1;min-height:120px;display:grid;grid-template-columns:repeat(4,1fr);gap:10px;align-items:end">
          {#each grades.buckets as b (b.grade)}
            <div class="bar-col" role="presentation" style="display:flex;flex-direction:column;justify-content:flex-end;gap:6px;height:100%;border-radius:4px;cursor:pointer" onclick={() => gradeOpen(b.grade)}>
              <span class="tab" style="font-size:11px;text-align:center;color:{b.n === 0 ? 'rgba(var(--ink-rgb),.45)' : color(b.pnl)}">{b.n ? shortMoney(b.pnl) : '—'}</span>
              <div style="height:{b.n === 0 ? 0 : Math.max(3, (Math.abs(b.pnl) / gradePeak) * 100).toFixed(1)}%;background:{color(b.pnl)};border-radius:3px 3px 0 0"></div>
            </div>
          {/each}
        </div>
        <div style="display:grid;grid-template-columns:repeat(4,1fr);gap:10px;margin-top:6px;font-size:10px;color:var(--ink55);text-align:center">
          {#each grades.buckets as b (b.grade)}<span>{b.grade} · {b.n}</span>{/each}
        </div>
      {/if}
    </div>
  </div>

  <!-- by symbol + review queue -->
  <div style="display:grid;grid-template-columns:calc((100% - 70px) / 6 * 4 + 42px) minmax(0,1fr);gap:14px;height:342px">
    <!-- by symbol -->
    <div class="card elev-sm" style="padding:16px 18px 12px;display:flex;flex-direction:column;min-height:0">
      <div style="display:flex;align-items:baseline;gap:12px;margin-bottom:6px"><h5>By symbol</h5></div>
      <div class="scroll" style="flex:1;min-height:0">
        <table class="table"><thead><tr>
          {#each bySymCols as c (c.key)}
            {@const on = sort.bySymbol.key === c.key}
            {@const right = c.align === 'right'}
            <th style="white-space:nowrap;text-align:{c.align || 'left'};cursor:pointer;position:sticky;top:0;z-index:1;color:{on ? 'var(--ink)' : 'rgba(var(--ink-rgb),.6)'}" onclick={() => toggleSort('bySymbol', c.key)}>
              <span class="th-in" style="flex-direction:{right ? 'row-reverse' : 'row'}">{c.label}<span class="arrow" style="color:{on ? 'var(--accent)' : 'transparent'}">{on && sort.bySymbol.dir === 'asc' ? '▲' : '▼'}</span></span>
            </th>
          {/each}
        </tr></thead><tbody>
          {#each bySymRows as r (r.symbol)}
            <tr class="tab" style="cursor:pointer" onclick={() => symbolOpen(r)}>
              <td style="font-weight:500;font-variant-numeric:normal">{symText(r.symbol)}</td>
              <td style="text-align:right" class={cls(r.pnl)}>{money(r.pnl)}</td>
              <td style="text-align:right">{r.n}</td>
              <td style="text-align:right">{pctPlain(r.winRate)}</td>
              <td style="text-align:right" class="dim">{hold(r.avgHold)}</td>
            </tr>
          {/each}
        </tbody></table>
      </div>
    </div>

    <!-- review queue -->
    <div class="card elev-sm" style="padding:16px 18px;display:flex;flex-direction:column;min-height:0">
      <div style="display:flex;align-items:baseline"><h5>Review queue</h5></div>
      <div class="muted" style="font-size:11px;margin-bottom:12px">Closed trades with no grade or thesis</div>
      <div class="scroll" style="flex:1;min-height:0;display:flex;flex-direction:column;gap:8px;padding-right:2px">
        {#if queue.length}
          {#each queue as r (r.id)}
            <div class="queue-row" role="presentation" onclick={() => openTrade(r.id)}>
              <div style="min-width:0"><div style="font-size:13px;font-weight:500;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;max-width:180px">{symText(r.symbol)}</div><div class="muted" style="font-size:11px">{r.date} · {r.missing}</div></div>
              <span class="tab" style="margin-left:auto;font-size:12px;color:{color(r.pnl)}">{money(r.pnl)}</span>
            </div>
          {/each}
        {:else}
          <div style="flex:1;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:5px;text-align:center;padding:8px 0">
            <svg width="20" height="20" viewBox="0 0 256 256" style="fill:var(--pos)"><path d="M232.5 82.5l-128 128a12 12 0 0 1-17 0l-56-56a12 12 0 0 1 17-17L96 185l119.5-119.5a12 12 0 0 1 17 17Z" /></svg>
            <div style="font-size:13px">Nothing left to review</div>
            <div class="dim" style="font-size:11px">Every closed trade has a grade and a thesis</div>
          </div>
        {/if}
      </div>
    </div>
  </div>
</div>
