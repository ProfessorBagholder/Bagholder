import { describe, it, expect } from 'vitest'
import { scanNotice } from './ui.svelte'
import type { WatchStatus } from './generated/model_api'

// SPEC §4, the header: after Scan now or Watch folder, a notice says what the scan
// brought in, as the server counts it (`lastScanAdded`); the page counts nothing.

const status = (lastScanAdded: number, scanError = ''): WatchStatus => ({ path: '/p', watching: true, account: '', lastScan: 't2', scanError, lastScanAdded, files: [] })

describe('the notice after a scan of the watched folder', () => {
  it('says how many rows the scan added that the book did not hold', () => {
    expect(scanNotice(status(7))).toBe('7 new activities imported')
  })

  it('says one activity in the singular', () => {
    expect(scanNotice(status(1))).toBe('1 new activity imported')
  })

  it('says nothing new when the scan added nothing', () => {
    expect(scanNotice(status(0))).toBe('Folder scanned · nothing new')
  })

  it('says nothing of a scan that failed: the folder dialog says why', () => {
    expect(scanNotice(status(4, 'the folder: gone'))).toBeNull()
  })
})
