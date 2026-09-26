import { describe, expect, it } from 'vitest'

// No count of what a figure left out is ever shown (docs/decisions.md, 2026-09-25):
// a figure the app cannot compute is fixed where it is computed, never labelled
// `N waiting` or explained under a chart. The page reads no left-out count and no
// list of filters a figure skipped.
const sources = import.meta.glob(['/src/**/*.svelte', '/src/**/*.ts', '!/src/**/*.test.ts', '!/src/lib/generated/**'], {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>

const LEFT_OUT = [/\bleftOut\b(?!'\s+in\b)/, /\brealizedLeftOut\b/, /\bskippedFilters\b/, /['"`] waiting\b/, /\}\s*waiting\b/]

describe('the page shows no count of what a figure left out', () => {
  it('reads the page source', () => {
    expect(Object.keys(sources).length).toBeGreaterThan(20)
  })

  it('renders no left-out count and no skipped-filter note', () => {
    const found: string[] = []
    for (const [path, text] of Object.entries(sources)) {
      text.split('\n').forEach((line, i) => {
        if (LEFT_OUT.some((re) => re.test(line))) found.push(`${path}:${i + 1}: ${line.trim()}`)
      })
    }
    expect(found).toEqual([])
  })
})
