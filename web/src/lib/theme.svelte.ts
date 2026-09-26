// Theme switching, done the idiomatic Svelte 5 way:
//   - `theme.name` is the single source of truth (reactive $state), persisted.
//   - the DOM is *projected from that state by an effect*, never mutated
//     imperatively inside the setter. Because the theme is app-global (not owned
//     by any one component), the effect lives in a module-level `$effect.root`,
//     which owns it for the app's lifetime.
//   - index.html sets html[data-theme] inline before the bundle loads, so the
//     first paint already has the saved theme (no flash) and this effect just
//     keeps it in sync thereafter.
// Nocturne is the default (:root with no data-theme); Midnight and Light set
// html[data-theme]. app.css defines the token set for each.

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

// Update the source of truth and persist it; the root effect below reflects it
// to the DOM. (This is what the menu calls.)
export function applyTheme(name: string): void {
  theme.name = THEMES.some((t) => t[0] === name) ? name : 'nocturne'
  try {
    localStorage.setItem('bh2.theme', theme.name)
  } catch {
    /* ignore */
  }
}

// Project theme state onto html[data-theme], reactively, for the app's lifetime.
if (typeof document !== 'undefined') {
  $effect.root(() => {
    $effect(() => {
      if (theme.name === 'nocturne') document.documentElement.removeAttribute('data-theme')
      else document.documentElement.setAttribute('data-theme', theme.name)
    })
  })
}
