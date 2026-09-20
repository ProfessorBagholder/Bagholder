<script lang="ts">
  import { ticketStore, ticketAccounts, closeTicket, fetchQuote, submit, vals } from './ticket.svelte'
  import { price, money, pct } from '../fmt'

  const TYPES: [string, string][] = [['MARKET', 'Market'], ['LIMIT', 'Limit'], ['STOP', 'Stop'], ['STOP_LIMIT', 'Stop limit']]
  const TIFS: [string, string][] = [['DAY', 'Day'], ['GTC', 'Good till cancelled']]

  const t = $derived(ticketStore.t!)
  const v = $derived(vals()!)
  const accounts = $derived(ticketAccounts())

  const parse = (s: string): number | null => { const n = parseFloat(s.replace(/[^0-9.\-]/g, '')); return isNaN(n) ? null : n }
  const amt = (n: number | null) => (n == null || !isFinite(n) ? '—' : money(n, '', Number.isInteger(+n.toFixed(2)) ? 0 : 2))
  const sgn = (n: number | null) => (n == null ? '—' : (n >= 0 ? '+' : '') + amt(n).replace('-', ''))

  function setSide(side: 'BUY' | 'SELL') { t.side = side; fetchQuote() }
  function setQty(s: string) { t.qty = parse(s) }
  function setAmt(s: string) { const n = parse(s); t.qty = n != null && v.entry ? n / (v.entry * v.mult) : t.qty }
  // stop-loss read line
  const slRead = $derived.by(() => {
    if (!v.slOn) return ['', '']
    const l = v.isTrail ? (v.buy ? 'Starts at ' : 'Starts at ') + price(v.slPrice) : 'Sells at ' + price(v.slPrice)
    const r = v.risk != null ? sgn(-Math.abs(v.risk)) + ' (' + pct(v.slPct) + ')' : '—'
    return [l, r]
  })
  const tpRead = $derived.by(() => {
    if (!v.tpOn) return ['', '']
    return ['Sells at ' + price(v.tpPrice), v.gain != null ? sgn(v.gain) + ' (' + pct(v.tpPct) + ')' : '—']
  })
  const rrStr = $derived(v.rr == null ? '—' : '1:' + (+v.rr.toFixed(1)))
</script>

