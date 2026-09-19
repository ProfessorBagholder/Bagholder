<script lang="ts">
  import type { CashflowHolding } from '../model'
  import { cad, per, pct, num } from '../fmt'

  let { holdings }: { holdings: CashflowHolding[] } = $props()
  const market = (h: CashflowHolding) => h.qty * h.last
</script>

<div class="card">
  <h5>Cashflow Positions</h5>
  <div class="scroll">
    <table>
      <thead>
        <tr>
          <th class="l">Holding</th><th>Qty</th><th>Avg</th><th>Book</th><th>Market</th>
          <th>Distribution</th><th>YTD</th><th>All time</th><th>Ex-Div</th><th>Pay Day</th>
          <th>Projected</th><th>Yield on cost</th><th>Current yield</th>
        </tr>
      </thead>
      <tbody>
        {#each holdings as h (h.id)}
          <tr>
            <td class="l sym">{h.symbol}</td>
            <td>{num(h.qty, 0)}</td>
            <td>{cad(h.avg, 2)}</td>
            <td>{cad(h.cost)}</td>
            <td>{cad(market(h))}</td>
            <td>{per(h.per)}</td>
            <td>{cad(h.ytd)}</td>
            <td>{cad(h.all)}</td>
            <td class:muted={h.exPast}>{h.nextExDate || '—'}</td>
            <td class:muted={h.payPast}>{h.nextPayDate || '—'}</td>
            <td>{cad(h.yob)}</td>
            <td>{pct(h.yoc)}</td>
            <td>{pct(h.currentYield)}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
</div>

<style>
  .card { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 16px 18px; }
  .card h5 { margin: 0 0 10px; font-size: 14px; font-weight: 600; }
  .scroll { overflow-x: auto; }
  table { width: 100%; border-collapse: collapse; font-size: 12px; font-variant-numeric: tabular-nums; }
  th { color: #8b93a7; font-weight: 500; text-align: right; padding: 6px 10px; white-space: nowrap; border-bottom: 1px solid #1c2230; }
  td { text-align: right; padding: 7px 10px; white-space: nowrap; border-bottom: 1px solid #12161f; }
  th.l, td.l { text-align: left; }
  .sym { font-weight: 600; }
  tr:hover td { background: #171d29; }
  .muted { color: #5b6474; }
</style>
