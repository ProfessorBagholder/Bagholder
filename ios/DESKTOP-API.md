# Desktop HTTP and store (Bagholder master `fb3ddc0`)

Source: `bagholder.py`, `store.py`, `ledger.html`. Update this file when those change.

## Bind

- Listen: `127.0.0.1` ports `8765`, then `8766`, then `8767` (`PORTS` in `bagholder.py`).
- UI: `http://127.0.0.1:<port>/` (`README.md`).

## `ledger.html`

`HAS_LOCAL_API` is true only when `location.hostname` is `127.0.0.1` or `localhost`.

## GET

| Path | Body |
| --- | --- |
| `/`, `/ledger.html` | `ledger.html` |
| `/favicon.png`, `/favicon.ico` | `favicon.png` |
| `/api/status` | `ok`, `connected`, `email`, `lastSync`, `activityCount`, `accountCount`, `capturing`, `syncing`, `listingsFilling`, `syncStep`, `error` |
| `/api/book` | `ok`, `activities`, `accounts`, `balances`, `navHistory`, `navByAccount`, `syncedAt`, `tradeGroups`, `notes`, `securities` |

## POST (write gate: local + Host `127.0.0.1:<port>` + `Sec-Fetch-Site: same-origin` or header `X-Bagholder`)

| Path | Body in | Body out |
| --- | --- | --- |
| `/api/login/start` | (empty JSON ok) | login start result |
| `/api/capture` | capture payload | capture result |
| `/api/refresh` | (empty JSON ok) | token refresh result |
| `/api/sync` | (empty JSON ok) | `{ok, syncing}` or not connected |
| `/api/disconnect` | (empty JSON ok) | `{ok: true}` |
| `/api/book/append` | manual row | append result |
| `/api/groups` | `{groups: ...}` | `{ok, groups}` |
| `/api/notes` | `{notes: ...}` | `{ok, notes}` |

OPTIONS is 403. JSON body cap 1 MB. No `Access-Control-Allow-Origin: *`.

## Wealthsimple URLs in `bagholder.py`

- `LOGIN_URL` = `https://my.wealthsimple.com/app/login`
- `OAUTH` = `https://api.production.wealthsimple.com/v1/oauth/v2`
- `GRAPHQL` = `https://my.wealthsimple.com/graphql`
- Cookie name = `_oauth2_access_v2`

Do not put token values in this file.

## sqlite (`store.py`, `SCHEMA_VERSION = 3`)

File on the computer: `~/.bagholder/bagholder.db`. iOS uses the app container, same tables.

- `meta(key, value)` — includes `trade_groups`, `trade_notes`, `synced_at`, `schema_version`
- `activities` — `id`, `canonical_id`, `occurred_at`, `transaction_date`, `settlement_date`, `account_id`, `book_id`, `fifo_id`, `account_type`, `activity_type`, `activity_sub_type`, `description`, `direction`, `symbol`, `name`, `currency`, `quantity`, `unit_price`, `commission`, `net_cash_amount`, `category`, `balance`, `source`, `raw_type`, `aft_type`, `counter_symbol`, `security_id`
- unique index on `activities.canonical_id` when set
- `securities` — `id`, `symbol`, `name`, `primary_exchange`, `primary_mic`, `currency`, `underlying_id`, `fetched_at`
- `accounts` — `id`, `nickname`, `unified_account_type`, `currency`, `status`, `type`, `net_liquidation_value`
- `balances` — `id`, `account_id`, `custodian_account_id`, `security_id`, `quantity`
- `nav_history` — PK `(account_id, date)`, `equity`, `currency`, `net_deposits`
- `grouped_trades` — `id`, `created_at`