<div class="scrim" role="presentation" onclick={() => closeTicket(false)}></div>
<div class="tk" role="dialog" aria-label={t.step === 'review' ? 'Review order' : 'New order'}>
  <div class="hd">
    <span class="title">{t.step === 'review' ? 'Review order' : 'New order'}</span>
    <button class="x" aria-label="Close" onclick={() => closeTicket(false)}>×</button>
  </div>

  {#if t.step === 'form'}
    <div class="body">
      <div class="quote">
        <div class="qtop">
          <div class="qsym">
            <div class="s">{v.q.symbol ?? t.symbol}</div>
            <div class="n">{v.q.name ?? ''}{v.q.exchange ? ' · ' + v.q.exchange : ''}</div>
          </div>
          <div class="qpx">
            <div class="last">{v.q.last == null ? '—' : price(v.q.last)}</div>
            <div class="chg {v.q.change == null ? '' : v.q.change < 0 ? 'neg' : 'pos'}">{v.q.change == null ? '—' : (v.q.change < 0 ? '−' : '+') + price(Math.abs(v.q.change)) + ' (' + pct(v.q.changePct ?? null, 2) + ')'}</div>
          </div>
        </div>
        <div class="qbba">
          <div><span>Bid</span> {v.q.bid == null ? '—' : price(v.q.bid)}</div>
          <div><span>Mid</span> {v.q.mid == null ? '—' : price(v.q.mid)}</div>
          <div><span>Ask</span> {v.q.ask == null ? '—' : price(v.q.ask)}</div>
        </div>
      </div>
      {#if t.error}<div class="err">{t.error}</div>{/if}

      <div class="seg">
        <button class="buy" class:on={v.buy} onclick={() => setSide('BUY')}>Buy</button>
        <button class="sell" class:on={!v.buy} onclick={() => setSide('SELL')}>Sell</button>
      </div>

      <div class="grid">
        <label>Account<select bind:value={t.accountId} onchange={fetchQuote}>{#each accounts as a (a.id)}<option value={a.id}>{a.name}</option>{/each}</select></label>
        <label>Shares<input value={v.qtyN} oninput={(e) => setQty((e.target as HTMLInputElement).value)} /></label>
        <label>Order type<select bind:value={t.type}>{#each TYPES.filter((x) => !v.d.orderTypes || v.d.orderTypes.includes(x[0])) as [k, l] (k)}<option value={k}>{l}</option>{/each}</select></label>
        {#if t.type === 'LIMIT' || t.type === 'STOP_LIMIT'}<label>Limit price<input value={v.limit == null ? '' : price(v.limit)} oninput={(e) => (t.limit = parse((e.target as HTMLInputElement).value))} /></label>{/if}
        {#if t.type === 'STOP' || t.type === 'STOP_LIMIT'}<label>Stop price<input value={v.stop == null ? '' : price(v.stop)} oninput={(e) => (t.stop = parse((e.target as HTMLInputElement).value))} /></label>{/if}
        {#if t.type !== 'MARKET'}<label>Time in force<select bind:value={t.tif}>{#each TIFS as [k, l] (k)}<option value={k}>{l}</option>{/each}</select></label>{/if}
        <label>Amount<input value={amt(v.notional)} oninput={(e) => setAmt((e.target as HTMLInputElement).value)} /></label>
      </div>

      {#if v.buy}
        <div class="rule"></div>
        <div class="sec">
          <div class="sech">Stop loss<button class="secbtn" onclick={() => (t.sl.on = !t.sl.on)}>{t.sl.on ? '×' : '+'}</button></div>
          {#if t.sl.on}
            <div class="grid">
              <label>Type<select bind:value={t.sl.kind}><option value="stop">Stop</option><option value="trail">Trailing stop</option></select></label>
              <label>{v.isTrail ? 'Trail' : 'Stop'}
                <div class="unitfield">
                  {#if v.isTrail}
                    <input value={t.sl.unit === 'pct' ? (t.sl.trail ?? 5) : (t.sl.trail ?? '')} oninput={(e) => (t.sl.trail = parse((e.target as HTMLInputElement).value))} />
                  {:else}
                    <input value={t.sl.priceUnit === 'pct' ? (t.sl.pct ?? 5) : (t.sl.price == null ? (v.slPrice ?? '') : t.sl.price)} oninput={(e) => { const n = parse((e.target as HTMLInputElement).value); if (t.sl.priceUnit === 'pct') t.sl.pct = n; else t.sl.price = n }} />
                  {/if}
                  <div class="unit">
                    <button class:on={(v.isTrail ? t.sl.unit : t.sl.priceUnit) === 'amt'} onclick={() => (v.isTrail ? (t.sl.unit = 'amt') : (t.sl.priceUnit = 'amt'))}>$</button>
                    <button class:on={(v.isTrail ? t.sl.unit : t.sl.priceUnit) === 'pct'} onclick={() => (v.isTrail ? (t.sl.unit = 'pct') : (t.sl.priceUnit = 'pct'))}>%</button>
                  </div>
                </div>
              </label>
            </div>
            <div class="read"><span>{slRead[0]}</span><span class="neg">{slRead[1]}</span></div>
          {/if}
        </div>
        <div class="rule"></div>
        <div class="sec">
          <div class="sech">Take profit<button class="secbtn" onclick={() => (t.tp.on = !t.tp.on)}>{t.tp.on ? '×' : '+'}</button></div>
          {#if t.tp.on}
            <div class="grid">
              <label>Target
                <div class="unitfield">
                  <input value={t.tp.unit === 'pct' ? (t.tp.pct ?? 10) : (t.tp.price == null ? (v.tpPrice ?? '') : t.tp.price)} oninput={(e) => { const n = parse((e.target as HTMLInputElement).value); if (t.tp.unit === 'pct') t.tp.pct = n; else t.tp.price = n }} />
                  <div class="unit">
                    <button class:on={t.tp.unit === 'amt'} onclick={() => (t.tp.unit = 'amt')}>$</button>
                    <button class:on={t.tp.unit === 'pct'} onclick={() => (t.tp.unit = 'pct')}>%</button>
                  </div>
                </div>
              </label><div></div>
            </div>
            <div class="read"><span>{tpRead[0]}</span><span class="pos">{tpRead[1]}</span></div>
          {/if}
        </div>
      {/if}
    </div>
    <div class="ft">
      <button class="cancel" onclick={() => closeTicket(true)}>Cancel</button>
      <button class="go" onclick={() => { if (!(v.qtyN > 0)) t.qty = 1; t.step = 'review'; t.submitError = '' }}>Review</button>
    </div>
  {:else}
    <div class="body">
      <div class="rsum">
        <div class="rt"><span class={v.buy ? 'pos' : 'neg'}>{v.buy ? 'Buy' : 'Sell'}</span> {v.qtyN} {v.q.symbol ?? t.symbol}</div>
        <div class="rl">{v.typeWord}{t.type === 'MARKET' ? '' : ' ' + price(v.entry) + ' · ' + v.tifWord} · {v.acct?.name ?? ''}</div>
      </div>
      {#if v.buy}
        <div class="rrows">
          <div class="row"><span>Stop loss</span><span>{!v.slOn ? 'None' : v.isTrail ? 'Trails ' + (t.sl.unit === 'pct' ? (t.sl.trail ?? 5) + '%' : price(t.sl.trail)) + ' under the high · starts at ' + price(v.slPrice) : 'Market at ' + price(v.slPrice)}</span></div>
          <div class="row"><span>Take profit</span><span>{v.tpOn ? 'Limit at ' + price(v.tpPrice) : 'None'}</span></div>
        </div>
      {/if}
      <div class="rrows border">
        {#if v.buy}
          <div class="row"><span>At risk</span><span class="neg">{v.slOn && v.risk != null ? sgn(-Math.abs(v.risk)) + ' (' + pct(v.slPct) + ')' : '—'}</span></div>
          <div class="row"><span>Target</span><span class="pos">{v.tpOn && v.gain != null ? sgn(v.gain) + ' (' + pct(v.tpPct) + ')' : '—'}</span></div>
          <div class="row"><span>Risk / reward</span><span>{rrStr}</span></div>
        {/if}
        <div class="row"><span>Position size</span><span>{v.cad != null && v.nav > 0 ? ((v.cad / v.nav) * 100).toFixed(1) + '% of net asset value' : '—'}</span></div>
        <div class="row"><span>{v.isMargin ? 'Available margin after' : 'Cash after'}</span><span class={v.after != null && v.after < 0 ? 'neg' : ''}>{v.after == null ? '—' : money(v.after, '', 0)}</span></div>
      </div>
      <div class="rcost"><span>{v.buy ? 'Estimated cost' : 'Estimated proceeds'}</span><span class="c">{t.type === 'MARKET' ? '≈ ' : ''}{amt(v.notional)}</span></div>
      {#if t.submitError}<div class="err">{t.submitError}</div>{/if}
    </div>
    <div class="ft">
      <button class="cancel" onclick={() => { t.step = 'form'; t.submitError = '' }}>Back</button>
      <button class="go" disabled={t.busy} onclick={submit}>{t.busy ? 'Submitting…' : 'Submit'}</button>
    </div>
  {/if}
</div>

<style>
  .scrim { position: fixed; inset: 0; background: rgba(0,0,0,0.4); z-index: 60; }
  .tk { position: fixed; top: 0; right: 0; height: 100vh; width: 400px; max-width: 92vw; background: #0b0e14; border-left: 1px solid #1c2230; z-index: 61; display: flex; flex-direction: column; }
  .hd { display: flex; align-items: center; justify-content: space-between; padding: 16px 18px; border-bottom: 1px solid #1c2230; }
  .title { font-size: 15px; font-weight: 600; }
  .x { background: none; border: 0; color: #8b93a7; font-size: 20px; cursor: pointer; line-height: 1; }
  .body { flex: 1; overflow-y: auto; padding: 16px 18px; display: flex; flex-direction: column; gap: 14px; }
  .quote { background: #141924; border: 1px solid #1c2230; border-radius: 10px; padding: 12px 14px; }
  .qtop { display: flex; justify-content: space-between; gap: 12px; }
  .qsym .s { font-size: 18px; font-weight: 600; } .qsym .n { color: #8b93a7; font-size: 12px; margin-top: 2px; }
  .qpx { text-align: right; } .qpx .last { font-size: 18px; font-weight: 600; } .qpx .chg { font-size: 12px; margin-top: 2px; }
  .qbba { display: grid; grid-template-columns: 1fr 1fr 1fr; text-align: center; font-size: 12px; margin-top: 10px; padding-top: 10px; border-top: 1px solid #1c2230; }
  .qbba span { color: #8b93a7; }
  .err { color: #f0616d; font-size: 12px; }
  .seg { display: grid; grid-template-columns: 1fr 1fr; gap: 6px; }
  .seg button { padding: 8px; border-radius: 8px; border: 1px solid #1c2230; background: #141924; color: #8b93a7; font: inherit; font-weight: 600; cursor: pointer; }
  .seg .buy.on { background: rgba(62,207,142,0.18); color: #3ecf8e; border-color: #3ecf8e; }
  .seg .sell.on { background: rgba(240,97,109,0.18); color: #f0616d; border-color: #f0616d; }
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
  label { display: flex; flex-direction: column; gap: 4px; font-size: 11px; color: #8b93a7; }
  input, select { background: #141924; border: 1px solid #1c2230; border-radius: 7px; color: #e6e9ef; font: inherit; font-size: 13px; padding: 7px 8px; }
  input:focus, select:focus { outline: none; border-color: #2a3242; }
  .rule { height: 1px; background: #1c2230; }
  .sec { display: flex; flex-direction: column; gap: 10px; }
  .sech { display: flex; align-items: center; font-size: 13px; font-weight: 500; }
  .secbtn { margin-left: auto; background: #1c2230; border: 0; color: #c4cbd8; width: 22px; height: 22px; border-radius: 6px; cursor: pointer; }
  .unitfield { display: flex; gap: 6px; }
  .unitfield input { flex: 1; min-width: 0; }
  .unit { display: inline-flex; background: #141924; border: 1px solid #1c2230; border-radius: 7px; overflow: hidden; }
  .unit button { background: none; border: 0; color: #8b93a7; padding: 0 9px; cursor: pointer; font: inherit; }
  .unit button.on { background: #2a3242; color: #e6e9ef; }
  .read { display: flex; justify-content: space-between; font-size: 12px; }
  .read span:first-child { color: #8b93a7; }
  .ft { display: grid; grid-template-columns: 1fr 2fr; gap: 10px; padding: 14px 18px; border-top: 1px solid #1c2230; }
  .cancel { background: #141924; border: 1px solid #1c2230; color: #c4cbd8; border-radius: 8px; padding: 9px; font: inherit; cursor: pointer; }
  .go { background: #3ecf8e; border: 0; color: #08110b; border-radius: 8px; padding: 9px; font: inherit; font-weight: 600; cursor: pointer; }
  .go:disabled { opacity: 0.6; }
  .rsum { padding-bottom: 14px; border-bottom: 1px solid #1c2230; }
  .rt { font-size: 18px; font-weight: 600; } .rl { color: #8b93a7; font-size: 12px; margin-top: 2px; }
  .rrows { display: flex; flex-direction: column; gap: 10px; }
  .rrows.border { padding-top: 14px; border-top: 1px solid #1c2230; }
  .row { display: flex; justify-content: space-between; gap: 12px; font-size: 13px; }
  .row span:first-child { color: #8b93a7; }
  .rcost { display: flex; justify-content: space-between; align-items: baseline; padding-top: 14px; border-top: 1px solid #1c2230; }
  .rcost .c { font-size: 18px; font-weight: 600; }
  .pos { color: #3ecf8e; } .neg { color: #f0616d; }
</style>
