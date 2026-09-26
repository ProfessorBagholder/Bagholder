<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import {
    ordersStore, panel, openOrders, closeOrders,
    ORDER_LIVE, BRACKET_LIVE, orderById, bracketOf, bracketLegs,
    orderTitle, orderDetailLine, orderFillLine, orderValue, orderPill,
    orderWhenWord, orderMultiplier, bracketEndWord, bracketExited,
    inOrdersScope, ordersScopeLabel,
    editOrder, cancelOrderEdit, orderEditSave, editBracket, cancelBracketEdit, bracketEditSave, bracketRemove,
    cancelOrderNow, cancelBracketNow,
    type Order, type Bracket,
  } from './orders.svelte'
  import { draftStore, resumeDraft, discardDraft } from '../ticket/ticket.svelte'
  import { ui } from '../ui.svelte'
  import { plain } from '../ticket/vals'
  import { px, money, qty as qtyFmt } from '../fmt'
  import { symText } from '../sym'
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

  const TABS: [string, string][] = [['pending', 'Pending'], ['filled', 'Filled'], ['cancelled', 'Cancelled']]

  interface Card { kind: 'order' | 'bracket'; at: string; o?: Order; b?: Bracket }

  const view = $derived.by(() => {
    const data = ordersStore.data
    if (!data && ordersStore.error) return { state: 'error' as const, head: '', cards: [] as Card[], empty: '' }
    if (!data) return { state: 'loading' as const, head: '', cards: [] as Card[], empty: '' }
    const entries = data.orders.filter((o) => o.role !== 'stop' && o.role !== 'target' && inOrdersScope(o.accountId))
    const tab = panel.tab
    const endedBrackets = (data.brackets || []).filter((b) => b.armedAt && !BRACKET_LIVE[b.status] && (() => { const e = orderById(b.orderId); return !e || inOrdersScope(e.accountId) })())
    const mixed = (rows: Order[], brackets: Bracket[]): Card[] =>
      (rows.map((o) => ({ kind: 'order' as const, at: o.createdAt, o })) as Card[])
        .concat(brackets.map((b) => ({ kind: 'bracket' as const, at: (b.updatedAt || b.armedAt) as string, b })))
        .sort((a, c) => (a.at < c.at ? 1 : a.at > c.at ? -1 : 0))
    if (tab === 'filled') {
      const rows = entries.filter((o) => o.status === 'filled')
      const exits = endedBrackets.filter(bracketExited)
      return { state: 'ok' as const, head: 'Filled orders', cards: mixed(rows, exits), empty: rows.length || exits.length ? '' : 'Nothing filled yet.' }
    }
    if (tab === 'cancelled') {
      const rows = entries.filter((o) => ['cancelled', 'expired', 'rejected', 'failed', 'dry'].indexOf(o.status) >= 0)
      const offs = endedBrackets.filter((b) => !bracketExited(b))
      return { state: 'ok' as const, head: 'Cancelled and rejected', cards: mixed(rows, offs), empty: rows.length || offs.length ? '' : 'Nothing cancelled or rejected.' }
    }
    // an order being sent, or left being sent when the app stopped, is pending until Wealthsimple is read
    const cards = (entries.filter((o) => ORDER_LIVE[o.status] || o.status === 'sending').map((o) => ({ kind: 'order' as const, at: o.createdAt, o })) as Card[])
      .concat((data.brackets || []).filter((b) => BRACKET_LIVE[b.status] && b.status !== 'waiting' && (() => { const e = orderById(b.orderId); return !e || inOrdersScope(e.accountId) })()).map((b) => ({ kind: 'bracket' as const, at: (b.armedAt || b.createdAt) as string, b })))
      .sort((a, c) => (a.at < c.at ? 1 : a.at > c.at ? -1 : 0))
    return { state: 'ok' as const, head: 'Pending orders', cards, empty: cards.length ? '' : draftStore.d ? '' : 'No pending orders.' }
  })

  const showDraft = $derived(panel.tab === 'pending' && !!draftStore.d)

  // draft card figures (draftCardHtml)
  const draftInfo = $derived.by(() => {
    const d = draftStore.d
    if (!d) return null
    const buy = d.side !== 'SELL', qtyN = d.qty || 0
    const entry = d.type === 'MARKET' ? null : d.type === 'STOP' ? d.stop : d.limit
    const typeWord = ({ MARKET: 'Market', LIMIT: 'limit', STOP: 'stop', STOP_LIMIT: 'stop limit' } as Record<string, string>)[d.type] || 'limit'
    const line = (buy ? 'Buy ' : 'Sell ') + qtyFmt(qtyN) + (entry != null ? ' at ' + px(entry) + ' ' + typeWord : ' ' + (d.type === 'MARKET' ? 'market' : typeWord)) + (d.type === 'MARKET' ? '' : ' · ' + (d.tif === 'DAY' ? 'Day' : 'GTC'))
    const legs: { label: string; tone: string; value: string; amount: string }[] = []
    const addLeg = (label: string, tone: string, price: number | null, note: string) => {
      if (price == null && !note) return
      legs.push({ label, tone, value: price != null ? qtyFmt(qtyN) + ' at ' + px(price) : note, amount: price != null ? money(qtyN * price, '', 2) : '' })
    }
    if (buy && d.sl && d.sl.on) {
      const sl = d.sl
      if (sl.kind === 'trail') addLeg('Stop loss', 'neg', null, 'trailing ' + (sl.trail != null ? (sl.unit === 'pct' ? plain(sl.trail) + '%' : px(sl.trail)) : '5%'))
      else if (sl.priceUnit === 'pct' ? sl.pct != null && entry != null : sl.price != null) addLeg('Stop loss', 'neg', sl.priceUnit === 'pct' ? +((entry as number) * (1 - (sl.pct as number) / 100)).toFixed(2) : sl.price, '')
    }
    if (buy && d.tp && d.tp.on) {
      const tp = d.tp
      if (tp.unit === 'pct' ? tp.pct != null && entry != null : tp.price != null) addLeg('Take profit', 'pos', tp.unit === 'pct' ? +((entry as number) * (1 + (tp.pct as number) / 100)).toFixed(2) : tp.price, '')
    }
    return { d, buy, qtyN, entry, line, legs, value: entry != null ? money(qtyN * entry, '', 2) : '' }
  })

  function resume() { onclose(); resumeDraft() }

  // per-order derived helpers
  const orderLive = (o: Order) => !!ORDER_LIVE[o.status] && o.status !== 'cancelling'
  const orderCanEdit = (o: Order) => orderLive(o) && o.type !== 'STOP'
  const orderHasLimit = (o: Order) => o.type === 'LIMIT' || o.type === 'STOP_LIMIT'
  const waitingLegs = (o: Order) => { const w = orderLive(o) ? bracketOf(o) : null; return w && w.status === 'waiting' ? bracketLegs(w) : [] }

  const bracketTitle = (b: Bracket) => { const e = orderById(b.orderId); return (e && e.exchange ? e.exchange + ': ' : '') + symText(b.symbol) }
  const bracketValue = (b: Bracket) => { const e = orderById(b.orderId); return e && e.avgFill ? money((b.quantity ?? 0) * e.avgFill * orderMultiplier(e), '', 2) : '' }
  const bracketLive = (b: Bracket) => !!BRACKET_LIVE[b.status]
  const bracketWhen = (b: Bracket) => orderWhenWord(bracketLive(b) ? (b.armedAt || b.createdAt) : (b.updatedAt || b.armedAt || b.createdAt))
  const bracketEditData = (b: Bracket) => {
    const isTrail = b.slKind === 'trail'
    const slLabel = isTrail ? (b.slTrailUnit === 'amt' ? 'Trail' : 'Trail %') : 'Stop price'
    const slCur = isTrail ? (b.slTrailUnit === 'amt' ? px(b.slTrail) : plain(b.slTrail)) : px(b.slPrice)
    return { slLabel, slCur }
  }
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
          <div class="od-head"><span class="od-group">{view.head}</span><span class="od-scope">{ordersScopeLabel()}</span></div>

          {#if showDraft && draftInfo}
            <div class="od-card draft">
              <div class="od-row1"><span class="od-title"><span class="od-draft">Draft</span> {(draftInfo.d.exchange ? draftInfo.d.exchange + ': ' : '') + symText(draftInfo.d.symbol)}</span><span class="od-value num">{draftInfo.value}</span></div>
              <div class="od-row2"><span class="od-line">{draftInfo.line}</span></div>
              {#each draftInfo.legs as leg}
                <div class="od-leg"><span class="od-leg-label {leg.tone}">{leg.label}</span><span class="od-leg-value num">{leg.value}</span><span class="od-leg-amt num {leg.tone}">{leg.amount}</span></div>
              {/each}
              <div class="od-foot"><span class="od-when">{orderWhenWord(draftInfo.d.at)}</span><span class="od-foot-acts"><button class="od-link draft" onclick={resume}>Resume</button><button class="od-link neg" onclick={discardDraft}>Discard</button></span></div>
            </div>
          {/if}

          {#each view.cards as card (card.kind + ':' + (card.o?.id ?? card.b?.id))}
            {#if card.kind === 'order' && card.o}
              {@const o = card.o}
              {@const editing = panel.orderEdit?.id === o.id}
              {@const live = orderLive(o)}
              {@const ended = panel.tab === 'cancelled' ? orderPill(o) : null}
              {@const fill = orderFillLine(o)}
              <div class="od-card">
                <div class="od-row1"><span class="od-title"><span style="color:{o.side === 'SELL' ? 'var(--neg)' : 'var(--pos)'}">{o.side === 'SELL' ? 'Sell' : 'Buy'}</span> {orderTitle(o)}</span><span class="od-value num">{orderValue(o)}</span></div>
                <div class="od-row2"><span class="od-line">{orderDetailLine(o)}</span></div>
                {#if fill}<div class="od-fill">{fill}</div>{/if}
                {#each waitingLegs(o) as leg}
                  <div class="od-leg"><span class="od-leg-label {leg.tone}">{leg.label}</span><span class="od-leg-value num">{leg.line}</span>{#if leg.note}<span class="od-leg-state">{leg.note}</span>{/if}<span class="od-leg-amt num {leg.tone}">{leg.amount}</span></div>
                {/each}
                {#if editing && panel.orderEdit}
                  <div class="od-edit">
                    <div class="tk-grid">
                      <label class="tk-f"><span class="tk-l">Shares</span><input class="tk-in num" id="od-qty" use:focusOnMount onkeydown={(e) => saveOnEnter(e, () => orderEditSave(o.id))} inputmode="decimal" autocomplete="off" value={panel.orderEdit.qty ?? qtyFmt(o.quantity)} oninput={(e) => (panel.orderEdit!.qty = (e.target as HTMLInputElement).value)} /></label>
                      {#if orderHasLimit(o)}
                        <label class="tk-f"><span class="tk-l">Limit price</span><input class="tk-in num" id="od-limit" onkeydown={(e) => saveOnEnter(e, () => orderEditSave(o.id))} inputmode="decimal" autocomplete="off" value={panel.orderEdit.limit ?? px(o.limitPrice)} oninput={(e) => (panel.orderEdit!.limit = (e.target as HTMLInputElement).value)} /></label>
                      {:else}<div></div>{/if}
                    </div>
                    <div class="od-edit-btns"><button class="od-link muted" onclick={cancelOrderEdit}>Cancel</button><button class="tk-go od-save" disabled={panel.busy === 'orders'} onclick={() => orderEditSave(o.id)}>{panel.busy === 'orders' ? 'Saving…' : 'Save'}</button></div>
                    {#if panel.orderEdit.error}<div class="status-err" style="font-size:12px">{panel.orderEdit.error}</div>{/if}
                  </div>
                {/if}
                <div class="od-foot"><span class="od-when">{orderWhenWord(o.createdAt)}</span>{#if live && !editing}<span class="od-foot-acts"><button class="od-link" disabled={!orderCanEdit(o)} onclick={() => editOrder(o.id)}>Edit</button><button class="od-link neg" onclick={() => (ui.confirm = 'cancel:' + o.id)}>Cancel</button></span>{:else if ended}<span class="od-state{ended[1] === 'neg' ? ' neg' : ''}">{ended[0]}</span>{:else if o.status === 'sending'}<span class="od-state">{orderPill(o)[0]}</span>{/if}</div>
              </div>
            {:else if card.kind === 'bracket' && card.b}
              {@const b = card.b}
              {@const editing = panel.bracketEdit?.id === b.id}
              {@const live = bracketLive(b)}
              {@const ended = !live && panel.tab === 'cancelled' ? bracketEndWord(b) : null}
              {@const bed = bracketEditData(b)}
              <div class="od-card">
                <div class="od-row1"><span class="od-title"><span style="color:var(--accent-300)">Bracket</span> {bracketTitle(b)}</span><span class="od-value num">{bracketValue(b)}</span></div>
                {#each bracketLegs(b) as leg}
                  <div class="od-leg"><span class="od-leg-label {leg.tone}">{leg.label}</span><span class="od-leg-value num">{leg.line}</span>{#if leg.note}<span class="od-leg-state">{leg.note}</span>{/if}<span class="od-leg-amt num {leg.tone}">{leg.amount}</span></div>
                {/each}
                {#if editing && panel.bracketEdit}
                  <div class="od-edit">
                    <div class="tk-grid">
                      {#if b.slKind}
                        <div class="tk-f"><span class="tk-l">{bed.slLabel}</span><input class="tk-in num" id="od-sl" use:focusOnMount onkeydown={(e) => saveOnEnter(e, () => bracketEditSave(b.id))} inputmode="decimal" autocomplete="off" value={panel.bracketEdit.sl ?? bed.slCur} oninput={(e) => (panel.bracketEdit!.sl = (e.target as HTMLInputElement).value)} /><button class="od-link neg od-remove" onclick={() => bracketRemove(b.id, 'sl')}>Remove stop loss</button></div>
                      {:else}<div></div>{/if}
                      {#if b.tpPrice}
                        <div class="tk-f"><span class="tk-l">Limit price</span><input class="tk-in num" id="od-tp" onkeydown={(e) => saveOnEnter(e, () => bracketEditSave(b.id))} inputmode="decimal" autocomplete="off" value={panel.bracketEdit.tp ?? px(b.tpPrice)} oninput={(e) => (panel.bracketEdit!.tp = (e.target as HTMLInputElement).value)} /><button class="od-link neg od-remove" onclick={() => bracketRemove(b.id, 'tp')}>Remove take profit</button></div>
                      {:else}<div></div>{/if}
                    </div>
                    <div class="od-edit-btns"><button class="od-link muted" onclick={cancelBracketEdit}>Cancel</button><button class="tk-go od-save" disabled={panel.busy === 'orders'} onclick={() => bracketEditSave(b.id)}>{panel.busy === 'orders' ? 'Saving…' : 'Save'}</button></div>
                    {#if panel.bracketEdit.error}<div class="status-err" style="font-size:12px">{panel.bracketEdit.error}</div>{/if}
                  </div>
                {/if}
                <div class="od-foot"><span class="od-when">{bracketWhen(b)}</span>{#if live && !editing}<span class="od-foot-acts"><button class="od-link" onclick={() => editBracket(b.id)}>Edit</button><button class="od-link neg" onclick={() => (ui.confirm = 'bracket:' + b.id)}>Cancel</button></span>{:else if ended}<span class="od-state{ended[1] === 'neg' ? ' neg' : ''}">{ended[0]}</span>{/if}</div>
              </div>
            {/if}
          {/each}

          {#if view.empty}<div class="dim" style="font-size:12px">{view.empty}</div>{/if}
        {/if}
      </div>
    </div>
  </div>
</div>
