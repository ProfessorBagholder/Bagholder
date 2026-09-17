package store

import (
	"database/sql"
	"strings"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

func cleanDateMap(raw map[string]float64) map[string]float64 {
	out := map[string]float64{}
	for key, v := range raw {
		d := strings.TrimSpace(key)
		if len(d) > 10 {
			d = d[:10]
		}
		if len(d) != 10 || d[4] != '-' || d[7] != '-' || v <= 0 {
			continue
		}
		out[d] = v
	}
	return out
}

func (s *Store) FXRates() map[string]float64 {
	return s.FXRatesFor(FXPair)
}

func (s *Store) FXRatesFor(pair string) map[string]float64 {
	s.must()
	out := map[string]float64{}
	for _, r := range mustRows(s.queryMaps("SELECT date, rate FROM fx_rates WHERE pair = ? ORDER BY date", pair)) {
		out[str(r["date"])] = py.Deref(fnum(r["rate"]), 0)
	}
	return out
}

func (s *Store) FXLastDate() string {
	s.must()
	var d sql.NullString
	_ = s.db.QueryRow("SELECT MAX(date) AS d FROM fx_rates WHERE pair = ?", FXPair).Scan(&d)
	return d.String
}

func (s *Store) UpsertFXRates(mapping map[string]float64) int {
	clean := cleanDateMap(mapping)
	if len(clean) == 0 {
		return 0
	}
	s.must()
	_ = s.tx(func(tx *sql.Tx) error {
		stmt, err := tx.Prepare("INSERT OR IGNORE INTO fx_rates(pair, date, rate) VALUES (?, ?, ?)")
		if err != nil {
			return err
		}
		defer stmt.Close()
		for _, d := range SortedKeys(clean) {
			if _, err := stmt.Exec(FXPair, d, clean[d]); err != nil {
				return err
			}
		}
		return nil
	})
	return len(clean)
}

func (s *Store) BenchmarkPrices(symbol string) map[string]float64 {
	s.must()
	out := map[string]float64{}
	for _, r := range mustRows(s.queryMaps("SELECT date, close FROM benchmark_prices WHERE symbol = ? ORDER BY date", symbol)) {
		out[str(r["date"])] = py.Deref(fnum(r["close"]), 0)
	}
	return out
}

func (s *Store) BenchmarkDays(symbol, start, end string) int {
	s.must()
	if len(start) > 10 {
		start = start[:10]
	}
	if len(end) > 10 {
		end = end[:10]
	}
	var n int
	_ = s.db.QueryRow("SELECT COUNT(*) AS n FROM benchmark_prices WHERE symbol = ? AND date >= ? AND date <= ?", symbol, start, end).Scan(&n)
	return n
}

func (s *Store) BenchmarkLastDate(symbol string) string {
	s.must()
	var d sql.NullString
	_ = s.db.QueryRow("SELECT MAX(date) AS d FROM benchmark_prices WHERE symbol = ?", symbol).Scan(&d)
	return d.String
}

func (s *Store) UpsertBenchmarkPrices(mapping map[string]float64, symbol string) int {
	clean := cleanDateMap(mapping)
	if len(clean) == 0 {
		return 0
	}
	s.must()
	_ = s.tx(func(tx *sql.Tx) error {
		stmt, err := tx.Prepare("INSERT OR IGNORE INTO benchmark_prices(symbol, date, close) VALUES (?, ?, ?)")
		if err != nil {
			return err
		}
		defer stmt.Close()
		for _, d := range SortedKeys(clean) {
			if _, err := stmt.Exec(symbol, d, clean[d]); err != nil {
				return err
			}
		}
		return nil
	})
	return len(clean)
}

type Distribution struct {
	ExDate   string  `json:"exDate"`
	PayDate  string  `json:"payDate"`
	Amount   float64 `json:"amount"`
	Currency string  `json:"currency"`
}

func (s *Store) Distributions() map[string][]Distribution {
	s.must()
	out := map[string][]Distribution{}
	for _, r := range mustRows(s.queryMaps("SELECT * FROM distributions ORDER BY symbol, ex_date DESC")) {
		sym := str(r["symbol"])
		out[sym] = append(out[sym], Distribution{ExDate: str(r["ex_date"]), PayDate: str(r["pay_date"]), Amount: py.Deref(fnum(r["amount"]), 0), Currency: str(r["currency"])})
	}
	return out
}

func (s *Store) UpsertDistributions(symbol string, rows []Distribution, source string) int {
	sym := strings.ToUpper(strings.TrimSpace(symbol))
	if sym == "" {
		return 0
	}
	if source == "" {
		source = "tmx"
	}
	type row struct {
		ex, pay, ccy any
		amt          float64
	}
	var clean []row
	for _, r := range rows {
		ex := r.ExDate
		if len(ex) > 10 {
			ex = ex[:10]
		}
		if len(ex) != 10 || r.Amount <= 0 {
			continue
		}
		pay := r.PayDate
		if len(pay) > 10 {
			pay = pay[:10]
		}
		clean = append(clean, row{ex, nullStr(pay), nullStr(r.Currency), r.Amount})
	}
	if len(clean) == 0 {
		return 0
	}
	s.must()
	_ = s.tx(func(tx *sql.Tx) error {
		for _, c := range clean {
			if _, err := tx.Exec("INSERT INTO distributions(symbol, ex_date, pay_date, amount, currency, source) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(symbol, ex_date, source) DO UPDATE SET pay_date = excluded.pay_date, amount = excluded.amount, currency = excluded.currency",
				sym, c.ex, c.pay, c.amt, c.ccy, source); err != nil {
				return err
			}
		}
		return nil
	})
	return len(clean)
}

type Quote struct {
	Price             *float64 `json:"price"`
	PriceChange       *float64 `json:"priceChange"`
	PercentChange     *float64 `json:"percentChange"`
	PrevClose         *float64 `json:"prevClose"`
	DividendAmount    *float64 `json:"dividendAmount"`
	DividendFrequency string   `json:"dividendFrequency"`
	ExDividendDate    string   `json:"exDividendDate"`
	Source            string   `json:"source"`
	FetchedAt         string   `json:"fetchedAt"`
	Currency          string   `json:"currency,omitempty"`
	Name              string   `json:"name,omitempty"`
	Exchange          string   `json:"exchange,omitempty"`
}

func (s *Store) Quotes() map[string]Quote {
	s.must()
	out := map[string]Quote{}
	for _, r := range mustRows(s.queryMaps("SELECT * FROM quotes")) {
		out[str(r["symbol"])] = Quote{Price: fnum(r["price"]), PriceChange: fnum(r["price_change"]), PercentChange: fnum(r["percent_change"]), PrevClose: fnum(r["prev_close"]), DividendAmount: fnum(r["dividend_amount"]),
			DividendFrequency: str(r["dividend_frequency"]), ExDividendDate: str(r["ex_dividend_date"]), Source: str(r["source"]), FetchedAt: str(r["fetched_at"])}
	}
	return out
}

func (s *Store) UpsertQuote(symbol string, rec Quote, source string) {
	sym := strings.ToUpper(strings.TrimSpace(symbol))
	if sym == "" {
		return
	}
	if source == "" {
		source = "tmx"
	}
	ex := rec.ExDividendDate
	if len(ex) > 10 {
		ex = ex[:10]
	}
	at := rec.FetchedAt
	if at == "" {
		at = nowISO()
	}
	s.must()
	_, _ = s.exec("INSERT INTO quotes(symbol, price, price_change, percent_change, prev_close, dividend_amount, dividend_frequency, ex_dividend_date, source, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(symbol) DO UPDATE SET price = excluded.price, price_change = excluded.price_change, percent_change = excluded.percent_change, prev_close = excluded.prev_close, dividend_amount = COALESCE(excluded.dividend_amount, quotes.dividend_amount), dividend_frequency = CASE WHEN excluded.dividend_frequency = '' THEN quotes.dividend_frequency ELSE excluded.dividend_frequency END, ex_dividend_date = CASE WHEN excluded.ex_dividend_date = '' THEN quotes.ex_dividend_date ELSE excluded.ex_dividend_date END, source = excluded.source, fetched_at = excluded.fetched_at",
		sym, nullable(rec.Price), nullable(rec.PriceChange), nullable(rec.PercentChange), nullable(rec.PrevClose), nullable(rec.DividendAmount), rec.DividendFrequency, ex, source, at)
}

func (s *Store) QuoteFetchedAt() map[string]string {
	s.must()
	out := map[string]string{}
	for _, r := range mustRows(s.queryMaps("SELECT symbol, fetched_at FROM quotes")) {
		out[str(r["symbol"])] = str(r["fetched_at"])
	}
	return out
}

func (s *Store) DistributionsFetchedAt() map[string]string {
	s.must()
	out := map[string]string{}
	for _, r := range mustRows(s.queryMaps("SELECT symbol, fetched_at FROM distribution_fetches")) {
		out[str(r["symbol"])] = str(r["fetched_at"])
	}
	return out
}

func (s *Store) MarkDistributionsFetched(symbol, when string) {
	sym := strings.ToUpper(strings.TrimSpace(symbol))
	if sym == "" || when == "" {
		return
	}
	s.must()
	_, _ = s.exec("INSERT INTO distribution_fetches(symbol, fetched_at) VALUES (?, ?) ON CONFLICT(symbol) DO UPDATE SET fetched_at = excluded.fetched_at", sym, when)
}

type DailyBar struct {
	Date   string   `json:"date"`
	Open   *float64 `json:"open"`
	High   *float64 `json:"high"`
	Low    *float64 `json:"low"`
	Close  float64  `json:"close"`
	Volume *float64 `json:"volume"`
}

func (s *Store) PriceHistory(symbol, start, end string) []DailyBar {
	sym := strings.ToUpper(strings.TrimSpace(symbol))
	if sym == "" {
		return []DailyBar{}
	}
	if len(start) > 10 {
		start = start[:10]
	}
	if len(end) > 10 {
		end = end[:10]
	}
	if start == "" {
		start = "0000-01-01"
	}
	if end == "" {
		end = "9999-12-31"
	}
	s.must()
	out := []DailyBar{}
	for _, r := range mustRows(s.queryMaps("SELECT date, open, high, low, close, volume FROM price_history WHERE symbol = ? AND date >= ? AND date <= ? ORDER BY date", sym, start, end)) {
		out = append(out, DailyBar{Date: str(r["date"]), Open: fnum(r["open"]), High: fnum(r["high"]), Low: fnum(r["low"]), Close: py.Deref(fnum(r["close"]), 0), Volume: fnum(r["volume"])})
	}
	return out
}

func (s *Store) UpsertPriceHistory(symbol string, bars []DailyBar, source string) int {
	sym := strings.ToUpper(strings.TrimSpace(symbol))
	var clean []DailyBar
	for _, b := range bars {
		d := b.Date
		if len(d) > 10 {
			d = d[:10]
		}
		if len(d) != 10 || d[4] != '-' || b.Close <= 0 {
			continue
		}
		b.Date = d
		clean = append(clean, b)
	}
	if sym == "" || len(clean) == 0 {
		return 0
	}
	s.must()
	_ = s.tx(func(tx *sql.Tx) error {
		var newest sql.NullString
		_ = tx.QueryRow("SELECT MAX(date) FROM price_history WHERE symbol = ?", sym).Scan(&newest)
		stmt, err := tx.Prepare("INSERT OR IGNORE INTO price_history(symbol, date, open, high, low, close, volume, source) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
		if err != nil {
			return err
		}
		defer stmt.Close()
		for _, c := range clean {
			if _, err := stmt.Exec(sym, c.Date, nullable(c.Open), nullable(c.High), nullable(c.Low), c.Close, nullable(c.Volume), source); err != nil {
				return err
			}
		}
		if newest.Valid && newest.String != "" {
			for _, c := range clean {
				if c.Date == newest.String {
					if _, err := tx.Exec("UPDATE price_history SET open = ?, high = ?, low = ?, close = ?, volume = ?, source = ? WHERE symbol = ? AND date = ?", nullable(c.Open), nullable(c.High), nullable(c.Low), c.Close, nullable(c.Volume), source, sym, c.Date); err != nil {
						return err
					}
				}
			}
		}
		return nil
	})
	return len(clean)
}

type Fetch struct {
	Start     string
	FetchedAt string
}

func (s *Store) HistoryFetch(symbol string) *Fetch {
	s.must()
	r, _ := s.queryOne("SELECT start, fetched_at FROM history_fetches WHERE symbol = ?", strings.ToUpper(strings.TrimSpace(symbol)))
	if r == nil {
		return nil
	}
	return &Fetch{Start: str(r["start"]), FetchedAt: str(r["fetched_at"])}
}

func (s *Store) MarkHistoryFetched(symbol, start, when string) {
	sym := strings.ToUpper(strings.TrimSpace(symbol))
	if sym == "" || when == "" {
		return
	}
	if len(start) > 10 {
		start = start[:10]
	}
	s.must()
	_, _ = s.exec("INSERT INTO history_fetches(symbol, start, fetched_at) VALUES (?, ?, ?) ON CONFLICT(symbol) DO UPDATE SET start = MIN(history_fetches.start, excluded.start), fetched_at = excluded.fetched_at", sym, start, when)
}

type Bar struct {
	Time   int64    `json:"time"`
	Open   *float64 `json:"open"`
	High   *float64 `json:"high"`
	Low    *float64 `json:"low"`
	Close  float64  `json:"close"`
	Volume *float64 `json:"volume"`
}

func (s *Store) PriceBars(symbol, tf string, startTs, endTs int64) []Bar {
	s.must()
	out := []Bar{}
	for _, r := range mustRows(s.queryMaps("SELECT ts, open, high, low, close, volume FROM price_bars WHERE symbol = ? AND tf = ? AND ts >= ? AND ts <= ? ORDER BY ts", strings.ToUpper(strings.TrimSpace(symbol)), tf, startTs, endTs)) {
		out = append(out, Bar{Time: inum(r["ts"]), Open: fnum(r["open"]), High: fnum(r["high"]), Low: fnum(r["low"]), Close: py.Deref(fnum(r["close"]), 0), Volume: fnum(r["volume"])})
	}
	return out
}

func (s *Store) UpsertPriceBars(symbol, tf string, bars []Bar, source string) int {
	sym := strings.ToUpper(strings.TrimSpace(symbol))
	var clean []Bar
	for _, b := range bars {
		if b.Close <= 0 {
			continue
		}
		clean = append(clean, b)
	}
	if sym == "" || len(clean) == 0 {
		return 0
	}
	s.must()
	_ = s.tx(func(tx *sql.Tx) error {
		var newest sql.NullInt64
		_ = tx.QueryRow("SELECT MAX(ts) FROM price_bars WHERE symbol = ? AND tf = ?", sym, tf).Scan(&newest)
		for _, c := range clean {
			if _, err := tx.Exec("INSERT OR IGNORE INTO price_bars(symbol, tf, ts, open, high, low, close, volume, source) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)", sym, tf, c.Time, nullable(c.Open), nullable(c.High), nullable(c.Low), c.Close, nullable(c.Volume), source); err != nil {
				return err
			}
		}
		if newest.Valid {
			for _, c := range clean {
				if c.Time == newest.Int64 {
					if _, err := tx.Exec("UPDATE price_bars SET open = ?, high = ?, low = ?, close = ?, volume = ?, source = ? WHERE symbol = ? AND tf = ? AND ts = ?", nullable(c.Open), nullable(c.High), nullable(c.Low), c.Close, nullable(c.Volume), source, sym, tf, c.Time); err != nil {
						return err
					}
				}
			}
		}
		return nil
	})
	return len(clean)
}

func (s *Store) LastBarTime(symbol, tf string) *int64 {
	s.must()
	var ts sql.NullInt64
	_ = s.db.QueryRow("SELECT MAX(ts) AS ts FROM price_bars WHERE symbol = ? AND tf = ?", strings.ToUpper(strings.TrimSpace(symbol)), tf).Scan(&ts)
	if !ts.Valid {
		return nil
	}
	v := ts.Int64
	return &v
}

type BarFetch struct {
	StartTs   int64
	FetchedAt string
}

func (s *Store) BarFetchOf(symbol, tf string) *BarFetch {
	s.must()
	r, _ := s.queryOne("SELECT start_ts, fetched_at FROM bar_fetches WHERE symbol = ? AND tf = ?", strings.ToUpper(strings.TrimSpace(symbol)), tf)
	if r == nil {
		return nil
	}
	return &BarFetch{StartTs: inum(r["start_ts"]), FetchedAt: str(r["fetched_at"])}
}

func (s *Store) MarkBarsFetched(symbol, tf string, startTs int64, when string) {
	sym := strings.ToUpper(strings.TrimSpace(symbol))
	if sym == "" || when == "" {
		return
	}
	s.must()
	_, _ = s.exec("INSERT INTO bar_fetches(symbol, tf, start_ts, fetched_at) VALUES (?, ?, ?, ?) ON CONFLICT(symbol, tf) DO UPDATE SET start_ts = MIN(bar_fetches.start_ts, excluded.start_ts), fetched_at = excluded.fetched_at", sym, tf, startTs, when)
}

type MarketData struct {
	FX            map[string]float64
	Benchmark     map[string]float64
	Benchmarks    map[string]map[string]float64
	Distributions map[string][]Distribution
	Quotes        map[string]Quote
}

func (s *Store) MarketData() MarketData {
	md := MarketData{FX: s.FXRates(), Benchmark: s.BenchmarkPrices(BenchmarkSymbol), Benchmarks: map[string]map[string]float64{}, Distributions: s.Distributions(), Quotes: s.Quotes()}
	for _, sym := range BenchmarkSymbols {
		md.Benchmarks[sym] = s.BenchmarkPrices(sym)
	}
	return md
}
