// `bin/npm-audit.mjs` over recorded reports: the listed advisory passes, one not
// listed fails at any severity, and a listed one no longer reported fails.
//
//     node --test bin/npm-audit.test.mjs
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { judge } from './npm-audit.mjs'

const list = readFileSync(new URL('../web/audit.toml', import.meta.url), 'utf8')
const report = (f) => JSON.parse(readFileSync(new URL(`./npm-audit/${f}`, import.meta.url), 'utf8'))

test('the advisory the list names, reported, passes', () => {
  assert.deepEqual(judge(report('recorded.json'), list), { unlisted: [], stale: [] })
})

test('an advisory the list does not name fails, a low one too', () => {
  assert.deepEqual(judge(report('edited-two-advisories.json'), list), { unlisted: ['GHSA-2222-3333-4444'], stale: [] })
})

test('a listed advisory the audit no longer reports fails, so it is removed', () => {
  assert.deepEqual(judge({ auditReportVersion: 2, vulnerabilities: {} }, list), { unlisted: [], stale: ['GHSA-gcqx-w48m-w2h4'] })
})

test('a report npm could not give fails rather than passing as empty', () => {
  assert.throws(() => judge({ error: { code: 'ENOAUDIT' } }, list), /did not answer/)
})
