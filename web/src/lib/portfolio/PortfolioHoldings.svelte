<script lang="ts">
  import type { Position } from '../model'
  import { cad, pct, price, pctRaw } from '../fmt'

  let { positions }: { positions: Position[] } = $props()
  // Sorted by unrealized P&L, per SPEC.
  const rows = $derived([...positions].sort((a, b) => b.unreal - a.unreal))
  const signed = (n: number) => (n >= 0 ? '+' : '') + cad(n, 2)
  const signedPct = (n: number) => (n >= 0 ? '+' : '') + pct(n) // for fractions (unrealPct)
</script>

<div class="card">
  <h5>Holdings</h5>
  <div class="scroll">
    <table>
      <thead>
        <tr>
          <th class="l">Symbol</th><th>Avg</th><th>Last</th><th>Book</th><th>Market</th>
          <th>Change ($)</th><th>Change (%)</th><th>Unrealized P&amp;L</th>
        </tr>
      </thead>
      <tbody>
        {#each rows as p (p.id)}
          <tr>
            <td class="l sym">{p.symbol}{#if p.short}<span class="short">SHORT</span>{/if}</td>
            <td>{price(p.avg)}</td>
            <td>{price(p.last)}</td>
            <td>{cad(p.cost)}</td>
            <td>{cad(p.mv)}</td>
            <td class={p.dayChange == null ? '' : p.dayChange >= 0 ? 'pos' : 'neg'}>{p.dayChange == null ? '—' : signed(p.dayChange)}</td>
            <td class={p.percentChange == null ? '' : p.percentChange >= 0 ? 'pos' : 'neg'}>{p.percentChange == null ? '—' : pctRaw(p.percentChange)}</td>
            <td class={p.unreal >= 0 ? 'pos' : 'neg'}>{signed(p.unreal)} ({signedPct(p.unrealPct)})</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
</div>

<style>
  .card { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 16px 18px; }
  .card h5 { margin: 0 0 10px; font-size: 14px; font-weight: 600; }
  .scroll { overflow: auto; max-height: 396px; }
  table { width: 100%; border-collapse: collapse; font-size: 12px; font-variant-numeric: tabular-nums; }
  th { position: sticky; top: 0; background: #141924; color: #8b93a7; font-weight: 500; text-align: right; padding: 6px 10px; white-space: nowrap; border-bottom: 1px solid #1c2230; }
  td { text-align: right; padding: 7px 10px; white-space: nowrap; border-bottom: 1px solid #12161f; }
  th.l, td.l { text-align: left; }
  .sym { font-weight: 600; }
  .short { color: #f0616d; font-size: 10px; margin-left: 6px; border: 1px solid #f0616d; border-radius: 3px; padding: 0 3px; }
  tr:hover td { background: #171d29; }
  .pos { color: #3ecf8e; }
  .neg { color: #f0616d; }
</style>
