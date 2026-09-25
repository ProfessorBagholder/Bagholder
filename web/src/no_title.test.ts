import { describe, expect, it } from 'vitest'

// No browser tooltip anywhere on the page (docs/decisions.md, 2026-09-25): a `title`
// attribute, an SVG <title> or a `title` set from script shows the browser's own
// unstyled tooltip. A hover that is needed goes through the app's styled tooltip.
const sources = import.meta.glob(['/src/**/*.svelte', '/src/**/*.ts', '!/src/**/*.test.ts'], {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>

const BROWSER_TOOLTIP = [
  /\stitle\s*=/, // an attribute, quoted or {expression}
  /<title[\s>]/, // an SVG title element
  /\.title\s*=[^=]/, // a DOM property set from script
  /setAttribute\(\s*['"`]title['"`]/,
  /\{\s*title\s*\}/, // the attribute shorthand {title}
]

describe('the page carries no browser tooltip', () => {
  it('reads the page source', () => {
    expect(Object.keys(sources).length).toBeGreaterThan(20)
  })

  it('has no title attribute, SVG title or title set from script', () => {
    const found: string[] = []
    for (const [path, text] of Object.entries(sources)) {
      text.split('\n').forEach((line, i) => {
        if (BROWSER_TOOLTIP.some((re) => re.test(line))) found.push(`${path}:${i + 1}: ${line.trim()}`)
      })
    }
    expect(found).toEqual([])
  })

  it('catches each form it looks for', () => {
    const forms = [
      '<span title="x">',
      '<span title={t}>',
      '<svg><title>x</title></svg>',
      'el.title = t',
      "el.setAttribute('title', t)",
      '<span {title}>',
    ]
    for (const f of forms) expect(BROWSER_TOOLTIP.some((re) => re.test(f)), f).toBe(true)
  })
})
