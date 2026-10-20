<script lang="ts">
  // Net worth: everything held less everything owed, inside Wealthsimple and out,
  // as Wealthsimple's own net worth feature counts it. The investments are the
  // Wealthsimple accounts at their value after margin; the rest is what the person
  // linked or entered in that feature.
  import { roll } from './actions/roll'
  import type { PositionsDoc } from './model'
  import { money0, stampDay, localDay } from './fmt'

  let { model }: { model: PositionsDoc } = $props()
  const nw = $derived(model.portfolio.netWorth)
  // the day a balance was stated, in the viewer's zone; none for the investments, which are now
  const asOf = (iso: string | null) => (iso ? stampDay(localDay(new Date(iso))) : 'Now')
</script>

<div style="padding:20px;min-height:380px;display:flex;flex-direction:column;gap:14px">
  {#if !nw}
    <div class="card elev-sm" style="padding:24px 18px">
      <div class="muted empty" style="font-size:12px">Nothing held or owed outside Wealthsimple yet. Link or add accounts in Wealthsimple's Net worth, and they show here at the next balances read.</div>
    </div>
  {:else}
    <div style="display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:14px">
      <div class="card elev-sm kpi"><div class="lbl">Net worth</div><div class="v" use:roll={money0(nw.netWorth)}></div><div class="s">Assets less liabilities</div></div>
      <div class="card elev-sm kpi"><div class="lbl">Assets</div><div class="v" use:roll={money0(nw.assetsTotal)}></div><div class="s">{nw.assets.length === 1 ? '1 account' : nw.assets.length + ' accounts'}</div></div>
      <div class="card elev-sm kpi"><div class="lbl">Liabilities</div><div class="v" use:roll={money0(nw.liabilitiesTotal)}></div><div class="s">{nw.liabilities.length === 1 ? '1 debt' : nw.liabilities.length + ' debts'}</div></div>
    </div>
    <div style="display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:14px;align-items:start">
      {#each [['Assets', nw.assets], ['Liabilities', nw.liabilities]] as const as [title, rows] (title)}
        <div class="card elev-sm" style="padding:16px 18px">
          <h5 style="margin-bottom:6px">{title}</h5>
          {#if rows.length}
            <table class="table" style="font-size:12px;table-layout:fixed;width:100%">
              <thead><tr><th style="text-align:left">Account</th><th style="text-align:left">Institution</th><th style="text-align:right;width:22%">As of</th><th style="text-align:right;width:22%">Balance</th></tr></thead>
              <tbody>
                {#each rows as r (r.id)}
                  <tr class="tab"><td>{r.name}</td><td style="color:var(--ink60)">{r.institution || '—'}</td><td style="text-align:right;color:var(--ink60)">{asOf(r.asOf)}</td><td style="text-align:right">{money0(r.amount)}</td></tr>
                {/each}
              </tbody>
            </table>
          {:else}
            <div class="muted empty" style="font-size:12px">None.</div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>
