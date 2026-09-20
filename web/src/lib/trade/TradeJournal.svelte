<script lang="ts">
  import type { Trade } from '../model'
  import { saveJournal } from '../state.svelte'

  let { trade }: { trade: Trade } = $props()

  const GRADES = ['A', 'B', 'C', 'D', 'F']
  let tagDraft = $state('')
  let thesisTimer: ReturnType<typeof setTimeout> | undefined

  function onThesis(e: Event) {
    const v = (e.target as HTMLTextAreaElement).value
    clearTimeout(thesisTimer)
    thesisTimer = setTimeout(() => {
      if (trade.thesis !== v) saveJournal(trade.id, { thesis: v })
    }, 600)
  }
  function setGrade(g: string) {
    saveJournal(trade.id, { grade: trade.grade === g ? '' : g })
  }
  function addTag() {
    const v = tagDraft.trim()
    if (v && !(trade.tags ?? []).includes(v)) saveJournal(trade.id, { tags: [...(trade.tags ?? []), v] })
    tagDraft = ''
  }
  function removeTag(tag: string) {
    saveJournal(trade.id, { tags: (trade.tags ?? []).filter((x) => x !== tag) })
  }
</script>

<div class="card">
  <h5>Journal</h5>

  <label class="field">
    <span>Thesis</span>
    <textarea value={trade.thesis ?? ''} oninput={onThesis} placeholder="Why this trade…" rows="3"></textarea>
  </label>

  <div class="field">
    <span>Grade</span>
    <div class="grades">
      {#each GRADES as g (g)}
        <button class="gchip g{g}" class:on={trade.grade === g} onclick={() => setGrade(g)}>{g}</button>
      {/each}
    </div>
  </div>

  <div class="field">
    <span>Tags</span>
    <div class="tags">
      {#each trade.tags ?? [] as tag (tag)}
        <span class="tag">{tag}<button class="x" onclick={() => removeTag(tag)} aria-label="Remove tag">×</button></span>
      {/each}
      <input
        class="taginput"
        bind:value={tagDraft}
        onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); addTag() } }}
        placeholder="Add tag…"
      />
    </div>
  </div>
</div>

<style>
  .card { background: #141924; border: 1px solid #1c2230; border-radius: 12px; padding: 16px 18px; display: flex; flex-direction: column; gap: 14px; }
  .card h5 { margin: 0; font-size: 14px; font-weight: 600; }
  .field { display: flex; flex-direction: column; gap: 6px; }
  .field > span { color: #8b93a7; font-size: 11px; text-transform: uppercase; letter-spacing: 0.03em; }
  textarea { background: #0b0e14; border: 1px solid #1c2230; border-radius: 8px; color: #e6e9ef; font: inherit; font-size: 13px; padding: 8px 10px; resize: vertical; }
  textarea:focus { outline: none; border-color: #2a3242; }
  .grades { display: flex; gap: 6px; }
  .gchip { width: 30px; height: 28px; border: 1px solid #1c2230; background: #0b0e14; color: #8b93a7; border-radius: 6px; font-weight: 600; cursor: pointer; }
  .gchip.on.gA { background: rgba(62,207,142,0.2); color: #3ecf8e; border-color: #3ecf8e; }
  .gchip.on.gB { background: rgba(120,199,120,0.18); color: #86c682; border-color: #86c682; }
  .gchip.on.gC { background: rgba(242,163,65,0.18); color: #f2a341; border-color: #f2a341; }
  .gchip.on.gD { background: rgba(240,140,90,0.18); color: #f08c5a; border-color: #f08c5a; }
  .gchip.on.gF { background: rgba(240,97,109,0.18); color: #f0616d; border-color: #f0616d; }
  .tags { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
  .tag { background: #232a38; border-radius: 5px; padding: 2px 4px 2px 8px; font-size: 12px; display: inline-flex; align-items: center; gap: 4px; }
  .x { background: none; border: 0; color: #8b93a7; cursor: pointer; font-size: 14px; line-height: 1; padding: 0 2px; }
  .x:hover { color: #f0616d; }
  .taginput { background: #0b0e14; border: 1px solid #1c2230; border-radius: 6px; color: #e6e9ef; font: inherit; font-size: 12px; padding: 4px 8px; width: 110px; }
  .taginput:focus { outline: none; border-color: #2a3242; }
</style>
