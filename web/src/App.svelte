<script lang="ts">
  import { onMount } from 'svelte'
  import { store, loadModel } from './lib/state.svelte'
  import { route, startRouter } from './lib/router.svelte'
  import TabBar from './lib/TabBar.svelte'
  import Dashboard from './lib/Dashboard.svelte'
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
    {#if route.tab === 'dashboard'}
      {#if store.error}
        <p class="msg err">Could not load model: {store.error}</p>
      {:else if !store.model}
        <p class="msg">Loading…</p>
      {:else}
        <Dashboard model={store.model} />
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
