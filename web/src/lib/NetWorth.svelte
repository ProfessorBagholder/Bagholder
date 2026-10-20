<script lang="ts">
  // Net worth: everything held less everything owed, inside Wealthsimple and out,
  // as Wealthsimple's own net worth feature counts it. The investments are the
  // Wealthsimple accounts at their value after margin; the rest is what the person
  // linked or entered in that feature.
  import { roll } from './actions/roll'
  import type { PositionsDoc, EquityPoint } from './model'
  import { money0, stamp, stampDay } from './fmt'
  import { abs, plot } from './dec'

  let { model }: { model: PositionsDoc } = $props()
  const nw = $derived(model.portfolio.netWorth)
  // what each kind of thing is called, from the type Wealthsimple's net worth feature gives it
  const KIND: Record<string, string> = {
    AutoLoan: 'Auto loan', Car: 'Vehicle', Cd: 'GIC', Chequing: 'Chequing', CreditCard: 'Credit card', Crypto: 'Crypto', Fhsa: 'FHSA',
    GroupNonRegistered: 'Group non-registered', GroupRrsp: 'Group RRSP', GroupTfsa: 'Group TFSA', Heloc: 'HELOC', InstallmentLoan: 'Instalment loan',
    Lif: 'LIF', LineOfCredit: 'Line of credit', Lira: 'LIRA', Margin: 'Margin', MoneyMarket: 'Money market', Mortgage: 'Mortgage',
    NonRegistered: 'Non-registered', OtherAsset: 'Other asset', OtherInvestment: 'Other investment', OtherLiability: 'Other liability',
    PersonalLoan: 'Personal loan', RealEstate: 'Real estate', RespFamily: 'Family RESP', RespIndividual: 'Individual RESP', Rrif: 'RRIF',
    Rrsp: 'RRSP', Savings: 'Savings', SpousalRrif: 'Spousal RRIF', SpousalRrsp: 'Spousal RRSP', StudentLoan: 'Student loan', Tfsa: 'TFSA',
  }
  // under each name: its kind and its institution, whichever are known and not the name itself
  const under = (r: { name: string; kind: string; institution: string }) =>
    [KIND[r.kind] ?? r.kind, r.institution].filter((x) => x && x !== r.name).join(' · ')

  // ---- net worth over time: the Dashboard's equity curve, its geometry and hover ----
  // `net`: net worth alone; `split`: everything held and everything owed, two lines on one scale
  let mode = $state<'net' | 'split'>('net')
  function niceMax(peak: number): number {
    if (!(peak > 0)) return 1
    const step = Math.pow(10, Math.max(0, String(Math.round(peak / 3)).length - 2))
    return Math.max(step * 3, Math.ceil(peak / (step * 3)) * step * 3)
  }
  function eqGeom(lines: EquityPoint[][]) {
    const vals = lines.map((l) => l.map((p) => plot(p.v)))
    const all = vals.flat().concat([0])
    const hi = niceMax(Math.max.apply(null, all))
    const low = Math.min.apply(null, all)
    const lo = low < 0 ? -niceMax(-low) : 0
    const W = 880, H = 260, pad = 8, top = 6
    const y = (v: number) => H - pad - ((v - lo) / (hi - lo)) * (H - pad - top)
    const base = y(0).toFixed(1)
    const ticks = [0, 1, 2, 3].map((i) => hi - ((hi - lo) * i) / 3)
    const drawn = vals.map((vs) => {
      const n = Math.max(1, vs.length - 1)
      const pts = vs.map((v, i) => [(i / n) * W, y(v)] as [number, number])
      const path = pts.map((p, i) => (i ? 'L' : 'M') + p[0].toFixed(1) + ' ' + p[1].toFixed(1)).join(' ')
      return { pts, path, area: path + ' L' + W + ' ' + base + ' L0 ' + base + ' Z' }
    })
    const first = vals[0] ?? []
    return { ticks, lines: drawn, up: first.length ? first[first.length - 1] >= 0 : true }
  }
  const split = $derived(mode === 'split')
  // the days drawn: each line's points, the first line's days the hover's
  const eqLines = $derived(split ? [nw?.assetsHistory ?? [], nw?.debtsHistory ?? []] : [nw?.history ?? []])
  const eqSeries = $derived(eqLines[0])
  const g = $derived(eqSeries.length > 1 ? eqGeom(eqLines) : null)
  const eqTick = (v: number) => (v < 0 ? '−$' : '$') + Math.abs(Math.round(v)).toLocaleString('en-US')
  const eqAxis = $derived.by(() => {
    const s = eqSeries
    if (!s.length) return [] as string[]
    const want = Math.min(6, s.length)
    const out: string[] = []
    for (let i = 0; i < want; i++) out.push(stamp(s[Math.round((i * (s.length - 1)) / Math.max(1, want - 1))].d))
    return [...new Set(out)]
  })
  // net worth by day, for the split view's hover
  const netOn = $derived(new Map((nw?.history ?? []).map((p) => [p.d, p.v])))
  let eqPlot = $state<HTMLElement>()
  let eqHover = $state<{ x: string; ys: string[]; rows: { label: string; v: string; cls: string }[]; tl: string; tx: string } | null>(null)
  function onEqMove(e: MouseEvent) {
    const s = eqSeries
    if (!s.length || !eqPlot || !g) return
    const r = eqPlot.getBoundingClientRect()
    const i = Math.max(0, Math.min(s.length - 1, Math.round(((e.clientX - r.left) / r.width) * (s.length - 1))))
    const frac = i / Math.max(1, s.length - 1)
    const sign = (v: EquityPoint['v']) => (abs(v) === v ? 'pos' : 'neg')
    const net = netOn.get(s[i].d)
    eqHover = {
      x: (frac * 100).toFixed(2) + '%',
      ys: g.lines.map((l) => ((l.pts[i][1] / 260) * 100).toFixed(2) + '%'),
      rows: split
        ? [
            { label: 'Assets', v: money0(s[i].v), cls: 'pos' },
            { label: 'Debts', v: money0(eqLines[1][i].v), cls: 'neg' },
            ...(net !== undefined ? [{ label: 'Net worth', v: money0(net), cls: '' }] : []),
          ]
        : [{ label: '', v: money0(s[i].v), cls: sign(s[i].v) }],
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
  // each line's colour: net worth by its sign, assets green and debts red
  const lineColor = (i: number) => (split ? (i === 0 ? 'pos' : 'neg') : g?.up ? 'pos' : 'neg')
</script>

<div style="padding:20px;min-height:380px;display:flex;flex-direction:column;gap:14px">
  {#if !nw}
    <div class="card elev-sm" style="padding:24px 18px">
      <div class="muted empty" style="font-size:12px">Nothing held or owed outside Wealthsimple yet. Link or add accounts in Wealthsimple's Net worth, and they show here at the next balances read.</div>
    </div>
  {:else}
    <div style="display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:14px">
      <div class="card elev-sm kpi"><div class="lbl">Net worth</div><div class="v" use:roll={money0(nw.netWorth)}></div><div class="s">Assets less liabilities</div></div>
      <div class="card elev-sm kpi"><div class="lbl">Assets</div><div class="v" use:roll={money0(nw.assetsTotal)}></div><div class="s">{nw.assets.length === 1 ? '1 asset' : nw.assets.length + ' assets'}</div></div>
      <div class="card elev-sm kpi"><div class="lbl">Liabilities</div><div class="v" use:roll={money0(nw.liabilitiesTotal)}></div><div class="s">{nw.liabilities.length === 1 ? '1 debt' : nw.liabilities.length + ' debts'}</div></div>
    </div>
    <!-- net worth over time: the Dashboard's equity curve card -->
    <div class="card elev-sm" style="padding:16px 18px 12px">
      <div style="display:flex;align-items:baseline;gap:14px;margin-bottom:8px">
        <h5>Net worth over time</h5>
        {#if split}
          <span style="font-size:11px;color:var(--ink55);display:flex;gap:10px;align-items:center">
            <span><span style="display:inline-block;width:8px;height:8px;border-radius:2px;background:var(--pos);margin-right:4px"></span>Assets</span>
            <span><span style="display:inline-block;width:8px;height:8px;border-radius:2px;background:var(--neg);margin-right:4px"></span>Debts</span>
          </span>
        {/if}
        <div style="display:flex;gap:4px;margin-left:auto">
          {#each [['net', 'Net worth'], ['split', 'Assets & debts']] as const as m (m[0])}
            <button class="pill" class:on={mode === m[0]} style="padding:2px 8px;font-size:11px;width:auto" onclick={() => (mode = m[0])}>{m[1]}</button>
          {/each}
        </div>
      </div>
      {#if !g}
        <div class="empty muted" style="padding:40px 0">{split && (nw.history ?? []).length > 1 ? 'What the debts came to by day is not read yet. It is read from Wealthsimple with the balances, once a day.' : 'No history yet. It is read from Wealthsimple with the balances, once a day.'}</div>
      {:else}
        <div style="display:flex;gap:10px">
          <div class="tab" style="position:relative;width:56px;height:260px;flex:none;font-size:10px;color:var(--ink55)">
            {#each [2.3, 33.8, 65.4, 96.9] as top, i (top)}
              <span style="position:absolute;left:0;top:{top}%;transform:translateY(-50%)">{eqTick(g.ticks[i])}</span>
            {/each}
          </div>
          <div bind:this={eqPlot} style="position:relative;flex:1;min-width:0" role="presentation" onmousemove={onEqMove} onmouseleave={onEqLeave}>
            <div class="xline" hidden={!eqHover} style="left:{eqHover?.x ?? '0'}"></div>
            {#each g.lines as _, i (i)}
              <div style="position:absolute;width:7px;height:7px;margin:-4px 0 0 -3px;border-radius:50%;background:var(--{lineColor(i)});box-shadow:0 0 0 2px var(--surface);pointer-events:none" hidden={!eqHover} style:left={eqHover?.x} style:top={eqHover?.ys[i]}></div>
            {/each}
            <div class="tip" hidden={!eqHover} style="left:{eqHover?.x ?? '0'};transform:translateX({eqHover?.tx ?? '-50%'})">
              {#each eqHover?.rows ?? [] as row (row.label)}
                <div class="tv {row.cls}">{#if row.label}<span style="font-weight:400;color:var(--ink55);margin-right:6px">{row.label}</span>{/if}{row.v}</div>
              {/each}
              <div class="tl">{eqHover?.tl ?? ''}</div>
            </div>
            <svg viewBox="0 0 880 260" preserveAspectRatio="none" style="width:100%;height:260px;display:block">
              <defs>
                {#each g.lines as _, i (i)}
                  <linearGradient id="bhNw{i}" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" style="stop-color:var(--{lineColor(i)});stop-opacity:var(--area)" /><stop offset="100%" style="stop-color:var(--{lineColor(i)});stop-opacity:0" /></linearGradient>
                {/each}
              </defs>
              <line x1="0" y1="6" x2="880" y2="6" style="stroke:var(--grid)" /><line x1="0" y1="88" x2="880" y2="88" style="stroke:var(--grid)" /><line x1="0" y1="170" x2="880" y2="170" style="stroke:var(--grid)" /><line x1="0" y1="252" x2="880" y2="252" style="stroke:var(--hair)" />
              {#each g.lines as l, i (i)}
                <path d={l.area} fill="url(#bhNw{i})" /><path d={l.path} fill="none" style="stroke:var(--{lineColor(i)})" stroke-width="2" stroke-linejoin="round" vector-effect="non-scaling-stroke" />
              {/each}
            </svg>
          </div>
        </div>
        <div style="display:flex;justify-content:space-between;font-size:10px;color:var(--ink55);padding:6px 0 0 66px">
          {#each eqAxis as a (a)}<span>{a}</span>{/each}
        </div>
      {/if}
    </div>
    <!-- the two cards one height, each list scrolling inside its card, as the app's paired cards do -->
    <div style="display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:14px;height:380px">
      {#each [['Assets', 'Asset', 'Value', nw.assets], ['Liabilities', 'Debt', 'Balance', nw.liabilities]] as const as [title, what, amount, rows] (title)}
        <div class="card elev-sm" style="padding:16px 18px;display:flex;flex-direction:column;min-height:0">
          <h5 style="margin-bottom:4px">{title}</h5>
          {#if rows.length}
            <div class="scroll" style="flex:1;min-height:0">
            <table class="table" style="font-size:12px;table-layout:fixed;width:100%">
              <thead><tr><th style="text-align:left;position:sticky;top:0;z-index:1">{what}</th><th style="text-align:right;width:30%;position:sticky;top:0;z-index:1">{amount}</th></tr></thead>
              <tbody>
                {#each rows as r (r.id)}
                  <tr class="tab">
                    <td>{r.name}{#if under(r)}<div style="font-size:11px;color:var(--ink55)">{under(r)}</div>{/if}</td>
                    <td style="text-align:right">{money0(r.amount)}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
            </div>
          {:else}
            <div class="muted empty" style="font-size:12px">None.</div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>
