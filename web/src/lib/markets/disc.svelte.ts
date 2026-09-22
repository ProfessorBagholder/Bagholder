// Disclosures the News card shows: one listing's (under the chip) or the newest
// across a scope. Either is sent only while the card shows it (live.ts `watchDoc`),
// whole once and then row by row as documents are read -- which the server does, in
// the background, for every followed listing. One listing's disclosures are the same
// resource the instrument page shows, so they come from the same store.
import type { FeedFiling, FilingsFeed } from '../model'
import { watchDoc } from '../live'
export { discStore as discBySym, showDisclosures } from '../trade/discStore.svelte'

export type DiscRow = FeedFiling

export const discFeed = $state<Record<string, { data: FilingsFeed | null }>>({})

/** Show a scope's feed for as long as the caller does. */
export function showDiscFeed(scope: string): () => void {
  if (!discFeed[scope]) discFeed[scope] = { data: null }
  return watchDoc<FilingsFeed>('filings-feed:' + scope, {}, discFeed[scope])
}

/** Whether a feed row's title is still on its way: something is reading, and has not reached it. */
export function discTitleComing(f: DiscRow, scope: string): boolean {
  return !f.subject && !f.enrichFinal && !!discFeed[scope]?.data?.reading
}
