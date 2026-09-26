import { describe, it, expect } from 'vitest'
import { scanNotice } from './ui.svelte'
import type { ImportReport, WatchStatus, WatchedFile } from './generated/model_api'

// SPEC §4, the header: after Scan now or Watch folder, a notice says what the scan
// brought in, from the server's report of each file it read in that scan.

const report = (added: number): ImportReport => ({ file: 'f.csv', layout: 'l', account: 'a', rows: added + 3, added, unchanged: 3, linked: 0, ambiguous: [], problems: [] })
const file = (scannedAt: string, read: WatchedFile['read']): WatchedFile => ({ file: 'f.csv', size: 1, modified: 'm', scannedAt, read })
const status = (lastScan: string, files: WatchedFile[]): WatchStatus => ({ path: '/p', watching: true, account: '', lastScan, scanError: '', files })

describe('the notice after a scan of the watched folder', () => {
  it('counts the rows the book did not hold, over every file the scan read', () => {
    const w = status('t2', [file('t2', { outcome: 'imported', report: report(2) }), file('t2', { outcome: 'imported', report: report(5) })])
    expect(scanNotice(w)).toBe('7 new activities imported')
  })

  it('says one activity in the singular', () => {
    expect(scanNotice(status('t2', [file('t2', { outcome: 'imported', report: report(1) })]))).toBe('1 new activity imported')
  })

  it('counts only the files read in this scan, and no file that failed', () => {
    const w = status('t2', [file('t1', { outcome: 'imported', report: report(9) }), file('t2', { outcome: 'failed', error: 'no' }), file('t2', { outcome: 'imported', report: report(0) })])
    expect(scanNotice(w)).toBe('Folder scanned · nothing new')
  })

  it('says nothing new for a folder with no files', () => {
    expect(scanNotice(status('t2', []))).toBe('Folder scanned · nothing new')
  })

  it('says nothing of a scan that failed: the folder dialog says why', () => {
    const w = { ...status('t2', [file('t2', { outcome: 'imported', report: report(4) })]), scanError: 'the folder: gone' }
    expect(scanNotice(w)).toBeNull()
  })
})
