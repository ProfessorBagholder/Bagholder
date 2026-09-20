<script lang="ts">
  import { roll } from './actions/roll'
  import type { Model, Cashflow, CashflowHolding, CashflowRow } from './model'
  import { money, money0, signedMoney, pctPlain, qty, px, color } from './fmt'
  import { symText } from './sym'
  import { sort, toggleSort, sortRows } from './sort.svelte'

  let { model }: { model: Model } = $props()

  const MON_LONG = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December']
  function monthLong(key: string): string {
    return MON_LONG[+key.slice(5, 7) - 1] + ' ' + key.slice(0, 4)
  }
  // The month's margin interest, CAD, from the Interest charge rows in scope.
  function cfInterestByMonth(c: Cashflow): Record<string, number> {
    const out: Record<string, number> = {}
    ;(c.other || []).forEach((r) => {
      if (r.kind === 'Interest charge') {
        const k = String(r.date).slice(0, 7)
        out[k] = (out[k] || 0) - r.amountCad
      }
    })
    return out
  }
  // The pie: each income holding's share of the whole by projected monthly income.
  function pieSlices(rows: CashflowHolding[]): { total: number; items: { symbol: string; account: string; v: number; share: number; color: string }[] } {
    const val = (h: CashflowHolding) => (h.annual != null ? h.annual / 12 : null)
    const items = rows
      .map((h) => ({ symbol: h.symbol, account: h.account, v: val(h) }))
      .filter((x): x is { symbol: string; account: string; v: number } => x.v != null && x.v > 0)
      .sort((a, b) => b.v - a.v)
    const total = items.reduce((a, x) => a + x.v, 0)
    return { total, items: items.map((x, i) => Object.assign(x, { share: total ? x.v / total : 0, color: 'var(--pie-' + ((i % 8) + 1) + ')' })) }
  }

  type Col = { key: string; label: string; align?: string; width?: string; padLeft?: string; padRight?: string }
  const ycols: Col[] = [
    { key: 'symbol', label: 'Holding', width: '8%' },
    { key: 'qty', label: 'Qty', align: 'right', width: '6%' },
    { key: 'avg', label: 'Avg', align: 'right', width: '5.5%' },
    { key: 'cost', label: 'Book', align: 'right', width: '7%' },
    { key: 'mv', label: 'Market', align: 'right', width: '7%' },
    { key: 'per', label: 'Distribution', align: 'right', width: '9.5%' },
    { key: 'ytd', label: 'YTD', align: 'right', width: '6%' },
    { key: 'all', label: 'All time', align: 'right', width: '7.5%' },
    { key: 'nextExDate', label: 'Ex-Div', align: 'right', width: '8%' },
    { key: 'nextPayDate', label: 'Pay Day', align: 'right', width: '8%' },
    { key: 'annual', label: 'Projected', align: 'right', width: '8.25%' },
    { key: 'yoc', label: 'Yield on cost', align: 'right', width: '9.75%' },
    { key: 'currentYield', label: 'Current yield', align: 'right', padRight: '0px', width: '9.5%' },
  ]
  const dcols: Col[] = [
    { key: 'date', label: 'Date', width: '17%' },
    { key: 'symbol', label: 'Symbol', width: '14%' },
    { key: 'account', label: 'Account', width: '27%' },
    { key: 'qty', label: 'Qty', align: 'right', width: '10%' },
    { key: 'per', label: 'Distribution', align: 'right', width: '18%' },
    { key: 'amount', label: 'Amount', align: 'right', padRight: '0px', width: '14%' },
  ]

  const c = $derived(model.cashflow)
  const ms = $derived(c.months || [])
  const interest = $derived(cfInterestByMonth(c))
  const peak = $derived(Math.max(...ms.map((x) => Math.max(x.value, interest[x.key] || 0)), 0))
  const head = $derived(Math.max(100, Math.ceil(peak / 100) * 100))
  const axis = $derived.by(() => {
    const want = Math.min(6, ms.length)
    const a: string[] = []
    for (let i = 0; i < want; i++) a.push(ms[Math.round((i * (ms.length - 1)) / Math.max(1, want - 1))].label)
    return [...new Set(a)]
  })
  const note = $derived.by(() => {
    const skipped = c.skippedFilters || []
    return skipped.length ? skipped.join(', ') + (skipped.length > 1 ? ' filters do not' : ' filter does not') + ' apply to distributions — only account, date and symbol narrow this page.' : ''
  })

  const holdingRows = $derived.by(() => {
    const posById: Record<string, { mv: number }> = {}
    ;(model.positions || []).forEach((p) => {
      posById[p.id] = p
    })
    return (c.holdings || []).map((h) => Object.assign({}, h, { mv: posById[h.id] ? posById[h.id].mv : null }) as CashflowHolding)
  })
  const holdings = $derived(sortRows(holdingRows, sort.yoc.key, sort.yoc.dir, (r, k) => (r as unknown as Record<string, unknown>)[k]))
  const rows = $derived(sortRows(c.rows || [], sort.cash.key, sort.cash.dir, (r, k) => (r as unknown as Record<string, unknown>)[k]))

  const pie = $derived(pieSlices(holdingRows))
  const pieFmt = (v: number) => money0(v) + '/mo'
  // arc geometry for the ring; accumulates share around the circle
  const pieArcs = $derived.by(() => {
    const R = 44, W = 14, C = 60
    let acc = 0
    return pie.items.map((x, i) => {
      const a0 = acc * 2 * Math.PI - Math.PI / 2
      const a1 = (acc + x.share) * 2 * Math.PI - Math.PI / 2
      acc += x.share
      if (x.share >= 0.9999) return { i, circle: true, color: x.color, C, R, W } as const
      const large = x.share > 0.5 ? 1 : 0
      const d = 'M' + (C + R * Math.cos(a0)).toFixed(2) + ' ' + (C + R * Math.sin(a0)).toFixed(2) + ' A' + R + ' ' + R + ' 0 ' + large + ' 1 ' + (C + R * Math.cos(a1)).toFixed(2) + ' ' + (C + R * Math.sin(a1)).toFixed(2)
      return { i, circle: false, d, color: x.color, C, R, W } as const
    })
  })

  // hover state (replaces the original's DOM id lookups)
  let cfH = $state<number | null>(null)
  let pieH = $state<number>(-1)

  function barOrder(b: { key: string; value: number; count: number }) {
    const hd = b.count === 0 ? 0 : Math.max(1.5, (b.value / head) * 94)
    const hi = ((interest[b.key] || 0) / head) * 94
    const dist = { h: hd, color: 'var(--accent-bar)' }
    const intr = { h: hi, color: 'var(--neg)' }
    const bars = hi > hd ? [intr, dist] : [dist, ...(hi > 0 ? [intr] : [])]
    return bars
  }
