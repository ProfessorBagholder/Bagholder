<script lang="ts">
  import { ticketStore, ticketAccounts, closeTicket, fetchQuote, submit, vals, maxQty, refreshPreview } from './ticket.svelte'
  import { neg, sign, ticketNumber } from '../dec'
  import { plain, parseNum, amt as tkAmt, sAmt as tkSAmt } from './vals'
  import { px, pct, money, qty as qtyFmt, num } from '../fmt'
  import { symText } from '../sym'
  import { ICONS } from '../icons'

  const TK_TYPES: [string, string][] = [['MARKET', 'Market'], ['LIMIT', 'Limit'], ['STOP', 'Stop'], ['STOP_LIMIT', 'Stop limit']]
  const TK_TIFS: [string, string][] = [['DAY', 'Day'], ['UNTIL_CANCEL', 'Good till cancelled']]

  const t = $derived(ticketStore.t!)
  const v = $derived(vals()!)
  // the server's figures, asked again whenever what the ticket holds changes
  $effect(() => {
    void JSON.stringify([t.side, t.type, t.accountId, t.qty, t.text.amt, t.limit, t.stop, t.sl, t.tp, t.data])
    refreshPreview()
  })
  const accounts = $derived(ticketAccounts())
  const maxOff = $derived(maxQty() == null)
  const types = $derived((t.data && t.data.orderTypes) || TK_TYPES.map((x) => x[0]))
  const showLimit = $derived(t.type === 'LIMIT' || t.type === 'STOP_LIMIT')
  const showStop = $derived(t.type === 'STOP' || t.type === 'STOP_LIMIT')
  const showTif = $derived(t.type !== 'MARKET') // the order's own; the brackets always go out good till cancelled

  // the value shown in a field: what was typed if it stands, otherwise the derived figure
  const val = (key: string, fmt: string): string => (t.text[key] != null ? (t.text[key] as string) : fmt)
  const size = (n: number | null | undefined) => (n == null ? '' : ' × ' + num(n, 0))

  // the stop field reads the trail or the stop in the unit chosen
  const slText = $derived(v.isTrail ? (t.sl.unit === 'pct' ? plain(v.trail) : px(v.trail)) : t.sl.priceUnit === 'pct' ? plain(v.slPctIn) : px(v.slPrice))
  const tpText = $derived(t.tp.unit === 'pct' ? plain(v.tpPctIn) : px(v.tpPrice))
  const slRead = $derived.by(() => {
    const verb = v.buy ? 'Sells at ' : 'Buys at '
    return [v.isTrail ? 'Starts at ' + px(v.slPrice) : verb + px(v.slPrice), v.risk == null ? '—' : tkSAmt(neg(v.risk)) + ' (' + pct(v.slPct) + ')']
  })
  const tpRead = $derived.by(() => [(v.buy ? 'Sells at ' : 'Buys at ') + px(v.tpPrice), v.gain == null ? '—' : tkSAmt(v.gain) + ' (' + pct(v.tpPct) + ')'])
  const rrStr = $derived(v.rr == null ? '—' : '1:' + plain(+v.rr.toFixed(1)))
  const line = $derived(v.typeWord + (t.type === 'MARKET' ? '' : ' ' + px(v.entry) + ' · ' + v.tifWord) + ' · ' + (v.acct ? v.acct.name : ''))
  const slLine = $derived(!v.slOn ? 'None' : v.isTrail ? 'Trails ' + v.trailWord + (v.buy ? ' under the high' : ' over the low') + ' · starts at ' + px(v.slPrice) : 'Market at ' + px(v.slPrice))

  // --- input / change handlers, mirroring tkInputEvent / tkBlur / tkChange ---
  function onInput(id: string, value: string) {
    const n = parseNum(value)
    switch (id) {
      case 'tk-qty': t.text.qty = value; t.qty = n == null ? 0 : n; t.text.amt = null; break
      // the whole units the amount buys are the server's (the preview's quantity)
      case 'tk-amt': t.text.amt = value; t.text.qty = null; break
      case 'tk-limit': t.text.limit = value; t.limit = n; break
      case 'tk-stop': t.text.stop = value; t.stop = n; break
      case 'tk-slprice': t.text.slprice = value; if (t.sl.priceUnit === 'pct') t.sl.pct = n; else t.sl.price = n; break
      case 'tk-sltrail': t.text.sltrail = value; t.sl.trail = n; break
      case 'tk-tp': t.text.tp = value; if (t.tp.unit === 'pct') t.tp.pct = n; else t.tp.price = n; break
    }
  }
  function onBlur(id: string) {
    const key = ({ 'tk-qty': 'qty', 'tk-amt': 'amt', 'tk-limit': 'limit', 'tk-stop': 'stop', 'tk-slprice': 'slprice', 'tk-sltrail': 'sltrail', 'tk-tp': 'tp' } as Record<string, string>)[id]
    if (!key) return
    // an amount typed leaves the units it bought
    if (key === 'amt' && t.text.amt != null) t.qty = ticketNumber(ticketStore.preview?.quantity) ?? t.qty
    t.text[key] = null
    if (key === 'qty' && !(t.qty != null && t.qty > 0)) t.qty = 1
    if (key === 'limit' && !(t.limit != null && t.limit > 0)) t.limit = null
    if (key === 'stop' && !(t.stop != null && t.stop > 0)) t.stop = null
    if (key === 'slprice') { if (!(t.sl.price != null && t.sl.price > 0)) t.sl.price = null; if (!(t.sl.pct != null && t.sl.pct > 0)) t.sl.pct = null }
    if (key === 'sltrail' && !(t.sl.trail != null && t.sl.trail > 0)) t.sl.trail = null
    if (key === 'tp') { if (!(t.tp.price != null && t.tp.price > 0)) t.tp.price = null; if (!(t.tp.pct != null && t.tp.pct > 0)) t.tp.pct = null }
  }
  function setAccount(id: string) { t.accountId = id; try { localStorage.setItem('bh2.ticketAccount', id) } catch { /* ignore */ } fetchQuote() }
  function setSide(side: 'BUY' | 'SELL') { t.side = side; t.sl.price = null; t.sl.pct = null; t.sl.trail = null; t.tp.price = null; t.tp.pct = null; t.limit = null; t.stop = null }
  function setSlUnit(u: 'amt' | 'pct') { if (t.sl.kind === 'trail') { t.sl.unit = u; t.sl.trail = null; t.text.sltrail = null } else { t.sl.priceUnit = u; t.sl.price = null; t.sl.pct = null; t.text.slprice = null } }
  function setTpUnit(u: 'amt' | 'pct') { t.tp.unit = u; t.tp.price = null; t.tp.pct = null; t.text.tp = null }
  function doMax() { const m = maxQty(); if (m != null) { t.qty = m; t.text.qty = null; t.text.amt = null } }
  function review() { if (!(t.qty != null && t.qty > 0)) t.qty = 1; t.step = 'review'; t.submitError = '' }
