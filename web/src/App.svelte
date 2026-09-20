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
  import Markets from './lib/Markets.svelte'
  import Placeholder from './lib/Placeholder.svelte'
  import FilterPopover from './lib/FilterPopover.svelte'
  import CommandPalette from './lib/CommandPalette.svelte'
  import OrderTicket from './lib/ticket/OrderTicket.svelte'
  import { ticketStore } from './lib/ticket/ticket.svelte'
  import { activeCount } from './lib/filters.svelte'

  let filterOpen = $state(false)
  let paletteOpen = $state(false)

  function onGlobalKey(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
      e.preventDefault()
      paletteOpen = true
    }
  }

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

<svelte:window onkeydown={onGlobalKey} />

<main>
  <header>
    <span class="brand">Bagholder</span>
    <span class="v3">/v3 · Svelte</span>
    <button class="search" onclick={() => (paletteOpen = true)} aria-label="Search">Search <kbd>⌘K</kbd></button>
    <button class="filter" class:on={activeCount() > 0} onclick={() => (filterOpen = true)} aria-label="Filters">
      ⚲ Filter{#if activeCount()}<span class="fcount">{activeCount()}</span>{/if}
    </button>
  </header>

  {#if filterOpen && store.model}
    <FilterPopover options={store.model.options} onclose={() => (filterOpen = false)} />
  {/if}
  {#if paletteOpen}
    <CommandPalette onclose={() => (paletteOpen = false)} />
  {/if}
  {#if ticketStore.t}
    <OrderTicket />
  {/if}

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
    {:else if route.tab === 'markets'}
      <Markets markets={store.model.markets} />
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
  .search { margin-left: auto; background: #141924; border: 1px solid #1c2230; color: #8b93a7; font: inherit; font-size: 12px; padding: 5px 12px; border-radius: 8px; cursor: pointer; display: inline-flex; align-items: center; gap: 6px; }
  .search:hover { border-color: #2a3242; color: #c4cbd8; }
  .search kbd { background: #1c2230; border-radius: 4px; padding: 1px 5px; font-size: 10px; font-family: inherit; }
  .filter { background: #141924; border: 1px solid #1c2230; color: #c4cbd8; font: inherit; font-size: 12px; padding: 5px 12px; border-radius: 8px; cursor: pointer; display: inline-flex; align-items: center; gap: 6px; }
  .filter:hover { border-color: #2a3242; }
  .filter.on { border-color: #3ecf8e; color: #3ecf8e; }
  .fcount { background: #3ecf8e; color: #08110b; border-radius: 10px; padding: 0 6px; font-size: 11px; }
  .page { margin-top: 16px; }
  .msg { color: #8b93a7; }
  .err { color: #f0616d; }
</style>
