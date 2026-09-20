<script lang="ts">
  // The Clear data / Disconnect confirmation, ported from confirmDialogHtml
  // (the bracket:/cancel: variants live with the Orders panel).
  import { ui, clearDataNow, disconnectNow } from './ui.svelte'

  const clear = $derived(ui.confirm === 'clear')
</script>

<div id="confirmDlg" style="position:fixed;inset:0;z-index:20;background:rgba(8,9,16,.55);display:flex;align-items:center;justify-content:center;padding:20px">
  <div class="card elev-md" style="width:380px;max-width:100%;padding:18px 20px">
    <h5>{clear ? 'Clear data' : 'Disconnect'}</h5>
    <div class="dim" style="font-size:12.5px;margin:8px 0 16px;line-height:1.5">
      {clear
        ? 'Deletes everything synced from Wealthsimple, your journal and the downloaded market data from this machine. Your Wealthsimple login stays.'
        : 'Signs out of Wealthsimple on this machine. Your synced history stays.'}
    </div>
    <div style="display:flex;gap:8px;justify-content:flex-end">
      <button class="btn btn-secondary" onclick={() => (ui.confirm = '')}>Cancel</button>
      <button class="btn" style="background:rgba(224,119,138,.18);color:var(--neg)" onclick={() => (clear ? clearDataNow() : disconnectNow())}>{clear ? 'Clear data' : 'Disconnect'}</button>
    </div>
  </div>
</div>
