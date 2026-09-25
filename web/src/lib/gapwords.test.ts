// Every word the engine can say a figure waits on (`Gap::word`, generated to
// generated/gaps.ts) has the short word the page shows after its dash (SPEC.md §3,
// Missing): a new gap on the server fails here until the page can say it.
import { describe, it, expect } from 'vitest'
import { GAP_WORDS } from './generated/gaps'
import { WAITS_WORDS } from './fmt'

describe('the words a figure waits on', () => {
  it('are every gap the engine names', () => {
    expect(GAP_WORDS.filter((w) => !WAITS_WORDS.includes(w))).toEqual([])
  })
})
