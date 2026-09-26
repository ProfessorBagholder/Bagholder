<script lang="ts">
  import { ICONS } from './icons'
  import { store } from './state.svelte'
  import { THEMES, theme, applyTheme } from './theme.svelte'
  import { ui, syncNow, refreshSession, connect, disconnect, openTradeModal, importCsv, openFolder, exportCsv, openData } from './ui.svelte'
  import { channel, notifyDead, notifyOn, notifyTest, notifyToggle } from './notes/channel.svelte'

  const status = $derived(store.model?.status)
  // A row's list opens under the pointer and on a tap (a touch has no hover); it closes when the
  // pointer leaves it, when a choice is made, or with the menu.
  let themeOpen = $state(false)
  let notifyOpen = $state(false)

  const NOTIFY_KINDS: [string, string][] = [
    ['fills', 'Fills'],
    ['problems', 'Order problems'],
    ['connection', 'Connection'],
    ['updates', 'Updates'],
  ]
  const NOTIFY_SCOPES: [string, string][] = [
    ['disclosuresHeld', 'Holdings'],
    ['disclosuresWatched', 'Watchlist'],
    ['disclosuresAll', 'All tickers'],
  ]
  const NOTIFY_RELEASE_SCOPES: [string, string][] = [
    ['releasesHeld', 'Holdings'],
    ['releasesWatched', 'Watchlist'],
    ['releasesAll', 'All tickers'],
  ]
  const notifyWord = $derived(
    channel() === 'unavailable' ? 'Unavailable' : channel() === 'denied' ? 'Blocked' : NOTIFY_KINDS.concat(NOTIFY_SCOPES, NOTIFY_RELEASE_SCOPES).some((k) => notifyOn(k[0])) ? 'On' : 'Off',
  )
  const themeWord = $derived((THEMES.find((t) => t[0] === theme.name) || THEMES[0])[1])
</script>

<div class="menu elev-md">
  {#if status?.connected}
    <button class="primary" onclick={syncNow}><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor" style="flex:none;opacity:1"><path d={ICONS.sync} /></svg>Sync now</button>
    <button onclick={refreshSession}><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor" style="flex:none;opacity:.7"><path d={ICONS.key} /></svg>Refresh session</button>
  {:else}
    <button class="primary" onclick={connect}><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor" style="flex:none;opacity:1"><path d={ICONS.link} /></svg>Connect Wealthsimple</button>
  {/if}
  <button onclick={openTradeModal}><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor" style="flex:none;opacity:.7"><path d={ICONS.plus} /></svg>Add trade</button>
  <button onclick={importCsv}><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor" style="flex:none;opacity:.7"><path d={ICONS.import} /></svg>Import CSV</button>
  <button onclick={openFolder}><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor" style="flex:none;opacity:.7"><path d={ICONS.folder} /></svg>Load folder</button>
  <button onclick={exportCsv}><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor" style="flex:none;opacity:.7"><path d={ICONS.export} /></svg>Export trades CSV</button>
  <button onclick={openData}><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor" style="flex:none;opacity:.7"><path d={ICONS.trash} /></svg>Clear data</button>

  <div class="sep"></div>
  <div style="position:relative" role="presentation" onmouseenter={() => (themeOpen = true)} onmouseleave={() => (themeOpen = false)}>
    <button class:open={themeOpen} onclick={() => { themeOpen = true; notifyOpen = false }}><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor" style="flex:none;opacity:.7"><path d={ICONS.palette} /></svg>Theme<span style="margin-left:auto;padding-left:28px;opacity:.6;box-sizing:border-box;width:90px;flex:none;text-align:right">{themeWord}</span><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor" style="flex:none;opacity:.5"><path d={ICONS.caret} /></svg></button>
    {#if themeOpen}
      <div class="menu sub">
        {#each THEMES as t (t[0])}
          <button class:primary={theme.name === t[0]} onclick={() => { applyTheme(t[0]); themeOpen = false }}>
            <span class="swatch" style="background:{t[2]}"></span>{t[1]}{#if theme.name === t[0]}<span style="margin-left:auto;padding-left:24px"><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor" style="flex:none;opacity:1"><path d={ICONS.check} /></svg></span>{/if}
          </button>
        {/each}
      </div>
    {/if}
  </div>
  <div style="position:relative" role="presentation" onmouseenter={() => (notifyOpen = true)} onmouseleave={() => (notifyOpen = false)}>
    <button class:open={notifyOpen} onclick={() => { notifyOpen = true; themeOpen = false }}><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor" style="flex:none;opacity:.7"><path d={ICONS.bell} /></svg>Notifications<span style="margin-left:auto;padding-left:28px;opacity:.6;box-sizing:border-box;width:90px;flex:none;text-align:right">{notifyWord}</span><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor" style="flex:none;opacity:.5"><path d={ICONS.caret} /></svg></button>
    {#if notifyOpen}
      <div class="menu sub">
        {#each NOTIFY_KINDS as k (k[0])}<button disabled={notifyDead()} onclick={() => notifyToggle(k[0])}>{k[1]}<span class="sw" class:on={notifyOn(k[0])} style="margin-left:auto"></span></button>{/each}
        <div class="sep"></div><div class="menu-h">Releases</div>
        {#each NOTIFY_RELEASE_SCOPES as k (k[0])}<button disabled={notifyDead()} onclick={() => notifyToggle(k[0])}>{k[1]}<span class="sw" class:on={notifyOn(k[0])} style="margin-left:auto"></span></button>{/each}
        <div class="sep"></div><div class="menu-h">Disclosures</div>
        {#each NOTIFY_SCOPES as k (k[0])}<button disabled={notifyDead()} onclick={() => notifyToggle(k[0])}>{k[1]}<span class="sw" class:on={notifyOn(k[0])} style="margin-left:auto"></span></button>{/each}
        <div class="sep"></div>
        <button disabled={notifyDead()} onclick={notifyTest}><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor" style="flex:none;opacity:.7"><path d={ICONS.bell} /></svg>Send a test notification</button>
      </div>
    {/if}
  </div>
  <div class="sep"></div>
  <button style={status?.connected || status?.email ? 'color:var(--neg)' : undefined} disabled={!status?.connected && !status?.email} onclick={disconnect}><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor" style="flex:none;opacity:.7"><path d={ICONS.signout} /></svg>Disconnect</button>
</div>
