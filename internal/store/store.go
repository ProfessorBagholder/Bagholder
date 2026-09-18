package store

import (
	"database/sql"
	"encoding/json"
	"fmt"
	"hash/fnv"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"
	_ "time/tzdata"

	_ "modernc.org/sqlite"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

const (
	SchemaVersion            = 13
	FXPair                   = "USDCAD"
	BenchmarkSymbol          = "SP500"
	JournalMeta              = "journal_v2"
	OptionUnitPriceScaleMeta = "option_unit_price_scale_v1"
	OptionRelabelMeta        = "option_relabel_rows_v1"
	ActivityPullHour         = 14
	ActivityPullMinute       = 0
	PullOverlapDays          = 14
	TilesMeta                = "market_tiles"
	NotificationsKept        = 200
)

var ActivityPullTZ = mustZone("America/Edmonton")

func mustZone(name string) *time.Location {
	loc, err := time.LoadLocation(name)
	if err != nil {
		panic(err)
	}
	return loc
}

var BenchmarkSymbols = []string{"SP500", "TSX", "TSX60"}

var SyncMetaKeys = []string{"synced_at", "last_activity_pull", "security_id_backfill_done"}

var invented = map[string]bool{"": true, "manual": true, "legacy": true, "statement": true, "canonical": true, "cad": true, "usd": true}

type Store struct {
	home string
	path string
	db   *sql.DB
	mu   sync.Mutex

	ready bool
	gen   atomic.Uint64

	verMu      sync.Mutex
	verGen     uint64
	verFull    string
	verCore    string
	bookGen    uint64
	bookVer    string
	ensuredGen uint64
}

func Home() string {
	if env := strings.TrimSpace(os.Getenv("BAGHOLDER_HOME")); env != "" {
		return env
	}
	h, err := os.UserHomeDir()
	if err != nil {
		h = "."
	}
	return filepath.Join(h, ".bagholder")
}

func Open(home string) (*Store, error) {
	if home == "" {
		home = Home()
	}
	if err := ensureHome(home); err != nil {
		return nil, err
	}
	path := filepath.Join(home, "bagholder.db")
	dsn := "file:" + path + "?_pragma=busy_timeout(10000)&_pragma=journal_mode(WAL)&_pragma=synchronous(NORMAL)&_pragma=cache_size(-16000)&_pragma=temp_store(MEMORY)&_pragma=foreign_keys(ON)&_txlock=immediate"
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(4)
	db.SetMaxIdleConns(4)
	db.SetConnMaxLifetime(0)
	s := &Store{home: home, path: path, db: db}
	if err := s.Ensure(); err != nil {
		db.Close()
		return nil, err
	}
	_ = os.Chmod(path, 0o600)
	return s, nil
}

func MustOpen(home string) *Store {
	s, err := Open(home)
	if err != nil {
		panic(err)
	}
	return s
}

func ensureHome(home string) error {
	if err := os.MkdirAll(home, 0o700); err != nil {
		return err
	}
	_ = os.Chmod(home, 0o700)
	return nil
}

func (s *Store) Home() string { return s.home }

func (s *Store) Path() string { return s.path }

func (s *Store) Close() error { return s.db.Close() }

func (s *Store) DB() *sql.DB { return s.db }

func nowISO() string { return py.NowStamp() }

func (s *Store) prepare() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.ready {
		return nil
	}
	if err := s.initSchema(); err != nil {
		return err
	}
	s.ready = true
	return nil
}

func (s *Store) must() {
	if err := s.prepare(); err != nil {
		panic(fmt.Sprintf("bagholder store: %v", err))
	}
}

func (s *Store) exec(q string, args ...any) (sql.Result, error) {
	res, err := s.db.Exec(q, args...)
	s.gen.Add(1)
	return res, err
}

func (s *Store) tx(fn func(tx *sql.Tx) error) error {
	tx, err := s.db.Begin()
	if err != nil {
		return err
	}
	if err := fn(tx); err != nil {
		_ = tx.Rollback()
		return err
	}
	err = tx.Commit()
	s.gen.Add(1)
	return err
}

func (s *Store) Generation() uint64 { return s.gen.Load() }

func scanRow(rows *sql.Rows) (map[string]any, error) {
	cols, err := rows.Columns()
	if err != nil {
		return nil, err
	}
	vals := make([]any, len(cols))
	ptrs := make([]any, len(cols))
	for i := range vals {
		ptrs[i] = &vals[i]
	}
	if err := rows.Scan(ptrs...); err != nil {
		return nil, err
	}
	out := make(map[string]any, len(cols))
	for i, c := range cols {
		v := vals[i]
		if b, ok := v.([]byte); ok {
			v = string(b)
		}
		out[c] = v
	}
	return out, nil
}

