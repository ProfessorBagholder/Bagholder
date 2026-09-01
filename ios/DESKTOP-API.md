# Desktop API dump

Source of truth: `master` (`bagholder.py`, `store.py`, `ledger.html`). Facts only from those files.

Routing: `do_GET` / `do_POST` use `path = self.path.split("?", 1)[0]` (query string ignored). JSON bodies are sent with `Content-Type: application/json; charset=utf-8` unless noted.

`_gate()` (GET): false unless client IP is `127.0.0.1` or `::1` and `Host` is `127.0.0.1:<bound-port>`.
`_gate(write=True)` (POST): same, plus `Sec-Fetch-Site` is `same-origin` or header `X-Bagholder` is non-empty.

---

## 1. HTTP paths (`bagholder.py` `do_GET` / `do_POST`)

### GET

**GET `/` and GET `/ledger.html`**

- 403 `{"ok": false}` if `_gate()` is false.
- 404 `{"ok": false, "error": "ledger.html missing"}` if `ledger.html` cannot be read (`OSError`).
- 200 file bytes of `ledger.html`, `Content-Type: text/html; charset=utf-8`.

**GET `/api/status`**

- 403 `{"ok": false}` if `_gate()` is false.
- 200 from `status_payload()`:

```json
{
  "ok": true,
  "connected": "<bool>",
  "email": "<string>",
  "lastSync": "<string>",
  "activityCount": "<int>",
  "accountCount": "<int>",
  "capturing": "<bool>",
  "syncing": "<bool>",
  "listingsFilling": "<bool>",
  "syncStep": "<string>",
  "error": "<string>"
}
```

**GET `/favicon.png` and GET `/favicon.ico`**

- 403 `{"ok": false}` if `_gate()` is false.
- 404 `{"ok": false, "error": "favicon missing"}` if `favicon.png` cannot be read (`OSError`).
- 200 file bytes of `favicon.png`, `Content-Type: image/png`.

**GET `/api/book`**

- 403 `{"ok": false}` if `_gate()` is false.
- 200:

```json
{
  "ok": true,
  "activities": [],
  "accounts": [],
  "balances": [],
  "navHistory": [],
  "navByAccount": {},
  "syncedAt": "",
  "tradeGroups": [],
  "notes": {},
  "securities": []
}
```

Empty arrays/objects/`""` are the defaults when the corresponding `load_book()` / `store.snapshot()` keys are missing.

**GET any other path**

- 404 `{"ok": false, "error": "not found"}` (no `_gate()` check).

### POST

`do_POST` runs `_gate(write=True)` before matching the path. If that fails: 403 `{"ok": false}` for every POST path below (and for unknown POST paths).

**POST `/api/login/start`**

- 403 as above.
- 200 body is `start_login_browser()`:
  - `{"ok": true}` on success.
  - `{"ok": false, "error": "Install Chrome. Passkey login has to happen on Wealthsimple’s site."}` if Chrome is not found or `Popen` raises.
- Status is always 200 (`200 if result.get("ok") else 200`).

**POST `/api/capture`**

- 403 as above.
- 200 body is `capture_tokens(body)`:
  - `{"ok": false, "error": "bad body"}` if body is not a dict.
  - `{"ok": false, "error": "missing access_token"}` if `access_token` is missing/empty.
  - `{"ok": true}` on success.

**POST `/api/refresh`**

- 403 as above.
- 200 body is `refresh_now()`:
  - `{"ok": false, "error": "not connected", "connected": false}` if no session or no `refresh_token`.
  - `{"ok": <bool>, "error": "<string>", "connected": <bool>}` after `refresh_session`.

**POST `/api/sync`**

- 403 as above.
- 200 `{"ok": false, "error": "not connected"}` if `load_session()` is falsy.
- 200 `{"ok": true, "syncing": true}` otherwise (starts a `run_sync` thread).

**POST `/api/disconnect`**

- 403 as above.
- 200 `{"ok": true}`.

**POST `/api/book/append`**

- 403 as above.
- 200 body is `append_manual(body)`:
  - Always includes `"ok": true`, `"added": <int>`, `"duplicates": <int>`.
  - Also `"activity": <row>` if exactly one saved row, or if none saved and exactly one input row.
  - Also `"activities": <list>` if more than one saved row.

**POST `/api/groups`**

- 403 as above.
- 200 `{"ok": true, "groups": <return of store.save_trade_groups>}`.
  `save_trade_groups` returns a list of `{"id": "<str>", "locked": <bool>, "members": ["<str>", ...]}`.

**POST `/api/notes`**

