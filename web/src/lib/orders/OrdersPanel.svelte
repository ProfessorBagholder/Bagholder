<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import { ordersStore, startOrdersPoll, stopOrdersPoll, cancelOrder, cancellable, type Order, type Bracket } from './orders.svelte'
  import { price, num } from '../fmt'

  let { onclose }: { onclose: () => void } = $props()

  onMount(() => startOrdersPoll())
  onDestroy(() => stopOrdersPoll())

  const orderPrice = (o: Order) => o.type.includes('STOP') && o.stopPrice != null ? 'stop ' + price(o.stopPrice) : o.limitPrice != null ? 'limit ' + price(o.limitPrice) : 'market'
  const brkStop = (b: Bracket) => b.slKind === 'trail' ? 'trail ' + (b.slTrailUnit === 'pct' ? b.slTrail + '%' : price(b.slTrail)) : b.slPrice != null ? price(b.slPrice) : '—'
</script>

<div class="scrim" role="presentation" onclick={onclose}></div>
<div class="panel" role="dialog" aria-label="Orders">
  <div class="hd"><span class="title">Orders</span><button class="x" onclick={onclose} aria-label="Close">×</button></div>
  <div class="body">
    {#if !ordersStore.loaded}
      <p class="msg">Reading…</p>
    {:else if !ordersStore.orders.length && !ordersStore.brackets.length}
      <p class="msg">No orders.</p>
    {:else}
      {#if ordersStore.orders.length}
        <div class="grp">
          <h6>Orders</h6>
          {#each ordersStore.orders as o (o.id)}
            <div class="card">
              <div class="top">
                <span class="sym {o.side === 'BUY' ? 'pos' : 'neg'}">{o.side} {num(o.quantity, 0)} {o.symbol}</span>
                <span class="status s-{o.status}">{o.status}</span>
              </div>
              <div class="meta">{o.type} · {orderPrice(o)} · {o.account}{o.avgFill != null ? ' · filled ' + price(o.avgFill) : ''}</div>
              {#if o.error}<div class="err">{o.error}</div>{/if}
              {#if cancellable(o)}<button class="cancel" onclick={() => cancelOrder(o.id)}>Cancel</button>{/if}
            </div>
          {/each}
        </div>
      {/if}
      {#if ordersStore.brackets.length}
        <div class="grp">
          <h6>Brackets</h6>
          {#each ordersStore.brackets as b (b.id)}
            <div class="card">
              <div class="top">
                <span class="sym">{num(b.quantity, 0)} {b.symbol}</span>
                <span class="status s-{b.status}">{b.outcome || b.status}</span>
              </div>
              <div class="meta">Stop {brkStop(b)}{b.tpPrice != null ? ' · Target ' + price(b.tpPrice) : ''}</div>
            </div>
          {/each}
        </div>
      {/if}
    {/if}
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; background: rgba(0,0,0,0.4); z-index: 60; }
  .panel { position: fixed; top: 0; right: 0; height: 100vh; width: 380px; max-width: 92vw; background: #0b0e14; border-left: 1px solid #1c2230; z-index: 61; display: flex; flex-direction: column; }
  .hd { display: flex; align-items: center; justify-content: space-between; padding: 16px 18px; border-bottom: 1px solid #1c2230; }
  .title { font-size: 15px; font-weight: 600; }
  .x { background: none; border: 0; color: #8b93a7; font-size: 20px; cursor: pointer; line-height: 1; }
  .body { flex: 1; overflow-y: auto; padding: 16px 18px; display: flex; flex-direction: column; gap: 18px; }
  .msg { color: #8b93a7; font-size: 13px; }
  .grp h6 { margin: 0 0 8px; color: #8b93a7; font-size: 11px; text-transform: uppercase; letter-spacing: 0.03em; }
  .card { background: #141924; border: 1px solid #1c2230; border-radius: 10px; padding: 12px 14px; margin-bottom: 8px; }
  .top { display: flex; justify-content: space-between; align-items: baseline; gap: 8px; }
  .sym { font-weight: 600; font-size: 13px; }
  .status { font-size: 11px; padding: 1px 7px; border-radius: 10px; background: #1c2230; color: #8b93a7; }
  .s-filled, .s-done { background: rgba(62,207,142,0.16); color: #3ecf8e; }
  .s-cancelled, .s-rejected { background: rgba(240,97,109,0.16); color: #f0616d; }
  .meta { color: #8b93a7; font-size: 12px; margin-top: 4px; }
  .err { color: #f0616d; font-size: 12px; margin-top: 4px; }
  .cancel { margin-top: 8px; background: #141924; border: 1px solid #f0616d; color: #f0616d; border-radius: 6px; padding: 4px 10px; font: inherit; font-size: 12px; cursor: pointer; }
  .pos { color: #3ecf8e; } .neg { color: #f0616d; }
</style>
