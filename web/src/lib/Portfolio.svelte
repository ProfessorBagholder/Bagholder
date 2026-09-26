<script lang="ts">
  import { roll } from './actions/roll'
  import type { Model } from './model'
  import { money0, signedMoney, pct, pctPlain, px, cls, color, waiting } from './fmt'
  import { waits } from './dec'
  import { symText } from './sym'
  import { sort, toggleSort, sortRows } from './sort.svelte'
  import { goSub, keepScroll } from './router.svelte'
  import Donut, { type DonutItem } from './Donut.svelte'

  let { model }: { model: Model } = $props()
  const pf = $derived(model.portfolio)

  // --- tiles (portfolioTilesHtml) ---
  // a share under a tile: its word where it waits, a dash where there is none
  const share = (v: number | null | { gaps: string[] }, then: (x: number) => string) => (v == null ? '—' : waits(v) ? waiting(v) : then(v))
  // a total's sub line, with how many it left out
  const tiles = $derived.by(() => {
    const n = pf.positionCount || 0
    const positions = n + (n === 1 ? ' position' : ' positions')
    const unavailable = pf.availableMarginUnavailable || []
    const out: { label: string; value: string; sub: string; vcls: string }[] = [
      {
        label: 'Net asset value',
        value: pf.nav == null ? '—' : money0(pf.nav),
        sub: pf.nav == null ? '—' : pf.navAccounts + (pf.navAccounts === 1 ? ' account, ' : ' accounts, ') + positions,
        vcls: '',
      },
      { label: 'Cost basis', value: money0(pf.costBasis.total), sub: 'Total book value', vcls: '' },
    ]
    if (pf.hasMargin) {
      out.push({
        label: 'Margin used',
        value: money0(pf.marginUsed),
        sub: share(pf.marginUsedPct, (x) => pctPlain(x) + ' of market value'),
        vcls: '',
      })
      out.push({
        label: 'Available margin',
        value: pf.availableMargin == null ? '—' : money0(pf.availableMargin),
        sub: unavailable.length ? 'Unavailable for ' + unavailable.join(', ') : pf.availableMargin == null ? '—' : 'Buying power',
        vcls: '',
      })
    } else {
      out.push({
        label: 'Cash',
        value: money0(pf.cash),
        sub: share(pf.cashPct, (x) => pctPlain(x) + ' of net asset value'),
        vcls: '',
      })
    }
    out.push({
      label: '1d change',
      value: pf.dayChange == null ? '—' : signedMoney(pf.dayChange.total, undefined, 2),
      sub: share(pf.dayChangePct, (x) => pct(x) + ' today'),
      vcls: pf.dayChange == null ? '' : cls(pf.dayChange.total),
    })
    out.push({
      label: 'Unrealized P&L',
      value: signedMoney(pf.unrealized.total, undefined, 2),
      sub: share(pf.unrealizedPct, (x) => pct(x) + (x >= 0 ? ' gain' : ' loss')),
      vcls: cls(pf.unrealized.total),
    })
    return out
  })

  // --- Allocation donut (portfolioSlices) ---
  // the server folds past ten holdings into `Other (n)`
  const allocItems = $derived<DonutItem[]>((pf.allocation || []).map((x, i) => ({ label: x.label, v: x.value, share: x.share, color: 'var(--pie-' + ((i % 11) + 1) + ')' })))

  // --- Sectors / Regions donuts (exposureSlices) ---
  // the server folds the long tail into `Other (n)` and puts what is not classified last
  type ExpRow = { name: string; value: number; share: number }
  const UNCLASSIFIED = 'Not classified'
  function exposureSlices(rows: ExpRow[]): DonutItem[] {
    return rows.map((x, i) => ({ label: x.name, v: x.value, share: x.share, color: x.name === UNCLASSIFIED ? 'rgba(var(--ink-rgb),.28)' : 'var(--pie-' + ((i % 11) + 1) + ')' }))
  }
  // the classified names the positions cover: the server's `Other (n)` counts as its n
  const expCount = (rows: ExpRow[]) =>
    String(rows.filter((x) => x.name !== UNCLASSIFIED && x.value > 0).reduce((n, x) => n + (Number(/^Other \((\d+)\)$/.exec(x.name)?.[1]) || 1), 0))
  const sec = $derived(exposureSlices(model.sectors))
  const reg = $derived(exposureSlices(model.regions))

  // --- Holdings table ---
  type Col = { key: string; label: string; align?: 'right' | 'center'; padRight?: string }
  const cols: Col[] = [
    { key: 'symbol', label: 'Symbol' },
    { key: 'avg', label: 'Avg', align: 'right' },
    { key: 'last', label: 'Last', align: 'right' },
    { key: 'cost', label: 'Book', align: 'right' },
    { key: 'mv', label: 'Market', align: 'right' },
    { key: 'day', label: 'Change ($)', align: 'right' },
    { key: 'dayPct', label: 'Change (%)', align: 'right' },
    { key: 'unreal', label: 'Unrealized P&L', align: 'right', padRight: '0' },
  ]

  // a holding with no value in the column sinks, whichever way the rest goes
  function posSortValue(p: (typeof model.positions)[number], key: string): unknown {
    if (key === 'day') return p.dayChange
    if (key === 'dayPct') return p.percentChange
    return (p as unknown as Record<string, unknown>)[key]
  }
  const rows = $derived(sortRows(model.positions || [], sort.positions.key, sort.positions.dir, posSortValue))
