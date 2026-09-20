<script lang="ts">
  import { onMount } from 'svelte'
  import { store, loadModel } from './lib/state.svelte'
  import { route, startRouter, go, TABS, TAB_LABEL, type Tab } from './lib/router.svelte'
  import { ICONS } from './lib/icons'
  import { symText } from './lib/sym'
  import { relTime } from './lib/fmt'
  import { activeCount, chips, clearField } from './lib/filters.svelte'
  import Dashboard from './lib/Dashboard.svelte'
  import Cashflow from './lib/Cashflow.svelte'
  import Portfolio from './lib/Portfolio.svelte'
  import Trades from './lib/Trades.svelte'
  import TradeDetail from './lib/TradeDetail.svelte'
  import Markets from './lib/Markets.svelte'
  import Placeholder from './lib/Placeholder.svelte'
  import FilterPopover from './lib/FilterPopover.svelte'
  import Menu from './lib/Menu.svelte'
  import ConfirmDialog from './lib/ConfirmDialog.svelte'
  import LoginView from './lib/LoginView.svelte'
  import { cancelConnect, loginInput } from './lib/ui.svelte'
  import Modals from './lib/Modals.svelte'
  import { ui } from './lib/ui.svelte'
  import { resetFilters } from './lib/filters.svelte'
  import OrderTicket from './lib/ticket/OrderTicket.svelte'
  import { ticketStore, closeTicket } from './lib/ticket/ticket.svelte'
  import OrdersPanel from './lib/orders/OrdersPanel.svelte'
  import { panel as ordersPanel } from './lib/orders/orders.svelte'
  import NotesPanel from './lib/notes/NotesPanel.svelte'
  import { notesStore, startNotesStream } from './lib/notes/notes.svelte'

  let filterOpen = $state(false)
  let filterField = $state<string | undefined>(undefined)

  function editChip(key: string) {
    filterField = key === 'search' ? 'fields' : key
    filterOpen = true
  }
  function removeChip(key: string) {
    clearField(key)
    loadModel()
  }
  let ordersOpen = $state(false)
  let notesOpen = $state(false)
  let menuWrap = $state<HTMLElement>()

  // Close the header menu on any pointerdown outside it (the original's data-pop
  // outside-click, without a click-blocking scrim over the menu).
  function onDocPointerDown(e: PointerEvent) {
    if (ui.menuOpen && menuWrap && !menuWrap.contains(e.target as Node)) ui.menuOpen = false
  }

  // Present a held position as a trade for the shared detail view (holdingAsTrade).
  function holdingAsTrade(p: any) {
    if (!p) return null
    return { ...p, holding: true, pnl: p.unreal, pnlPct: p.unrealPct, entryDate: p.opened, exitDate: '', entry: p.avg, exit: p.last, holdDays: p.held, status: 'open', legs: [] }
  }

  const isFieldFocused = () => {
    const el = document.activeElement
    return !!el && ['INPUT', 'TEXTAREA', 'SELECT'].includes(el.tagName)
  }

  // The global keyboard shortcuts, ported from ledger.html's document keydown:
  // ⌘/Ctrl+O toggles Orders, ⌘/Ctrl+K opens the filter popover, ←/→ move between
  // tabs, and Escape unwinds whatever is open (ticket → notes → orders → menu →
  // filter → modal → confirm → back out of a trade → clear filters).
  const LOGIN_KEYS = ['Enter', 'Tab', 'Backspace', 'Delete', 'Escape', 'ArrowLeft', 'ArrowUp', 'ArrowRight', 'ArrowDown', 'Home', 'End']
  function onKey(e: KeyboardEvent) {
    if (ui.loginView) {
      // forward keystrokes to the streamed sign-in browser (ledger loginKey)
      if (e.metaKey || e.ctrlKey) return // paste arrives as its own event
      if (e.key.length === 1) { e.preventDefault(); loginInput({ kind: 'text', text: e.key }) }
      else if (LOGIN_KEYS.includes(e.key)) { e.preventDefault(); loginInput({ kind: 'key', key: e.key }) }
      return
    }
    if (ticketStore.t) {
      if (e.key === 'Escape') { e.preventDefault(); closeTicket(); return }
      const t = e.target as HTMLElement | null
      if (e.key === 'Enter' && t && /^tk-/.test(t.id) && t.tagName === 'INPUT') { e.preventDefault(); t.blur() }
      return
    }
    const mod = e.metaKey || e.ctrlKey
    if (mod && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'o') {
      e.preventDefault()
      ordersOpen = !ordersOpen
      return
    }
    if (notesOpen && !ui.confirm && !filterOpen) {
      if (e.key === 'Escape') { e.preventDefault(); notesOpen = false; return }
    }
    if (ordersOpen && !ui.confirm && !filterOpen) {
      // Esc closes an open editor first, then the panel; ←/→ move between tabs
      if (e.key === 'Escape') {
        e.preventDefault()
        if (ordersPanel.orderEdit || ordersPanel.bracketEdit) {
          ordersPanel.orderEdit = null
          ordersPanel.bracketEdit = null
        } else ordersOpen = false
        return
      }
      if ((e.key === 'ArrowLeft' || e.key === 'ArrowRight') && !mod && !e.altKey && !isFieldFocused()) {
        const tabs = ['pending', 'filled', 'cancelled'] as const
        const j = tabs.indexOf(ordersPanel.tab) + (e.key === 'ArrowRight' ? 1 : -1)
        if (j >= 0 && j < tabs.length) {
          e.preventDefault()
          ordersPanel.tab = tabs[j]
          ordersPanel.orderEdit = null
          ordersPanel.bracketEdit = null
        }
      }
      return
    }
    if (mod && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'k') {
      e.preventDefault()
      ui.menuOpen = false
      filterField = 'fields'
      filterOpen = true
      return
    }
    if ((e.key === 'ArrowLeft' || e.key === 'ArrowRight') && !mod && !e.altKey && !filterOpen && !ui.menuOpen && !ui.modal && !ui.confirm && !isFieldFocused()) {
      const j = TABS.indexOf(route.tab) + (e.key === 'ArrowRight' ? 1 : -1)
      if (j >= 0 && j < TABS.length) { e.preventDefault(); go(TABS[j]) }
      return
    }
    if (e.key === 'Escape') {
      if (ui.confirm) { ui.confirm = ''; return }
      if (ui.modal) { ui.modal = ''; return }
      if (filterOpen || ui.menuOpen) { filterOpen = false; filterField = undefined; ui.menuOpen = false; return }
      const el = document.activeElement as HTMLElement | null
      if (route.sub && route.tab === 'trades' && el && el.tagName !== 'TEXTAREA' && el.id !== 'tagInput') { history.back(); return }
      if (activeCount() > 0 && !isFieldFocused()) { resetFilters(); loadModel(); return }
    }
  }

  const status = $derived(store.model?.status ?? null)
  const DETAIL_PAGES: Tab[] = ['trades', 'portfolio', 'markets']

  const sel = $derived(
    DETAIL_PAGES.includes(route.tab) && route.sub && store.model
      ? store.model.trades.find((t) => t.id === route.sub) ??
          store.model.positions?.find((p) => p.id === route.sub) ??
          null
      : null,
  )

  function syncLine(): string {
    const s = status
    if (!s) return ''
    if (s.syncing) return s.syncStep || 'Syncing…'
    if (s.error) return s.error.length > 60 ? s.error.slice(0, 57) + '…' : s.error
    if (!s.connected) return 'Not connected'
    return 'Synced ' + (relTime(s.lastSync) || '—')
  }

  const notesUnread = $derived(notesStore.unread || status?.notify?.unread || 0)

  // The 2px bar under the active tab slides and resizes rather than jumping,
  // driven by the active button's own measurements (placeTabIndicator).
  function tabIndicator(bar: HTMLElement) {
    const place = () => {
      const parent = bar.parentElement
      const on = parent?.querySelector('.tabbtn.on') as HTMLElement | null
      if (!on) {
        bar.style.opacity = '0'
        return
      }
      bar.style.left = on.offsetLeft + 'px'
      bar.style.width = on.offsetWidth + 'px'
      bar.style.opacity = '1'
    }
    place()
    const ro = new ResizeObserver(place)
    ro.observe(bar.parentElement!)
    if (document.fonts?.ready) document.fonts.ready.then(place)
    const mo = new MutationObserver(place)
    mo.observe(bar.parentElement!, { attributes: true, subtree: true, attributeFilter: ['class'] })
    window.addEventListener('resize', place)
    return {
      update: place,
      destroy() {
        ro.disconnect()
        mo.disconnect()
        window.removeEventListener('resize', place)
      },
    }
  }

  onMount(() => {
    loadModel()
    const stopRouter = startRouter()
    const stopNotes = startNotesStream()
    const id = setInterval(loadModel, 30_000)
    return () => {
      stopRouter()
      stopNotes()
      clearInterval(id)
    }
  })

  // Selecting a trade or holding (or leaving one) changes the &trade= detail the
  // model is fetched with — its fills/legs travel only for the open item — so
  // reload when the drill-down id changes. (Runs once on mount too; harmless.)
  let lastSub: string | null = null
  $effect(() => {
    const sub = route.sub
    if (sub !== lastSub) {
      lastSub = sub
      loadModel()
    }
  })
