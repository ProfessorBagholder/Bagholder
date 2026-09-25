<script lang="ts">
  // The three menu modals — Add trade, Import CSV report, Load folder — ported
  // from tradeModalHtml / importModalHtml / folderModalHtml + modalShell.
  import { store } from './state.svelte'
  import { ui, closeModal, saveTrade, watchFolder, scanFolder, stopWatch } from './ui.svelte'
  import { symText } from './sym'
  import { relTime } from './fmt'

  import { qty } from './fmt'

  const accounts = $derived((store.model?.accounts ?? []).filter((a) => a.status !== 'closed'))
  // units that arrived without a cost: what an opening balance prices
  const arrivals = $derived((store.model?.waiting ?? []).filter((w) => w.what === 'cost-of-arrival'))
  const arrival = $derived(arrivals.find((a) => a.transaction === ui.tradeForm.arrival) ?? null)
  const f = $derived(ui.tradeForm)
  const r = $derived(ui.importReport)
  const w = $derived(ui.watch)
  const width = $derived(ui.modal === 'trade' ? 460 : 520)
</script>

<div id="modalDlg" style="position:fixed;inset:0;z-index:20;background:rgba(8,9,16,.55);display:flex;align-items:center;justify-content:center;padding:20px" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) closeModal() }}>
  <div class="card elev-md" style="width:{width}px;max-width:100%;padding:18px 20px;max-height:90vh;overflow:auto">
    <div style="display:flex;align-items:center;gap:10px;margin-bottom:12px">
      <h5>{ui.modal === 'trade' ? 'Add trade' : ui.modal === 'import' ? 'Import CSV' : 'Load folder'}</h5>
      <button class="btn btn-icon btn-secondary" aria-label="Close" style="margin-left:auto;width:28px;height:28px" onclick={closeModal}>×</button>
    </div>

    {#if ui.modal === 'trade' && arrivals.length}
      <div class="seg" style="margin-bottom:12px">
        {#each [['trade', 'Trade'], ['opening', 'Opening balance']] as o (o[0])}<label class="seg-opt" class:on={f.mode === o[0]}><input type="radio" bind:group={f.mode} value={o[0]} />{o[1]}</label>{/each}
      </div>
    {/if}
    {#if ui.modal === 'trade' && f.mode === 'opening'}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div style="display:grid;grid-template-columns:1fr 1fr;gap:12px" onkeydown={(e) => { if (e.key === 'Enter' && (e.target as HTMLElement).tagName === 'INPUT') { e.preventDefault(); saveTrade() } }}>
        <div style="grid-column:1/-1"><div class="lbl" style="margin-bottom:5px">Arrival</div>
          <select class="input" bind:value={f.arrival} aria-label="Arrival">
            <option value="">Choose</option>
            {#each arrivals as a (a.transaction)}<option value={a.transaction}>{symText(a.symbol)} · {a.accountName} · {a.units == null ? '' : qty(a.units) + ' · '}{a.day}</option>{/each}
          </select>
        </div>
        <div><div class="lbl" style="margin-bottom:5px">Cost</div>
          <div style="display:flex;align-items:center;gap:6px"><input class="input" inputmode="decimal" bind:value={f.cost} aria-label="Cost" /><span class="muted" style="font-size:12px">{arrival?.currency ?? ''}</span></div>
        </div>
        <div><div class="lbl" style="margin-bottom:5px">Acquired</div><input class="input" type="date" bind:value={f.acquired} aria-label="Acquired" /></div>
      </div>
      {#if f.error}<div class="status-err" style="font-size:12px;margin-top:10px">{f.error}</div>{/if}
      <div style="display:flex;gap:8px;justify-content:flex-end;margin-top:16px">
        <button class="btn btn-secondary" onclick={closeModal}>Cancel</button>
        <button class="btn btn-primary" disabled={ui.busy === 'trade'} onclick={() => saveTrade()}>{ui.busy === 'trade' ? 'Saving…' : 'Add opening balance'}</button>
      </div>
    {:else if ui.modal === 'trade'}
      <!-- Enter in any of its boxes adds the trade -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div style="display:grid;grid-template-columns:1fr 1fr;gap:12px" onkeydown={(e) => { if (e.key === 'Enter' && (e.target as HTMLElement).tagName === 'INPUT') { e.preventDefault(); saveTrade() } }}>
        <div><div class="lbl" style="margin-bottom:5px">Date</div><input class="input" type="date" bind:value={f.date} /></div>
        <div><div class="lbl" style="margin-bottom:5px">Account</div>
          <select class="input" bind:value={f.account}>
            <option value="">Manual</option>
            {#each accounts.filter((a) => a.name && a.brokerAccount !== 'manual') as a (a.id)}<option value={a.id}>{a.name}</option>{/each}
          </select>
        </div>
        <div><div class="lbl" style="margin-bottom:5px">Symbol</div><input class="input" bind:value={f.symbol} placeholder="e.g. LUNR or LUNR 15JAN27 12.00 CALL" autocapitalize="characters" /></div>
        <div><div class="lbl" style="margin-bottom:5px">Side</div>
          <div class="seg">
            {#each ['BUY', 'SELL'] as o (o)}<label class="seg-opt" class:on={f.side === o}><input type="radio" bind:group={f.side} value={o} />{o}</label>{/each}
          </div>
        </div>
        <div><div class="lbl" style="margin-bottom:5px">Quantity</div><input class="input" inputmode="decimal" bind:value={f.qty} /></div>
        <div><div class="lbl" style="margin-bottom:5px">Price</div><input class="input" inputmode="decimal" bind:value={f.price} /></div>
        <div><div class="lbl" style="margin-bottom:5px">Currency</div>
          <div class="seg">
            {#each ['CAD', 'USD'] as o (o)}<label class="seg-opt" class:on={f.currency === o}><input type="radio" bind:group={f.currency} value={o} />{o}</label>{/each}
          </div>
        </div>
        <div><div class="lbl" style="margin-bottom:5px">Fees</div><input class="input" inputmode="decimal" bind:value={f.fees} placeholder="0" /></div>
      </div>
      {#if f.error}<div class="status-err" style="font-size:12px;margin-top:10px">{f.error}</div>{/if}
      <div style="display:flex;gap:8px;justify-content:flex-end;margin-top:16px">
        <button class="btn btn-secondary" onclick={closeModal}>Cancel</button>
        <button class="btn btn-primary" disabled={ui.busy === 'trade'} onclick={() => saveTrade()}>{ui.busy === 'trade' ? 'Saving…' : 'Add trade'}</button>
      </div>

    {:else if ui.modal === 'import'}
      {#if !r}
        <div class="muted" style="padding:16px 0"><span class="spin"></span>Reading…</div>
      {:else}
        <div style="font-size:12.5px">{r.files.length}{r.files.length === 1 ? ' file' : ' files'} · {r.added} new {r.added === 1 ? 'activity' : 'activities'} · {r.duplicates} already stored</div>
        {#each r.files as x (x.file)}
          <div style="padding:8px 0;border-top:1px solid rgba(var(--ink-rgb),.08)">
            <div style="display:flex;gap:10px;font-size:12.5px"><span style="font-weight:500;min-width:0;overflow:hidden;text-overflow:ellipsis">{x.file}</span><span class="muted" style="margin-left:auto;white-space:nowrap">{x.error ? x.error : x.format + ' · ' + x.rows + ' rows · ' + x.added + ' new · ' + x.duplicates + ' duplicates'}</span></div>
            {#if x.skipped && x.skipped.length}
              <div class="muted" style="font-size:11px;margin-top:4px">{x.skippedCount} skipped{(x.skippedCount ?? 0) > x.skipped.length ? ' (first ' + x.skipped.length + ' shown)' : ''}<ul style="margin:4px 0 0;padding-left:16px">{#each x.skipped.slice(0, 8) as s (s.row)}<li>row {s.row}: {s.message}</li>{/each}</ul></div>
            {/if}
          </div>
        {/each}
        <div style="display:flex;gap:8px;justify-content:flex-end;margin-top:16px"><button class="btn btn-primary" onclick={closeModal}>Done</button></div>
      {/if}

    {:else}
      <div class="dim" style="font-size:12.5px;line-height:1.5;margin-bottom:10px">Every CSV at the top level of this folder is imported, and the folder is checked every 10 minutes while the app runs. Files that haven't changed are not re-read.</div>
      <input class="input" bind:value={ui.folderPath} onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); watchFolder() } }} placeholder="/Users/you/Downloads/wealthsimple" spellcheck="false" />
      {#if ui.folderError}<div class="status-err" style="font-size:12px;margin-top:8px">{ui.folderError}</div>{/if}
      <div style="display:flex;gap:8px;margin-top:12px;align-items:center">
        {#if w && w.watching}
          <button class="btn btn-secondary" onclick={stopWatch}>Stop watching</button>
          <button class="btn btn-secondary" disabled={ui.busy === 'folder'} onclick={scanFolder}>Scan now</button>
        {/if}
        <button class="btn btn-primary" style="margin-left:auto" disabled={ui.busy === 'folder'} onclick={watchFolder}>{ui.busy === 'folder' ? 'Scanning…' : w && w.watching ? 'Change folder' : 'Watch folder'}</button>
      </div>
      {#if w && w.watching}<div class="muted" style="font-size:11px;margin-top:12px">Watching {w.path}{w.lastScan ? ' · last scan ' + relTime(w.lastScan) : ''}</div>{/if}
      {#if w && w.files && w.files.length}
        <div class="scroll" style="max-height:220px;margin-top:6px">
          {#each w.files as x (x.file)}<div style="display:flex;gap:10px;font-size:12px;padding:6px 0;border-top:1px solid rgba(var(--ink-rgb),.08)"><span style="min-width:0;overflow:hidden;text-overflow:ellipsis">{x.file}</span><span class="muted" style="margin-left:auto;white-space:nowrap">{(x.format || '') + ' · ' + x.added + ' new · ' + x.duplicates + ' dup'}</span></div>{/each}
        </div>
      {/if}
    {/if}
  </div>
</div>