</script>

<div style="padding:20px;min-height:380px;display:flex;flex-direction:column;gap:14px">
  <!-- tiles -->
  <div style="display:grid;grid-template-columns:repeat({pf.hasMargin ? 6 : 5},minmax(0,1fr));gap:14px">
    {#each tiles as t (t.label)}
      <div class="card elev-sm kpi"><div class="lbl">{t.label}</div><div class="v {t.vcls}" use:roll={t.value}></div><div class="s">{t.sub}</div></div>
    {/each}
  </div>

  <div style="display:grid;grid-template-columns:calc((100% - 70px) / 6 * 2 + 14px) minmax(0,1fr);gap:14px;align-items:stretch">
    <!-- Allocation card (portfolioPieHtml → donutCardHtml) -->
    <div class="card elev-sm" style="padding:16px 18px 12px;display:flex;flex-direction:column;min-height:0">
      <div style="display:flex;align-items:baseline;justify-content:space-between;gap:8px;margin-bottom:10px"><h5>Allocation</h5></div>
      {#if allocItems.length}
        <div style="display:flex;gap:16px;align-items:center;justify-content:center;flex:1;min-height:0">
          <Donut items={allocItems} total={pf.marketValue.total} centreLabel="Market value" side="l" />
        </div>
      {:else}
        <div class="muted empty" style="flex:1;font-size:12px">No open positions in scope.</div>
      {/if}
    </div>

    <!-- Holdings card -->
    <div class="card elev-sm" style="padding:14px 16px 8px;display:flex;flex-direction:column;min-height:0">
      <div style="display:flex;align-items:baseline;gap:10px;margin-bottom:8px"><h5>Holdings</h5></div>
      <div class="scroll-xy" style="flex:1;min-height:0;max-height:362px" use:keepScroll={'positions'}>
        <table class="table" style="min-width:640px">
          <thead><tr>
            {#each cols as c (c.key)}
              {@const on = sort.positions.key === c.key}
              {@const right = c.align === 'right'}
              <th
                style="white-space:nowrap;text-align:{c.align || 'left'};cursor:pointer;position:sticky;top:0;z-index:1;color:{on ? 'var(--ink)' : 'rgba(var(--ink-rgb),.6)'}{c.padRight ? ';padding-right:' + c.padRight : ''}"
                onclick={() => toggleSort('positions', c.key)}>
                <span class="th-in" style="flex-direction:{right ? 'row-reverse' : 'row'}">{c.label}<span class="arrow" style="color:{on ? 'var(--accent)' : 'transparent'}">{on && sort.positions.dir === 'asc' ? '▲' : '▼'}</span></span>
              </th>
            {/each}
          </tr></thead>
          <tbody>
            {#each rows as r (r.id)}
              <tr class="tab" style="cursor:pointer" onclick={() => goSub('portfolio', r.id)}>
                <td style="font-weight:500;font-variant-numeric:normal;white-space:nowrap;max-width:140px;overflow:hidden;text-overflow:ellipsis">{symText(r.symbol)}{#if r.short}{' '}<span class="muted" style="font-size:10px">SHORT</span>{/if}</td>
                <td style="text-align:right">{px(r.avg)}</td><td style="text-align:right">{px(r.last)}</td>
                <td style="text-align:right">{money0(r.cost, r.currency)}</td><td style="text-align:right">{money0(r.mv, r.currency)}</td>
                <td style="text-align:right;white-space:nowrap;color:{r.dayChange == null ? 'var(--ink55)' : color(r.dayChange)}">{r.dayChange == null ? '—' : signedMoney(r.dayChange, r.currency)}</td>
                <td style="text-align:right;color:{r.percentChange == null ? 'var(--ink55)' : color(r.percentChange)}">{r.percentChange == null ? '—' : pct(r.percentChange, 2)}</td>
                <td style="text-align:right;padding-right:0;white-space:nowrap;font-weight:500;color:{color(r.unreal)}">{signedMoney(r.unreal, r.currency)} ({pct(r.unrealPct)})</td>
              </tr>
            {/each}
          </tbody>
        </table>
        {#if !rows.length}<div class="muted" style="padding:26px 4px;font-size:12px">No open positions match these filters.</div>{/if}
      </div>
    </div>
  </div>

  <!-- Sectors / Regions card (exposureCardsHtml) -->
  <div class="card elev-sm" style="padding:16px 18px 12px">
    <div style="display:flex;align-items:baseline;justify-content:space-between;gap:8px;margin-bottom:10px"><h5>Sectors</h5><h5>Regions</h5></div>
    {#if !sec.length && !reg.length}
      <div class="muted empty" style="font-size:12px">No open positions in scope.</div>
    {:else}
      <div style="display:grid;grid-template-columns:minmax(max-content,1fr) minmax(240px,340px) minmax(240px,340px) minmax(max-content,1fr);column-gap:40px;align-items:center">
        <div style="display:contents">
          <Donut items={sec} total={null} centreLabel="Sectors" centreText={expCount(model.sectors)} side="l" size="100%" legendFirst />
        </div>
        <div style="display:contents">
          <Donut items={reg} total={null} centreLabel="Regions" centreText={expCount(model.regions)} side="r" size="100%" />
        </div>
      </div>
    {/if}
  </div>
</div>
