// Generated from the server's differ (`bagholder_model::patch::keys_of`). Do not edit:
// change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_row_keys`.

/** Each document's lists of rows, by path (`*` for a list's rows or a map's values), and the field that tells the rows apart. */
export const ROW_KEYS: Record<string, Record<string, string>> = {
  'model': {
    'options.accounts': 'id',
    'options.instruments': 'id',
    'equity.series': 'd',
    'years': 'year',
    'monthly': 'key',
    'bySymbol': 'id',
    'grades.buckets': 'grade',
    'queue': 'id',
    'trades': 'id',
    'positions': 'id',
    'portfolio.allocation': 'label',
    'cashflow.tiles': 'label',
    'cashflow.months': 'key',
    'cashflow.holdings': 'id',
    'cashflow.income': 'label',
    'cashflow.rows': 'id',
    'accounts': 'id',
    'markets.holdings': 'id',
    'markets.watchlist': 'symbol',
    'markets.news': 'id',
    'markets.news.*.tags': 'symbol',
    'markets.universes.*': 'id',
    'markets.tiles': 'symbol',
    'markets.instruments': 'symbol',
    'sectors': 'name',
    'regions': 'name',
  },
  'orders': {
    'orders': 'id',
    'brackets': 'id',
  },
  'shorts': {
    'rows': 'symbol',
    'rows.*.series': 'date',
  },
  'notifications': {
    'rows': 'id',
  },
  'filings': {
    'filings': 'id',
  },
  'filings-feed': {
    'filings': 'id',
  },
  'fear': {
    'gauge.previous': 'label',
    'gauge.parts': 'name',
    'gauge.series': 'date',
  },
  'quote': {
    'accounts': 'id',
  },
  'history': {
  },
}
