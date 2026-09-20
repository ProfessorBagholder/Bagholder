<script lang="ts">
  import type { WatchItem } from '../model'
  import { removeWatch } from '../state.svelte'
  import { price } from '../fmt'

  let { watchlist }: { watchlist: WatchItem[] } = $props()
  const chg = (n: number | null) => (n == null ? '—' : (n >= 0 ? '+' : '') + n.toFixed(2) + '%')
</script>

<div class="card">
  <h5>Watchlist</h5>
  {#if !watchlist.length}
    <p class="msg">Nothing watched.</p>
  {:else}
    <div class="scroll">
      {#each watchlist as w (w.symbol + w.exchange)}
        <div class="row">
          <div class="id">
            <div class="sym">{w.symbol}</div>
            <div class="name">{w.name}</div>
          </div>
          <div class="exch">{w.exchange}</div>
          <div class="last">{w.last != null ? price(w.last) : '—'}</div>
          <div class="chg {w.percentChange == null ? '' : w.percentChange >= 0 ? 'pos' : 'neg'}">{chg(w.percentChange)}</div>
          <button class="trash" aria-label="Remove {w.symbol}" onclick={() => removeWatch(w.symbol, w.exchange)}>🗑</button>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .card { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 16px 18px; }
  .card h5 { margin: 0 0 10px; font-size: 14px; font-weight: 600; }
  .msg { color: #8b93a7; font-size: 13px; }
  .scroll { max-height: 396px; overflow-y: auto; }
  .row { display: grid; grid-template-columns: minmax(0, 1fr) 58px 70px 66px 22px; gap: 10px; align-items: center; padding: 6px 0; border-bottom: 1px solid #12161f; }
  .row:hover { background: #171d29; }
  .id { min-width: 0; }
  .sym { font-weight: 500; font-size: 13px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .name { color: #8b93a7; font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .exch { color: #8b93a7; font-size: 11px; }
  .last { text-align: right; font-size: 12px; font-variant-numeric: tabular-nums; }
  .chg { text-align: right; font-size: 12px; font-variant-numeric: tabular-nums; color: #8b93a7; }
  .pos { color: #3ecf8e; } .neg { color: #f0616d; }
  .trash { background: none; border: 0; cursor: pointer; opacity: 0.45; font-size: 12px; padding: 0; }
  .trash:hover { opacity: 1; }
</style>
