<script lang="ts">
  // The in-app Wealthsimple sign-in window (loginViewHtml): Wealthsimple's page,
  // streamed as frames from the app's headless browser, with clicks, the wheel,
  // keystrokes and paste sent back to it. Ported from ledger.html.
  import { onMount } from 'svelte'
  import { cancelConnect, loginInput } from './ui.svelte'

  let img = $state<HTMLImageElement>()

  onMount(() => {
    if (img && !img.getAttribute('src')) img.src = '/api/login/stream?t=' + Date.now()
  })

  function point(e: MouseEvent | WheelEvent): { x: number; y: number } | null {
    if (!img || !img.naturalWidth) return null
    const r = img.getBoundingClientRect()
    return { x: ((e.clientX - r.left) * img.naturalWidth) / r.width, y: ((e.clientY - r.top) * img.naturalHeight) / r.height }
  }
  function onClick(e: MouseEvent) {
    const p = point(e)
    if (p) loginInput({ kind: 'click', x: p.x, y: p.y })
  }
  function onWheel(e: WheelEvent) {
    const p = point(e)
    if (!p) return
    e.preventDefault()
    loginInput({ kind: 'wheel', x: p.x, y: p.y, deltaY: e.deltaY })
  }
  function onPaste(e: ClipboardEvent) {
    const text = e.clipboardData?.getData('text') || ''
    if (text) loginInput({ kind: 'text', text })
    e.preventDefault()
  }
</script>

<svelte:window onpaste={onPaste} />

<div id="loginDlg" style="position:fixed;inset:0;z-index:20;background:rgba(8,9,16,.55);display:flex;align-items:center;justify-content:center;padding:20px">
  <div class="card elev-md" style="max-width:100%;padding:14px 16px 12px;display:flex;flex-direction:column;gap:10px">
    <div style="display:flex;align-items:center;justify-content:space-between;gap:16px">
      <h5>Sign in to Wealthsimple</h5>
      <button class="btn btn-secondary" onclick={cancelConnect}>Cancel</button>
    </div>
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions -->
    <img
      bind:this={img}
      alt=""
      draggable="false"
      onclick={onClick}
      onwheel={onWheel}
      style="display:block;width:auto;height:auto;max-width:100%;max-height:calc(100vh - 120px);min-width:320px;min-height:200px;background:#fff;border-radius:6px;cursor:default;user-select:none" />
  </div>
</div>
