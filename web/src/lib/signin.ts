// What a request does on finding the sign-in in front of Bagholder expired: the
// stream's own renewal (`live.svelte.ts`), handed here because `api.ts`, which finds
// it, cannot import the stream.
let renew: () => void = () => {}

/** What to do when a request finds the sign-in in front of Bagholder expired. */
export function onSignInExpired(go: () => void): void {
  renew = go
}

/** The sign-in in front of Bagholder found expired: renewed. */
export function signInExpired(): void {
  renew()
}
