<script lang="ts">
  import type { Portfolio } from '../model'
  import { cad, pct } from '../fmt'

  let { pf }: { pf: Portfolio } = $props()

  type Tile = { label: string; value: string; sub: string; tone?: 'pos' | 'neg' }

  const tiles = $derived.by(() => {
    const t: Tile[] = [
      { label: 'Net asset value', value: cad(pf.nav), sub: `${pf.accountCount} accounts, ${pf.positionCount} positions` },
      { label: 'Cost basis', value: cad(pf.costBasis), sub: 'Total book value' },
    ]
    if (pf.hasMargin) {
      t.push({ label: 'Margin used', value: cad(pf.marginUsed), sub: pct(pf.marginUsedPct) + ' of market value' })
      t.push({
        label: 'Available margin',
        value: pf.availableMargin != null ? cad(pf.availableMargin) : '—',
        sub: pf.availableMarginUnavailable ? `Unavailable for ${pf.availableMarginUnavailable}` : 'Buying power',
      })
    } else {
      t.push({ label: 'Cash', value: cad(pf.cash), sub: pf.cashPct != null ? pct(pf.cashPct) + ' of net asset value' : '—' })
    }
    t.push({
      label: '1d change',
      value: pf.dayChange != null ? cad(pf.dayChange) : '—',
      sub: pf.dayChangePct != null ? (pf.dayChangePct >= 0 ? '+' : '') + pct(pf.dayChangePct) + ' today' : '',
      tone: pf.dayChange == null ? undefined : pf.dayChange >= 0 ? 'pos' : 'neg',
    })
    t.push({
      label: 'Unrealized P&L',
      value: cad(pf.unrealized),
      sub: pct(Math.abs(pf.unrealizedPct)) + ' ' + (pf.unrealized >= 0 ? 'gain' : 'loss'),
      tone: pf.unrealized >= 0 ? 'pos' : 'neg',
    })
    return t
  })
</script>

<div class="tiles" style="grid-template-columns: repeat({tiles.length}, 1fr)">
  {#each tiles as t (t.label)}
    <div class="tile">
      <div class="tile-label">{t.label}</div>
      <div class="tile-value {t.tone ?? ''}">{t.value}</div>
      <div class="tile-sub">{t.sub}</div>
    </div>
  {/each}
</div>

<style>
  .tiles { display: grid; gap: 12px; }
  .tile { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 12px 14px; }
  .tile-label { color: #8b93a7; font-size: 11px; text-transform: uppercase; letter-spacing: 0.04em; }
  .tile-value { font-size: 20px; font-weight: 600; margin-top: 6px; font-variant-numeric: tabular-nums; }
  .tile-sub { color: #8b93a7; font-size: 11px; margin-top: 2px; }
  .pos { color: #3ecf8e; }
  .neg { color: #f0616d; }
  @media (max-width: 1100px) { .tiles { grid-template-columns: repeat(3, 1fr) !important; } }
</style>
