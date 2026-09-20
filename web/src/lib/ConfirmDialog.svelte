<script lang="ts">
  // The confirmation dialog, ported from confirmDialogHtml: Clear data / Disconnect,
  // and the order/bracket cancel variants (confirm = `cancel:<id>` / `bracket:<id>`).
  import { ui, clearDataNow, disconnectNow } from './ui.svelte'
  import { ordersStore, orderLine, bracketLegs, cancelOrderNow, cancelBracketNow } from './orders/orders.svelte'
  import { symText } from './sym'

  const cancelId = $derived(ui.confirm.startsWith('cancel:') ? ui.confirm.slice(7) : '')
  const bracketId = $derived(ui.confirm.startsWith('bracket:') ? ui.confirm.slice(8) : '')
  const order = $derived(cancelId ? ordersStore.data?.orders?.find((o) => o.id === cancelId) : null)
  const bracket = $derived(bracketId ? ordersStore.data?.brackets?.find((b) => b.id === bracketId) : null)
  const clear = $derived(ui.confirm === 'clear')

  function title(): string {
    if (bracketId) return 'Cancel bracket'
    if (cancelId) return 'Cancel order'
    return clear ? 'Clear data' : 'Disconnect'
  }
  function body(): string {
    if (bracketId) return bracket ? symText(bracket.symbol) + ' · ' + bracketLegs(bracket).map((l) => (l.label + ' ' + l.line).trim()).join(' · ') : ''
    if (cancelId) return order ? orderLine(order) : ''
    return clear
      ? 'Deletes everything synced from Wealthsimple, your journal and the downloaded market data from this machine. Your Wealthsimple login stays.'
      : 'Signs out of Wealthsimple on this machine. Your synced history stays.'
  }
  function keepLabel(): string {
    return bracketId ? 'Keep bracket' : cancelId ? 'Keep order' : 'Cancel'
  }
  function goLabel(): string {
    if (bracketId) return 'Cancel bracket'
    if (cancelId) return 'Cancel order'
    return clear ? 'Clear data' : 'Disconnect'
  }
  function go() {
    const oid = cancelId
    const bid = bracketId
    const wasClear = clear
    ui.confirm = ''
    if (bid) cancelBracketNow(bid)
    else if (oid) cancelOrderNow(oid)
    else if (wasClear) clearDataNow()
    else disconnectNow()
  }
</script>

<div id="confirmDlg" style="position:fixed;inset:0;z-index:20;background:rgba(8,9,16,.55);display:flex;align-items:center;justify-content:center;padding:20px">
  <div class="card elev-md" style="width:380px;max-width:100%;padding:18px 20px">
    <h5>{title()}</h5>
    <div class="dim" style="font-size:12.5px;margin:8px 0 16px;line-height:1.5">{body()}</div>
    <div style="display:flex;gap:8px;justify-content:flex-end">
      <button class="btn btn-secondary" onclick={() => (ui.confirm = '')}>{keepLabel()}</button>
      <button class="btn" style="background:rgba(224,119,138,.18);color:var(--neg)" onclick={go}>{goLabel()}</button>
    </div>
  </div>
</div>