</script>

<div style="padding:20px;display:flex;flex-direction:column;gap:14px;min-height:380px">
  <div style="display:grid;grid-template-columns:repeat(6,minmax(0,1fr));gap:14px">
    {#each c.tiles as t (t.label)}
      {#if 'marginUsed' in t}
        <div class="card elev-sm kpi"><div class="lbl">Margin used</div><div class="tab v" use:roll={money0(t.marginUsed)}></div><div class="s">{money0(t.interestPerMonth)}/mo margin interest</div></div>
      {:else if 'yield' in t}
        <div class="card elev-sm kpi"><div class="lbl">Yield on cost</div><div class="tab v" style="color:var(--accent-300)" use:roll={t.yield == null ? '—' : pctPlain(t.yield, 2)}></div><div class="s">{money0(t.projected)}/mo</div></div>
      {:else}
        <div class="card elev-sm kpi"><div class="lbl">{String(t.label).replace(/^\d{4} YTD$/, 'YTD')}</div><div class="tab v" use:roll={money0(t.total)}></div><div class="s">{t.label === 'All time' ? 'Total earned' : money0(t.perMonth) + '/mo avg'}</div></div>
      {/if}
    {/each}
  </div>

  {#if note}
    <div class="dim" style="font-size:11px;line-height:1.5">{note}</div>
  {/if}

  <div class="card elev-sm" style="padding:16px 18px 12px">
    <div style="display:flex;align-items:baseline;justify-content:space-between;gap:8px"><h5>Cashflow</h5>
      <div style="display:flex;align-items:center;gap:14px;font-size:11px;color:var(--ink60)"><span style="display:inline-flex;align-items:center;gap:6px"><span style="width:9px;height:9px;border-radius:2px;background:var(--accent-bar)"></span>Distributions</span><span style="display:inline-flex;align-items:center;gap:6px"><span style="width:9px;height:9px;border-radius:2px;background:var(--neg)"></span>Margin interest</span></div>
    </div>
    <div style="display:flex;gap:10px;flex:1;min-height:232px;margin-top:14px">
      <div style="position:relative;width:58px;flex:none;font-size:10px;color:var(--ink60)" class="tab">
        {#each [0, 1, 2, 3] as i (i)}
          <span style="position:absolute;left:0;top:{(i / 3 * 94).toFixed(1)}%;transform:translateY(-50%)">{money0(head * (1 - i / 3))}</span>
        {/each}
      </div>
      <div id="cfPlot" style="position:relative;flex:1;min-width:0;display:flex;gap:4px;align-items:stretch" onmouseleave={() => (cfH = null)} role="presentation">
        {#if cfH !== null}
          {@const b = ms[cfH]}
          {@const n = ms.length}
          {@const x = ((cfH + 0.5) / n * 100).toFixed(2) + '%'}
          {@const intr = interest[b.key] || 0}
          {@const net = b.value - intr}
          <div class="xline" style="left:{x}"></div>
          <div class="tip" style="left:{x};transform:translateX({cfH > n * 0.7 ? '-100%' : '-8px'})">
            <div class="tl" style="margin-bottom:2px">{monthLong(b.key)}</div>
            <div class="tv" style="display:flex;justify-content:space-between;gap:16px;color:var(--accent-300)"><span style="font-weight:400;color:var(--ink60)">Distributions</span><span>{money0(b.value)}</span></div>
            <div class="tv" style="display:flex;justify-content:space-between;gap:16px;color:var(--neg)"><span style="font-weight:400;color:var(--ink60)">Margin interest</span><span>{intr ? signedMoney(-intr, '', 0) : money0(0)}</span></div>
            <div class="tv" style="display:flex;justify-content:space-between;gap:16px;color:{color(net)}"><span style="font-weight:400;color:var(--ink60)">Net cashflow</span><span>{signedMoney(net, '', 0)}</span></div>
          </div>
        {/if}
        {#each [0, 1, 2, 3] as i (i)}
          <div style="position:absolute;left:0;right:0;top:{(i / 3 * 94).toFixed(1)}%;height:1px;background:{i === 3 ? 'var(--hair)' : 'var(--grid)'}"></div>
        {/each}
        {#if ms.length}
          {#each ms as b, i (b.key)}
            <div class="bar-col" data-i={i} style="cursor:default" onmouseenter={() => (cfH = i)} role="presentation">
              {#each barOrder(b) as bar (bar.color)}
                <div style="position:absolute;left:0;right:0;bottom:6%;height:{bar.h.toFixed(1)}%;background:{bar.color};border-radius:2px 2px 0 0"></div>
              {/each}
            </div>
          {/each}
        {:else}
          <div class="muted empty" style="flex:1;font-size:12px">No distributions in this range.</div>
        {/if}
      </div>
    </div>
    <div style="display:flex;justify-content:space-between;font-size:10px;color:var(--ink60);padding-left:68px;margin-top:6px">
      {#each axis as a (a)}<span>{a}</span>{/each}
    </div>
  </div>

  <div class="card elev-sm" style="padding:16px 18px">
    <h5 style="margin-bottom:4px">Cashflow Positions</h5>
    <div class="scroll" style="max-height:344px">
      <table class="table" style="font-size:12px;table-layout:fixed">
        <thead><tr>
          {#each ycols as col (col.key)}
            {@const on = sort.yoc.key === col.key}
            {@const right = col.align === 'right'}
            <th style="white-space:nowrap;text-align:{col.align || 'left'};cursor:pointer;position:sticky;top:0;z-index:1;color:{on ? 'var(--ink)' : 'rgba(var(--ink-rgb),.6)'}{col.padLeft ? ';padding-left:' + col.padLeft : ''}{col.padRight ? ';padding-right:' + col.padRight : ''}{col.width ? ';width:' + col.width : ''}" onclick={() => toggleSort('yoc', col.key)}>
              <span class="th-in" style="flex-direction:{right ? 'row-reverse' : 'row'}">{col.label}<span class="arrow" style="color:{on ? 'var(--accent)' : 'transparent'}">{on && sort.yoc.dir === 'asc' ? '▲' : '▼'}</span></span>
            </th>
          {/each}
        </tr></thead>
        <tbody>
          {#if holdings.length}
            {#each holdings as h (h.id)}
              <tr class="tab">
                <td style="font-weight:500;font-variant-numeric:normal;white-space:nowrap">{symText(h.symbol)}</td>
                <td style="text-align:right;color:var(--ink75)">{qty(h.qty)}</td>
                <td style="text-align:right;color:var(--ink75)">{px(h.avg)}</td>
                <td style="text-align:right;color:var(--ink75)">{money0(h.cost)}</td>
                <td style="text-align:right;color:var(--ink75)">{h.mv == null ? '—' : money0(h.mv)}</td>
                <td style="text-align:right;color:var(--ink75)">{h.per == null ? '—' : '$' + h.per.toFixed(h.per < 1 ? 4 : 2)}</td>
                <td style="text-align:right">{money0(h.ytd)}</td>
                <td style="text-align:right">{money0(h.all)}</td>
                <td style="text-align:right;color:{h.exPast ? 'var(--ink55)' : 'var(--ink)'}">{h.nextExDate || '—'}</td>
                <td style="text-align:right;color:{h.payPast ? 'var(--ink55)' : 'var(--ink)'}">{h.nextPayDate || '—'}</td>
                <td style="text-align:right">{h.annual == null ? '—' : money0(h.annual / 12)}</td>
                <td style="text-align:right;font-weight:500;color:var(--accent-300)">{h.yoc == null ? '—' : pctPlain(h.yoc, 2)}</td>
                <td style="text-align:right;padding-right:0;color:var(--ink75)">{h.currentYield == null ? '—' : pctPlain(h.currentYield, 2)}</td>
              </tr>
            {/each}
          {:else}
            <tr><td colspan="13" class="muted" style="padding:22px 0">No income holdings in scope.</td></tr>
          {/if}
        </tbody>
      </table>
    </div>
  </div>

  <div style="display:grid;grid-template-columns:calc((100% - 70px) / 6 * 3 + 28px) minmax(0,1fr);gap:14px;height:380px">
    <div class="card elev-sm" style="padding:16px 18px 12px;display:flex;flex-direction:column;min-height:0">
      <div style="display:flex;align-items:baseline;justify-content:space-between;gap:8px;margin-bottom:10px"><h5>Allocation</h5></div>
      {#if pie.items.length}
        <div id="piePlot" style="display:flex;gap:28px;align-items:center;flex:1;min-height:0" onmouseleave={() => (pieH = -1)} role="presentation">
          <div style="position:relative;flex:none;width:max(200px, min(322px, 100% - 198px));aspect-ratio:1/1">
            <svg viewBox="0 0 120 120" style="width:100%;height:100%;display:block">
              {#each pieArcs as arc (arc.i)}
                {@const op = pieH < 0 || arc.i === pieH ? '1' : '.35'}
                {#if arc.circle}
                  <circle cx={arc.C} cy={arc.C} r={arc.R} fill="none" stroke={arc.color} stroke-width={arc.W} data-slice={arc.i} style="opacity:{op}" onmouseenter={() => (pieH = arc.i)} role="presentation" />
                {:else}
                  <path d={arc.d} fill="none" stroke={arc.color} stroke-width={arc.W} data-slice={arc.i} style="cursor:default;opacity:{op}" onmouseenter={() => (pieH = arc.i)} role="presentation" />
                {/if}
              {/each}
            </svg>
            <div style="position:absolute;inset:0;display:flex;flex-direction:column;align-items:center;justify-content:center;text-align:center;pointer-events:none">
              {#if pieH >= 0}
                {@const x = pie.items[pieH]}
                <div class="lbl">{symText(x.symbol)}</div>
                <div class="tab" style="font-size:17px;font-weight:500;margin-top:2px">{pieFmt(x.v)}</div>
                <div class="muted" style="font-size:11px;margin-top:2px">{pctPlain(x.share)}</div>
              {:else}
                <div class="lbl">Projected</div>
                <div class="tab" style="font-size:17px;font-weight:500;margin-top:2px">{pieFmt(pie.total)}</div>
              {/if}
            </div>
          </div>
          <div class="scroll" style="flex:1 1 auto;min-width:0;min-height:0;max-height:100%;display:grid;grid-template-columns:auto minmax(0,1fr) auto auto;column-gap:12px;row-gap:12px;align-content:center;align-items:center;font-size:12.5px">
            {#each pie.items as x, i (i)}
              {@const legOp = pieH < 0 || i === pieH ? '1' : '.45'}
              <span data-slice={i} style="display:contents" onmouseenter={() => (pieH = i)} role="presentation">
                <span style="width:9px;height:9px;border-radius:2px;background:{x.color};opacity:{legOp}"></span>
                <span style="font-weight:500;font-variant-numeric:normal;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;opacity:{legOp}">{symText(x.symbol)}</span>
                <span class="tab" style="text-align:right;white-space:nowrap;opacity:{legOp}">{pieFmt(x.v)}</span>
                <span class="tab muted" style="text-align:right;white-space:nowrap;min-width:44px;opacity:{legOp}">{pctPlain(x.share)}</span>
              </span>
            {/each}
          </div>
        </div>
      {:else}
        <div class="muted empty" style="flex:1;font-size:12px">No income holdings in scope.</div>
      {/if}
    </div>

    <div class="card elev-sm" style="padding:16px 18px;display:flex;flex-direction:column;min-height:0">
      <h5 style="margin-bottom:4px">Distribution history</h5>
      <div class="scroll-xy" style="flex:1;min-height:0">
        <table class="table" style="min-width:560px;width:100%;table-layout:fixed;font-size:12px">
          <thead><tr>
            {#each dcols as col (col.key)}
              {@const on = sort.cash.key === col.key}
              {@const right = col.align === 'right'}
              <th style="white-space:nowrap;text-align:{col.align || 'left'};cursor:pointer;position:sticky;top:0;z-index:1;color:{on ? 'var(--ink)' : 'rgba(var(--ink-rgb),.6)'}{col.padLeft ? ';padding-left:' + col.padLeft : ''}{col.padRight ? ';padding-right:' + col.padRight : ''}{col.width ? ';width:' + col.width : ''}" onclick={() => toggleSort('cash', col.key)}>
                <span class="th-in" style="flex-direction:{right ? 'row-reverse' : 'row'}">{col.label}<span class="arrow" style="color:{on ? 'var(--accent)' : 'transparent'}">{on && sort.cash.dir === 'asc' ? '▲' : '▼'}</span></span>
              </th>
            {/each}
          </tr></thead>
          <tbody>
            {#if rows.length}
              {#each rows as r (r.id)}
                <tr class="tab">
                  <td style="color:var(--ink75);white-space:nowrap">{r.date}</td>
                  <td style="font-weight:500;font-variant-numeric:normal;white-space:nowrap">{symText(r.symbol)}</td>
                  <td style="font-variant-numeric:normal;color:var(--ink75);white-space:nowrap;overflow:hidden;text-overflow:ellipsis">{r.account}</td>
                  <td style="text-align:right;color:var(--ink75)">{r.qty ? qty(r.qty) : '—'}</td>
                  <td style="text-align:right;color:var(--ink75)">{r.per ? '$' + r.per.toFixed(4) : '—'}</td>
                  <td style="text-align:right;font-weight:500">{money(r.amount, r.currency)}</td>
                </tr>
              {/each}
            {:else}
              <tr><td colspan="6" class="dim" style="padding:22px 0;font-variant-numeric:normal">No distributions match these filters.</td></tr>
            {/if}
          </tbody>
        </table>
      </div>
    </div>
  </div>
</div>
