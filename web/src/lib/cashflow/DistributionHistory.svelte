<script lang="ts">
  import type { CashflowRow } from '../model'
  import { money, per, num } from '../fmt'

  let { rows }: { rows: CashflowRow[] } = $props()
</script>

<div class="card">
  <h5>Distribution history</h5>
  <div class="scroll">
    <table>
      <colgroup>
        <col style="width:17%" /><col style="width:14%" /><col style="width:27%" />
        <col style="width:10%" /><col style="width:18%" /><col style="width:14%" />
      </colgroup>
      <thead>
        <tr><th class="l">Date</th><th class="l">Symbol</th><th class="l">Account</th><th>Qty</th><th>Distribution</th><th>Amount</th></tr>
      </thead>
      <tbody>
        {#each rows as r (r.id)}
          <tr>
            <td class="l">{r.date}</td>
            <td class="l sym">{r.symbol}</td>
            <td class="l acct">{r.account}</td>
            <td>{r.qty != null ? num(r.qty, 0) : '—'}</td>
            <td>{per(r.per)}</td>
            <td>{money(r.amount, r.currency)}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
</div>

<style>
  .card { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 16px 18px; height: 380px; display: flex; flex-direction: column; }
  .card h5 { margin: 0 0 10px; font-size: 14px; font-weight: 600; }
  .scroll { overflow-y: auto; flex: 1; min-height: 0; }
  table { width: 100%; table-layout: fixed; border-collapse: collapse; font-size: 12px; font-variant-numeric: tabular-nums; }
  th { position: sticky; top: 0; background: #141924; color: #8b93a7; font-weight: 500; text-align: right; padding: 6px 8px; border-bottom: 1px solid #1c2230; }
  td { text-align: right; padding: 6px 8px; border-bottom: 1px solid #12161f; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  th.l, td.l { text-align: left; }
  .sym { font-weight: 600; }
  .acct { color: #c4cbd8; }
</style>
