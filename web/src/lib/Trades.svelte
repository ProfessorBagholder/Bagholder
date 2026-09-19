<script lang="ts">
  import type { Trade } from './model'
  import { price, money, pct, num } from './fmt'

  let { trades }: { trades: Trade[] } = $props()

  type Col = { key: string; label: string; align?: 'right' | 'center' }
  const cols: Col[] = [
    { key: 'entryDate', label: 'Open' },
    { key: 'exitDate', label: 'Close' },
    { key: 'symbol', label: 'Symbol' },
    { key: 'exchange', label: 'Exchange' },
    { key: 'qty', label: 'Qty', align: 'right' },
    { key: 'entry', label: 'Entry', align: 'right' },
    { key: 'exit', label: 'Exit', align: 'right' },
    { key: 'currency', label: 'FX', align: 'center' },
    { key: 'pnl', label: 'P&L', align: 'right' },
    { key: 'pnlPct', label: 'P&L %', align: 'right' },
    { key: 'holdDays', label: 'Hold', align: 'right' },
    { key: 'grade', label: 'Grade' },
    { key: 'tags', label: 'Tags' },
  ]

  // Client-side sort — instant, no refetch (the rich-client behaviour). Newest
  // close first by default.
  let sort = $state<{ key: string; dir: 1 | -1 }>({ key: 'exitDate', dir: -1 })

  const GRADE_RANK: Record<string, number> = { A: 1, B: 2, C: 3, D: 4, F: 5 }
  function sortValue(t: Trade, key: string): number | string {
    if (key === 'tags') return (t.tags && t.tags[0]) || ''
    return (t as unknown as Record<string, number | string>)[key] ?? ''
  }

  const rows = $derived.by(() => {
    const { key, dir } = sort
    const arr = [...trades]
    arr.sort((a, b) => {
      // Grade: A first ascending, F first descending, ungraded always last.
      if (key === 'grade') {
        const ra = a.grade ? GRADE_RANK[a.grade] ?? 98 : 99
        const rb = b.grade ? GRADE_RANK[b.grade] ?? 98 : 99
        if (ra === 99 || rb === 99) return ra - rb // ungraded to the end either way
        return (ra - rb) * dir
      }
      const va = sortValue(a, key)
      const vb = sortValue(b, key)
      if (typeof va === 'number' && typeof vb === 'number') return (va - vb) * dir
      return String(va).localeCompare(String(vb)) * dir
    })
    return arr
  })

  function toggle(key: string) {
    if (sort.key === key) sort = { key, dir: sort.dir === 1 ? -1 : 1 }
    else sort = { key, dir: key === 'grade' ? 1 : -1 }
  }
  const qtyFmt = (q: number) => (Number.isInteger(q) ? num(q, 0) : String(q))
  const isDeposit = (t: Trade) => (t.flags || []).includes('basis-unknown')
</script>

<div class="card">
  <div class="head"><h5>Trades</h5><span class="count">{trades.length}</span></div>
  <div class="scroll">
    <table>
      <thead>
        <tr>
          {#each cols as c (c.key)}
            <th class={c.align ?? 'l'} onclick={() => toggle(c.key)}>
              {c.label}{#if sort.key === c.key}<span class="arr">{sort.dir === 1 ? '▲' : '▼'}</span>{/if}
            </th>
          {/each}
        </tr>
      </thead>
      <tbody>
        {#each rows as t (t.id)}
          <tr>
            <td class="l dim">{t.entryDate}</td>
            <td class="l dim">{t.exitDate}</td>
            <td class="l sym">{t.symbol}</td>
            <td class="l dim">{t.exchange || '—'}</td>
            <td class="r">{qtyFmt(t.qty)}</td>
            <td class="r">{price(t.entry)}</td>
            <td class="r">{price(t.exit)}</td>
            <td class="c dim">{t.currency}</td>
            {#if isDeposit(t)}
              <td class="r dim">—</td><td class="r dim">deposited</td>
            {:else}
              <td class="r {t.pnl >= 0 ? 'pos' : 'neg'}">{money(t.pnl, t.currency)}</td>
              <td class="r {t.pnl >= 0 ? 'pos' : 'neg'}">{pct(t.pnlPct)}</td>
            {/if}
            <td class="r dim">{t.holdDays}d</td>
            <td class="l">
              {#if t.grade}<span class="grade g{t.grade}">{t.grade}</span>{:else}<span class="dim">—</span>{/if}
            </td>
            <td class="l">
              {#if t.tags && t.tags.length}<span class="tag">{t.tags[0]}</span>{#if t.tags.length > 1}<span class="more">+{t.tags.length - 1}</span>{/if}{:else}<span class="dim">—</span>{/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
</div>

<style>
  .card { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 14px 16px; }
  .head { display: flex; align-items: baseline; gap: 10px; margin-bottom: 8px; }
  .head h5 { margin: 0; font-size: 14px; font-weight: 600; }
  .count { color: #8b93a7; font-size: 12px; }
  .scroll { overflow: auto; max-height: 640px; }
  table { width: 100%; min-width: 1150px; border-collapse: collapse; font-size: 12px; font-variant-numeric: tabular-nums; table-layout: fixed; }
  th { position: sticky; top: 0; background: #141924; color: #8b93a7; font-weight: 500; padding: 6px 10px; white-space: nowrap; border-bottom: 1px solid #1c2230; cursor: pointer; user-select: none; }
  th.r { text-align: right; } th.c { text-align: center; } th.l { text-align: left; }
  th:hover { color: #c4cbd8; }
  .arr { font-size: 9px; margin-left: 3px; }
  td { padding: 7px 10px; white-space: nowrap; border-bottom: 1px solid #12161f; overflow: hidden; text-overflow: ellipsis; }
  td.r { text-align: right; } td.c { text-align: center; } td.l { text-align: left; }
  .sym { font-weight: 500; }
  .dim { color: #8b93a7; }
  tr:hover td { background: #171d29; }
  .pos { color: #3ecf8e; } .neg { color: #f0616d; }
  .grade { display: inline-block; min-width: 18px; text-align: center; border-radius: 4px; padding: 1px 5px; font-weight: 600; font-size: 11px; }
  .gA { background: rgba(62,207,142,0.18); color: #3ecf8e; }
  .gB { background: rgba(120,199,120,0.16); color: #86c682; }
  .gC { background: rgba(242,163,65,0.16); color: #f2a341; }
  .gD { background: rgba(240,140,90,0.16); color: #f08c5a; }
  .gF { background: rgba(240,97,109,0.16); color: #f0616d; }
  .tag { background: #232a38; border-radius: 4px; padding: 1px 6px; font-size: 11px; }
  .more { color: #8b93a7; font-size: 10px; margin-left: 4px; }
</style>
