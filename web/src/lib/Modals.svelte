<script lang="ts">
  import { book, status } from './subs.svelte'
  // The three menu modals — Add trade, Import CSV report, Load folder — ported
  // from tradeModalHtml / importModalHtml / folderModalHtml + modalShell.
  import { ui, closeModal, saveTrade, chooseFiles, watchFolder, scanFolder, stopWatch, stopImport } from './ui.svelte'
  import type { ImportReport as FileReport } from './generated/model_api'
  import { symText } from './sym'
  import { relTime } from './fmt'

  import { qty } from './fmt'

  const accounts = $derived((book.data?.accounts ?? []).filter((a) => a.status !== 'closed'))
  // units that arrived without a cost: what an opening balance prices
  const arrivals = $derived((book.data?.waiting ?? []).filter((w) => w.what === 'cost-of-arrival'))
  const arrival = $derived(arrivals.find((a) => a.transaction === ui.tradeForm.arrival) ?? null)
  const f = $derived(ui.tradeForm)
  const r = $derived(ui.importReport)
  const w = $derived(ui.watch)
  const width = $derived(ui.modal === 'trade' ? 460 : 520)
  const total = (k: 'added' | 'linked' | 'unchanged') => (r?.files ?? []).reduce((n, x) => n + ('report' in x ? x.report[k] : 0), 0)
  const fileLine = (x: FileReport) => (x.stopped ? 'Stopped · ' : '') + x.account + ' · ' + x.layout + ' · ' + qty(x.rows) + ' rows · ' + qty(x.added) + ' new · ' + qty(x.linked) + ' linked · ' + qty(x.unchanged) + ' already stored'
  // an import running, this page's or one that runs on from before it opened
  const busy = $derived(ui.busy === 'import' || !!status.data?.importing)
  // how far the import running has come, on its busy button
  const importing = $derived.by(() => {
    const i = status.data?.importing
    if (!i) return 'Reading…'
    if (i.total != null) return 'Reading ' + i.file + ' · ' + qty(i.rows) + ' of ' + qty(i.total) + ' rows'
    if (i.checked > 0) return 'Checking ' + i.file + ' · ' + Math.floor((i.checked * 100) / Math.max(i.received, 1)) + '%'
    return 'Sending ' + i.file + (i.size ? ' · ' + Math.floor((i.received * 100) / i.size) + '%' : '')
  })
</script>

{#snippet notes(x: FileReport)}
  {#each [{ what: 'not linked', rows: x.ambiguous, count: x.ambiguousRows }, { what: 'with a problem', rows: x.problems, count: x.problemRows }] as { what, rows, count } (what)}
    {#if count}
      <div class="muted" style="font-size:11px;margin-top:4px">{qty(count)} {what}{count > rows.length ? ' (first ' + rows.length + ' shown)' : ''}<ul style="margin:4px 0 0;padding-left:16px">{#each rows as n, i (i)}<li>line {n.line}: {n.message}</li>{/each}</ul></div>
    {/if}
  {/each}
{/snippet}

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
        <div><div class="lbl" style="margin-bottom:5px">Account</div>
          <select class="input" bind:value={ui.importAccount} aria-label="Account">
            <option value="">Manual</option>
            {#each accounts.filter((a) => a.name && a.brokerAccount !== 'manual') as a (a.id)}<option value={a.id}>{a.name}</option>{/each}
          </select>
        </div>
        <div style="display:flex;gap:8px;justify-content:flex-end;margin-top:16px">
          <button class="btn btn-secondary" onclick={() => (busy ? stopImport() : closeModal())}>{busy ? 'Stop' : 'Cancel'}</button>
          <button class="btn btn-primary" disabled={busy} onclick={chooseFiles}>{#if busy}<span class="spin"></span>{importing}{:else}Choose files{/if}</button>
        </div>
      {:else}
        <div style="font-size:12.5px">{r.files.length}{r.files.length === 1 ? ' file' : ' files'} · {qty(total('added'))} new · {qty(total('linked'))} linked · {qty(total('unchanged'))} already stored</div>
        {#each r.files as x, i (i)}
          <div style="padding:8px 0;border-top:1px solid rgba(var(--ink-rgb),.08)">
            <div style="display:flex;gap:10px;font-size:12.5px"><span style="font-weight:500;min-width:0;overflow:hidden;text-overflow:ellipsis">{x.file}</span><span class="muted" style="margin-left:auto;min-width:0;text-align:right">{'error' in x ? x.error : fileLine(x.report)}</span></div>
            {#if 'report' in x}{@render notes(x.report)}{/if}
          </div>
        {/each}
        <div style="display:flex;gap:8px;justify-content:flex-end;margin-top:16px"><button class="btn btn-primary" onclick={closeModal}>Done</button></div>
      {/if}

    {:else}
      <div class="dim" style="font-size:12.5px;line-height:1.5;margin-bottom:10px">Every CSV at the top level of this folder is imported, and the folder is checked every 10 minutes while the app runs. Files that haven't changed are not re-read.</div>
      <div style="display:grid;grid-template-columns:1fr 160px;gap:8px">
        <input class="input" bind:value={ui.folderPath} aria-label="Folder" onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); watchFolder() } }} placeholder="/Users/you/Downloads/wealthsimple" spellcheck="false" />
        <select class="input" bind:value={ui.importAccount} aria-label="Account">
          <option value="">Manual</option>
          {#each accounts.filter((a) => a.name && a.brokerAccount !== 'manual') as a (a.id)}<option value={a.id}>{a.name}</option>{/each}
        </select>
      </div>
      {#if ui.folderError}<div class="status-err" style="font-size:12px;margin-top:8px">{ui.folderError}</div>{/if}
      {#if w && w.scanError}<div class="status-err" style="font-size:12px;margin-top:8px">{w.scanError}</div>{/if}
      <div style="display:flex;gap:8px;margin-top:12px;align-items:center">
        {#if w && w.watching}
          <button class="btn btn-secondary" onclick={stopWatch}>Stop watching</button>
          <button class="btn btn-secondary" disabled={ui.busy === 'folder'} onclick={scanFolder}>Scan now</button>
        {/if}
        <button class="btn btn-primary" style="margin-left:auto" disabled={ui.busy === 'folder'} onclick={watchFolder}>{ui.busy === 'folder' ? 'Scanning…' : w && w.watching ? 'Change folder' : 'Watch folder'}</button>
      </div>
      {#if w && w.watching}<div class="muted" style="font-size:11px;margin-top:12px">Watching {w.path}{w.lastScan ? ' · last scan ' + relTime(w.lastScan) : ''}</div>{/if}
      {#if w && w.files.length}
        <div class="scroll" style="max-height:220px;margin-top:6px">
          {#each w.files as x (x.file)}<div style="font-size:12px;padding:6px 0;border-top:1px solid rgba(var(--ink-rgb),.08)"><div style="display:flex;gap:10px"><span style="min-width:0;overflow:hidden;text-overflow:ellipsis">{x.file}</span><span class="muted" style="margin-left:auto;min-width:0;text-align:right">{x.read.outcome === 'failed' ? x.read.error : fileLine(x.read.report)}</span></div>{#if x.read.outcome === 'imported'}{@render notes(x.read.report)}{/if}</div>{/each}
        </div>
      {/if}
    {/if}
  </div>
</div>