</script>

<svelte:window onkeydown={onKey} onpointerdown={onDocPointerDown} />

{#if store.model}
  <!-- header -->
  <div id="hdr" style="display:flex;align-items:center;gap:22px;padding:12px 20px;background:var(--bg);box-shadow:inset 0 -1px 0 rgba(var(--ink-rgb),.08)">
    <div style="display:flex;align-items:center;gap:9px;margin-right:8px">
      <img src="/favicon.png" alt="" style="width:24px;height:24px;border-radius:6px" />
      <span style="font-size:15px;font-weight:600;letter-spacing:var(--brand-spacing);color:var(--brand-color);text-transform:var(--brand-transform)">Bagholder</span>
      {#if status?.version}<span class="muted" style="font-size:11px;margin-left:8px">v{status.version}</span>{/if}
    </div>
    <div style="margin-left:auto;display:flex;align-items:center;gap:12px">
      <span style="font-size:12px;color:var(--ink55)">
        {#if ui.connecting}<span class="spin"></span>Waiting for Wealthsimple login… <button class="pill" style="padding:1px 8px;font-size:11px;width:auto;margin-left:6px" onclick={cancelConnect}>Cancel</button>
        {:else if ui.notice}<span class={ui.noticeKind === 'err' ? 'status-err' : ''}>{ui.notice}</span>
        {:else}{syncLine()}{/if}
      </span>
      <button class="btn btn-icon btn-secondary" aria-label="Orders" style="position:relative" onclick={() => (ordersOpen = true)}>
        <svg width="16" height="16" viewBox="0 0 256 256" fill="currentColor"><path d={ICONS.receipt} /></svg>
        {#if status?.openOrders}<span class="od-badge quiet">{status.openOrders}</span>{/if}
      </button>
      <button class="btn btn-icon btn-secondary" aria-label="Notifications" style="position:relative" onclick={() => (notesOpen = true)}>
        <svg width="16" height="16" viewBox="0 0 256 256" fill="currentColor"><path d={ICONS.bell} /></svg>
        {#if notesUnread}<span class="od-badge">{notesUnread}</span>{/if}
      </button>
      <div style="position:relative">
        <button class="btn btn-icon btn-secondary" aria-label="Filters" onclick={() => (filterOpen = !filterOpen)}>
          <svg width="15" height="15" viewBox="0 0 256 256" fill="currentColor"><path d={ICONS.funnel} /></svg>
        </button>
        {#if activeCount() > 0}<span style="position:absolute;top:-1px;right:-1px;width:7px;height:7px;border-radius:50%;background:var(--accent);box-shadow:0 0 0 2px var(--bg);pointer-events:none"></span>{/if}
        {#if filterOpen}<FilterPopover options={store.model.options} field={filterField} onclose={() => { filterOpen = false; filterField = undefined }} />{/if}
      </div>
      <div style="position:relative" bind:this={menuWrap}>
        <button class="btn btn-icon btn-secondary" aria-label="Menu" onclick={() => (ui.menuOpen = !ui.menuOpen)}>
          <svg width="16" height="16" viewBox="0 0 256 256" fill="currentColor"><path d={ICONS.menu} /></svg>
        </button>
        {#if ui.menuOpen}<Menu />{/if}
      </div>
    </div>
  </div>

  <!-- tabs -->
  <div class="tabbar" style="display:flex;gap:22px;padding:0 20px;background:var(--bg);box-shadow:inset 0 -1px 0 rgba(var(--ink-rgb),.08)">
    <div class="tabind" use:tabIndicator></div>
    {#each TABS as tab (tab)}
      <button
        class="tabbtn"
        class:on={route.tab === tab && !(DETAIL_PAGES.includes(tab) && sel)}
        onclick={() => go(tab)}>{TAB_LABEL[tab]}</button>
    {/each}
    {#if sel}
      <div style="display:flex;align-items:center;gap:6px;flex:none;min-width:0;white-space:nowrap;margin-left:6px">
        <span style="font-size:13px;color:rgba(var(--ink-rgb),.4)">›</span>
        <span style="font:500 13px Inter,system-ui;padding:11px 0;max-width:220px;overflow:hidden;text-overflow:ellipsis;color:var(--accent-300);box-shadow:inset 0 -2px 0 var(--accent)">{symText(sel.symbol)}</span>
      </div>
    {/if}
    <div style="margin-left:auto;min-width:0;display:flex;align-items:center;gap:7px;padding:6px 0">
      <div style="flex:1;min-width:0;display:flex;align-items:center;gap:7px;overflow-x:auto;padding-bottom:1px">
        {#each chips() as c (c.key)}
          <span class="chip"><span class="cf">{c.field}</span><button class="cv" onclick={() => editChip(c.key)}>{c.value}</button><button class="cx" aria-label="Remove filter" onclick={() => removeChip(c.key)}>×</button></span>
        {/each}
      </div>
    </div>
  </div>

  <!-- page -->
  <div id="page">
    {#if route.tab === 'dashboard'}
      <Dashboard model={store.model} />
    {:else if route.tab === 'cashflow'}
      <Cashflow model={store.model} />
    {:else if route.tab === 'portfolio'}
      {#if sel}{#key sel.id}<TradeDetail trade={holdingAsTrade(sel) as import('./lib/model').Trade} />{/key}{:else}<Portfolio model={store.model} />{/if}
    {:else if route.tab === 'trades'}
      {#if sel}{#key sel.id}<TradeDetail trade={sel as import('./lib/model').Trade} />{/key}{:else}<Trades trades={store.model.trades} />{/if}
    {:else if route.tab === 'markets'}
      <Markets markets={store.model.markets} />
    {:else}
      <Placeholder tab={route.tab} />
    {/if}
  </div>

  {#if ticketStore.t}<OrderTicket />{/if}
  {#if ordersOpen}<OrdersPanel onclose={() => (ordersOpen = false)} />{/if}
  {#if notesOpen}<NotesPanel onclose={() => (notesOpen = false)} />{/if}
  {#if ui.loginView}<LoginView />{/if}
  {#if ui.confirm}<ConfirmDialog />{/if}
  {#if ui.modal}<Modals />{/if}
{:else if store.error}
  <div class="empty muted" style="padding:80px 20px">Could not load model: {store.error}</div>
{:else}
  <div class="empty muted" style="padding:80px 20px"><span class="spin"></span>Loading…</div>
{/if}
