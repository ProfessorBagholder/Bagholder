// What this page and the server agree they speak. The server names its own in the
// status; when the two differ the page is a build the running server is not (an
// update put new files under an old process), and says so rather than misreading it.
// Held equal to `PROTOCOL` in rust/crates/server/src/app.rs by that crate's tests.
export const PROTOCOL = '2026-09-19.1'
