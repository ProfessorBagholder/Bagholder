import { describe, expect, it } from 'vitest'
import { localDay, localWhen } from './fmt'

// The viewer's zone is Toronto for every page test (vite.config.ts, test.env).
describe('times in the viewer zone', () => {
  it('an instant is shown as the viewer day and time, not UTC', () => {
    // 02:30 UTC is still the evening before in Toronto (EDT, UTC−4)
    expect(localWhen('2026-07-02T02:30:00+00:00', '2026-07-02')).toEqual({ day: '2026-07-01', time: '22:30' })
    expect(localWhen('2026-01-15T15:00:00Z', '2026-01-15')).toEqual({ day: '2026-01-15', time: '10:00' })
  })
  it('a bare date keeps its day and has no time', () => {
    expect(localWhen('2026-03-04', '2026-03-04')).toEqual({ day: '2026-03-04', time: '' })
    expect(localWhen('', '2026-03-04')).toEqual({ day: '2026-03-04', time: '' })
  })
  it("today is the viewer's calendar day, which differs from UTC's in the evening", () => {
    expect(localDay(new Date('2026-07-02T02:30:00Z'))).toBe('2026-07-01')
  })
})
