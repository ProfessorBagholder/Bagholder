import { createSubscriber } from 'svelte/reactivity'

// The minute, for text that says how long ago something was ("Synced 4 min ago").
// Nothing else can tell the page that a minute has passed, so this is a timer -- but
// it runs only while something on screen reads it and the tab is visible, wakes on
// the minute rather than on a period, and asks the server nothing.
const subscribe = createSubscriber((update) => {
  let timer: ReturnType<typeof setTimeout> | undefined
  const arm = () => {
    clearTimeout(timer)
    if (document.hidden) return
    timer = setTimeout(() => {
      update()
      arm()
    }, 60000 - (Date.now() % 60000) + 50)
  }
  const seen = () => {
    if (!document.hidden) update() // whatever passed while hidden
    arm()
  }
  arm()
  document.addEventListener('visibilitychange', seen)
  return () => {
    clearTimeout(timer)
    document.removeEventListener('visibilitychange', seen)
  }
})

/** The time now; read inside a template or an effect, it is read again each minute. */
export function minuteNow(): number {
  subscribe()
  return Date.now()
}
