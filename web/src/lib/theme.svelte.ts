// The three themes, ported from ledger.html (THEMES + applyTheme). Nocturne is
// the default (:root, no data-theme attribute); Midnight and Light set
// html[data-theme]. Persisted so a reload keeps the choice.
export const THEMES: [string, string, string][] = [
  ['nocturne', 'Nocturne', '#161826'],
  ['midnight', 'Midnight', '#0b0d12'],
  ['light', 'Light', '#eef0f5'],
]

function initial(): string {
  try {
    const v = localStorage.getItem('bh2.theme')
    if (v && THEMES.some((t) => t[0] === v)) return v
  } catch {
    /* ignore */
  }
  return 'nocturne'
}

export const theme = $state<{ name: string }>({ name: initial() })

export function applyTheme(name: string): void {
  theme.name = THEMES.some((t) => t[0] === name) ? name : 'nocturne'
  if (typeof document !== 'undefined') {
    if (theme.name === 'nocturne') document.documentElement.removeAttribute('data-theme')
    else document.documentElement.setAttribute('data-theme', theme.name)
  }
  try {
    localStorage.setItem('bh2.theme', theme.name)
  } catch {
    /* ignore */
  }
}

// Apply the saved theme as soon as this module loads, so the first paint is right.
if (typeof document !== 'undefined') applyTheme(theme.name)
