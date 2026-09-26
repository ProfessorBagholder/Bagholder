<script lang="ts">
  import type { TradesDoc } from './model'
  import { money, pct, px, qty as fqty, hold, cls, color, waiting } from './fmt'
  import { waits } from './dec'
  import { symText } from './sym'
  import { sort, toggleSort } from './sort.svelte'
  import { more } from './subs.svelte'
  import { atEnd } from './actions/atEnd'
  import { goSub, keepScroll } from './router.svelte'
  import { resetFilters } from './filters.svelte'
  import { refilter } from './state.svelte'

  // the trades in scope, sorted by the server as the header says, as far down as scrolled
  let { doc }: { doc: TradesDoc } = $props()

  const cols: { key: string; label: string; align?: string; width?: string; padLeft?: string }[] = [
    { key: 'entryDate', label: 'Open', width: '8%' },
    { key: 'exitDate', label: 'Close', width: '8%' },
    { key: 'symbol', label: 'Symbol', width: '15.5%' },
    { key: 'exchange', label: 'Exchange', width: '8%' },
    { key: 'qty', label: 'Qty', align: 'right', width: '7.5%' },
    { key: 'entry', label: 'Entry', align: 'right', width: '7%' },
    { key: 'exit', label: 'Exit', align: 'right', width: '7%' },
    { key: 'currency', label: 'FX', align: 'center', width: '5%' },
    { key: 'pnl', label: 'P&L', align: 'right', width: '8%' },
    { key: 'pnlPct', label: 'P&L %', align: 'right', width: '7%' },
    { key: 'holdDays', label: 'Hold', align: 'right', width: '5%' },
    { key: 'grade', label: 'Grade', padLeft: '50px', width: '7.5%' },
    { key: 'tags', label: 'Tags' },
  ]

  const rows = $derived(doc.trades)

  function gradeClass(g: string): string {
    return g === 'A' || g === 'B' ? 'g-ab' : g === 'F' ? 'g-f' : g === 'C' ? 'g-c' : 'g-none'
  }
  function clearAll(e: Event) {
    e.preventDefault()
    resetFilters()
    refilter()
  }
</script>

<div style="padding:20px;min-height:380px;display:flex;flex-direction:column;gap:14px">
  <div class="card elev-sm" style="min-width:0;padding:14px 16px 8px;display:flex;flex-direction:column;min-height:0;max-height:620px">
    <div style="display:flex;align-items:baseline;gap:10px;margin-bottom:8px"><h5>Trades</h5></div>
    <div class="scroll-xy" style="flex:1;min-height:0" use:keepScroll={'trades'}>
      <table class="table" style="min-width:1150px;table-layout:fixed">
        <thead><tr>
          {#each cols as c (c.key)}
            {@const on = sort.trades.key === c.key}
            {@const right = c.align === 'right'}
            {@const center = c.align === 'center'}
            <th
              style="white-space:nowrap;text-align:{c.align || 'left'};cursor:pointer;position:sticky;top:0;z-index:1;color:{on ? 'var(--ink)' : 'rgba(var(--ink-rgb),.6)'}{c.padLeft ? ';padding-left:' + c.padLeft : ''}{c.width ? ';width:' + c.width : ''}"
              onclick={() => toggleSort('trades', c.key)}>
              <span class="th-in" style="flex-direction:{right ? 'row-reverse' : 'row'}">{c.label}<span class="arrow" style="color:{on ? 'var(--accent)' : 'transparent'}{center ? ';position:absolute;left:100%;margin-left:4px' : ''}">{on && sort.trades.dir === 'asc' ? '▲' : '▼'}</span></span>
            </th>
          {/each}
        </tr></thead>
        <tbody>
          {#each rows as t (t.id)}
            <tr class="tab" style="cursor:pointer" onclick={() => (t.position ? goSub('portfolio', t.position) : goSub('trades', t.id))}>
              <td class="dim" style="white-space:nowrap;padding-right:12px">{t.entryDate}</td>
              <td class="dim" style="white-space:nowrap">{t.exitDate ?? 'Open'}</td>
              <td style="font-weight:500;font-variant-numeric:normal;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">{symText(t.symbol)}</td>
              <td class="dim" style="font-variant-numeric:normal;white-space:nowrap;overflow:hidden;text-overflow:ellipsis">{t.exchange || '—'}</td>
              <td style="text-align:right">{fqty(t.qty)}</td>
              <td style="text-align:right">{px(t.entry)}</td>
              <td style="text-align:right">{px(t.exit)}</td>
              <td class="dim" style="font-variant-numeric:normal;text-align:center">{t.currency}</td>
              {#if waits(t.pnl)}
                <!-- the dash in the figure's place, and what it waits for beside it -->
                <td style="text-align:right" class="dim">—</td>
                <td style="text-align:right" class="dim">{waiting(t.pnl).replace(/^— ?/, '')}</td>
              {:else}
                <td style="text-align:right;font-weight:500;color:{color(t.pnl)}">{money(t.pnl, t.currency)}</td>
                <td style="text-align:right;color:{color(t.pnl)}">{pct(t.pnlPct)}</td>
              {/if}
              <td style="text-align:right" class="dim">{hold(t.holdDays)}</td>
              <td style="white-space:nowrap;padding-left:50px"><span class="grade {gradeClass(t.grade)}">{t.grade || '—'}</span></td>
              <td style="font-variant-numeric:normal">
                {#if t.tags && t.tags.length}
                  <span class="tagchip">{t.tags[0]}</span>{#if t.tags.length > 1}<span class="muted" style="font-size:10.5px;margin-left:4px">+{t.tags.length - 1}</span>{/if}
                {:else}
                  <span style="color:rgba(var(--ink-rgb),.5)">—</span>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
      {#if rows.length < doc.total}<div use:atEnd={() => more('trades', doc.total)} style="height:1px"></div>{/if}
      {#if !rows.length}
        <div class="muted" style="padding:26px 4px;font-size:12px">No trades match these filters. <a href="#" onclick={clearAll}>Reset all</a></div>
      {/if}
    </div>
  </div>
</div>
