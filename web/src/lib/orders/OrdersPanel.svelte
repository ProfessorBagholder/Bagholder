<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import {
    ordersStore, panel, openOrders, closeOrders, tabCards, type Tab,
    listing, orderDetailLine, orderFillLine, orderValue, orderEndWord, orderUnconfirmed, SENT_WORD,
    orderWhenWord, legRow, bracketEditor, hasLimit,
    ordersScopeLabel,
    editOrder, cancelOrderEdit, orderEditSave, editBracket, cancelBracketEdit, bracketEditSave, bracketRemove,
    draftCard,
  } from './orders.svelte'
  import { draftStore, resumeDraft, discardDraft } from '../ticket/ticket.svelte'
  import { ui } from '../ui.svelte'
  import { money, px, qty as qtyFmt } from '../fmt'
  import { ICONS } from '../icons'
  import { focusOnMount } from '../actions/focus'

  // Enter in an editor's box saves it
  function saveOnEnter(e: KeyboardEvent, save: () => void) {
    if (e.key !== 'Enter') return
    e.preventDefault()
    save()
  }

  let { onclose }: { onclose: () => void } = $props()

  onMount(() => openOrders())
  onDestroy(() => closeOrders())

  const TABS: [Tab, string][] = [['pending', 'Pending'], ['filled', 'Filled'], ['cancelled', 'Cancelled']]
  const HEAD: Record<Tab, string> = { pending: 'Pending orders', filled: 'Filled orders', cancelled: 'Cancelled and rejected' }
  const EMPTY: Record<Tab, string> = { pending: 'No pending orders.', filled: 'Nothing filled yet.', cancelled: 'Nothing cancelled or rejected.' }

  const view = $derived.by(() => {
    const data = ordersStore.data
    if (ordersStore.error) return { state: 'error' as const, cards: [] }
    if (!data) return { state: 'loading' as const, cards: [] }
    return { state: 'ok' as const, cards: tabCards(data, panel.tab) }
  })

  const showDraft = $derived(panel.tab === 'pending' && !!draftStore.d)
  const draftInfo = $derived(draftStore.d ? draftCard(draftStore.d) : null)
  const empty = $derived(view.cards.length || showDraft ? '' : EMPTY[panel.tab])

  function resume() { onclose(); resumeDraft() }
</script>

