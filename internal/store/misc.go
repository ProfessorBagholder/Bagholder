package store

import (
	"database/sql"
	"encoding/json"
	"strings"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

type Exposure struct {
	Sectors   map[string]float64 `json:"sectors"`
	Countries map[string]float64 `json:"countries"`
	Coverage  float64            `json:"coverage"`
	Source    string             `json:"source"`
	AsOf      string             `json:"asOf"`
	Industry  string             `json:"industry"`
	Error     string             `json:"error"`
	FetchedAt string             `json:"fetchedAt"`
}

func jsonMap(v string) map[string]float64 {
	out := map[string]float64{}
	if v == "" {
		return out
	}
	var raw map[string]any
	if json.Unmarshal([]byte(v), &raw) != nil {
		return out
	}
	for k, x := range raw {
		out[k] = py.Num(x, 0)
	}
	return out
}

func exposureFromRow(r map[string]any) Exposure {
	return Exposure{Sectors: jsonMap(str(r["sectors"])), Countries: jsonMap(str(r["countries"])), Coverage: py.Deref(fnum(r["coverage"]), 0), Source: str(r["source"]), AsOf: str(r["as_of"]), Industry: str(r["industry"]), Error: str(r["error"]), FetchedAt: str(r["fetched_at"])}
}

func (s *Store) ReplaceExposure(key string, rec Exposure) {
	s.must()
	sectors, _ := json.Marshal(nonNilMap(rec.Sectors))
	countries, _ := json.Marshal(nonNilMap(rec.Countries))
	_, _ = s.exec("INSERT INTO exposures (key, sectors, countries, coverage, source, as_of, industry, error, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(key) DO UPDATE SET sectors = excluded.sectors, countries = excluded.countries, coverage = excluded.coverage, source = excluded.source, as_of = excluded.as_of, industry = excluded.industry, error = excluded.error, fetched_at = excluded.fetched_at",
		key, string(sectors), string(countries), rec.Coverage, rec.Source, rec.AsOf, rec.Industry, rec.Error, nowISO())
}

func nonNilMap(m map[string]float64) map[string]float64 {
	if m == nil {
		return map[string]float64{}
	}
	return m
}

func (s *Store) ExposureRecord(key string) *Exposure {
	s.must()
	r, _ := s.queryOne("SELECT * FROM exposures WHERE key = ?", key)
	if r == nil {
		return nil
	}
	e := exposureFromRow(r)
	return &e
}

func (s *Store) ExposuresMap() map[string]Exposure {
	s.must()
	out := map[string]Exposure{}
	for _, r := range mustRows(s.queryMaps("SELECT * FROM exposures")) {
		out[str(r["key"])] = exposureFromRow(r)
	}
	return out
}

type Watch struct {
	Symbol     string `json:"symbol"`
	Exchange   string `json:"exchange"`
	Name       string `json:"name"`
	Currency   string `json:"currency"`
	SecurityID string `json:"securityId"`
	AddedAt    string `json:"addedAt"`
}

func watchFromRow(r map[string]any) Watch {
	return Watch{Symbol: str(r["symbol"]), Exchange: str(r["exchange"]), Name: str(r["name"]), Currency: str(r["currency"]), SecurityID: str(r["security_id"]), AddedAt: str(r["added_at"])}
}

func (s *Store) ListWatchlist() []Watch {
	s.must()
	out := []Watch{}
	for _, r := range mustRows(s.queryMaps("SELECT * FROM watchlist ORDER BY added_at, symbol")) {
		out = append(out, watchFromRow(r))
	}
	return out
}

func (s *Store) AddWatch(symbol, exchange, name, currency, securityID, now string) *Watch {
	sym := strings.ToUpper(strings.TrimSpace(symbol))
	ex := strings.ToUpper(strings.TrimSpace(exchange))
	if sym == "" {
		return nil
	}
	when := now
	if when == "" {
		when = nowISO()
	}
	ccy := strings.ToUpper(currency)
	s.must()
	var out *Watch
	_ = s.tx(func(tx *sql.Tx) error {
		if _, err := tx.Exec("INSERT OR IGNORE INTO watchlist (symbol, exchange, name, currency, security_id, added_at) VALUES (?, ?, ?, ?, ?, ?)", sym, ex, name, ccy, securityID, when); err != nil {
			return err
		}
		if _, err := tx.Exec("UPDATE watchlist SET name = CASE WHEN COALESCE(name, '') = '' THEN ? ELSE name END, currency = CASE WHEN COALESCE(currency, '') = '' THEN ? ELSE currency END, security_id = CASE WHEN COALESCE(security_id, '') = '' THEN ? ELSE security_id END WHERE symbol = ? AND exchange = ?", name, ccy, securityID, sym, ex); err != nil {
			return err
		}
		rows, err := tx.Query("SELECT * FROM watchlist WHERE symbol = ? AND exchange = ?", sym, ex)
		if err != nil {
			return err
		}
		defer rows.Close()
		if rows.Next() {
			m, err := scanRow(rows)
			if err != nil {
				return err
			}
			w := watchFromRow(m)
			out = &w
		}
		return nil
	})
	return out
}

func (s *Store) RemoveWatch(symbol, exchange string) bool {
	s.must()
	res, err := s.exec("DELETE FROM watchlist WHERE symbol = ? AND exchange = ?", strings.ToUpper(strings.TrimSpace(symbol)), strings.ToUpper(strings.TrimSpace(exchange)))
	if err != nil {
		return false
	}
	n, _ := res.RowsAffected()
	return n > 0
}

func NewsKey(symbol, exchange string) string {
	return strings.ToUpper(strings.TrimSpace(symbol)) + "@" + strings.ToUpper(strings.TrimSpace(exchange))
}

type NewsItem struct {
	ID          string `json:"id"`
	Symbol      string `json:"symbol"`
	Exchange    string `json:"exchange"`
	Source      string `json:"source"`
	Headline    string `json:"headline"`
	Wire        string `json:"wire"`
	URL         string `json:"url"`
	PublishedAt string `json:"publishedAt"`
	FetchedAt   string `json:"fetchedAt"`
	Kind        string `json:"kind"`
}

func newsFromRow(r map[string]any) NewsItem {
	kind := str(r["kind"])
	if kind == "" {
		kind = "story"
	}
	return NewsItem{ID: str(r["id"]), Symbol: str(r["symbol"]), Exchange: str(r["exchange"]), Source: str(r["source"]), Headline: str(r["headline"]), Wire: str(r["wire"]), URL: str(r["url"]), PublishedAt: str(r["published_at"]), FetchedAt: str(r["fetched_at"]), Kind: kind}
}

type WireItem struct {
	ID          string `json:"id"`
	Headline    string `json:"headline"`
	Source      string `json:"source"`
	URL         string `json:"url"`
	PublishedAt string `json:"publishedAt"`
	Kind        string `json:"kind"`
	Via         string `json:"via,omitempty"`
}

func (s *Store) ReplaceNews(symbol, exchange, source string, rows []WireItem, now string) {
	sym, ex := strings.ToUpper(strings.TrimSpace(symbol)), strings.ToUpper(strings.TrimSpace(exchange))
	when := now
	if when == "" {
		when = nowISO()
	}
	s.must()
	_ = s.tx(func(tx *sql.Tx) error {
		if _, err := tx.Exec("DELETE FROM news WHERE symbol = ? AND exchange = ?", sym, ex); err != nil {
			return err
		}
		for _, r := range rows {
			if r.ID == "" {
				continue
			}
			kind := r.Kind
			if kind == "" {
				kind = "story"
			}
			via := r.Via
			if via == "" {
				via = source
			}
			if _, err := tx.Exec("INSERT OR REPLACE INTO news (id, symbol, exchange, source, headline, wire, url, published_at, fetched_at, kind) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", r.ID, sym, ex, via, r.Headline, r.Source, r.URL, r.PublishedAt, when, kind); err != nil {
				return err
			}
		}
		_, err := tx.Exec("INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)", "news_fetched:"+NewsKey(sym, ex), when)
		return err
	})
}

func (s *Store) NewsFor(symbol, exchange string) []NewsItem {
	s.must()
	out := []NewsItem{}
	for _, r := range mustRows(s.queryMaps("SELECT * FROM news WHERE symbol = ? AND exchange = ? ORDER BY published_at DESC, id", strings.ToUpper(strings.TrimSpace(symbol)), strings.ToUpper(strings.TrimSpace(exchange)))) {
		out = append(out, newsFromRow(r))
	}
	return out
}

func (s *Store) NewsIDs(symbol, exchange string) map[string]bool {
	s.must()
	out := map[string]bool{}
	for _, r := range mustRows(s.queryMaps("SELECT id FROM news WHERE symbol = ? AND exchange = ?", strings.ToUpper(strings.TrimSpace(symbol)), strings.ToUpper(strings.TrimSpace(exchange)))) {
		out[str(r["id"])] = true
	}
	return out
}

func (s *Store) HasWireRelease(symbol string) bool {
	sym := strings.ToUpper(strings.TrimSpace(symbol))
	if sym == "" {
		return false
	}
	s.must()
	var one int
	err := s.db.QueryRow("SELECT 1 FROM news WHERE kind = 'release' AND (symbol = ? OR symbol LIKE ?) LIMIT 1", sym, sym+".%").Scan(&one)
	return err == nil
}

func (s *Store) NewsFetchedAt() map[string]string {
	return stripPrefix(s.MetaLike("news_fetched:"), "news_fetched:")
}

func stripPrefix(rows map[string]string, prefix string) map[string]string {
	out := make(map[string]string, len(rows))
	for k, v := range rows {
		out[strings.TrimPrefix(k, prefix)] = v
	}
	return out
}

func (s *Store) ForgetNews(symbol, exchange string) {
	s.must()
	_ = s.tx(func(tx *sql.Tx) error {
		if _, err := tx.Exec("DELETE FROM news WHERE symbol = ? AND exchange = ?", strings.ToUpper(strings.TrimSpace(symbol)), strings.ToUpper(strings.TrimSpace(exchange))); err != nil {
			return err
		}
		tail := ":" + NewsKey(symbol, exchange)
		_, err := tx.Exec("DELETE FROM meta WHERE key = ? OR (key LIKE 'news_source_fetched:%' AND substr(key, -length(?)) = ?)", "news_fetched:"+NewsKey(symbol, exchange), tail, tail)
		return err
	})
}

func (s *Store) TrimNews(keep int) {
	s.must()
	_, _ = s.exec("DELETE FROM news WHERE rowid NOT IN (SELECT rowid FROM news ORDER BY published_at DESC, id LIMIT ?)", keep)
}

func FilingKey(symbol string) string { return strings.ToUpper(strings.TrimSpace(symbol)) }

type Filing struct {
	ID            string `json:"id"`
	Source        string `json:"source"`
	Category      string `json:"category"`
	ProfileNo     string `json:"profileNo"`
	Issuer        string `json:"issuer"`
	Type          string `json:"type"`
	Title         string `json:"title"`
	Date          string `json:"date"`
	DateText      string `json:"dateText"`
	Size          string `json:"size"`
	URL           string `json:"url"`
	Subject       string `json:"subject"`
	Summary       string `json:"summary"`
	EnrichedAt    string `json:"enrichedAt"`
	EnrichVersion int    `json:"enrichVersion"`
	EnrichFinal   bool   `json:"enrichFinal"`
	FetchedAt     string `json:"fetchedAt"`
	Symbol        string `json:"symbol,omitempty"`
	Exchange      string `json:"exchange,omitempty"`
}

func (f Filing) MarshalJSON() ([]byte, error) {
	type plain Filing
	type out struct {
		plain
		EnrichVersion any `json:"enrichVersion"`
	}
	o := out{plain: plain(f)}
	if f.EnrichVersion == 0 {
		o.EnrichVersion = ""
	} else {
		o.EnrichVersion = f.EnrichVersion
	}
	return json.Marshal(o)
}

func filingFromRow(r map[string]any) Filing {
	get := func(k string) string {
		if v, ok := r[k]; ok {
			return str(v)
		}
		return ""
	}
	typ := get("type")
	if typ == "" {
		typ = get("file")
	}
	date := get("date")
	if date == "" {
		date = get("submitted_at")
	}
	dateText := get("date_text")
	if dateText == "" {
		dateText = get("submitted")
	}
	return Filing{ID: get("id"), Source: get("source"), Category: get("category"), ProfileNo: get("profile_no"), Issuer: get("issuer"), Type: typ, Title: get("title"),
		Date: date, DateText: dateText, Size: get("size"), URL: get("url"), Subject: get("subject"), Summary: get("summary"), EnrichedAt: get("enriched_at"),
		EnrichVersion: int(inum(r["enrich_version"])), EnrichFinal: inum(r["enrich_final"]) != 0, FetchedAt: get("fetched_at")}
}

func (s *Store) Filings(symbol string) []Filing {
	s.must()
	out := []Filing{}
	for _, r := range mustRows(s.queryMaps("SELECT * FROM filings WHERE symbol = ? ORDER BY date DESC, id", FilingKey(symbol))) {
		out = append(out, filingFromRow(r))
	}
	return out
}

func (s *Store) AllFilings() map[string][]Filing {
	s.must()
	out := map[string][]Filing{}
	for _, r := range mustRows(s.queryMaps("SELECT * FROM filings ORDER BY symbol, date DESC, id")) {
		sym := str(r["symbol"])
		out[sym] = append(out[sym], filingFromRow(r))
	}
	return out
}

func (s *Store) Filing(symbol, docID string) *Filing {
	s.must()
	r, _ := s.queryOne("SELECT * FROM filings WHERE symbol = ? AND id = ?", FilingKey(symbol), docID)
	if r == nil {
		return nil
	}
	f := filingFromRow(r)
	return &f
}

func (s *Store) SetFilingEnrichment(symbol, docID string, subject, summary *string, version *int, final *bool) {
	sets := []string{"enriched_at = ?"}
	args := []any{nowISO()}
	if final != nil {
		sets = append(sets, "enrich_final = ?")
		if *final {
			args = append(args, 1)
		} else {
			args = append(args, 0)
		}
	}
	if subject != nil {
		sets = append(sets, "subject = ?")
		args = append(args, *subject)
	}
	if summary != nil {
		sets = append(sets, "summary = ?")
		args = append(args, *summary)
	}
	if version != nil {
		sets = append(sets, "enrich_version = ?")
		args = append(args, *version)
	}
	args = append(args, FilingKey(symbol), docID)
	s.must()
	_, _ = s.exec("UPDATE filings SET "+strings.Join(sets, ", ")+" WHERE symbol = ? AND id = ?", args...)
}

type FilingItem struct {
	ID        string `json:"id"`
	Source    string `json:"source"`
	Category  string `json:"category"`
	Date      string `json:"date"`
	DateText  string `json:"dateText"`
	Type      string `json:"type"`
	Title     string `json:"title"`
	Size      string `json:"size"`
	URL       string `json:"url"`
	Issuer    string `json:"issuer,omitempty"`
	ProfileNo string `json:"profileNo,omitempty"`
}

func (s *Store) ReplaceFilings(symbol, source string, items []FilingItem, now string) int {
	sym := FilingKey(symbol)
	when := now
	if when == "" {
		when = nowISO()
	}
	var clean []FilingItem
	for _, r := range items {
		if r.ID == "" {
			continue
		}
		clean = append(clean, r)
	}
	s.must()
	_ = s.tx(func(tx *sql.Tx) error {
		type read struct {
			subject, summary string
			enrichedAt       any
			version, final   any
		}
		kept := map[string]read{}
		rows, err := tx.Query("SELECT id, subject, summary, enriched_at, enrich_version, enrich_final FROM filings WHERE symbol = ? AND source = ?", sym, source)
		if err != nil {
			return err
		}
		for rows.Next() {
			var id string
			var subject, summary, enrichedAt sql.NullString
			var version, final sql.NullInt64
			if err := rows.Scan(&id, &subject, &summary, &enrichedAt, &version, &final); err != nil {
				rows.Close()
				return err
			}
			rd := read{subject: subject.String, summary: summary.String}
			if enrichedAt.Valid {
				rd.enrichedAt = enrichedAt.String
			}
			if version.Valid {
				rd.version = version.Int64
			}
			if final.Valid {
				rd.final = final.Int64
			}
			kept[id] = rd
		}
		rows.Close()
		if _, err := tx.Exec("DELETE FROM filings WHERE symbol = ? AND source = ?", sym, source); err != nil {
			return err
		}
		for _, r := range clean {
			src := source
			if src == "" {
				src = r.Source
			}
			rd, ok := kept[r.ID]
			var subject, summary string
			var enrichedAt, version, final any
			if ok {
				subject, summary, enrichedAt, version, final = rd.subject, rd.summary, rd.enrichedAt, rd.version, rd.final
			}
			if _, err := tx.Exec("INSERT OR REPLACE INTO filings (symbol, id, source, category, profile_no, issuer, type, title, date, date_text, size, url, fetched_at, subject, summary, enriched_at, enrich_version, enrich_final) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
				sym, r.ID, src, r.Category, r.ProfileNo, r.Issuer, r.Type, r.Title, r.Date, r.DateText, r.Size, r.URL, when, subject, summary, enrichedAt, version, final); err != nil {
				return err
			}
		}
		return nil
	})
	return len(clean)
}

func (s *Store) MarkFilingsFetched(symbol, profileNo, now string) {
	sym := FilingKey(symbol)
	when := now
	if when == "" {
		when = nowISO()
	}
	s.must()
	_ = s.tx(func(tx *sql.Tx) error {
		if _, err := tx.Exec("INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)", "filings_fetched:"+sym, when); err != nil {
			return err
		}
		if profileNo != "" {
			if _, err := tx.Exec("INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)", "sedar_profile:"+sym, profileNo); err != nil {
				return err
			}
		}
		return nil
	})
}

func (s *Store) FilingsFetchedAt(symbol string) string {
	return s.GetMeta("filings_fetched:" + FilingKey(symbol))
}

func (s *Store) AllFilingsFetchedAt() map[string]string {
	return stripPrefix(s.MetaLike("filings_fetched:"), "filings_fetched:")
}

func (s *Store) SedarProfile(symbol string) string {
	return s.GetMeta("sedar_profile:" + FilingKey(symbol))
}

func (s *Store) ForgetFilings(symbol string) {
	sym := FilingKey(symbol)
	s.must()
	_ = s.tx(func(tx *sql.Tx) error {
		if _, err := tx.Exec("DELETE FROM filings WHERE symbol = ?", sym); err != nil {
			return err
		}
		_, err := tx.Exec("DELETE FROM meta WHERE key IN (?, ?)", "filings_fetched:"+sym, "sedar_profile:"+sym)
		return err
	})
}

type ShortPoint struct {
	Date   string   `json:"date"`
	Shares *float64 `json:"shares"`
}

type Short struct {
	Symbol        string       `json:"symbol"`
	Exchange      string       `json:"exchange"`
	FetchedAt     string       `json:"fetchedAt"`
	ReadVersion   int          `json:"readVersion"`
	Market        string       `json:"market"`
	AsOf          string       `json:"asOf"`
	Shares        *float64     `json:"shares"`
	Previous      *float64     `json:"previous"`
	PreviousOf    string       `json:"previousOf"`
	Change        *float64     `json:"change"`
	Float         *float64     `json:"float"`
	OfFloat       *float64     `json:"ofFloat"`
	AverageVolume *float64     `json:"averageVolume"`
	DaysToCover   *float64     `json:"daysToCover"`
	VolumeOf      string       `json:"volumeOf"`
	VolumeSpan    string       `json:"volumeSpan"`
	ShortVolume   *float64     `json:"shortVolume"`
	TotalVolume   *float64     `json:"totalVolume"`
	VolumePct     *float64     `json:"volumePct"`
	Name          string       `json:"name"`
	Series        []ShortPoint `json:"series"`
	Source        string       `json:"source,omitempty"`
	PositionID    string       `json:"positionId,omitempty"`
	Held          *bool        `json:"held,omitempty"`
	Watched       *bool        `json:"watched,omitempty"`
}

func (sh Short) MarshalJSON() ([]byte, error) {
	type plain Short
	type out struct {
		plain
		Market     *string `json:"market"`
		AsOf       *string `json:"asOf"`
		PreviousOf *string `json:"previousOf"`
		VolumeOf   *string `json:"volumeOf"`
		VolumeSpan *string `json:"volumeSpan"`
		Name       *string `json:"name"`
	}
	o := out{plain: plain(sh)}
	set := func(v string) *string {
		if v == "" {
			return nil
		}
		return &v
	}
	o.Market, o.AsOf, o.PreviousOf, o.VolumeOf, o.VolumeSpan, o.Name = set(sh.Market), set(sh.AsOf), set(sh.PreviousOf), set(sh.VolumeOf), set(sh.VolumeSpan), set(sh.Name)
	if o.Series == nil {
		o.Series = []ShortPoint{}
	}
	return json.Marshal(o)
}

func shortFromRow(r map[string]any) Short {
	sh := Short{Symbol: str(r["symbol"]), Exchange: str(r["exchange"]), FetchedAt: str(r["fetched_at"]), ReadVersion: int(inum(r["read_version"])),
		Market: str(r["market"]), AsOf: str(r["as_of"]), Shares: fnum(r["shares"]), Previous: fnum(r["previous"]), PreviousOf: str(r["previous_of"]), Change: fnum(r["change"]),
		Float: fnum(r["float_shares"]), OfFloat: fnum(r["of_float"]), AverageVolume: fnum(r["average_volume"]), DaysToCover: fnum(r["days_to_cover"]), VolumeOf: str(r["volume_of"]),
		VolumeSpan: str(r["volume_span"]), ShortVolume: fnum(r["short_volume"]), TotalVolume: fnum(r["total_volume"]), VolumePct: fnum(r["volume_pct"]), Name: str(r["name"]), Series: []ShortPoint{}}
	if v := str(r["series"]); v != "" {
		var series []ShortPoint
		if json.Unmarshal([]byte(v), &series) == nil && series != nil {
			sh.Series = series
		}
	}
	return sh
}

func (s *Store) SaveShorts(symbol, exchange string, rec Short, now string, version int, hasSeries bool) {
	sym, ex := strings.ToUpper(strings.TrimSpace(symbol)), strings.ToUpper(strings.TrimSpace(exchange))
	when := now
	if when == "" {
		when = nowISO()
	}
	s.must()
	_ = s.tx(func(tx *sql.Tx) error {
		series := rec.Series
		if !hasSeries {
			series = []ShortPoint{}
			var held sql.NullString
			_ = tx.QueryRow("SELECT series FROM shorts WHERE symbol = ? AND exchange = ?", sym, ex).Scan(&held)
			if held.Valid && held.String != "" {
				_ = json.Unmarshal([]byte(held.String), &series)
			}
		}
		if series == nil {
			series = []ShortPoint{}
		}
		sj, _ := json.Marshal(series)
		_, err := tx.Exec("INSERT OR REPLACE INTO shorts (symbol, exchange, market, as_of, shares, previous, previous_of, change, float_shares, of_float, average_volume, days_to_cover, volume_of, volume_span, short_volume, total_volume, volume_pct, name, series, read_version, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
			sym, ex, nullStr(rec.Market), nullStr(rec.AsOf), nullable(rec.Shares), nullable(rec.Previous), nullStr(rec.PreviousOf), nullable(rec.Change), nullable(rec.Float), nullable(rec.OfFloat), nullable(rec.AverageVolume), nullable(rec.DaysToCover), nullStr(rec.VolumeOf), nullStr(rec.VolumeSpan), nullable(rec.ShortVolume), nullable(rec.TotalVolume), nullable(rec.VolumePct), nullStr(rec.Name), string(sj), version, when)
		return err
	})
}

func (s *Store) ShortsFor(symbol, exchange string) *Short {
	s.must()
	r, _ := s.queryOne("SELECT * FROM shorts WHERE symbol = ? AND exchange = ?", strings.ToUpper(strings.TrimSpace(symbol)), strings.ToUpper(strings.TrimSpace(exchange)))
	if r == nil {
		return nil
	}
	sh := shortFromRow(r)
	return &sh
}

func (s *Store) AllShorts() []Short {
	s.must()
	out := []Short{}
	for _, r := range mustRows(s.queryMaps("SELECT * FROM shorts")) {
		out = append(out, shortFromRow(r))
	}
	return out
}

type Gauge struct {
	Index       string         `json:"index"`
	Source      string         `json:"source"`
	Score       *float64       `json:"score"`
	Rating      string         `json:"rating"`
	AsOf        string         `json:"asOf"`
	FetchedAt   string         `json:"fetchedAt"`
	ReadVersion int            `json:"readVersion"`
	Rest        map[string]any `json:"-"`
}

func (g Gauge) MarshalJSON() ([]byte, error) {
	out := map[string]any{"index": g.Index, "source": nullStr(g.Source), "score": nullable(g.Score), "rating": nullStr(g.Rating), "asOf": nullStr(g.AsOf), "fetchedAt": nullStr(g.FetchedAt), "readVersion": g.ReadVersion}
	for k, v := range g.Rest {
		out[k] = v
	}
	return json.Marshal(out)
}

func (s *Store) SaveGauge(name string, rec map[string]any, now string, version int) {
	key := strings.ToLower(strings.TrimSpace(name))
	when := now
	if when == "" {
		when = nowISO()
	}
	rest := map[string]any{}
	for k, v := range rec {
		switch k {
		case "index", "source", "score", "rating", "asOf":
			continue
		}
		rest[k] = v
	}
	payload, _ := json.Marshal(rest)
	var score any
	if f, ok := py.NumOK(rec["score"]); ok {
		score = f
	}
	s.must()
	_, _ = s.exec("INSERT OR REPLACE INTO gauges (name, source, score, rating, as_of, payload, read_version, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)", key, py.S(rec["source"]), score, py.S(rec["rating"]), py.S(rec["asOf"]), string(payload), version, when)
}

func (s *Store) Gauge(name string) *Gauge {
	s.must()
	r, _ := s.queryOne("SELECT * FROM gauges WHERE name = ?", strings.ToLower(strings.TrimSpace(name)))
	if r == nil {
		return nil
	}
	g := Gauge{Index: str(r["name"]), Source: str(r["source"]), Score: fnum(r["score"]), Rating: str(r["rating"]), AsOf: str(r["as_of"]), FetchedAt: str(r["fetched_at"]), ReadVersion: int(inum(r["read_version"])), Rest: map[string]any{}}
	if v := str(r["payload"]); v != "" {
		_ = json.Unmarshal([]byte(v), &g.Rest)
	}
	return &g
}

type Universe struct {
	Symbol        string   `json:"symbol"`
	Name          string   `json:"name"`
	Value         *float64 `json:"value"`
	PercentChange *float64 `json:"percentChange"`
	Sector        string   `json:"sector"`
	Country       string   `json:"country"`
	FetchedAt     string   `json:"fetchedAt"`
}

func (s *Store) universes() map[string][]Universe {
	out := map[string][]Universe{}
	for _, r := range mustRows(s.queryMaps("SELECT * FROM universes ORDER BY key, value DESC, symbol")) {
		key := str(r["key"])
		out[key] = append(out[key], Universe{Symbol: str(r["symbol"]), Name: str(r["name"]), Value: fnum(r["value"]), PercentChange: fnum(r["percent_change"]), Sector: str(r["sector"]), Country: str(r["country"]), FetchedAt: str(r["fetched_at"])})
	}
	return out
}

func (s *Store) Universes() map[string][]Universe {
	s.must()
	return s.universes()
}

func (s *Store) ReplaceUniverse(key string, rows []Universe, now string) {
	when := now
	if when == "" {
		when = nowISO()
	}
	s.must()
	_ = s.tx(func(tx *sql.Tx) error {
		if _, err := tx.Exec("DELETE FROM universes WHERE key = ?", key); err != nil {
			return err
		}
		for _, r := range rows {
			if r.Symbol == "" {
				continue
			}
			if _, err := tx.Exec("INSERT OR REPLACE INTO universes (key, symbol, name, value, percent_change, sector, country, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)", key, r.Symbol, r.Name, nullable(r.Value), nullable(r.PercentChange), r.Sector, r.Country, when); err != nil {
				return err
			}
		}
		return nil
	})
}

type Notification struct {
	ID     int64          `json:"id"`
	At     string         `json:"at"`
	Kind   string         `json:"kind"`
	Key    string         `json:"key"`
	Title  string         `json:"title"`
	Body   string         `json:"body"`
	Extra  map[string]any `json:"extra"`
	SeenAt string         `json:"seenAt"`
	ReadAt string         `json:"readAt"`
}

func notificationFromRow(r map[string]any) Notification {
	n := Notification{ID: inum(r["id"]), At: str(r["at"]), Kind: str(r["kind"]), Key: str(r["key"]), Title: str(r["title"]), Body: str(r["body"]), Extra: map[string]any{}, SeenAt: str(r["seen_at"]), ReadAt: str(r["read_at"])}
	if v := str(r["extra"]); v != "" {
		_ = json.Unmarshal([]byte(v), &n.Extra)
		if n.Extra == nil {
			n.Extra = map[string]any{}
		}
	}
	return n
}

func (s *Store) AddNotification(kind, key, title, body string, extra map[string]any, seen bool) *Notification {
	now := nowISO()
	if extra == nil {
		extra = map[string]any{}
	}
	ej, _ := json.Marshal(extra)
	var out *Notification
	s.must()
	_ = s.tx(func(tx *sql.Tx) error {
		var seenAt any
		if seen {
			seenAt = now
		}
		res, err := tx.Exec("INSERT OR IGNORE INTO notifications(at, kind, key, title, body, extra, seen_at) VALUES (?, ?, ?, ?, ?, ?, ?)", now, kind, key, title, body, string(ej), seenAt)
		if err != nil {
			return err
		}
		if n, _ := res.RowsAffected(); n == 0 {
			return nil
		}
		rid, _ := res.LastInsertId()
		if _, err := tx.Exec("DELETE FROM notifications WHERE id <= (SELECT id FROM notifications ORDER BY id DESC LIMIT 1 OFFSET ?)", NotificationsKept); err != nil {
			return err
		}
		rows, err := tx.Query("SELECT * FROM notifications WHERE id = ?", rid)
		if err != nil {
			return err
		}
		defer rows.Close()
		if rows.Next() {
			m, err := scanRow(rows)
			if err != nil {
				return err
			}
			n := notificationFromRow(m)
			out = &n
		}
		return nil
	})
	return out
}

func (s *Store) ListNotifications(afterID int64, since string, unseen bool, limit int, newest bool) []Notification {
	q := "SELECT * FROM notifications WHERE id > ?"
	args := []any{afterID}
	if since != "" {
		q += " AND at >= ?"
		args = append(args, since)
	}
	if unseen {
		q += " AND seen_at IS NULL"
	}
	if newest {
		q += " ORDER BY id DESC LIMIT ?"
	} else {
		q += " ORDER BY id ASC LIMIT ?"
	}
	if limit <= 0 {
		limit = 50
	}
	args = append(args, limit)
	s.must()
	out := []Notification{}
	for _, r := range mustRows(s.queryMaps(q, args...)) {
		out = append(out, notificationFromRow(r))
	}
	return out
}

func idList(ids []int64) (string, []any) {
	marks := make([]string, len(ids))
	args := make([]any, len(ids))
	for i, id := range ids {
		marks[i] = "?"
		args[i] = id
	}
	return strings.Join(marks, ","), args
}

func (s *Store) MarkNotificationsSeen(ids []int64) int64 {
	if len(ids) == 0 {
		return 0
	}
	marks, args := idList(ids)
	s.must()
	res, err := s.exec("UPDATE notifications SET seen_at = ? WHERE seen_at IS NULL AND id IN ("+marks+")", append([]any{nowISO()}, args...)...)
	if err != nil {
		return 0
	}
	n, _ := res.RowsAffected()
	return n
}

func (s *Store) LatestNotificationID() int64 {
	s.must()
	var m sql.NullInt64
	_ = s.db.QueryRow("SELECT MAX(id) AS m FROM notifications").Scan(&m)
	return m.Int64
}

func (s *Store) UnreadNotifications() int {
	s.must()
	var n int
	_ = s.db.QueryRow("SELECT COUNT(*) AS n FROM notifications WHERE read_at IS NULL").Scan(&n)
	return n
}

func (s *Store) MarkNotificationsRead(ids []int64, all bool) int64 {
	s.must()
	now := nowISO()
	var res sql.Result
	var err error
	if all {
		res, err = s.exec("UPDATE notifications SET read_at = ? WHERE read_at IS NULL", now)
	} else {
		if len(ids) == 0 {
			return 0
		}
		marks, args := idList(ids)
		res, err = s.exec("UPDATE notifications SET read_at = ? WHERE read_at IS NULL AND id IN ("+marks+")", append([]any{now}, args...)...)
	}
	if err != nil {
		return 0
	}
	n, _ := res.RowsAffected()
	return n
}

func (s *Store) ClearNotifications() int64 {
	s.must()
	res, err := s.exec("DELETE FROM notifications")
	if err != nil {
		return 0
	}
	n, _ := res.RowsAffected()
	return n
}

const ToldKeptDays = 400

func (s *Store) EventsTold(scope string, events []string) map[string]bool {
	out := map[string]bool{}
	want := make([]string, 0, len(events))
	for _, e := range events {
		if py.Strip(e) != "" {
			want = append(want, e)
		}
	}
	if len(want) == 0 {
		return out
	}
	s.must()
	for i := 0; i < len(want); i += 400 {
		end := i + 400
		if end > len(want) {
			end = len(want)
		}
		chunk := want[i:end]
		args := make([]any, 0, len(chunk)+1)
		args = append(args, scope)
		marks := make([]string, len(chunk))
		for j, e := range chunk {
			marks[j] = "?"
			args = append(args, e)
		}
		rows, err := s.queryMaps("SELECT event FROM told WHERE scope = ? AND event IN ("+strings.Join(marks, ",")+")", args...)
		if err != nil {
			continue
		}
		for _, r := range rows {
			out[str(r["event"])] = true
		}
	}
	return out
}

func (s *Store) MarkTold(scope string, events []string, now string) int {
	when := py.Strip(now)
	if when == "" {
		when = nowISO()
	}
	rows := make([]string, 0, len(events))
	for _, e := range events {
		if py.Strip(e) != "" {
			rows = append(rows, e)
		}
	}
	if len(rows) == 0 {
		return 0
	}
	cut := time.Now().UTC().Add(-ToldKeptDays * 24 * time.Hour).Format("2006-01-02T15:04:05Z")
	s.must()
	_ = s.tx(func(tx *sql.Tx) error {
		for _, e := range rows {
			if _, err := tx.Exec("INSERT OR IGNORE INTO told(scope, event, at) VALUES (?, ?, ?)", scope, e, when); err != nil {
				return err
			}
		}
		_, err := tx.Exec("DELETE FROM told WHERE at < ?", cut)
		return err
	})
	return len(rows)
}
