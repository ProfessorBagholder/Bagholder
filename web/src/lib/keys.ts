// A key for each row of a list, for `{#each ... (key)}`: the row's own id, so a row that
// stays is the same element wherever it moves to. Only an id met twice in one list is
// told apart by how many times it has been met -- never by the row's place, which
// changes for every row when one arrives at the top.
export function keyed<T>(rows: T[], id: (row: T) => string): { row: T; key: string }[] {
  const met = new Map<string, number>()
  return rows.map((row) => {
    const k = id(row)
    const n = met.get(k) ?? 0
    met.set(k, n + 1)
    return { row, key: n ? k + '#' + n : k }
  })
}
