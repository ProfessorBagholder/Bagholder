// `npm audit --json`'s report held to the advisories the project lets pass
// (`web/audit.toml`, in `cargo audit`'s own form: `[advisories] ignore = [...]`, each
// with why it cannot be reached and when it leaves). An advisory not listed fails, at
// any severity; so does a listed one the audit no longer reports, so a stale entry is
// removed rather than left to excuse a later one. Node alone, nothing installed.
//
//     node bin/npm-audit.mjs <npm audit --json report> <list>
import { readFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'

/** The ids in the list's `ignore` array: its quoted strings, its comments aside. */
export function listed(toml) {
  const body = toml.split('\n').map((l) => l.replace(/#.*$/, '')).join('\n')
  const ignore = /ignore\s*=\s*\[([\s\S]*?)\]/.exec(body)
  return new Set(ignore ? [...ignore[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]) : [])
}

/** Every advisory the report names, by its id (`GHSA-…`), whatever its severity. */
export function reported(report) {
  if (report.error) throw new Error(`npm audit did not answer: ${JSON.stringify(report.error)}`)
  const ids = new Set()
  for (const v of Object.values(report.vulnerabilities ?? {})) {
    for (const via of v.via ?? []) {
      // a package named here is reached through another, whose own entry names the advisory
      if (typeof via !== 'object') continue
      const id = /GHSA-[0-9a-z]{4}-[0-9a-z]{4}-[0-9a-z]{4}/i.exec(via.url ?? '')?.[0]
      if (!id) throw new Error(`an advisory with no id: ${JSON.stringify(via)}`)
      ids.add(id)
    }
  }
  return ids
}

/** What fails: each advisory reported and not listed, and each listed and not reported. */
export function judge(report, toml) {
  const [have, allowed] = [reported(report), listed(toml)]
  return { unlisted: [...have].filter((id) => !allowed.has(id)).sort(), stale: [...allowed].filter((id) => !have.has(id)).sort() }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [report, list] = process.argv.slice(2)
  const { unlisted, stale } = judge(JSON.parse(readFileSync(report, 'utf8')), readFileSync(list, 'utf8'))
  for (const id of unlisted) console.error(`${id}: reported by npm audit and not in ${list}: fix it, or list it with why it cannot be reached and when it leaves`)
  for (const id of stale) console.error(`${id}: in ${list} and no longer reported: remove it`)
  process.exit(unlisted.length || stale.length ? 1 : 0)
}
