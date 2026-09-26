// Generated from rust/crates/ws/src/session.rs, session.rs and login.rs. Do not edit: change
// the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_session_types`.

export type IdentityKeys = { identity_canonical_id: string, identityCanonicalId: string, canonical_id: string, identity_id: string, resource_owner_id: string, sub: string, };

export type Expiry = number | string;

export type Capture = { access_token: string, refresh_token: string, client_id: string, wssdi: string, session_id: string, user_agent: string, expires_at: Expiry | null, identity_canonical_id: string, identityCanonicalId: string, canonical_id: string, identity_id: string, resource_owner_id: string, sub: string, };

export type LoginInput = { kind?: string, x?: number, y?: number, text?: string, key?: string, deltaY?: number, };

export type StartLoginAnswer = { ok: boolean, reused?: boolean, error?: string, };

export type CancelLoginAnswer = { ok: boolean, cancelled: boolean, };

export type RefreshAnswer = { ok: boolean, error: string, connected: boolean, };

export type SyncAnswer = { ok: boolean, error?: string, syncing?: boolean, };
