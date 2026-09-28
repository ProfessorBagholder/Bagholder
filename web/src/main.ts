import { mount } from 'svelte'
// Inter, the page's typeface (--font), shipped with the page: the app is local-first and
// asks no font host for it. The variable font with its weight and optical-size axes, the
// file the page was designed on.
import '@fontsource-variable/inter/opsz.css'
import './app.css'
import App from './App.svelte'
import { keptIn } from './lib/live.svelte'
import { follow } from './lib/router.svelte'

// Drawn once what the browser kept is read (a few milliseconds): the first frame is the
// last state, never a placeholder that the kept state then replaces.
// Drawn once in the page's own typeface too, served beside it: never first in a stand-in
// whose widths the typeface then moves. The address as it is when the page is drawn,
// which may have changed while those were read.
const typeface = document.fonts?.load('400 1em "Inter Variable"').catch(() => [])
void Promise.all([keptIn, typeface]).then(() => {
  follow()
  mount(App, { target: document.getElementById('app')! })
})
