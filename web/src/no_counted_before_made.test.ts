import { describe, expect, it } from 'vitest'

// A browser test counts a request once it has been made, never on the timing of the
// action that makes it (#530): a count read on the line straight after a click, a
// key or a fill passes or fails by how fast the page answers. Wait for the request
// (\`page.waitForRequest\`), or poll the count (\`expect.poll\`), first.
const specs = import.meta.glob(['/e2e/*.ts'], { query: '?raw', import: 'default', eager: true }) as Record<string, string>

const ACTION = /await [^\n]*\.(click|press|fill|type|check|dblclick)\(/
const COUNT = /^expect\((\w+)(\.length)?\)\.(toBe|toEqual|toHaveLength)\(/

function countedBeforeMade(text: string): string[] {
  const lines = text.split('\n')
  return lines.flatMap((line, i) => (ACTION.test(line) && COUNT.test((lines[i + 1] ?? '').trim()) ? [`${i + 2}: ${lines[i + 1].trim()}`] : []))
}

describe('a browser test counts a request once it is made', () => {
  it('reads the browser tests', () => {
    expect(Object.keys(specs).length).toBeGreaterThan(20)
  })

  it('finds a count read straight after the action, and nothing else', () => {
    expect(countedBeforeMade(`  await page.getByRole('button').click()\n  expect(asked).toBe(1)`)).toHaveLength(1)
    expect(countedBeforeMade(`  await page.keyboard.press('a')\n  expect(inputs.length).toBe(1)`)).toHaveLength(1)
    expect(countedBeforeMade(`  await page.getByRole('button').click()\n  await made\n  expect(asked).toBe(1)`)).toEqual([])
    expect(countedBeforeMade(`  await page.getByRole('button').click()\n  await expect.poll(() => asked).toBe(1)`)).toEqual([])
  })

  it('reads no count straight after the action that makes the request', () => {
    const found = Object.entries(specs).flatMap(([path, text]) => countedBeforeMade(text).map((f) => `${path}:${f}`))
    expect(found).toEqual([])
  })
})
