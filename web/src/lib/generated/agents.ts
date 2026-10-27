// Generated from the server's agents module. Do not edit: change the Rust type, then
// `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_agents_types`.

export type Provider = "tailscale" | "cloudflare" | "ngrok" | "zrok" | "own";

export type Setup = { provider: Provider, 
/**
 * Cloudflare: the name on their domain the tunnel's DNS route gives; ngrok: the
 * account's dev domain; zrok: the reserved name; their own proxy: the public
 * name. Tailscale's is read from Tailscale itself.
 */
name: string, 
/**
 * Tailscale only: 443, or 8443 when their Funnel already serves something on 443.
 */
funnelPort: number | null, 
/**
 * Their own proxy only: the `Host` it sends, when not the public name.
 */
sendsHost: string | null, };

export type TailscaleSays = { installed: boolean, signedIn: boolean, https: boolean, funnel: boolean, name: string | null, 
/**
 * This machine's first address on its tailnet.
 */
address: string | null, 
/**
 * Whether another program answers on 443 at that address: Funnel's traffic on
 * 443 would reach it, not Bagholder, so Funnel serves on 8443.
 */
httpsPortTaken: boolean, 
/**
 * Why Tailscale could not be asked, in its words (its app not running, say).
 */
said: string, };

export type TunnelState = { 
/**
 * The provider of the tunnel running, if one is.
 */
running: Provider | null, 
/**
 * The command it was started with.
 */
command: Array<string>, 
/**
 * Its last lines, as it wrote them.
 */
lines: Array<string>, 
/**
 * How the last one ended, when it ended on its own.
 */
exited: string | null, };

export type Ready = { "state": "ready" } | { "state": "unconfirmed", said: string, } | { "state": "failed", said: string, };

export type Waiting = { id: string, code: string, 
/**
 * Who is asking: the metadata document's host, or a registered client's redirect host.
 */
who: string, name: string, 
/**
 * Whether the name was read from the agent's own published document (`true`),
 * or given by the agent when it registered, unchecked.
 */
checked: boolean, 
/**
 * The host the agent is sent back to.
 */
redirectHost: string, madeAt: string, };

export type Connected = { id: string, name: string, 
/**
 * `sign-in`, `mcp-key` or `api-key`.
 */
way: string, 
/**
 * For a sign-in, who it was granted to.
 */
who: string, madeAt: string, lastUsedAt: string | null, cutOffAt: string | null, };

export type ProgramFound = { provider: Provider, 
/**
 * Where it was found; none when it is not installed.
 */
path: string | null, };

export type RefusedCount = { why: string, count: number, };

export type AgentsStatus = { on: boolean, port: number | null, 
/**
 * The tunnel the person chose, and what they entered for it.
 */
setup: Setup | null, 
/**
 * The address an agent is given, once its name is known.
 */
address: string, programs: Array<ProgramFound>, 
/**
 * What Tailscale said of this machine when last asked: asked when Tailscale is
 * chosen, when its tunnel starts, and when the person asks again.
 */
tailscale: TailscaleSays | null, tunnel: TunnelState, ready: Ready | null, waiting: Array<Waiting>, 
/**
 * Every agent that may read: each grant and each key.
 */
allowed: Array<Connected>, 
/**
 * The requests refused from callers no grant or key names since the address
 * opened, counted by reason (none is written).
 */
refused: Array<RefusedCount>, 
/**
 * What could not be read, in words.
 */
error: string, };

export type SwitchAsked = { on: boolean, };

export type AgentId = { id: string, };

export type KeyAsked = { name: string, 
/**
 * `mcp` or `api`.
 */
way: string, };

export type MadeKey = { id: string, name: string, way: string, key: string, };

export type LogAsked = { agent: string | null, };

export type LogLine = { at: string, tool: string | null, arguments: string | null, refusal: string | null, };

export type AgentLog = { lines: Array<LogLine>, };
