<script lang="ts">
  import type { CashflowTile } from '../model'
  import { cad } from '../fmt'

  let { tiles }: { tiles: CashflowTile[] } = $props()

  function value(t: CashflowTile): string {
    if (t.marginUsed != null) return cad(t.marginUsed)
    if (t.yield != null) return (t.yield * 100).toFixed(2) + '%'
    return cad(t.total ?? 0)
  }
  function subtitle(t: CashflowTile): string {
    if (t.marginUsed != null) return cad(t.interestPerMonth ?? 0) + '/mo margin interest'
    if (t.yield != null) return cad((t.projected ?? 0) / 12) + '/mo'
    if (t.label === 'All time') return 'Total earned'
    return cad(t.perMonth ?? 0) + '/mo avg'
  }
</script>

<div class="tiles">
  {#each tiles as t (t.label)}
    <div class="tile">
      <div class="tile-label">{t.label}</div>
      <div class="tile-value">{value(t)}</div>
      <div class="tile-sub">{subtitle(t)}</div>
    </div>
  {/each}
</div>

<style>
  .tiles { display: grid; grid-template-columns: repeat(6, 1fr); gap: 12px; }
  .tile { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 12px 14px; }
  .tile-label { color: #8b93a7; font-size: 11px; text-transform: uppercase; letter-spacing: 0.04em; }
  .tile-value { font-size: 20px; font-weight: 600; margin-top: 6px; font-variant-numeric: tabular-nums; }
  .tile-sub { color: #8b93a7; font-size: 11px; margin-top: 2px; }
  @media (max-width: 1100px) { .tiles { grid-template-columns: repeat(3, 1fr); } }
</style>