</script>

<div id="tkWrap">
  <div class="tk-scrim" role="presentation" onclick={() => closeTicket(false)}></div>
  <div class="tk" role="dialog" aria-label={t.step === 'review' ? 'Review order' : 'New order'}>
    <div class="tk-hd">
      <span class="tk-title">{t.step === 'review' ? 'Review order' : 'New order'}</span>
      <button class="tk-x" aria-label="Close" onclick={() => closeTicket(false)}><svg width="14" height="14" viewBox="0 0 256 256" fill="currentColor"><path d={ICONS.x} /></svg></button>
    </div>

    {#if t.step === 'form'}
      <div class="tk-body">
        <div class="tk-quote" id="tkQuote">
          <div style="display:flex;justify-content:space-between;gap:12px">
            <div style="min-width:0">
              <div style="font-size:20px;line-height:1.2;font-weight:500;white-space:nowrap;overflow:hidden;text-overflow:ellipsis">{symText(v.q.symbol || t.symbol)}</div>
              <div style="font-size:12px;color:var(--ink55);margin-top:2px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis">{(v.q.name || '') + (v.q.exchange ? ' · ' + v.q.exchange : '')}</div>
            </div>
            <div style="text-align:right;flex:none">
              <div class="num" style="font-size:20px;line-height:1.2;font-weight:500">{v.q.last == null ? '—' : px(v.q.last)}</div>
              <div class="num" style="font-size:12px;margin-top:2px;color:{v.q.change != null && v.q.change < 0 ? 'var(--neg)' : 'var(--pos)'}">{v.q.change == null ? '—' : (v.q.change < 0 ? '−' : '+') + px(Math.abs(v.q.change)) + ' (' + pct(v.q.changePct, 2) + ')'}</div>
            </div>
          </div>
          <div class="num" style="display:grid;grid-template-columns:1fr 1fr 1fr;text-align:center;font-size:12px;padding-top:10px;box-shadow:inset 0 1px 0 rgba(var(--ink-rgb),.10)">
            <div><span style="color:var(--ink55)">Bid</span> {v.q.bid == null ? '—' : px(v.q.bid) + size(v.q.bidSize)}</div>
            <div><span style="color:var(--ink55)">Mid</span> {v.q.mid == null ? '—' : px(v.q.mid)}</div>
            <div><span style="color:var(--ink55)">Ask</span> {v.q.ask == null ? '—' : px(v.q.ask) + size(v.q.askSize)}</div>
          </div>
        </div>
        {#if t.error}<div class="status-err" style="font-size:12px;margin-top:-12px">{t.error}</div>{/if}

        <div class="tk-seg">
          <button class="tk-segopt buy" class:on={v.buy} onclick={() => setSide('BUY')}>Buy</button>
          <button class="tk-segopt sell" class:on={!v.buy} onclick={() => setSide('SELL')}>Sell</button>
        </div>

        <div class="tk-grid">
          <label class="tk-f"><span class="tk-l">Account</span>
            <span class="tk-sel"><select class="tk-in" id="tk-account" value={t.accountId} onchange={(e) => setAccount((e.target as HTMLSelectElement).value)}>{#each accounts as a (a.id)}<option value={a.id}>{a.name}</option>{/each}</select><svg width="12" height="12" viewBox="0 0 256 256" fill="currentColor"><path d={ICONS.caretDown} /></svg></span>
          </label>
          <label class="tk-f"><span class="tk-l">Shares</span>
            <span class="tk-wrap"><input class="tk-in num has-max" id="tk-qty" inputmode="decimal" autocomplete="off" value={val('qty', qtyFmt(v.qty))} oninput={(e) => onInput('tk-qty', (e.target as HTMLInputElement).value)} onblur={() => onBlur('tk-qty')} /><button type="button" class="tk-max" class:off={maxOff} onclick={doMax}>Max</button></span>
          </label>
          <label class="tk-f"><span class="tk-l">Order type</span>
            <span class="tk-sel"><select class="tk-in" id="tk-type" bind:value={t.type}>{#each TK_TYPES.filter((x) => types.indexOf(x[0]) >= 0) as [k, l] (k)}<option value={k}>{l}</option>{/each}</select><svg width="12" height="12" viewBox="0 0 256 256" fill="currentColor"><path d={ICONS.caretDown} /></svg></span>
          </label>
          {#if showLimit}
            <label class="tk-f"><span class="tk-l">Limit price</span><input class="tk-in num" id="tk-limit" inputmode="decimal" autocomplete="off" value={val('limit', px(v.limit))} oninput={(e) => onInput('tk-limit', (e.target as HTMLInputElement).value)} onblur={() => onBlur('tk-limit')} /></label>
          {/if}
          {#if showStop}
            <label class="tk-f"><span class="tk-l">Stop price</span><input class="tk-in num" id="tk-stop" inputmode="decimal" autocomplete="off" value={val('stop', px(v.stop))} oninput={(e) => onInput('tk-stop', (e.target as HTMLInputElement).value)} onblur={() => onBlur('tk-stop')} /></label>
          {/if}
          {#if showTif}
            <label class="tk-f"><span class="tk-l">Time in force</span>
              <span class="tk-sel"><select class="tk-in" id="tk-tif" bind:value={t.tif}>{#each TK_TIFS as [k, l] (k)}<option value={k}>{l}</option>{/each}</select><svg width="12" height="12" viewBox="0 0 256 256" fill="currentColor"><path d={ICONS.caretDown} /></svg></span>
            </label>
          {/if}
          <label class="tk-f"><span class="tk-l">Amount</span>
            <span class="tk-wrap"><input class="tk-in num has-max" id="tk-amt" inputmode="decimal" autocomplete="off" value={val('amt', tkAmt(v.notional))} oninput={(e) => onInput('tk-amt', (e.target as HTMLInputElement).value)} onblur={() => onBlur('tk-amt')} /><button type="button" class="tk-max" class:off={maxOff} onclick={doMax}>Max</button></span>
          </label>
        </div>

        {#if v.buy}
          <div class="tk-rule"></div>
          <div class="tk-sec">
            <div class="tk-sech">Stop loss
              {#if t.sl.on}<button class="tk-rowbtn sell" style="margin-left:auto" aria-label="Remove stop loss" onclick={() => (t.sl.on = false)}><svg width="12" height="12" viewBox="0 0 256 256" fill="currentColor"><path d={ICONS.x} /></svg></button>
              {:else}<button class="tk-rowbtn buy" style="margin-left:auto" aria-label="Add stop loss" onclick={() => (t.sl.on = true)}><svg width="12" height="12" viewBox="0 0 256 256" fill="currentColor"><path d={ICONS.plus} /></svg></button>{/if}
            </div>
            {#if t.sl.on}
              <div class="tk-grid">
                <label class="tk-f"><span class="tk-l">Type</span>
                  <span class="tk-sel"><select class="tk-in" id="tk-slkind" bind:value={t.sl.kind}><option value="stop">Stop</option><option value="trail">Trailing stop</option></select><svg width="12" height="12" viewBox="0 0 256 256" fill="currentColor"><path d={ICONS.caretDown} /></svg></span>
                </label>
                <label class="tk-f"><span class="tk-l">{v.isTrail ? 'Trail' : 'Stop'}</span>
                  <span class="tk-wrap">
                    {#if v.isTrail}
                      <input class="tk-in num has-unit" id="tk-sltrail" inputmode="decimal" autocomplete="off" value={val('sltrail', slText)} oninput={(e) => onInput('tk-sltrail', (e.target as HTMLInputElement).value)} onblur={() => onBlur('tk-sltrail')} />
                    {:else}
                      <input class="tk-in num has-unit" id="tk-slprice" inputmode="decimal" autocomplete="off" value={val('slprice', slText)} oninput={(e) => onInput('tk-slprice', (e.target as HTMLInputElement).value)} onblur={() => onBlur('tk-slprice')} />
                    {/if}
                    <span class="tk-unit">
                      <button type="button" class="tk-unitopt" class:on={(v.isTrail ? t.sl.unit : t.sl.priceUnit) === 'amt'} onclick={() => setSlUnit('amt')}>$</button>
                      <button type="button" class="tk-unitopt" class:on={(v.isTrail ? t.sl.unit : t.sl.priceUnit) === 'pct'} onclick={() => setSlUnit('pct')}>%</button>
                    </span>
                  </span>
                </label>
              </div>
              <div class="tk-read"><span class="l" id="tk-sl-l">{slRead[0]}</span><span class="r num" id="tk-sl-r" style="color:var(--neg)">{slRead[1]}</span></div>
            {/if}
          </div>
          <div class="tk-rule"></div>
          <div class="tk-sec">
            <div class="tk-sech">Take profit
              {#if t.tp.on}<button class="tk-rowbtn sell" style="margin-left:auto" aria-label="Remove take profit" onclick={() => (t.tp.on = false)}><svg width="12" height="12" viewBox="0 0 256 256" fill="currentColor"><path d={ICONS.x} /></svg></button>
              {:else}<button class="tk-rowbtn buy" style="margin-left:auto" aria-label="Add take profit" onclick={() => (t.tp.on = true)}><svg width="12" height="12" viewBox="0 0 256 256" fill="currentColor"><path d={ICONS.plus} /></svg></button>{/if}
            </div>
            {#if t.tp.on}
              <div class="tk-grid">
                <label class="tk-f"><span class="tk-l">Target</span>
                  <span class="tk-wrap">
                    <input class="tk-in num has-unit" id="tk-tp" inputmode="decimal" autocomplete="off" value={val('tp', tpText)} oninput={(e) => onInput('tk-tp', (e.target as HTMLInputElement).value)} onblur={() => onBlur('tk-tp')} />
                    <span class="tk-unit">
                      <button type="button" class="tk-unitopt" class:on={t.tp.unit === 'amt'} onclick={() => setTpUnit('amt')}>$</button>
                      <button type="button" class="tk-unitopt" class:on={t.tp.unit === 'pct'} onclick={() => setTpUnit('pct')}>%</button>
                    </span>
                  </span>
                </label><div></div>
              </div>
              <div class="tk-read"><span class="l" id="tk-tp-l">{tpRead[0]}</span><span class="r num" id="tk-tp-r" style="color:var(--pos)">{tpRead[1]}</span></div>
            {/if}
          </div>
        {/if}
      </div>
      <div class="tk-ft"><button class="tk-cancel" onclick={() => closeTicket(true)}>Cancel</button><button class="tk-go" onclick={review}>Review</button></div>
    {:else}
      <div class="tk-body">
        <div style="padding-bottom:14px;box-shadow:inset 0 -1px 0 rgba(var(--ink-rgb),.10)">
          <div style="font-size:20px;line-height:1.2;font-weight:500"><span style="color:{v.buy ? 'var(--pos)' : 'var(--neg)'}">{v.buy ? 'Buy' : 'Sell'}</span> {qtyFmt(v.qty)} {symText(v.q.symbol || t.symbol)}</div>
          <div style="font-size:12px;color:var(--ink55);margin-top:2px">{line}</div>
        </div>
        {#if v.buy}
          <div style="display:flex;flex-direction:column;gap:10px">
            <div class="tk-row"><span class="l">Stop loss</span><span class="num" style="text-align:right">{slLine}</span></div>
            <div class="tk-row"><span class="l">Take profit</span><span class="num" style="text-align:right">{v.tpOn ? 'Limit at ' + px(v.tpPrice) : 'None'}</span></div>
          </div>
        {/if}
        <div style="display:flex;flex-direction:column;gap:10px;padding-top:16px;box-shadow:inset 0 1px 0 rgba(var(--ink-rgb),.10)">
          {#if v.buy}
            <div class="tk-row"><span class="l">At risk</span><span class="num" style="text-align:right;color:var(--neg);font-weight:500">{v.slOn && v.risk != null ? tkSAmt(neg(v.risk)) + ' (' + pct(v.slPct) + ')' : '—'}</span></div>
            <div class="tk-row"><span class="l">Target</span><span class="num" style="text-align:right;color:var(--pos);font-weight:500">{v.tpOn && v.gain != null ? tkSAmt(v.gain) + ' (' + pct(v.tpPct) + ')' : '—'}</span></div>
            <div class="tk-row"><span class="l">Risk / reward</span><span class="num" style="text-align:right">{rrStr}</span></div>
          {/if}
          <div class="tk-row"><span class="l">Position size</span><span class="num" style="text-align:right">{v.positionShare != null ? (v.positionShare * 100).toFixed(1) + '% of net asset value' : '—'}</span></div>
          <div class="tk-row"><span class="l">{v.isMargin ? 'Available margin after' : 'Cash after'}</span><span class="num" style="text-align:right{v.after != null && sign(v.after) < 0 ? ';color:var(--neg)' : ''}">{v.after == null ? '—' : money(v.after, '', 0)}</span></div>
          {#if v.linkedMargin}
            <div class="tk-row"><span class="l">Available margin after</span><span class="num" style="text-align:right{v.marginAfter != null && sign(v.marginAfter) < 0 ? ';color:var(--neg)' : ''}">{v.marginAfter == null ? '—' : money(v.marginAfter, '', 0)}</span></div>
          {/if}
        </div>
        <div style="display:flex;justify-content:space-between;align-items:baseline;padding-top:16px;box-shadow:inset 0 1px 0 rgba(var(--ink-rgb),.10)"><span style="font-size:13px;font-weight:500">{v.buy ? 'Estimated cost' : 'Estimated proceeds'}</span><span class="num" style="font-size:20px;line-height:1.2;font-weight:500">{(t.type === 'MARKET' ? '≈ ' : '') + tkAmt(v.notional)}</span></div>
        {#if t.submitError}<div class="status-err" style="font-size:12px">{t.submitError}</div>{/if}
      </div>
      <div class="tk-ft"><button class="tk-cancel" onclick={() => { t.step = 'form'; t.submitError = '' }}>Back</button><button class="tk-go" disabled={t.busy} onclick={submit}>{t.busy ? 'Submitting…' : 'Submit'}</button></div>
    {/if}
  </div>
</div>
