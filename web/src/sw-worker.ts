// The service worker as built (/sw.js): the handlers of `sw.ts` bound to this
// worker's events. A file of its own, imported by nothing, so the built worker is
// one plain script with no import or export, as a classic service worker must be.
import { bind } from './sw'

bind(globalThis as unknown as Parameters<typeof bind>[0])
