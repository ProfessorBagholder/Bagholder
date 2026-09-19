<script lang="ts">
  import { onMount } from 'svelte'
  import { store, loadModel } from './lib/state.svelte'
  import { route, startRouter } from './lib/router.svelte'
  import TabBar from './lib/TabBar.svelte'
  import Dashboard from './lib/Dashboard.svelte'
  import Cashflow from './lib/Cashflow.svelte'
  import Portfolio from './lib/Portfolio.svelte'
  import Trades from './lib/Trades.svelte'
  import TradeDetail from './lib/TradeDetail.svelte'
  import Placeholder from './lib/Placeholder.svelte'

  onMount(() => {
    loadModel()
    const stopRouter = startRouter()
    const id = setInterval(loadModel, 30_000)
    return () => {
      stopRouter()
      clearInterval(id)
    }
  })
</script>

<main>
  <header>
    <span class="brand">Bagholder</span>
    <span class="v3">/v3 · Svelte</span>
  </header>

  <TabBar />

  <section class="page">
    {#if store.error}
      <p class="msg err">Could not load model: {store.error}</p>
    {:else if !store.model}
      <p class="msg">Loading…</p>
    {:else if route.tab === 'dashboard'}
      <Dashboard model={store.model} />
    {:else if route.tab === 'cashflow'}
      <Cashflow cashflow={store.model.cashflow} />
    {:else if route.tab === 'portfolio'}
      <Portfolio model={store.model} />
    {:else if route.tab === 'trades'}
      {#if route.sub}
        {@const sel = store.model.trades.find((t) => t.id === route.sub)}
        {#if sel}{#key sel.id}<TradeDetail trade={sel} />{/key}{:else}<Trades trades={store.model.trades} />{/if}
      {:else}
        <Trades trades={store.model.trades} />
      {/if}
    {:else}
      <Placeholder tab={route.tab} />
    {/if}
  </section>
</main>

<style>
  :global(body) { margin: 0; background: #0b0e14; color: #e6e9ef; font-family: system-ui, -apple-system, sans-serif; }
  main { max-width: 1200px; margin: 0 auto; padding: 20px 16px 60px; }
  header { display: flex; align-items: baseline; gap: 12px; margin-bottom: 14px; }
  .brand { font-size: 20px; font-weight: 700; }
  .v3 { color: #8b93a7; font-size: 12px; }
  .page { margin-top: 16px; }
  .msg { color: #8b93a7; }
  .err { color: #f0616d; }
</style>