<div id="odWrap">
  <div class="tk-scrim" role="presentation" onclick={onclose}></div>
  <div class="tk" role="dialog" aria-label="Orders">
    <div class="tk-hd">
      <span class="tk-title">Orders</span>
      <button class="tk-x" aria-label="Close" onclick={onclose}><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor"><path d={ICONS.x} /></svg></button>
    </div>
    <div class="tk-body" style="gap:16px">
      <div class="od-segs">
        {#each TABS as [id, label] (id)}
          <button class="od-seg" class:on={panel.tab === id} onclick={() => (panel.tab = id as typeof panel.tab)}>{label}</button>
        {/each}
      </div>
      <div id="odBody" class="od-list">
        {#if view.state === 'error'}
          <div class="status-err" style="font-size:12px">{ordersStore.error}</div>
        {:else if view.state === 'loading'}
          <div class="dim" style="font-size:12px"><span class="spin"></span>Loading…</div>
        {:else}
          <div class="od-head"><span class="od-group">{HEAD[panel.tab]}</span><span class="od-scope">{ordersScopeLabel()}</span></div>

          {#if showDraft && draftInfo}
            <div class="od-card draft">
              <div class="od-row1"><span class="od-title"><span class="od-draft">Draft</span> {listing(draftInfo.d)}</span><span class="od-value num">{draftInfo.value}</span></div>
              <div class="od-row2"><span class="od-line">{draftInfo.line}</span></div>
              {#each draftInfo.legs as leg}
                <div class="od-leg"><span class="od-leg-label {leg.tone}">{leg.label}</span><span class="od-leg-value num">{leg.value}</span><span class="od-leg-amt num {leg.tone}">{leg.amount}</span></div>
              {/each}
              <div class="od-foot"><span class="od-when">{orderWhenWord(draftInfo.d.at)}</span><span class="od-foot-acts"><button class="od-link draft" onclick={resume}>Resume</button><button class="od-link neg" onclick={discardDraft}>Discard</button></span></div>
            </div>
          {/if}

          {#each view.cards as card (card.kind + ':' + (card.kind === 'order' ? card.o.id : card.b.id))}
            {#if card.kind === 'order'}
              {@const o = card.o}
              {@const editing = panel.orderEdit?.id === o.id}
              {@const sent = orderUnconfirmed(o)}
              {@const ended = o.tab === 'cancelled' ? orderEndWord(o) : null}
              {@const fill = orderFillLine(o)}
              <div class="od-card">
                <div class="od-row1"><span class="od-title"><span style="color:{o.side === 'sell' ? 'var(--neg)' : 'var(--pos)'}">{o.side === 'sell' ? 'Sell' : 'Buy'}</span> {listing(o)}</span><span class="od-value num">{orderValue(o)}</span></div>
                <div class="od-row2"><span class="od-line">{orderDetailLine(o)}</span></div>
                {#if fill}<div class="od-fill">{fill}</div>{/if}
                {#each o.legs.map(legRow) as leg (leg.key)}
                  <div class="od-leg"><span class="od-leg-label {leg.tone}">{leg.label}</span><span class="od-leg-value num">{leg.line}</span>{#if leg.note}<span class="od-leg-state">{leg.note}</span>{/if}<span class="od-leg-amt num {leg.tone}">{leg.amount}</span></div>
                {/each}
                {#if editing && panel.orderEdit}
                  <div class="od-edit">
                    <div class="tk-grid">
                      <label class="tk-f"><span class="tk-l">Shares</span><input class="tk-in num" id="od-qty" use:focusOnMount onkeydown={(e) => saveOnEnter(e, () => orderEditSave(o.id))} inputmode="decimal" autocomplete="off" value={panel.orderEdit.qty ?? qtyFmt(o.quantity)} oninput={(e) => (panel.orderEdit!.qty = (e.target as HTMLInputElement).value)} /></label>
                      {#if hasLimit(o)}
                        <label class="tk-f"><span class="tk-l">Limit price</span><input class="tk-in num" id="od-limit" onkeydown={(e) => saveOnEnter(e, () => orderEditSave(o.id))} inputmode="decimal" autocomplete="off" value={panel.orderEdit.limit ?? px(o.limitPrice)} oninput={(e) => (panel.orderEdit!.limit = (e.target as HTMLInputElement).value)} /></label>
                      {:else}<div></div>{/if}
                    </div>
                    <div class="od-edit-btns"><button class="od-link muted" onclick={cancelOrderEdit}>Cancel</button><button class="tk-go od-save" disabled={panel.busy === 'orders'} onclick={() => orderEditSave(o.id)}>{panel.busy === 'orders' ? 'Saving…' : 'Save'}</button></div>
                    {#if panel.orderEdit.error}<div class="status-err" style="font-size:12px">{panel.orderEdit.error}</div>{/if}
                  </div>
                {/if}
                <div class="od-foot"><span class="od-when">{orderWhenWord(o.at)}</span>{#if sent}<span class="od-state">{SENT_WORD}</span>{:else if o.live && !editing}<span class="od-foot-acts"><button class="od-link" disabled={!o.editable} onclick={() => editOrder(o.id)}>Edit</button><button class="od-link neg" onclick={() => (ui.confirm = 'cancel:' + o.id)}>Cancel</button></span>{:else if ended && ended[0]}<span class="od-state" class:neg={ended[1]}>{ended[0]}</span>{/if}</div>
              </div>
            {:else}
              {@const b = card.b}
              {@const editing = panel.bracketEdit?.id === b.id}
              {@const ed = bracketEditor(b)}
              <div class="od-card">
                <div class="od-row1"><span class="od-title"><span style="color:var(--accent-300)">Bracket</span> {listing(b)}</span><span class="od-value num">{money(b.value, '', 2)}</span></div>
                {#each b.legs.map(legRow) as leg (leg.key)}
                  <div class="od-leg"><span class="od-leg-label {leg.tone}">{leg.label}</span><span class="od-leg-value num">{leg.line}</span>{#if leg.note}<span class="od-leg-state">{leg.note}</span>{/if}<span class="od-leg-amt num {leg.tone}">{leg.amount}</span></div>
                {/each}
                {#if editing && panel.bracketEdit}
                  <div class="od-edit">
                    <div class="tk-grid">
                      {#if ed.sl}
                        <div class="tk-f"><span class="tk-l">{ed.sl.label}</span><input class="tk-in num" id="od-sl" use:focusOnMount onkeydown={(e) => saveOnEnter(e, () => bracketEditSave(b.id))} inputmode="decimal" autocomplete="off" value={panel.bracketEdit.sl ?? ed.sl.start} oninput={(e) => (panel.bracketEdit!.sl = (e.target as HTMLInputElement).value)} /><button class="od-link neg od-remove" onclick={() => bracketRemove(b.id, 'sl')}>Remove stop loss</button></div>
                      {:else}<div></div>{/if}
                      {#if ed.tp}
                        <div class="tk-f"><span class="tk-l">Limit price</span><input class="tk-in num" id="od-tp" onkeydown={(e) => saveOnEnter(e, () => bracketEditSave(b.id))} inputmode="decimal" autocomplete="off" value={panel.bracketEdit.tp ?? ed.tp.start} oninput={(e) => (panel.bracketEdit!.tp = (e.target as HTMLInputElement).value)} /><button class="od-link neg od-remove" onclick={() => bracketRemove(b.id, 'tp')}>Remove take profit</button></div>
                      {:else}<div></div>{/if}
                    </div>
                    <div class="od-edit-btns"><button class="od-link muted" onclick={cancelBracketEdit}>Cancel</button><button class="tk-go od-save" disabled={panel.busy === 'orders'} onclick={() => bracketEditSave(b.id)}>{panel.busy === 'orders' ? 'Saving…' : 'Save'}</button></div>
                    {#if panel.bracketEdit.error}<div class="status-err" style="font-size:12px">{panel.bracketEdit.error}</div>{/if}
                  </div>
                {/if}
                <div class="od-foot"><span class="od-when">{orderWhenWord(b.at)}</span>{#if b.live && !editing}<span class="od-foot-acts"><button class="od-link" onclick={() => editBracket(b.id)}>Edit</button><button class="od-link neg" onclick={() => (ui.confirm = 'bracket:' + b.id)}>Cancel</button></span>{:else if b.tab === 'cancelled' && b.endWord}<span class="od-state">{b.endWord}</span>{/if}</div>
              </div>
            {/if}
          {/each}

          {#if empty}<div class="dim" style="font-size:12px">{empty}</div>{/if}
        {/if}
      </div>
    </div>
  </div>
</div>
