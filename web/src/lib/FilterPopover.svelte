<script lang="ts">
  import type { Options } from './model'
  import { filters, resetFilters, activeCount, type ListKey, type RangeKey } from './filters.svelte'
  import { loadModel } from './state.svelte'

  let { options, onclose }: { options: Options; onclose: () => void } = $props()

  let timer: ReturnType<typeof setTimeout> | undefined
  function apply() {
    clearTimeout(timer)
    timer = setTimeout(() => loadModel(), 250)
  }

  const LISTS: { key: ListKey; label: string; opt: keyof Options }[] = [
    { key: 'account', label: 'Account', opt: 'accounts' },
    { key: 'kind', label: 'Kind', opt: 'kinds' },
    { key: 'exchange', label: 'Exchange', opt: 'exchanges' },
    { key: 'grade', label: 'Grade', opt: 'grades' },
    { key: 'tag', label: 'Tag', opt: 'tags' },
    { key: 'side', label: 'Side', opt: 'sides' },
    { key: 'result', label: 'Result', opt: 'results' },
  ]
  const RANGES: { key: RangeKey; label: string }[] = [
    { key: 'price', label: 'Price' }, { key: 'hold', label: 'Hold (days)' }, { key: 'pnl', label: 'P&L' }, { key: 'qty', label: 'Qty' },
  ]

  function toggleList(key: ListKey, value: string) {
    const arr = filters.lists[key]
    const i = arr.indexOf(value)
    if (i >= 0) arr.splice(i, 1)
    else arr.push(value)
    apply()
  }
  function toggleYear(y: string) {
    const i = filters.years.indexOf(y)
    if (i >= 0) filters.years.splice(i, 1)
    else filters.years.push(y)
    apply()
  }
  function setRange(key: RangeKey, op: string, v: string) {
    filters.ranges[key] = { op, v: v === '' ? null : Number(v) }
    apply()
  }
  function reset() { resetFilters(); apply() }
  const acctLabel = (s: string) => s.replace(/_/g, ' ').toLowerCase().replace(/\b\w/g, (c) => c.toUpperCase())
</script>

<div class="scrim" role="presentation" onclick={onclose}></div>
<div class="pop" role="dialog" aria-label="Filters">
  <div class="phead">
    <h5>Filters {#if activeCount()}<span class="count">{activeCount()}</span>{/if}</h5>
    <div class="pactions">
      {#if activeCount()}<button class="reset" onclick={reset}>Reset</button>{/if}
      <button class="close" onclick={onclose} aria-label="Close">×</button>
    </div>
  </div>

  <div class="pbody">
    <label class="search">
      <input placeholder="Search symbol or name…" bind:value={filters.search} oninput={apply} />
    </label>

    {#if options.years?.length}
      <div class="grp">
        <span class="glabel">Year</span>
        <div class="chips">
          {#each options.years as y (y)}
            <button class="chip" class:on={filters.years.includes(y)} onclick={() => toggleYear(y)}>{y}</button>
          {/each}
        </div>
      </div>
    {/if}

    {#each LISTS as l (l.key)}
      {#if (options[l.opt] as string[])?.length}
        <div class="grp">
          <span class="glabel">{l.label}</span>
          <div class="chips">
            {#each options[l.opt] as string[] as v (v)}
              <button class="chip" class:on={filters.lists[l.key].includes(v)} onclick={() => toggleList(l.key, v)}>{l.key === 'account' ? acctLabel(v) : v}</button>
            {/each}
          </div>
        </div>
      {/if}
    {/each}

    <div class="grp">
      <span class="glabel">Ranges</span>
      <div class="ranges">
        {#each RANGES as r (r.key)}
          <div class="range">
            <span>{r.label}</span>
            <select value={filters.ranges[r.key].op} onchange={(e) => setRange(r.key, (e.target as HTMLSelectElement).value, String(filters.ranges[r.key].v ?? ''))}>
              <option value=">">&gt;</option><option value="<">&lt;</option><option value="=">=</option>
            </select>
            <input type="number" value={filters.ranges[r.key].v ?? ''} oninput={(e) => setRange(r.key, filters.ranges[r.key].op, (e.target as HTMLInputElement).value)} />
          </div>
        {/each}
      </div>
    </div>

    <div class="grp">
      <span class="glabel">Closed between</span>
      <div class="dates">
        <input type="date" bind:value={filters.from} onchange={apply} />
        <span>–</span>
        <input type="date" bind:value={filters.to} onchange={apply} />
      </div>
    </div>
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; background: rgba(0,0,0,0.4); z-index: 40; }
  .pop { position: fixed; top: 60px; right: 20px; width: 380px; max-height: 80vh; overflow-y: auto; background: #141924; border: 1px solid #2a3242; border-radius: 12px; z-index: 41; box-shadow: 0 12px 40px rgba(0,0,0,0.5); }
  .phead { position: sticky; top: 0; background: #141924; display: flex; align-items: center; justify-content: space-between; padding: 14px 16px; border-bottom: 1px solid #1c2230; }
  .phead h5 { margin: 0; font-size: 14px; font-weight: 600; }
  .count { background: #3ecf8e; color: #08110b; border-radius: 10px; padding: 0 6px; font-size: 11px; margin-left: 6px; }
  .pactions { display: flex; align-items: center; gap: 8px; }
  .reset { background: none; border: 0; color: #8b93a7; font: inherit; font-size: 12px; cursor: pointer; }
  .reset:hover { color: #e6e9ef; }
  .close { background: none; border: 0; color: #8b93a7; font-size: 18px; cursor: pointer; line-height: 1; }
  .pbody { padding: 12px 16px 20px; display: flex; flex-direction: column; gap: 14px; }
  .search input, .dates input, .range input, .range select { background: #0b0e14; border: 1px solid #1c2230; border-radius: 7px; color: #e6e9ef; font: inherit; font-size: 12px; padding: 6px 8px; }
  .search input { width: 100%; }
  .grp { display: flex; flex-direction: column; gap: 6px; }
  .glabel { color: #8b93a7; font-size: 11px; text-transform: uppercase; letter-spacing: 0.03em; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .chip { background: #1c2230; border: 1px solid transparent; color: #c4cbd8; border-radius: 6px; padding: 3px 9px; font-size: 12px; cursor: pointer; }
  .chip.on { background: rgba(62,207,142,0.18); color: #3ecf8e; border-color: #3ecf8e; }
  .ranges { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  .range { display: flex; align-items: center; gap: 5px; font-size: 12px; }
  .range span { color: #8b93a7; flex: 1; }
  .range input { width: 60px; }
  .dates { display: flex; align-items: center; gap: 8px; }
  .dates span { color: #8b93a7; }
</style>
