import { describe, expect, it } from 'vitest'

// A read that failed is said as a failure, in the failure style, never in the grey
// note that reads as "nothing to show" (SPEC §4, #529). An element styled only
// `.muted` renders no error and no failed outcome; one that can show either
// chooses its style by which it shows (`class={failed ? 'status-err' : 'muted'}`).
// A card whose note is worked out beside its markup (the Short interest card's
// empty line) is held by its own component test.
const sources = import.meta.glob(['/src/**/*.svelte'], { query: '?raw', import: 'default', eager: true }) as Record<string, string>

/** A static `.muted` element on one line, and what it renders. */
const MUTED = /<(\w+)\b[^>]*\bclass="(?:[^"]*\s)?muted(?:\s[^"]*)?"[^>]*>(.*?)<\/\1>/g
/** What names a failure: an error field, or a failed outcome. */
const FAILURE = /(?:\.|\b)error\b|'error' in\b|=== 'failed'|\bfailure\b/

function quietFailures(text: string): string[] {
  const found: string[] = []
  text.split('\n').forEach((line, i) => {
    for (const m of line.matchAll(MUTED)) {
      const rendered = [...m[2].matchAll(/\{([^{}]*)\}/g)].map((e) => e[1])
      if (rendered.some((e) => FAILURE.test(e))) found.push(`${i + 1}: ${m[0].slice(0, 120)}`)
    }
  })
  return found
}

describe('a failure is never said as a quiet note', () => {
  it('reads the page source', () => {
    expect(Object.keys(sources).length).toBeGreaterThan(20)
  })

  it('finds each form it is meant to', () => {
    expect(quietFailures(`<div class="muted" style="x">{view.error}</div>`)).toHaveLength(1)
    expect(quietFailures(`<span class="muted" style="y">{'error' in x ? x.error : fileLine(x.report)}</span>`)).toHaveLength(1)
    expect(quietFailures(`<span class="muted">{x.read.outcome === 'failed' ? x.read.error : fileLine(x.read.report)}</span>`)).toHaveLength(1)
    // a note that is no failure, and a failure in its own style
    expect(quietFailures(`<div class="muted">{loaded.hist.reason || 'No price history for this span.'}</div>`)).toEqual([])
    expect(quietFailures(`<div class={failed ? 'status-err' : 'muted'}>{x.error}</div>`)).toEqual([])
    expect(quietFailures(`<div class="status-err">{view.error}</div>`)).toEqual([])
  })

  it('renders no error or failed outcome in an element styled only as a note', () => {
    const found = Object.entries(sources).flatMap(([path, text]) => quietFailures(text).map((f) => `${path}:${f}`))
    expect(found).toEqual([])
  })
})
