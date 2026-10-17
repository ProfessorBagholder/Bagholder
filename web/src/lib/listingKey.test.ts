import { describe, expect, it } from 'vitest'
import { listingKey } from './listingKey'
import vectors from './generated/listing_vectors.json'

describe('a listing key', () => {
  it('is the key the server makes of every generated symbol and venue', () => {
    const rows = vectors as [string, string, string | null][]
    expect(rows.length).toBeGreaterThan(500)
    const wrong = rows.filter(([s, v, k]) => listingKey(s, v) !== k).map(([s, v, k]) => `${JSON.stringify(s)} on ${JSON.stringify(v)}: ${listingKey(s, v)} for ${k}`)
    expect(wrong).toEqual([])
  })
})
