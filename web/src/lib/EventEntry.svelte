<script lang="ts">
  // What a corporate event did to cost, entered by the person while the event waits
  // on it (SPEC.md §2, What you enter): for a spin-off, the share of the parent's
  // cost this holding takes; for a return of capital, the capital returned a unit.
  import type { Model } from './model'
  import { call } from './api'
  import { flash } from './ui.svelte'
  import { symText } from './sym'

  type Waiting = Model['waiting'][number]
  let { event, model }: { event: Waiting; model: Model } = $props()

  let kind = $state<'spin-off' | 'return-of-capital'>('spin-off')
  let parent = $state('')
  let share = $state('')
  let perUnit = $state('')
  let error = $state('')
  let busy = $state(false)

  // the holdings of the same account it may have come out of
  const parents = $derived.by(() => {
    const seen = new Map<string, string>()
    for (const r of [...model.positions, ...model.trades]) {
      if (r.accountId === event.account && r.instrument !== event.instrument && !seen.has(r.instrument)) seen.set(r.instrument, r.symbol)
    }
    return [...seen.entries()].map(([id, symbol]) => ({ id, symbol })).sort((a, b) => a.symbol.localeCompare(b.symbol))
  })

  async function save() {
    const text = (v: string) => v.trim().replace(/[$,]/g, '')
    if (kind === 'spin-off' ? !parent || !text(share) : !text(perUnit)) {
      error = kind === 'spin-off' ? 'The parent and its share of cost are required.' : 'The capital returned a unit is required.'
      return
    }
    busy = true
    error = ''
    const r = await call('POST /api/entries', {
      body:
        kind === 'spin-off'
          ? { entry: 'spin-off', event: event.transaction, parent, children: [{ instrument: event.instrument, costShare: text(share) }] }
          : { entry: 'return-of-capital', distribution: event.transaction, perUnit: text(perUnit) },
    })
    busy = false
    if (!r.ok) {
      error = r.error || 'Could not save it.'
      return
    }
    flash('Entered')
  }
</script>

<div style="margin-top:14px">
  <div class="lbl" style="margin-bottom:6px">Corporate event · {event.day}</div>
  <div class="seg" style="margin-bottom:8px">
    {#each [['spin-off', 'Spin-off'], ['return-of-capital', 'Return of capital']] as o (o[0])}<label class="seg-opt" class:on={kind === o[0]}><input type="radio" bind:group={kind} value={o[0]} />{o[1]}</label>{/each}
  </div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div style="display:flex;gap:8px;align-items:center" onkeydown={(e) => { if (e.key === 'Enter' && (e.target as HTMLElement).tagName === 'INPUT') { e.preventDefault(); save() } }}>
    {#if kind === 'spin-off'}
      <select class="input" bind:value={parent} aria-label="Parent" style="flex:1">
        <option value="">Parent</option>
        {#each parents as p (p.id)}<option value={p.id}>{symText(p.symbol)}</option>{/each}
      </select>
      <input class="input" inputmode="decimal" bind:value={share} aria-label="Share of cost" placeholder="Share of cost" style="width:120px" />
    {:else}
      <input class="input" inputmode="decimal" bind:value={perUnit} aria-label="Capital returned a unit" placeholder="Per unit" style="flex:1" />
      <span class="muted" style="font-size:12px">{event.currency}</span>
    {/if}
    <button class="btn btn-primary" disabled={busy} onclick={save} style="font-size:12px;padding:5px 12px">{busy ? 'Saving…' : 'Enter'}</button>
  </div>
  {#if error}<div class="status-err" style="font-size:12px;margin-top:6px">{error}</div>{/if}
</div>
