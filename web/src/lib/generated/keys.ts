// Generated from the server's differ (`bagholder_model::patch::keys_of`). Do not edit:
// change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_row_keys`.

/** Each document's lists of rows, by path (`*` for a list's rows or a map's values), and the field that tells the rows apart. */
export const ROW_KEYS: Record<string, Record<string, string>> = {
  'book': {
    'options.accounts': 'id',
    'options.instruments': 'id',
    'accounts': 'id',
    'waiting': 'transaction',
  },
  'dashboard': {
    'equity.series': 'd',
    'equity.pnl.series': 'd',
    'years': 'year',
    'monthly': 'key',
    'bySymbol': 'id',
    'grades.buckets': 'grade',
    'queue': 'id',
  },
  'positions': {
    'portfolio.allocation': 'label',
    'positions': 'id',
  },
  'trades': {
    'trades': 'id',
  },
  'cashflow': {
    'cashflow.tiles': 'label',
    'cashflow.months': 'key',
    'cashflow.holdings': 'id',
    'cashflow.income': 'label',
    'cashflow.rows': 'id',
  },
  'trade': {
  },
  'exposure': {
    'sectors': 'label',
    'regions': 'label',
  },
  'markets': {
    'tiles': 'id',
    'watchlist': 'id',
    'directory': 'key',
  },
  'heatmap': {
    'blocks': 'label',
    'blocks.*.tiles': 'key',
  },
  'headlines': {
    'items': 'id',
    'items.*.tags': 'symbol',
  },
  'status': {
  },
  'orders': {
    'orders': 'id',
    'orders.*.legs': 'key',
    'brackets': 'id',
    'brackets.*.legs': 'key',
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
  'universe': {
  },
  'news': {
  },
}
