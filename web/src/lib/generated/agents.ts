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

export type Address = { provider: Provider, 
/**
 * `https://{host}`, once its name is known.
 */
url: string, 
/**
 * Served on 443: every agent reaches it, and Claude.ai reaches no other port
 * (anthropics/claude-ai-mcp#1155).
 */
port443: boolean, running: boolean, ready: Ready | null, 
/**
 * How its tunnel last ended, when it ended on its own.
 */
exited: string | null, 
/**
 * Its tunnel's last lines.
 */
lines: Array<string>, };

export type CloudflareSays = { 
/**
 * Bagholder's own sign-in to Cloudflare is kept.
 */
signedIn: boolean, 
/**
 * A sign-in under way: the page Cloudflare asks the person to open, once it said it.
 */
signingIn: boolean, signInUrl: string | null, 
/**
 * Why the last sign-in or setup did not finish, in plain words.
 */
said: string, 
/**
 * The domain the sign-in was made for, once Cloudflare said its name.
 */
domain: string | null, };

export type Installing = { provider: Provider, 
/**
 * Empty while it runs; why it failed, once it did.
 */
failed: string, };

export type TailscaleSays = { installed: boolean, signedIn: boolean, https: boolean, funnel: boolean, name: string | null, 
/**
 * This machine's first address on its tailnet.
 */
address: string | null, 
/**
 * Whether another program answers on 443 at that address: Funnel's traffic on
 * 443 would reach it, not Bagholder. The panel says so and offers 8443.
 */
httpsPortTaken: boolean, 
/**
 * Why Tailscale could not be asked, in its words (its app not running, say).
 */
said: string, 
/**
 * On Linux, whether Funnel may be started by this user: `None` elsewhere, where
 * no such step is; `Some(false)` until it is known to be so (root), never taken
 * for done before a start is refused (brief 80 §5.6).
 */
operator: boolean | null, };

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
checked: boolean, madeAt: string, };

export type Connected = { id: string, name: string, 
/**
 * `sign-in`, `mcp-key` or `api-key`.
 */
way: string, 
/**
 * For a sign-in, who it was granted to.
 */
who: string, 
/**
 * The server it was made for (`https://{host}`). Empty for a key made before
 * keys named theirs and not used since.
 */
at: string, madeAt: string, lastUsedAt: string | null, cutOffAt: string | null, };

export type ProgramFound = { provider: Provider, 
/**
 * Where it was found; none when it is not installed.
 */
path: string | null, };

export type RefusedCount = { why: string, count: number, };

export type AgentsStatus = { on: boolean, port: number | null, 
/**
 * Every address agents reach Bagholder at, each with its tunnel (brief 81 §3).
 */
addresses: Array<Address>, programs: Array<ProgramFound>, 
/**
 * The computer Bagholder runs on, by its name: what a program missing is
 * missing from (`tunnel::computer`).
 */
computer: string, 
/**
 * Its system (`linux`, `macos` or `windows`): what Tailscale is installed
 * with there.
 */
os: string, 
/**
 * The programs Bagholder is installing there, or failed to (`agents::install`).
 */
installing: Array<Installing>, 
/**
 * What Tailscale said of this machine when last asked: asked when Tailscale is
 * chosen, when its tunnel starts, and when the person asks again.
 */
tailscale: TailscaleSays | null, 
/**
 * Cloudflare as Bagholder sets it up: signed in, or signing in.
 */
cloudflare: CloudflareSays, waiting: Array<Waiting>, 
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
 * Whether sign-in is paused: codes were typed wrong too often, on any request,
 * and no code is taken until the person resumes it (brief 81 §2).
 */
codesPaused: boolean, 
/**
 * What could not be read, in words.
 */
error: string, };

export type SwitchAsked = { on: boolean, };

export type ProviderAsked = { provider: Provider, };

export type AgentId = { id: string, };

export type Renamed = { id: string, name: string, };

export type NgrokToken = { token: string, };

export type KeyAsked = { name: string, 
/**
 * `mcp` or `api`.
 */
way: string, 
/**
 * The server it is for (`https://{host}`): it is taken only there.
 */
at: string, };

export type MadeKey = { id: string, name: string, way: string, key: string, };

export type LogAsked = { agent: string | null, };

export type LogLine = { at: string, tool: string | null, arguments: string | null, refusal: string | null, };

export type AgentLog = { lines: Array<LogLine>, };

export type RemoteStatus = { 
/**
 * The address, once set up: `https://{name}`.
 */
url: string, 
/**
 * The one email let in.
 */
email: string, 
/**
 * Whether the person gave Bagholder a token for Access that Cloudflare took.
 */
token: boolean, 
/**
 * Cloudflare's page for that token, filled in.
 */
tokenPage: string, 
/**
 * The token found the account has no Zero Trust team yet, which only
 * Cloudflare's dashboard makes (`team_page`).
 */
noTeam: boolean, teamPage: string, 
/**
 * The person's API tokens at Cloudflare, where the token made for Bagholder
 * is deleted once the address is removed.
 */
tokensPage: string, 
/**
 * Bagholder's own GitHub sign-in: its name once Cloudflare holds it as a login
 * method, else empty.
 */
github: string, 
/**
 * Its manifest waits on the person in GitHub's tab.
 */
makingGithub: boolean, 
/**
 * Why Cloudflare didn't take the GitHub app GitHub made, until it does.
 */
githubSaid: string, 
/**
 * The GitHub app's own page, where only the person can delete it.
 */
githubPage: string, 
/**
 * The tunnel runs with Bagholder.
 */
on: boolean, running: boolean, 
/**
 * What the check from outside found: none while it is still trying, since the
 * tunnel started.
 */
ready: boolean | null, 
/**
 * What the check's last try found, while it keeps trying.
 */
trying: string, 
/**
 * Why it isn't reachable, or how the tunnel ended, in words.
 */
said: string, };

export type TokenAsked = { token: string, };

export type SetUpAsked = { 
/**
 * The name before the domain ("app"), or the whole address.
 */
name: string, 
/**
 * The one email Access lets in.
 */
email: string, };

export type RemoteSwitch = { on: boolean, };

export type GitHubAsked = { 
/**
 * The page's own address (`location.origin`): where GitHub sends the person
 * back once the app is made.
 */
origin: string, };

export type GitHubManifest = { ok: boolean, error?: string, action: string, manifest: string, };