func (s *Store) queryMaps(q string, args ...any) ([]map[string]any, error) {
	rows, err := s.db.Query(q, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []map[string]any
	for rows.Next() {
		m, err := scanRow(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, m)
	}
	return out, rows.Err()
}

func (s *Store) queryOne(q string, args ...any) (map[string]any, error) {
	rows, err := s.queryMaps(q, args...)
	if err != nil || len(rows) == 0 {
		return nil, err
	}
	return rows[0], nil
}

func (s *Store) each(q string, args []any, fn func(*sql.Rows) error) error {
	rows, err := s.db.Query(q, args...)
	if err != nil {
		return err
	}
	defer rows.Close()
	for rows.Next() {
		if err := fn(rows); err != nil {
			return err
		}
	}
	return rows.Err()
}

type realCell struct{ v *float64 }

func (c *realCell) Scan(src any) error {
	c.v = fnum(src)
	return nil
}

func (c *realCell) or0() float64 { return py.Deref(c.v, 0) }

type intCell struct{ v int64 }

func (c *intCell) Scan(src any) error {
	c.v = inum(src)
	return nil
}

func str(v any) string {
	switch x := v.(type) {
	case nil:
		return ""
	case string:
		return x
	case []byte:
		return string(x)
	case int64:
		return strconv.FormatInt(x, 10)
	case float64:
		return py.Repr(x)
	case bool:
		if x {
			return "1"
		}
		return "0"
	}
	return fmt.Sprint(v)
}

func fnum(v any) *float64 {
	switch x := v.(type) {
	case nil:
		return nil
	case float64:
		return py.Ptr(x)
	case int64:
		return py.Ptr(float64(x))
	case int:
		return py.Ptr(float64(x))
	case string:
		if f, err := strconv.ParseFloat(strings.TrimSpace(x), 64); err == nil {
			return py.Ptr(f)
		}
		return nil
	case []byte:
		if f, err := strconv.ParseFloat(strings.TrimSpace(string(x)), 64); err == nil {
			return py.Ptr(f)
		}
		return nil
	}
	return nil
}

func inum(v any) int64 {
	switch x := v.(type) {
	case int64:
		return x
	case float64:
		return int64(x)
	case int:
		return int64(x)
	case string:
		n, _ := strconv.ParseInt(x, 10, 64)
		return n
	}
	return 0
}

func nullable(p *float64) any {
	if p == nil {
		return nil
	}
	return *p
}

func nullStr(s string) any {
	if s == "" {
		return nil
	}
	return s
}

const schemaSQL = `
        CREATE TABLE IF NOT EXISTS meta (
            key TEXT PRIMARY KEY,
            value TEXT
        );

        CREATE TABLE IF NOT EXISTS activities (
            id TEXT PRIMARY KEY,
            canonical_id TEXT,
            occurred_at TEXT,
            transaction_date TEXT NOT NULL,
            settlement_date TEXT,
            account_id TEXT,
            book_id TEXT,
            fifo_id TEXT,
            account_type TEXT,
            activity_type TEXT,
            activity_sub_type TEXT,
            description TEXT,
            direction TEXT,
            symbol TEXT,
            name TEXT,
            currency TEXT,
            quantity REAL,
            unit_price REAL,
            commission REAL,
            net_cash_amount REAL,
            category TEXT,
            balance REAL,
            source TEXT,
            raw_type TEXT,
            aft_type TEXT,
            counter_symbol TEXT,
            security_id TEXT
        );

        CREATE TABLE IF NOT EXISTS securities (
            id TEXT PRIMARY KEY,
            symbol TEXT,
            name TEXT,
            primary_exchange TEXT,
            primary_mic TEXT,
            currency TEXT,
            underlying_id TEXT,
            fetched_at TEXT
        );

        CREATE UNIQUE INDEX IF NOT EXISTS activities_canonical_id_uq
            ON activities (canonical_id)
            WHERE canonical_id IS NOT NULL AND canonical_id != '';

        CREATE TABLE IF NOT EXISTS accounts (
            id TEXT PRIMARY KEY,
            nickname TEXT,
            unified_account_type TEXT,
            currency TEXT,
            status TEXT,
            type TEXT,
            net_liquidation_value REAL
        );

        CREATE TABLE IF NOT EXISTS balances (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id TEXT,
            custodian_account_id TEXT,
            security_id TEXT,
            quantity REAL
        );

        CREATE TABLE IF NOT EXISTS nav_history (
            account_id TEXT NOT NULL DEFAULT '',
            date TEXT NOT NULL,
            equity REAL,
            currency TEXT,
            net_deposits REAL,
            PRIMARY KEY (account_id, date)
        );

        CREATE TABLE IF NOT EXISTS grouped_trades (
            id TEXT PRIMARY KEY,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS fx_rates (
            pair TEXT NOT NULL,
            date TEXT NOT NULL,
            rate REAL NOT NULL,
            PRIMARY KEY (pair, date)
        );

        CREATE TABLE IF NOT EXISTS benchmark_prices (
            symbol TEXT NOT NULL,
            date TEXT NOT NULL,
            close REAL NOT NULL,
            PRIMARY KEY (symbol, date)
        );

        CREATE TABLE IF NOT EXISTS distributions (
            symbol TEXT NOT NULL,
            ex_date TEXT NOT NULL,
            pay_date TEXT,
            amount REAL NOT NULL,
            currency TEXT,
            source TEXT NOT NULL DEFAULT 'tmx',
            PRIMARY KEY (symbol, ex_date, source)
        );

        CREATE TABLE IF NOT EXISTS quotes (
            symbol TEXT PRIMARY KEY,
            price REAL,
            dividend_amount REAL,
            dividend_frequency TEXT,
            ex_dividend_date TEXT,
            source TEXT,
            fetched_at TEXT
        );

        CREATE TABLE IF NOT EXISTS margin (
            account_id TEXT PRIMARY KEY,
            buying_power REAL,
            currency TEXT,
            unavailable TEXT,
            fetched_at TEXT
        );

        CREATE TABLE IF NOT EXISTS distribution_fetches (
            symbol TEXT PRIMARY KEY,
            fetched_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS price_history (
            symbol TEXT NOT NULL,
            date TEXT NOT NULL,
            open REAL,
            high REAL,
            low REAL,
            close REAL NOT NULL,
            volume REAL,
            source TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (symbol, date)
        );

        CREATE TABLE IF NOT EXISTS history_fetches (
            symbol TEXT PRIMARY KEY,
            start TEXT NOT NULL,
            fetched_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS price_bars (
            symbol TEXT NOT NULL,
            tf TEXT NOT NULL,
            ts INTEGER NOT NULL,
            open REAL,
            high REAL,
            low REAL,
            close REAL NOT NULL,
            volume REAL,
            source TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (symbol, tf, ts)
        );

        CREATE TABLE IF NOT EXISTS bar_fetches (
            symbol TEXT NOT NULL,
            tf TEXT NOT NULL,
            start_ts INTEGER NOT NULL,
            fetched_at TEXT NOT NULL,
            PRIMARY KEY (symbol, tf)
        );

        CREATE TABLE IF NOT EXISTS brackets (
            id TEXT PRIMARY KEY,
            order_id TEXT NOT NULL,
            created_at TEXT NOT NULL,
            account_id TEXT NOT NULL,
            security_id TEXT NOT NULL,
            symbol TEXT,
            currency TEXT,
            quantity REAL,
            tif TEXT,
            sl_kind TEXT,
            sl_price REAL,
            sl_trail REAL,
            sl_trail_unit TEXT,
            sl_order_id TEXT,
            sl_native INTEGER,
            sl_mode TEXT,
            high_water REAL,
            tp_price REAL,
            tp_order_id TEXT,
            status TEXT NOT NULL,
            outcome TEXT,
            error TEXT,
            attempts INTEGER,
            moved_at TEXT,
            armed_at TEXT,
            seen_held INTEGER,
            missed_at TEXT,
            updated_at TEXT
        );

        CREATE TABLE IF NOT EXISTS exposures (
            key TEXT PRIMARY KEY,
            sectors TEXT,
            countries TEXT,
            coverage REAL,
            source TEXT,
            as_of TEXT,
            industry TEXT,
            error TEXT,
            fetched_at TEXT
        );

        CREATE TABLE IF NOT EXISTS watchlist (
            symbol TEXT NOT NULL,
            exchange TEXT NOT NULL DEFAULT '',
            name TEXT,
            currency TEXT,
            security_id TEXT,
            added_at TEXT,
            PRIMARY KEY (symbol, exchange)
        );

        CREATE TABLE IF NOT EXISTS told (
            scope TEXT NOT NULL,
            event TEXT NOT NULL,
            at TEXT NOT NULL,
            PRIMARY KEY (scope, event)
        );
        CREATE INDEX IF NOT EXISTS told_at ON told (at);

        CREATE TABLE IF NOT EXISTS news (
            id TEXT NOT NULL,
            symbol TEXT NOT NULL,
            exchange TEXT NOT NULL DEFAULT '',
            source TEXT,
            headline TEXT,
            wire TEXT,
            url TEXT,
            published_at TEXT,
            fetched_at TEXT,
            kind TEXT,
            summary TEXT,
            PRIMARY KEY (id, symbol, exchange)
        );
        CREATE INDEX IF NOT EXISTS news_published ON news (published_at);
        CREATE INDEX IF NOT EXISTS activities_when ON activities (COALESCE(occurred_at, transaction_date), id);
        CREATE INDEX IF NOT EXISTS activities_security ON activities (security_id, account_id);
        CREATE INDEX IF NOT EXISTS balances_account_security ON balances (account_id, security_id);

        CREATE TABLE IF NOT EXISTS universes (
            key TEXT NOT NULL,
            symbol TEXT NOT NULL,
            name TEXT,
            value REAL,
            percent_change REAL,
            sector TEXT,
            country TEXT,
            fetched_at TEXT,
            PRIMARY KEY (key, symbol)
        );

        CREATE TABLE IF NOT EXISTS orders (
            id TEXT PRIMARY KEY,
            created_at TEXT NOT NULL,
            account_id TEXT NOT NULL,
            account TEXT,
            security_id TEXT NOT NULL,
            symbol TEXT,
            currency TEXT,
            side TEXT NOT NULL,
            type TEXT NOT NULL,
            quantity REAL NOT NULL,
            limit_price REAL,
            stop_price REAL,
            tif TEXT NOT NULL,
            stop_loss TEXT,
            take_profit TEXT,
            status TEXT NOT NULL,
            ws_order_id TEXT,
            error TEXT,
            request TEXT,
            updated_at TEXT
        );

        CREATE TABLE IF NOT EXISTS filings (
            symbol TEXT NOT NULL,
            id TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT '',
            category TEXT,
            profile_no TEXT,
            issuer TEXT,
            type TEXT,
            title TEXT,
            date TEXT,
            date_text TEXT,
            size TEXT,
            url TEXT,
            subject TEXT,
            summary TEXT,
            enriched_at TEXT,
            enrich_version INTEGER,
            enrich_final INTEGER,
            fetched_at TEXT,
            PRIMARY KEY (symbol, id)
        );
        CREATE INDEX IF NOT EXISTS filings_date ON filings (symbol, date DESC);

        CREATE TABLE IF NOT EXISTS gauges (
            name TEXT PRIMARY KEY,
            source TEXT,
            score REAL,
            rating TEXT,
            as_of TEXT,
            payload TEXT,
            read_version INTEGER,
            fetched_at TEXT
        );
        CREATE TABLE IF NOT EXISTS shorts (
            symbol TEXT NOT NULL,
            exchange TEXT NOT NULL DEFAULT '',
            market TEXT,
            as_of TEXT,
            shares REAL,
            previous REAL,
            previous_of TEXT,
            change REAL,
            float_shares REAL,
            of_float REAL,
            average_volume REAL,
            days_to_cover REAL,
            volume_of TEXT,
            volume_span TEXT,
            short_volume REAL,
            total_volume REAL,
            volume_pct REAL,
            name TEXT,
            series TEXT,
            read_version INTEGER,
            fetched_at TEXT,
            PRIMARY KEY (symbol, exchange)
        );
`

const notificationsSQL = `
        CREATE TABLE IF NOT EXISTS notifications (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at TEXT NOT NULL,
            kind TEXT NOT NULL,
            key TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL,
            body TEXT,
            extra TEXT,
            seen_at TEXT,
            read_at TEXT
        );
`

func execScript(tx *sql.Tx, script string) error {
	for _, stmt := range strings.Split(script, ";") {
		if strings.TrimSpace(stmt) == "" {
			continue
		}
		if _, err := tx.Exec(stmt); err != nil {
			return fmt.Errorf("%w in %q", err, strings.TrimSpace(stmt)[:min(60, len(strings.TrimSpace(stmt)))])
		}
	}
	return nil
}

func (s *Store) initSchema() error {
	return s.tx(func(tx *sql.Tx) error {
		if err := execScript(tx, schemaSQL); err != nil {
			return err
		}
		for _, step := range []func(*sql.Tx) error{migrateNavHistory, ensureBarColumns, ensureActivitySecurityID, migrateSpyMeta, ensureQuoteColumns, ensureShortsColumns, ensureOrderColumns, ensureAccountColumns} {
			if err := step(tx); err != nil {
				return err
			}
		}
		if err := execScript(tx, notificationsSQL); err != nil {
			return err
		}
		for _, step := range []func(*sql.Tx) error{ensureNotificationsColumns, ensureNewsColumns, ensureFilingsColumns, migrateHistorySources} {
			if err := step(tx); err != nil {
				return err
			}
		}
		_, err := tx.Exec("INSERT INTO meta(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value", "schema_version", strconv.Itoa(SchemaVersion))
		return err
	})
}

func tableColumns(tx *sql.Tx, table string) (map[string]bool, []string, map[string]bool, error) {
	rows, err := tx.Query("PRAGMA table_info(" + table + ")")
	if err != nil {
		return nil, nil, nil, err
	}
	defer rows.Close()
	cols := map[string]bool{}
	pk := map[string]bool{}
	var order []string
	for rows.Next() {
		var cid int
		var name, typ string
		var notnull int
		var dflt any
		var pkv int
		if err := rows.Scan(&cid, &name, &typ, &notnull, &dflt, &pkv); err != nil {
			return nil, nil, nil, err
		}
		cols[name] = true
		order = append(order, name)
		if pkv != 0 {
			pk[name] = true
		}
	}
	return cols, order, pk, rows.Err()
}

func tableExists(tx *sql.Tx, name string) (bool, error) {
	var n string
	err := tx.QueryRow("SELECT name FROM sqlite_master WHERE type='table' AND name=?", name).Scan(&n)
	if err == sql.ErrNoRows {
		return false, nil
	}
	return err == nil, err
}

func migrateNavHistory(tx *sql.Tx) error {
	ok, err := tableExists(tx, "nav_history")
	if err != nil || !ok {
		return err
	}
	cols, _, pk, err := tableColumns(tx, "nav_history")
	if err != nil {
		return err
	}
	if cols["account_id"] && len(pk) == 2 && pk["account_id"] && pk["date"] {
		return nil
	}
	if _, err := tx.Exec(`CREATE TABLE nav_history_new (
            account_id TEXT NOT NULL DEFAULT '',
            date TEXT NOT NULL,
            equity REAL,
            currency TEXT,
            net_deposits REAL,
            PRIMARY KEY (account_id, date)
        )`); err != nil {
		return err
	}
	if cols["account_id"] {
		if _, err := tx.Exec("INSERT INTO nav_history_new (account_id, date, equity, currency, net_deposits) SELECT COALESCE(account_id, ''), date, equity, currency, net_deposits FROM nav_history"); err != nil {
			return err
		}
	} else {
		if _, err := tx.Exec("INSERT INTO nav_history_new (account_id, date, equity, currency, net_deposits) SELECT '', date, equity, currency, net_deposits FROM nav_history"); err != nil {
			return err
		}
	}
	if _, err := tx.Exec("DROP TABLE nav_history"); err != nil {
		return err
	}
	_, err = tx.Exec("ALTER TABLE nav_history_new RENAME TO nav_history")
	return err
}

func ensureActivitySecurityID(tx *sql.Tx) error {
	cols, _, _, err := tableColumns(tx, "activities")
	if err != nil {
		return err
	}
	if !cols["security_id"] {
		_, err = tx.Exec("ALTER TABLE activities ADD COLUMN security_id TEXT")
	}
	return err
}

func addColumns(tx *sql.Tx, table string, wanted [][2]string) error {
	cols, _, _, err := tableColumns(tx, table)
	if err != nil {
		return err
	}
	for _, c := range wanted {
		if !cols[c[0]] {
			if _, err := tx.Exec("ALTER TABLE " + table + " ADD COLUMN " + c[0] + " " + c[1]); err != nil {
				return err
			}
		}
	}
	return nil
}

func ensureShortsColumns(tx *sql.Tx) error {
	return addColumns(tx, "shorts", [][2]string{{"read_version", "INTEGER"}, {"name", "TEXT"}})
}

func ensureQuoteColumns(tx *sql.Tx) error {
	return addColumns(tx, "quotes", [][2]string{{"price_change", "REAL"}, {"percent_change", "REAL"}, {"prev_close", "REAL"}})
}

func ensureOrderColumns(tx *sql.Tx) error {
	if err := addColumns(tx, "orders", [][2]string{{"source", "TEXT"}, {"ws_status", "TEXT"}, {"filled_qty", "REAL"}, {"avg_fill", "REAL"}, {"submitted_at", "TEXT"}, {"expires_at", "TEXT"}, {"parent_id", "TEXT"}, {"role", "TEXT"}, {"fill_booked_qty", "REAL"}}); err != nil {
		return err
	}
	return addColumns(tx, "brackets", [][2]string{{"seen_held", "INTEGER"}, {"missed_at", "TEXT"}})
}

func ensureAccountColumns(tx *sql.Tx) error {
	return addColumns(tx, "accounts", [][2]string{{"margin_account_id", "TEXT"}})
}

func migrateSpyMeta(tx *sql.Tx) error {
	var one int
	if err := tx.QueryRow("SELECT 1 FROM benchmark_prices WHERE symbol = ? LIMIT 1", BenchmarkSymbol).Scan(&one); err == nil {
		return nil
	}
	var raw sql.NullString
	if err := tx.QueryRow("SELECT value FROM meta WHERE key = 'spy_by_date'").Scan(&raw); err != nil || !raw.Valid || raw.String == "" {
		return nil
	}
	var data map[string]any
	if err := json.Unmarshal([]byte(raw.String), &data); err != nil {
		return nil
	}
	for day, px := range data {
		d := strings.TrimSpace(day)
		if len(d) > 10 {
			d = d[:10]
		}
		v, ok := py.NumOK(px)
		if len(d) != 10 || !ok || v <= 0 {
			continue
		}
		if _, err := tx.Exec("INSERT OR IGNORE INTO benchmark_prices(symbol, date, close) VALUES (?, ?, ?)", BenchmarkSymbol, d, v); err != nil {
			return err
		}
	}
	return nil
}

func migrateHistorySources(tx *sql.Tx) error {
	var v string
	if err := tx.QueryRow("SELECT value FROM meta WHERE key = 'history_sources_migrated'").Scan(&v); err == nil {
		return nil
	}
	for _, q := range []string{
		"DELETE FROM price_history WHERE source = 'coingecko'",
		"DELETE FROM history_fetches WHERE symbol NOT IN (SELECT DISTINCT symbol FROM price_history WHERE source NOT IN ('coingecko', 'cboe_ca'))",
		"DELETE FROM price_bars WHERE source = 'coingecko'",
		"DELETE FROM bar_fetches WHERE symbol NOT IN (SELECT DISTINCT symbol FROM price_bars)",
		"DELETE FROM history_fetches WHERE symbol IN (SELECT h.symbol FROM history_fetches h JOIN (SELECT symbol, MIN(date) AS first FROM price_history GROUP BY symbol) p ON p.symbol = h.symbol WHERE julianday(p.first) - julianday(h.start) > 7)",
		"DELETE FROM bar_fetches WHERE (symbol, tf) IN (SELECT b.symbol, b.tf FROM bar_fetches b JOIN (SELECT symbol, tf, MIN(ts) AS first FROM price_bars GROUP BY symbol, tf) p ON p.symbol = b.symbol AND p.tf = b.tf WHERE p.first - b.start_ts > 7 * 86400)",
		"INSERT INTO meta(key, value) VALUES ('history_sources_migrated', '1')",
	} {
		if _, err := tx.Exec(q); err != nil {
			return err
		}
	}
	return nil
}

func ensureBarColumns(tx *sql.Tx) error {
	cols, _, _, err := tableColumns(tx, "price_bars")
	if err != nil {
		return err
	}
	if len(cols) > 0 && !cols["open"] {
		for _, q := range []string{"DROP TABLE price_bars", "DELETE FROM bar_fetches",
			"CREATE TABLE price_bars (symbol TEXT NOT NULL, tf TEXT NOT NULL, ts INTEGER NOT NULL, open REAL, high REAL, low REAL, close REAL NOT NULL, volume REAL, source TEXT NOT NULL DEFAULT '', PRIMARY KEY (symbol, tf, ts))"} {
			if _, err := tx.Exec(q); err != nil {
				return err
			}
		}
	}
	return nil
}

func ensureNewsColumns(tx *sql.Tx) error {
	ok, err := tableExists(tx, "news")
	if err != nil || !ok {
		return err
	}
	// what the source said beneath the headline; a row read before this is simply without one
	if err := addColumns(tx, "news", [][2]string{{"kind", "TEXT"}, {"summary", "TEXT"}}); err != nil {
		return err
	}
	_, err = tx.Exec("UPDATE news SET kind = CASE WHEN LOWER(COALESCE(wire, '')) LIKE '%wire%' OR LOWER(COALESCE(wire, '')) LIKE '%newsfile%' OR LOWER(COALESCE(wire, '')) LIKE '%cision%' OR LOWER(COALESCE(wire, '')) LIKE '%cnw%' THEN 'release' ELSE 'story' END WHERE kind IS NULL OR kind = ''")
	return err
}

func ensureNotificationsColumns(tx *sql.Tx) error {
	ok, err := tableExists(tx, "notifications")
	if err != nil || !ok {
		return err
	}
	return addColumns(tx, "notifications", [][2]string{{"read_at", "TEXT"}})
}

func ensureFilingsColumns(tx *sql.Tx) error {
	ok, err := tableExists(tx, "filings")
	if err != nil || !ok {
		return err
	}
	return addColumns(tx, "filings", [][2]string{{"source", "TEXT"}, {"category", "TEXT"}, {"type", "TEXT"}, {"title", "TEXT"}, {"date", "TEXT"}, {"date_text", "TEXT"}, {"subject", "TEXT"}, {"summary", "TEXT"}, {"enriched_at", "TEXT"}, {"enrich_version", "INTEGER"}, {"enrich_final", "INTEGER"}})
}

func relabelWhenRowsChanged(tx *sql.Tx) (bool, error) {
	key, err := relabelKey(tx)
	if err != nil {
		return false, err
	}
	var stamped sql.NullString
	err = tx.QueryRow("SELECT value FROM meta WHERE key = ?", OptionRelabelMeta).Scan(&stamped)
	if err == nil && stamped.Valid && stamped.String == key {
		return false, nil
	}
	if err := relabelOptionTrades(tx); err != nil {
		return false, err
	}
	_, err = tx.Exec("INSERT INTO meta(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value", OptionRelabelMeta, key)
	return true, err
}

func pyNone(v sql.NullString) string {
	if !v.Valid {
		return "None"
	}
	return v.String
}

func relabelOptionTrades(tx *sql.Tx) error {
	stmts := []string{
		"UPDATE activities SET activity_sub_type = 'BUYTOOPEN', category = 'trade', quantity = ABS(quantity), net_cash_amount = -ABS(net_cash_amount) WHERE UPPER(REPLACE(IFNULL(raw_type,''), '-', '_')) = 'OPTIONS_BUY' AND UPPER(REPLACE(IFNULL(activity_sub_type,''), '-', '_')) NOT IN ('BUY', 'BUYTOOPEN', 'BTO', 'BUYTOCLOSE', 'BTC')",
		"UPDATE activities SET activity_sub_type = 'SELLTOOPEN', category = 'trade', quantity = -ABS(quantity), net_cash_amount = ABS(net_cash_amount) WHERE UPPER(REPLACE(IFNULL(raw_type,''), '-', '_')) = 'OPTIONS_SELL' AND UPPER(REPLACE(IFNULL(activity_sub_type,''), '-', '_')) NOT IN ('SELL', 'SELLTOOPEN', 'STO', 'SELLTOCLOSE', 'STC', 'COVER')",
	}
	for _, q := range stmts {
		if _, err := tx.Exec(q); err != nil {
			return err
		}
	}
	return relabelOptionCloses(tx)
}

func relabelOptionCloses(tx *sql.Tx) error {
	raw := "UPPER(REPLACE(IFNULL(raw_type,''), '-', '_'))"
	stmts := []string{
		"UPDATE activities SET activity_type = 'OPTIONS_BUY', activity_sub_type = 'BUYTOCLOSE', category = 'trade' WHERE " + raw + " LIKE '%MULTILEG%' AND IFNULL(net_cash_amount, 0) < 0",
		"UPDATE activities SET activity_type = 'OPTIONS_SELL', activity_sub_type = 'SELLTOOPEN', category = 'trade' WHERE " + raw + " LIKE '%MULTILEG%' AND IFNULL(net_cash_amount, 0) >= 0",
		"UPDATE activities SET activity_type = 'ASSIGN', activity_sub_type = 'BUYTOCLOSE', category = 'option_event', quantity = ABS(quantity), unit_price = 0 WHERE " + raw + " LIKE '%ASSIGN%'",
		"UPDATE activities SET activity_type = 'EXPIR', activity_sub_type = 'BUY', category = 'option_event', quantity = ABS(quantity), unit_price = CASE WHEN ABS(IFNULL(net_cash_amount, 0)) < 1e-12 THEN 0 ELSE unit_price END WHERE " + raw + " LIKE '%SHORT%EXPIR%'",
		"UPDATE activities SET activity_type = 'EXPIR', activity_sub_type = 'SELL', category = 'option_event', quantity = -ABS(quantity), unit_price = CASE WHEN ABS(IFNULL(net_cash_amount, 0)) < 1e-12 THEN 0 ELSE unit_price END WHERE " + raw + " LIKE '%EXPIR%' AND " + raw + " NOT LIKE '%SHORT%'",
	}
	for _, q := range stmts {
		if _, err := tx.Exec(q); err != nil {
			return err
		}
	}
	return nil
}

func isOptionSymbolStore(symbol string) bool {
	compact := strings.ToUpper(py.Strip(symbol))
	if compact == "" {
		return false
	}
	padded := " " + compact + " "
	if strings.Contains(padded, " CALL ") || strings.Contains(padded, " PUT ") {
		return true
	}
	return strings.HasSuffix(compact, " C") || strings.HasSuffix(compact, " P")
}

func cashNear(a, b float64) bool {
	rel, absTol := 0.02, 0.02
	m := absTol
	if rel*max3(abs(a), abs(b), 1e-9) > m {
		m = rel * max3(abs(a), abs(b), 1e-9)
	}
	return abs(a-b) <= m
}

func abs(x float64) float64 {
	if x < 0 {
		return -x
	}
	return x
}

func max3(a, b, c float64) float64 {
	m := a
	if b > m {
		m = b
	}
	if c > m {
		m = c
	}
	return m
}

func scaleOptionUnitPrices(tx *sql.Tx) error {
	rows, err := tx.Query("SELECT id, symbol, quantity, unit_price, net_cash_amount FROM activities")
	if err != nil {
		return err
	}
	type upd struct {
		id string
		px float64
	}
	var updates []upd
	for rows.Next() {
		var id string
		var symbol sql.NullString
		var qty, px, cash sql.NullFloat64
		if err := rows.Scan(&id, &symbol, &qty, &px, &cash); err != nil {
			rows.Close()
			return err
		}
		if !isOptionSymbolStore(symbol.String) {
			continue
		}
		q, p, c := abs(qty.Float64), abs(px.Float64), abs(cash.Float64)
		if q <= 0 || p <= 0 || c <= 0 {
			continue
		}
		implied := p * q
		if cashNear(c, implied*100.0) {
			continue
		}
		if cashNear(c, implied) {
			updates = append(updates, upd{id, p / 100.0})
		}
	}
	rows.Close()
	for _, u := range updates {
		if _, err := tx.Exec("UPDATE activities SET unit_price = ? WHERE id = ?", u.px, u.id); err != nil {
			return err
		}
	}
	return nil
}

func (s *Store) Ensure() error {
	s.mu.Lock()
	if s.ready {
		var v string
		err := s.db.QueryRow("SELECT value FROM meta WHERE key = 'schema_version'").Scan(&v)
		if err != nil || v != strconv.Itoa(SchemaVersion) {
			s.ready = false
		}
	}
	s.mu.Unlock()
	if err := s.prepare(); err != nil {
		return err
	}
	err := s.migrate()
	s.verMu.Lock()
	s.ensuredGen = s.gen.Load()
	s.verMu.Unlock()
	return err
}

func (s *Store) EnsureChanged() error {
	s.verMu.Lock()
	same := s.ensuredGen == s.gen.Load() && s.ensuredGen != 0
	s.verMu.Unlock()
	if same {
		return nil
	}
	return s.Ensure()
}

func (s *Store) migrate() error {
	key, err := relabelKey(s.db)
	if err != nil {
		return err
	}
	var stamped, scaled sql.NullString
	_ = s.db.QueryRow("SELECT value FROM meta WHERE key = ?", OptionRelabelMeta).Scan(&stamped)
	scaleErr := s.db.QueryRow("SELECT value FROM meta WHERE key = ?", OptionUnitPriceScaleMeta).Scan(&scaled)
	if stamped.Valid && stamped.String == key && scaleErr == nil {
		return nil
	}
	return s.tx(func(tx *sql.Tx) error {
		if _, err := relabelWhenRowsChanged(tx); err != nil {
			return err
		}
		var stamped sql.NullString
		err := tx.QueryRow("SELECT value FROM meta WHERE key = ?", OptionUnitPriceScaleMeta).Scan(&stamped)
		if err == sql.ErrNoRows {
			if err := scaleOptionUnitPrices(tx); err != nil {
				return err
			}
			_, err = tx.Exec("INSERT INTO meta(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value", OptionUnitPriceScaleMeta, "1")
		}
		return err
	})
}

type rowQuerier interface {
	QueryRow(query string, args ...any) *sql.Row
}

func relabelKey(q rowQuerier) (string, error) {
	var n int64
	var m sql.NullString
	if err := q.QueryRow("SELECT COUNT(*) AS n, MAX(COALESCE(occurred_at, transaction_date)) AS m FROM activities").Scan(&n, &m); err != nil {
		return "", err
	}
	return fmt.Sprintf("%d|%s", n, pyNone(m)), nil
}

func (s *Store) GetMeta(key string) string {
	return s.GetMetaDefault(key, "")
}

func (s *Store) GetMetaDefault(key, def string) string {
	s.must()
	var v sql.NullString
	err := s.db.QueryRow("SELECT value FROM meta WHERE key = ?", key).Scan(&v)
	if err != nil || !v.Valid {
		return def
	}
	return v.String
}

func (s *Store) SetMeta(key, value string) {
	s.must()
	_, _ = s.exec("INSERT INTO meta(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value", key, value)
}

func (s *Store) DeleteMeta(key string) {
	s.must()
	_, _ = s.exec("DELETE FROM meta WHERE key = ?", key)
}

func (s *Store) MetaLike(prefix string) map[string]string {
	s.must()
	rows, err := s.queryMaps("SELECT key, value FROM meta WHERE key LIKE ?", prefix+"%")
	out := map[string]string{}
	if err != nil {
		return out
	}
	for _, r := range rows {
		out[str(r["key"])] = str(r["value"])
	}
	return out
}

var versionSQL = []string{
	"SELECT COUNT(*), MAX(COALESCE(occurred_at, transaction_date)) FROM activities",
	"SELECT COUNT(*), MAX(date) FROM nav_history",
	"SELECT COUNT(*), MAX(date) FROM fx_rates",
	"SELECT COUNT(*), MAX(date) FROM benchmark_prices",
	"SELECT COUNT(*), MAX(ex_date) FROM distributions",
	"SELECT COUNT(*), MAX(fetched_at) FROM securities",
	"SELECT COUNT(*), SUM(quantity) FROM balances",
	"SELECT COUNT(*), MAX(id) FROM accounts",
	"SELECT COUNT(*), MAX(fetched_at) FROM margin",
	"SELECT COUNT(*), MAX(fetched_at) FROM exposures",
	"SELECT COUNT(*), MAX(added_at) FROM watchlist",
	"SELECT COUNT(*), MAX(fetched_at) FROM news",
	"SELECT COUNT(*), MAX(fetched_at) FROM universes",
	"SELECT COUNT(*), SUM(COALESCE(net_liquidation_value, 0)) FROM accounts",
}

const quotesSQL = "SELECT COUNT(*), MAX(fetched_at), TOTAL(price) FROM quotes"

var versionMeta = []string{"synced_at", "trade_groups", "trade_notes", JournalMeta, TilesMeta}

func hashString(s string) uint64 {
	h := fnv.New64a()
	h.Write([]byte(s))
	return h.Sum64()
}

func (s *Store) Versions() (string, string) {
	s.must()
	gen := s.gen.Load()
	s.verMu.Lock()
	if s.verGen == gen && s.verFull != "" {
		full, core := s.verFull, s.verCore
		s.verMu.Unlock()
		return full, core
	}
	s.verMu.Unlock()
	full, core := s.computeVersions()
	s.verMu.Lock()
	if s.gen.Load() == gen {
		s.verGen, s.verFull, s.verCore = gen, full, core
	}
	s.verMu.Unlock()
	return full, core
}

func (s *Store) computeVersions() (string, string) {
	parts := make([]string, 0, len(versionSQL)+len(versionMeta))
	for _, q := range versionSQL {
		var a, b any
		if err := s.db.QueryRow(q).Scan(&a, &b); err != nil {
			parts = append(parts, "?:?")
			continue
		}
		parts = append(parts, versionPart(a)+":"+versionPart(b))
	}
	for _, key := range versionMeta {
		val := s.GetMeta(key)
		parts = append(parts, fmt.Sprintf("%s:%d:%d", key, len(val), hashString(val)))
	}
	core := strings.Join(parts, "|")
	var a, b, c any
	_ = s.db.QueryRow(quotesSQL).Scan(&a, &b, &c)
	return core + "|q:" + versionPart(a) + ":" + versionPart(b) + ":" + versionPart(c), core
}

func versionPart(v any) string {
	if v == nil {
		return "None"
	}
	if b, ok := v.([]byte); ok {
		return string(b)
	}
	return str(v)
}

func (s *Store) DataVersion() string {
	full, _ := s.Versions()
	return full
}

func (s *Store) CoreVersion() string {
	_, core := s.Versions()
	return core
}

func (s *Store) BookVersion() string {
	s.must()
	gen := s.gen.Load()
	s.verMu.Lock()
	if s.bookGen == gen && s.bookVer != "" {
		v := s.bookVer
		s.verMu.Unlock()
		return v
	}
	s.verMu.Unlock()
	parts := []string{}
	for _, q := range []string{"SELECT COUNT(*), MAX(COALESCE(occurred_at, transaction_date)) FROM activities", "SELECT COUNT(*), MAX(fetched_at) FROM securities"} {
		var a, b any
		_ = s.db.QueryRow(q).Scan(&a, &b)
		parts = append(parts, versionPart(a)+":"+versionPart(b))
	}
	v := strings.Join(parts, "|")
	s.verMu.Lock()
	if s.gen.Load() == gen {
		s.bookGen, s.bookVer = gen, v
	}
	s.verMu.Unlock()
	return v
}

type StatusCounts struct {
	ActivityCount int    `json:"activityCount"`
	AccountCount  int    `json:"accountCount"`
	SyncedAt      string `json:"syncedAt"`
}

func (s *Store) StatusCounts() StatusCounts {
	s.must()
	var acts, accounts int
	_ = s.db.QueryRow("SELECT COUNT(*) FROM activities").Scan(&acts)
	_ = s.db.QueryRow("SELECT COUNT(*) FROM accounts").Scan(&accounts)
	return StatusCounts{ActivityCount: acts, AccountCount: accounts, SyncedAt: s.GetMeta("synced_at")}
}

func (s *Store) DataSummary() map[string]any {
	s.must()
	count := func(q string) int {
		var n int
		_ = s.db.QueryRow(q).Scan(&n)
		return n
	}
	journalN := 0
	if raw := s.GetMeta(JournalMeta); raw != "" {
		var m map[string]any
		if json.Unmarshal([]byte(raw), &m) == nil {
			journalN = len(m)
		}
	}
	var first, last sql.NullString
	_ = s.db.QueryRow("SELECT MIN(transaction_date), MAX(transaction_date) FROM activities").Scan(&first, &last)
	return map[string]any{
		"path":          s.path,
		"activities":    count("SELECT COUNT(*) FROM activities"),
		"firstActivity": first.String,
		"lastActivity":  last.String,
		"accounts":      count("SELECT COUNT(*) FROM accounts"),
		"balances":      count("SELECT COUNT(*) FROM balances"),
		"navDays":       count("SELECT COUNT(*) FROM nav_history"),
		"securities":    count("SELECT COUNT(*) FROM securities"),
		"journal":       journalN,
		"fxDays":        count("SELECT COUNT(*) FROM fx_rates"),
		"benchmarkDays": count("SELECT COUNT(*) FROM benchmark_prices"),
		"filings":       count("SELECT COUNT(*) FROM filings"),
		"syncedAt":      s.GetMeta("synced_at"),
	}
}

func (s *Store) ClearSyncedData(keepJournal, keepMarket bool) map[string]any {
	s.must()
	_ = s.tx(func(tx *sql.Tx) error {
		for _, table := range []string{"activities", "accounts", "balances", "margin", "nav_history", "securities", "grouped_trades"} {
			if _, err := tx.Exec("DELETE FROM " + table); err != nil {
				return err
			}
		}
		keys := append(append([]string{}, SyncMetaKeys...), "trade_groups", "trade_notes")
		if !keepJournal {
			keys = append(keys, JournalMeta)
		}
		for _, k := range keys {
			if _, err := tx.Exec("DELETE FROM meta WHERE key = ?", k); err != nil {
				return err
			}
		}
		if !keepMarket {
			for _, table := range []string{"fx_rates", "benchmark_prices", "distributions", "distribution_fetches", "quotes", "price_history", "history_fetches", "price_bars", "bar_fetches"} {
				if _, err := tx.Exec("DELETE FROM " + table); err != nil {
					return err
				}
			}
			if _, err := tx.Exec("DELETE FROM meta WHERE key IN ('spy_by_date', 'market_attempt_at')"); err != nil {
				return err
			}
			for _, prefix := range []string{"bars_miss:", "bars_source:", "coinbase_product:", "coingecko_id:", "tmx_form:", "yahoo_miss:"} {
				if _, err := tx.Exec("DELETE FROM meta WHERE key LIKE ?", prefix+"%"); err != nil {
					return err
				}
			}
		}
		return nil
	})
	return s.DataSummary()
}
