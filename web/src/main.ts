import { mount } from 'svelte'
// Inter, the page's typeface (--font), shipped with the page: the app is local-first and
// asks no font host for it. The variable font with its weight and optical-size axes, the
// file the page was designed on.
import '@fontsource-variable/inter/opsz.css'
import './app.css'
import App from './App.svelte'

const app = mount(App, {
  target: document.getElementById('app')!,
})

export default app