- 403 as above.
- 200 `{"ok": true, "notes": <return of store.save_trade_notes>}`.
  `save_trade_notes` returns an object keyed by trade id; each value is `{"thesis": "<str>", "tag": "<str>", "grade": "<str>", "tradeId": "<str>"}`. `grade` is `"A"`, `"B"`, `"C"`, `"F"`, or `""`.

**POST any other path** (after a passing write gate)

- 404 `{"ok": false, "error": "not found"}`.

---

## 2. sqlite (`store.py`, `SCHEMA_VERSION = 3`)

`_init_schema` creates the tables below, then runs `_migrate_nav_history` and `_ensure_activity_security_id`, then writes `meta.schema_version` = `"3"`.

### `meta`

| Column | Type | Constraints |
| --- | --- | --- |
| `key` | TEXT | PRIMARY KEY |
| `value` | TEXT | |

### `activities`

| Column | Type | Constraints |
| --- | --- | --- |
| `id` | TEXT | PRIMARY KEY |
| `canonical_id` | TEXT | |
| `occurred_at` | TEXT | |
| `transaction_date` | TEXT | NOT NULL |
| `settlement_date` | TEXT | |
| `account_id` | TEXT | |
| `book_id` | TEXT | |
| `fifo_id` | TEXT | |
| `account_type` | TEXT | |
| `activity_type` | TEXT | |
| `activity_sub_type` | TEXT | |
| `description` | TEXT | |
| `direction` | TEXT | |
| `symbol` | TEXT | |
| `name` | TEXT | |
| `currency` | TEXT | |
| `quantity` | REAL | |
| `unit_price` | REAL | |
| `commission` | REAL | |
| `net_cash_amount` | REAL | |
| `category` | TEXT | |
| `balance` | REAL | |
| `source` | TEXT | |
| `raw_type` | TEXT | |
| `aft_type` | TEXT | |
| `counter_symbol` | TEXT | |
| `security_id` | TEXT | |

Index: `CREATE UNIQUE INDEX IF NOT EXISTS activities_canonical_id_uq ON activities (canonical_id) WHERE canonical_id IS NOT NULL AND canonical_id != ''`.

Migration `_ensure_activity_security_id`: if `activities` exists without `security_id`, `ALTER TABLE activities ADD COLUMN security_id TEXT`.

### `securities`

| Column | Type | Constraints |
| --- | --- | --- |
| `id` | TEXT | PRIMARY KEY |
| `symbol` | TEXT | |
| `name` | TEXT | |
| `primary_exchange` | TEXT | |
| `primary_mic` | TEXT | |
| `currency` | TEXT | |
| `underlying_id` | TEXT | |
| `fetched_at` | TEXT | |

### `accounts`

| Column | Type | Constraints |
| --- | --- | --- |
| `id` | TEXT | PRIMARY KEY |
| `nickname` | TEXT | |
| `unified_account_type` | TEXT | |
| `currency` | TEXT | |
| `status` | TEXT | |
| `type` | TEXT | |
| `net_liquidation_value` | REAL | |

### `balances`

| Column | Type | Constraints |
| --- | --- | --- |
| `id` | INTEGER | PRIMARY KEY AUTOINCREMENT |
| `account_id` | TEXT | |
| `custodian_account_id` | TEXT | |
| `security_id` | TEXT | |
| `quantity` | REAL | |

### `nav_history`

| Column | Type | Constraints |
| --- | --- | --- |
| `account_id` | TEXT | NOT NULL DEFAULT `''` |
| `date` | TEXT | NOT NULL |
| `equity` | REAL | |
| `currency` | TEXT | |
| `net_deposits` | REAL | |

PRIMARY KEY `(account_id, date)`.

Migration `_migrate_nav_history`: if `nav_history` exists and does not already have `account_id` plus PK `(account_id, date)`, rebuilds the table to this shape. Old rows without `account_id` get `account_id = ''`.

### `grouped_trades`

| Column | Type | Constraints |
| --- | --- | --- |
| `id` | TEXT | PRIMARY KEY |
| `created_at` | TEXT | NOT NULL |

---

## 3. `HAS_LOCAL_API` (`ledger.html`)

True only when `location.hostname` is `127.0.0.1` or `localhost`. Actual check:

```javascript
const HAS_LOCAL_API = (location.hostname === "127.0.0.1" || location.hostname === "localhost");
```

---

## 4. Constants (`bagholder.py`)

```
OAUTH = "https://api.production.wealthsimple.com/v1/oauth/v2"
GRAPHQL = "https://my.wealthsimple.com/graphql"
LOGIN_URL = "https://my.wealthsimple.com/app/login"
```
