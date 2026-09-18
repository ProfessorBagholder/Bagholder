package app

import (
	"crypto/sha1"
	"encoding/hex"
	"encoding/json"
	"regexp"
	"slices"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/disclosures"
	"github.com/ProfessorBagholder/Bagholder/internal/enrich"
	"github.com/ProfessorBagholder/Bagholder/internal/fear"
	"github.com/ProfessorBagholder/Bagholder/internal/instruments"
	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/model"
	"github.com/ProfessorBagholder/Bagholder/internal/news"
	"github.com/ProfessorBagholder/Bagholder/internal/notify"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/shorts"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
	"github.com/ProfessorBagholder/Bagholder/internal/universes"
)

func (a *App) watchAdd(body map[string]any) map[string]any {
	if body == nil {
		body = map[string]any{}
	}
	sym := market.TMXSymbol(py.S(body["symbol"]))
	if sym == "" {
		return map[string]any{"ok": false, "error": "symbol required"}
	}
	exchange := py.S(body["exchange"])
	inst := instruments.Find(sym, exchange)
	name, currency := py.S(body["name"]), py.S(body["currency"])
	if inst != nil {
		if inst.Name != "" {
			name = inst.Name
		}
		if inst.Currency != "" {
			currency = inst.Currency
		}
	}
	row := a.st.AddWatch(sym, exchange, name, currency, py.S(body["securityId"]), "")
	a.invalidate(false)
	go func() {
		a.mk.RefreshQuotes(quoteRecs(model.QuoteSymbols(a.model.Base())), a.mk.Clock())
		a.invalidate(false)
		if inst != nil || strings.ToUpper(exchange) == "CRYPTO" {
			return
		}
		if row != nil {
			rec := a.expo.ShareExposure(row.Symbol, row.Exchange, row.Currency)
			if rec.Error != "" && rec.Source == "" {
				a.logf("bagholder watchlist: sector for %s failed: %s\n", sym, rec.Error)
			}
			a.invalidate(false)
		}
	}()
	return map[string]any{"ok": true, "watchlist": a.st.ListWatchlist()}
}

func (a *App) watchRemove(body map[string]any) map[string]any {
	if body == nil {
		body = map[string]any{}
	}
	sym := market.TMXSymbol(py.S(body["symbol"]))
	if sym == "" {
		return map[string]any{"ok": false, "error": "symbol required"}
	}
	exchange := py.S(body["exchange"])
	a.st.RemoveWatch(sym, exchange)
	a.st.RemoveWatch(strings.ToUpper(py.Strip(py.S(body["symbol"]))), exchange)
	a.st.ForgetNews(sym, exchange)
	a.invalidate(false)
	return map[string]any{"ok": true, "watchlist": a.st.ListWatchlist()}
}

func (a *App) tilesSet(body map[string]any) map[string]any {
	if body == nil {
		body = map[string]any{}
	}
	rows := []store.Tile{}
	seen := map[string]bool{}
	tiles, _ := body["tiles"].([]any)
	for _, raw := range tiles {
		r, ok := raw.(map[string]any)
		if !ok {
			continue
		}
		inst := instruments.Find(py.S(r["symbol"]), py.S(r["exchange"]))
		if inst != nil && !seen[inst.Symbol] {
			seen[inst.Symbol] = true
			rows = append(rows, store.Tile{Symbol: inst.Symbol, Exchange: inst.Exchange})
		}
	}
	if len(rows) > model.TilesMax {
		return map[string]any{"ok": false, "error": "at most " + strconv.Itoa(model.TilesMax) + " tiles"}
	}
	a.st.SaveTiles(rows)
	a.invalidate(false)
	go func() {
		a.mk.RefreshQuotes(quoteRecs(model.QuoteSymbols(a.model.Base())), a.mk.Clock())
		a.invalidate(false)
	}()
	return map[string]any{"ok": true, "tiles": model.TileRows(a.model.Base())}
}

func quoteRecs(rows []model.QuoteSymbol) []market.Rec {
	out := make([]market.Rec, 0, len(rows))
	for _, r := range rows {
		out = append(out, market.Rec{Symbol: r.Symbol, Exchange: r.Exchange, Currency: r.Currency, Kind: r.Kind, QuoteKey: r.QuoteKey, Yahoo: r.Yahoo})
	}
	return out
}

func heldRecs(rows []model.HeldSymbol) []market.Rec {
	out := make([]market.Rec, 0, len(rows))
	for _, r := range rows {
		out = append(out, market.Rec{Symbol: r.Symbol, Exchange: r.Exchange, Currency: r.Currency, Kind: r.Kind})
	}
	return out
}

func (a *App) newsListings() []news.Listing {
	base := a.model.Base()
	seen := map[[2]string]bool{}
	out := []news.Listing{{Symbol: news.Market[0], Exchange: news.Market[1], Currency: news.Market[2]}}
	for _, p := range base.Positions {
		if p.Kind != "Shares" {
			continue
		}
		key := [2]string{market.TMXSymbol(p.Symbol), strings.ToUpper(p.Exchange)}
		if key[0] != "" && !seen[key] {
			seen[key] = true
			out = append(out, news.Listing{Symbol: key[0], Exchange: p.Exchange, Currency: p.Currency, Name: p.Name})
		}
	}
	for _, w := range base.Watchlist {
		key := [2]string{market.TMXSymbol(w.Symbol), strings.ToUpper(w.Exchange)}
		if key[0] != "" && !seen[key] && instruments.Find(w.Symbol, w.Exchange) == nil && key[1] != "CRYPTO" {
			seen[key] = true
			out = append(out, news.Listing{Symbol: key[0], Exchange: w.Exchange, Currency: w.Currency, Name: w.Name})
		}
	}
	return out
}

func (a *App) refreshNews() int {
	if !a.singleFlightStart("news") {
		return 0
	}
	defer a.singleFlightEnd("news")
	key := func(l news.Listing) string {
		k := market.TMXSymbol(l.Symbol)
		if k == "" {
			k = l.Symbol
		}
		return strings.ToUpper(k)
	}
	start := func(due []news.Listing) {
		a.newsMu.Lock()
		a.newsLeft = map[string]bool{}
		for _, l := range due {
			a.newsLeft[key(l)] = true
		}
		a.newsMu.Unlock()
	}
	done := func(l news.Listing, answered bool) {
		a.newsMu.Lock()
		delete(a.newsLeft, key(l))
		a.newsMu.Unlock()
		a.invalidateSoon()
	}
	defer func() {
		a.newsMu.Lock()
		a.newsLeft = map[string]bool{}
		a.newsMu.Unlock()
	}()
	n := news.Refresh(a.mk, a.newsListings(), a.mk.Clock(), a.noteWireReleases, start, done)
	if n > 0 {
		a.invalidate(false)
	}
	return n
}

func (a *App) newsReading() []string {
	a.newsMu.Lock()
	defer a.newsMu.Unlock()
	out := make([]string, 0, len(a.newsLeft))
	for k := range a.newsLeft {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}

func (a *App) newsLoop() {
	for !a.stopped() {
		a.refreshNews()
		if a.wait(300 * time.Second) {
			return
		}
	}
}

const (
	FilingsStaleHours    = 24
	FilingsSweepEverySec = 300
	FilingsHoldMaxMin    = 120
	FilingsHoldKey       = "filings:held-since"
	FilingsSweepAgeMin   = 30
	FearStaleMin         = 15
	FearVersion          = 1
	FearSweepEverySec    = 900
	ShortsStaleHours     = 6
	ShortsVersion        = 5
	ShortsSweepEverySec  = 1800
)

var FeedScopes = map[string][]string{"holdings": {"held"}, "watchlist": {"watched"}, "all": {"all"}}

func (a *App) instrumentMeta(symbol string) (string, string, string) {
	sym := strings.ToUpper(py.Strip(symbol))
	for _, sec := range a.st.ListSecurities() {
		if strings.ToUpper(py.Strip(sec.Symbol)) == sym {
			name := py.Strip(sec.Name)
			if name == "" {
				name = sym
			}
			return name, py.Strip(sec.PrimaryExchange), py.Strip(sec.Currency)
		}
	}
	return sym, "", ""
}

func parseStamp(text string) (time.Time, bool) {
	t, _, ok := py.ParseISO(strings.Replace(text, "Z", "+00:00", 1))
	return t, ok
}

func (a *App) filingsStale(symbol string, now time.Time, hours float64) bool {
	when := a.st.FilingsFetchedAt(symbol)
	if when == "" {
		return true
	}
	t, ok := parseStamp(when)
	if !ok {
		return true
	}
	return now.Sub(t) > time.Duration(hours*float64(time.Hour))
}

func (a *App) canNameDocuments() bool {
	if a.enricher.SummaryAvailable() {
		return true
	}
	status := a.enricher.SummaryStatus()
	if status == "detecting" || status == "starting" {
		return a.enricher.WaitForSummary(enrich.SummaryWaitSec)
	}
	return status != "downloading"
}

func (a *App) namingHeld(now time.Time, holding bool) bool {
	if !holding {
		a.st.SetMeta(FilingsHoldKey, "")
		return false
	}
	since := py.S(a.st.GetMeta(FilingsHoldKey))
	if since == "" {
		a.st.SetMeta(FilingsHoldKey, py.Stamp(now))
		return true
	}
	t, ok := parseStamp(since)
	if !ok {
		return true
	}
	return now.Sub(t) <= FilingsHoldMaxMin*time.Minute
}

func filingMark(r store.Filing) string {
	return r.Source + "\x00" + r.Date + "\x00" + r.Type + "\x00" + r.Title + "\x00" + r.Size
}

func filingMarkSlash(r store.Filing) string {
	return strings.Join([]string{r.Source, r.Date, r.Type, r.Title, r.Size}, "/")
}

type filingSymbol struct {
	Symbol   string
	Name     string
	Exchange string
	Currency string
}

type symbolRow struct {
	symbol, name, exchange, currency, kind string
}

func (a *App) knownFilingSymbols(scopes map[string]bool) []filingSymbol {
	out := []filingSymbol{}
	seen := map[string]bool{}
	var rows []symbolRow
	var base *model.Base
	if scopes["held"] || scopes["all"] {
		base = a.model.Base()
	}
	if base != nil && (scopes["held"] || scopes["all"]) {
		for _, h := range model.HeldSymbols(base) {
			rows = append(rows, symbolRow{symbol: h.Symbol, exchange: h.Exchange, currency: h.Currency, kind: h.Kind})
		}
	}
	if base != nil && scopes["all"] {
		for _, t := range base.Trades {
			rec := symbolRow{symbol: t.Symbol, exchange: t.Exchange, currency: t.Currency, kind: t.Kind}
			if rec.kind == "Options" {
				under := model.UnderlyingSymbol(rec.symbol)
				if under == "" || under == "—" {
					continue
				}
				rec = symbolRow{symbol: under, exchange: rec.exchange, currency: rec.currency, kind: "Shares"}
			}
			rows = append(rows, rec)
		}
	}
	if scopes["watched"] || scopes["all"] {
		for _, w := range a.st.ListWatchlist() {
			rows = append(rows, symbolRow{symbol: w.Symbol, name: w.Name, exchange: w.Exchange, currency: w.Currency})
		}
	}
	for _, r := range rows {
		sym := strings.ToUpper(py.Strip(r.symbol))
		if sym == "" || seen[sym] || strings.Contains(sym, " ") || r.kind == "Options" || r.kind == "Crypto" {
			continue
		}
		if len(a.pipeline.ProvidersFor(sym, r.exchange, r.currency)) == 0 {
			continue
		}
		seen[sym] = true
		out = append(out, filingSymbol{Symbol: sym, Name: r.name, Exchange: r.exchange, Currency: r.currency})
	}
	return out
}

func unionScopes(a, b map[string]bool) map[string]bool {
	out := map[string]bool{}
	for k := range a {
		out[k] = true
	}
	for k := range b {
		out[k] = true
	}
	return out
}

func sha1Short(text string, n int) string {
	sum := sha1.Sum([]byte(text))
	return hex.EncodeToString(sum[:])[:n]
}

func (a *App) sweepFilings(now time.Time) int {
	if now.IsZero() {
		now = time.Now().UTC()
	}
	scopes, relScopes := a.notify.DisclosureScopes(), a.notify.ReleaseScopes()
	if len(scopes) == 0 && len(relScopes) == 0 {
		return 0
	}
	told := 0
	discSyms := map[string]bool{}
	if len(scopes) > 0 {
		for _, i := range a.knownFilingSymbols(scopes) {
			discSyms[i.Symbol] = true
		}
	}
	hold := len(discSyms) > 0 && a.namingHeld(now, !a.canNameDocuments())
	for _, inst := range a.knownFilingSymbols(unionScopes(scopes, relScopes)) {
		sym := inst.Symbol
		if !a.filingsStale(sym, now, float64(FilingsSweepAgeMin)/60.0) {
			continue
		}
		if hold && discSyms[sym] {
			continue
		}
		before := map[string]bool{}
		for _, r := range a.st.Filings(sym) {
			before[filingMark(r)] = true
		}
		wrote := a.refreshFilings(sym, inst.Name, &inst.Exchange, &inst.Currency)
		if wrote < 0 {
			continue
		}
		bySource := map[string][]store.Filing{}
		var order []string
		for _, r := range a.st.Filings(sym) {
			if _, ok := bySource[r.Source]; !ok {
				order = append(order, r.Source)
			}
			bySource[r.Source] = append(bySource[r.Source], r)
		}
		var fresh []store.Filing
		for _, src := range order {
			fresh = append(fresh, notify.FreshSince(a.st, "filings:"+sym+":"+src, bySource[src], func(r store.Filing) string { return r.Date }, func(r store.Filing) string { return r.ID }, func(r store.Filing) bool { return before[filingMark(r)] })...)
		}
		if len(fresh) == 0 {
			continue
		}
		var rel, rest []store.Filing
		for _, r := range fresh {
			if isNewsRelease(r) {
				rel = append(rel, r)
			} else {
				rest = append(rest, r)
			}
		}
		said := false
		if len(rel) > 0 && a.inReleaseScope(sym, relScopes) && !a.st.HasWireRelease(sym) {
			t, b := releaseNoticeFilings(sym, rel)
			if a.notify.Emit("releases", releaseKeyFilings(sym, rel), t, b, map[string]any{"symbol": sym}) != nil {
				said = true
			}
		}
		if len(rest) > 0 && discSyms[sym] {
			title, body := a.filingsNotice(sym, rest)
			marks := make([]string, 0, len(rest))
			for _, r := range rest {
				marks = append(marks, filingMarkSlash(r))
			}
			sort.Strings(marks)
			digest := sha1Short(strings.Join(marks, "|"), 12)
			if a.notify.Emit("disclosures", "filings:"+sym+":"+digest, title, body, map[string]any{"symbol": sym}) != nil {
				said = true
			}
		}
		if said {
			told++
		}
	}
	return told
}

func (a *App) readFear(index string) map[string]any {
	rec := fear.Read(a.mk, index)
	if len(rec) > 0 {
		a.st.SaveGauge(index, rec, "", FearVersion)
	}
	return rec
}

func staleStamp(fetchedAt string, now time.Time, maxAge time.Duration) bool {
	t, ok := parseStamp(fetchedAt)
	if !ok {
		return true
	}
	return now.Sub(t) > maxAge
}

func (a *App) fearStale(rec *store.Gauge, now time.Time) bool {
	if rec.ReadVersion < FearVersion {
		return true
	}
	return staleStamp(rec.FetchedAt, now, FearStaleMin*time.Minute)
}

func (a *App) fearPayload(index string) map[string]any {
	which := strings.ToLower(py.Strip(index))
	if !slices.Contains(fear.Indexes, which) {
		return map[string]any{"ok": false, "error": "no such index"}
	}
	held := a.st.Gauge(which)
	if held != nil && held.Score != nil {
		if a.fearStale(held, time.Now().UTC()) {
			a.kick("fear:"+which, func() { a.readFear(which) })
		}
		return map[string]any{"ok": true, "gauge": held}
	}
	rec := a.readFear(which)
	if len(rec) == 0 {
		return map[string]any{"ok": false, "error": "the index did not answer"}
	}
	return map[string]any{"ok": true, "gauge": rec}
}

func (a *App) sweepFear(now time.Time) int {
	done := 0
	for _, which := range fear.Indexes {
		held := a.st.Gauge(which)
		if held != nil && held.Score != nil && !a.fearStale(held, now) {
			continue
		}
		if len(a.readFear(which)) > 0 {
			done++
		}
	}
	return done
}

func (a *App) fearSweepLoop() {
	for {
		a.sweepFear(time.Now().UTC())
		if a.wait(FearSweepEverySec * time.Second) {
			return
		}
	}
}

func (a *App) shortsStale(rec *store.Short, now time.Time) bool {
	if rec.ReadVersion < ShortsVersion {
		return true
	}
	return staleStamp(rec.FetchedAt, now, ShortsStaleHours*time.Hour)
}

func (a *App) readShorts(symbol, exchange, currency string, trend bool, now time.Time, name string) *store.Short {
	rec, ok := a.shorts.ForListing(symbol, exchange, currency, now, trend, name)
	if !ok {
		return nil
	}
	ex := rec.Exchange
	if ex == "" {
		ex = exchange
	}
	a.st.SaveShorts(symbol, ex, rec, py.Stamp(now), ShortsVersion, rec.Series != nil)
	return &rec
}

func venueOfForm(form string) string {
	tail := ""
	if strings.Contains(form, ":") {
		tail = form[strings.LastIndex(form, ":")+1:]
	}
	return map[string]string{"CNX": "CSE", "AQL": "Cboe Canada"}[tail]
}

func (a *App) shortsPayload(symbol, exchange, currency string, trend bool) map[string]any {
	sym := strings.ToUpper(py.Strip(symbol))
	if sym == "" {
		return map[string]any{"ok": false, "error": "symbol required"}
	}
	listedAs, _, _ := a.instrumentMeta(sym)
	ex, ccy := py.Strip(exchange), py.Strip(currency)
	if listedAs == sym {
		listedAs = ""
	}
	if ex == "" {
		var held string
		_, ex, held = a.instrumentMeta(sym)
		if ccy == "" {
			ccy = held
		}
		if ex == "" {
			form := py.S(a.mk.TMXResolve(market.TMXSymbol(sym)))
			if form != "" && !strings.HasSuffix(form, ":US") {
				ex = venueOfForm(form)
				if ccy == "" {
					ccy = "CAD"
				}
			} else {
				ex, ccy = "NASDAQ", "USD"
			}
		}
	}
	if shorts.MarketOf(sym, ex, ccy) == "" {
		return map[string]any{"ok": true, "covered": false}
	}
	held := a.st.ShortsFor(sym, ex)
	if held != nil && (!trend || len(held.Series) > 0) {
		if a.shortsStale(held, time.Now().UTC()) {
			a.kick("shorts:"+sym+"|"+ex, func() { a.readShorts(sym, ex, ccy, true, time.Now().UTC(), listedAs) })
		}
		return map[string]any{"ok": true, "covered": true, "shorts": held}
	}
	rec := a.readShorts(sym, ex, ccy, trend, time.Now().UTC(), listedAs)
	if rec == nil {
		return map[string]any{"ok": true, "covered": false}
	}
	return map[string]any{"ok": true, "covered": true, "shorts": rec}
}

type shortsRow struct {
	store.Short
	PositionID *string
	Held       bool
	Watched    bool
}

func (r shortsRow) MarshalJSON() ([]byte, error) {
	b, err := r.Short.MarshalJSON()
	if err != nil {
		return nil, err
	}
	tail, err := json.Marshal(struct {
		PositionID *string `json:"positionId"`
		Held       bool    `json:"held"`
		Watched    bool    `json:"watched"`
	}{r.PositionID, r.Held, r.Watched})
	if err != nil {
		return nil, err
	}
	out := append(b[:len(b)-1], ',')
	return append(out, tail[1:]...), nil
}

func (a *App) shortsFeed() map[string]any {
	base := a.model.Base()
	type src struct {
		name, exchange, positionID string
	}
	held, watched := map[[2]string]src{}, map[[2]string]src{}
	for _, p := range base.Positions {
		if p.Kind == "Shares" {
			held[[2]string{strings.ToUpper(market.TMXSymbol(p.Symbol)), strings.ToUpper(p.Exchange)}] = src{p.Name, p.Exchange, p.ID}
		}
	}
	for _, w := range base.Watchlist {
		watched[[2]string{strings.ToUpper(market.TMXSymbol(w.Symbol)), strings.ToUpper(w.Exchange)}] = src{w.Name, w.Exchange, ""}
	}
	rows := []shortsRow{}
	for _, r := range a.st.AllShorts() {
		key := [2]string{r.Symbol, r.Exchange}
		s, ok := held[key]
		if !ok {
			s, ok = watched[key]
		}
		if !ok || r.Shares == nil {
			continue
		}
		row := shortsRow{Short: r}
		if s.name != "" {
			row.Short.Name = s.name
		}
		if s.exchange != "" {
			row.Short.Exchange = s.exchange
		}
		if s.positionID != "" {
			row.PositionID = strPtr(s.positionID)
		}
		_, row.Held = held[key]
		_, row.Watched = watched[key]
		rows = append(rows, row)
	}
	a.shortsMu.Lock()
	reading := a.shortsLeft > 0
	a.shortsMu.Unlock()
	return map[string]any{"ok": true, "rows": rows, "reading": reading}
}

func (a *App) listingPayload(symbol, exchange, currency, name string) map[string]any {
	sym := strings.ToUpper(py.Strip(market.TMXSymbol(py.S(symbol))))
	if sym == "" {
		return map[string]any{"ok": false, "error": "symbol required"}
	}
	ex, ccy := py.Strip(exchange), py.Strip(currency)
	base := a.model.Base()
	same := func(kind, rsym, rex string) bool {
		if kind == "Options" || strings.ToUpper(py.Strip(market.TMXSymbol(rsym))) != sym {
			return false
		}
		there := strings.ToUpper(py.Strip(rex))
		return ex == "" || there == "" || there == strings.ToUpper(ex)
	}
	for _, p := range base.Positions {
		if same(p.Kind, p.Symbol, p.Exchange) {
			return map[string]any{"ok": true, "symbol": sym, "positionId": p.ID}
		}
	}
	var trades []*model.Trade
	for _, t := range base.Trades {
		if same(t.Kind, t.Symbol, t.Exchange) {
			trades = append(trades, t)
		}
	}
	var watched *store.Watch
	for i := range base.Watchlist {
		w := base.Watchlist[i]
		if same("", w.Symbol, w.Exchange) {
			watched = &w
			break
		}
	}
	var knownEx, knownCcy, knownKind, knownName, knownSec string
	if len(trades) > 0 {
		t := trades[0]
		knownEx, knownCcy, knownKind, knownName, knownSec = t.Exchange, t.Currency, t.Kind, t.Name, t.SecurityID
	} else if watched != nil {
		knownEx, knownCcy, knownName, knownSec = watched.Exchange, watched.Currency, watched.Name, watched.SecurityID
	}
	metaName, metaEx, metaCcy := a.instrumentMeta(sym)
	if ex == "" {
		ex = knownEx
	}
	if ex == "" {
		ex = metaEx
	}
	if ccy == "" {
		ccy = knownCcy
	}
	if ccy == "" {
		ccy = metaCcy
	}
	kind := knownKind
	if kind == "" {
		kind = "Shares"
	}
	fills := []model.Fill{}
	for _, t := range trades {
		fills = append(fills, t.Fills...)
	}
	sort.SliceStable(fills, func(i, j int) bool { return fills[i].When < fills[j].When })
	outName := py.Strip(name)
	if outName == "" {
		outName = knownName
	}
	if outName == "" && metaName != sym {
		outName = metaName
	}
	out := map[string]any{"ok": true, "symbol": sym, "exchange": ex, "currency": ccy, "kind": kind, "name": outName, "securityId": knownSec, "fills": fills, "price": nil, "percentChange": nil}
	if kind == "Shares" {
		if q := a.mk.PeekQuote(market.Rec{Symbol: sym, Exchange: ex, Currency: ccy, Kind: kind}, a.mk.Clock()); q != nil {
			out["price"], out["percentChange"] = q.Price, q.PercentChange
		}
	}
	return out
}

type shortListing struct {
	symbol, exchange, currency, name string
}

func (a *App) shortsListings(scope string) []shortListing {
	base := a.model.Base()
	seen := map[[2]string]bool{}
	out := []shortListing{}
	var groups [][]symbolRow
	if scope == "holdings" || scope == "all" {
		var g []symbolRow
		for _, p := range base.Positions {
			g = append(g, symbolRow{symbol: p.Symbol, name: p.Name, exchange: p.Exchange, currency: p.Currency})
		}
		groups = append(groups, g)
	}
	if scope == "watchlist" || scope == "all" {
		var g []symbolRow
		for _, w := range base.Watchlist {
			g = append(g, symbolRow{symbol: w.Symbol, name: w.Name, exchange: w.Exchange, currency: w.Currency})
		}
		groups = append(groups, g)
	}
	for _, group := range groups {
		for _, row := range group {
			sym := market.TMXSymbol(row.symbol)
			key := [2]string{strings.ToUpper(sym), strings.ToUpper(row.exchange)}
			if sym == "" || seen[key] || shorts.MarketOf(sym, row.exchange, row.currency) == "" {
				continue
			}
			seen[key] = true
			out = append(out, shortListing{sym, row.exchange, row.currency, row.name})
		}
	}
	return out
}

func (a *App) sweepShorts(now time.Time) int {
	var due []shortListing
	for _, l := range a.shortsListings("all") {
		held := a.st.ShortsFor(l.symbol, l.exchange)
		if !(held != nil && len(held.Series) > 0 && !a.shortsStale(held, now)) {
			due = append(due, l)
		}
	}
	done := 0
	a.shortsMu.Lock()
	a.shortsLeft = len(due)
	a.shortsMu.Unlock()
	defer func() {
		a.shortsMu.Lock()
		a.shortsLeft = 0
		a.shortsMu.Unlock()
	}()
	for _, l := range due {
		if a.readShorts(l.symbol, l.exchange, l.currency, true, now, l.name) != nil {
			done++
		}
		a.shortsMu.Lock()
		a.shortsLeft--
		a.shortsMu.Unlock()
	}
	return done
}

func (a *App) shortsSweepLoop() {
	for {
		a.sweepShorts(time.Now().UTC())
		if a.wait(ShortsSweepEverySec * time.Second) {
			return
		}
	}
}

func (a *App) newsSymbolPayload(symbol, exchange, currency string) map[string]any {
	sym := strings.ToUpper(py.Strip(symbol))
	if sym == "" {
		return map[string]any{"ok": false, "error": "symbol required"}
	}
	ex, ccy := py.Strip(exchange), py.Strip(currency)
	known, knownEx, knownCcy := a.instrumentMeta(sym)
	name := ""
	if known != sym {
		name = known
	}
	if ex == "" {
		ex = knownEx
		if ccy == "" {
			ccy = knownCcy
		}
	}
	var listing *market.Listing
	form := market.TMXForm(ex, ccy)
	if ex == "" || (name == "" && form != nil && *form != ":US") {
		listing = a.mk.TMXListing(sym)
	}
	if listing != nil {
		if name == "" && strings.ToUpper(listing.Name) != sym {
			name = listing.Name
		}
		if ex == "" {
			ex, ccy = listing.Exchange, listing.Currency
		}
	}
	if ex == "" {
		resolved := a.mk.TMXResolve(market.TMXSymbol(sym))
		if resolved != "" && !strings.HasSuffix(resolved, ":US") {
			if ccy == "" {
				ccy = "CAD"
			}
		} else {
			ex, ccy = "NASDAQ", "USD"
		}
	}
	src, rows, ok := news.ReadListing(a.mk, sym, ex, ccy, a.mk.Clock(), name, true, nil)
	if !ok {
		return map[string]any{"ok": false, "error": "the wire did not answer"}
	}
	a.st.TrimNews(news.Keep)
	a.invalidate(false)
	return map[string]any{"ok": true, "count": len(rows), "source": src, "exchange": ex}
}

func (a *App) filingsFeed(scope string, limit int) map[string]any {
	key := strings.ToLower(py.Strip(scope))
	if key == "" {
		key = "all"
	}
	scopes := FeedScopes[key]
	if scopes == nil {
		scopes = []string{"all"}
	}
	set := map[string]bool{}
	for _, s := range scopes {
		set[s] = true
	}
	rows := []store.Filing{}
	for _, inst := range a.knownFilingSymbols(set) {
		for _, r := range a.freshFilings(inst.Symbol) {
			r.Symbol = inst.Symbol
			r.Exchange = inst.Exchange
			rows = append(rows, r)
		}
	}
	sort.SliceStable(rows, func(i, j int) bool { return rows[i].Date > rows[j].Date })
	if limit < 1 {
		limit = 1
	}
	if len(rows) > limit {
		rows = rows[:limit]
	}
	return map[string]any{"ok": true, "scope": key, "filings": rows}
}

var newsReleaseDoc = regexp.MustCompile(`(?i)news release|press release`)

func isNewsRelease(f store.Filing) bool { return newsReleaseDoc.MatchString(f.Type) }

func sameListing(s, sym string) bool {
	t := market.TMXSymbol(s)
	if t == "" {
		t = s
	}
	return strings.ToUpper(py.Strip(t)) == sym
}

func (a *App) inReleaseScope(sym string, scopes map[string]bool) bool {
	if scopes == nil {
		scopes = a.notify.ReleaseScopes()
	}
	if len(scopes) == 0 {
		return false
	}
	if scopes["all"] {
		return true
	}
	sym = strings.ToUpper(py.Strip(sym))
	if scopes["held"] {
		for _, p := range a.model.Base().Positions {
			if sameListing(p.Symbol, sym) {
				return true
			}
		}
	}
	if scopes["watched"] {
		for _, w := range a.st.ListWatchlist() {
			if sameListing(w.Symbol, sym) {
				return true
			}
		}
		return false
	}
	return false
}

func releaseNoticeFilings(sym string, rows []store.Filing) (string, string) {
	newest := append([]store.Filing{}, rows...)
	sort.SliceStable(newest, func(i, j int) bool { return newest[i].Date > newest[j].Date })
	head := newest[0].Subject
	if head == "" {
		head = newest[0].Type
	}
	if head == "" {
		head = "A new release."
	}
	title := "Press release · "
	if len(rows) != 1 {
		title = strconv.Itoa(len(rows)) + " press releases · "
	}
	return title + sym, head
}

func releaseKeyFilings(sym string, rows []store.Filing) string {
	ids := make([]string, 0, len(rows))
	for _, r := range rows {
		ids = append(ids, r.ID)
	}
	sort.Strings(ids)
	return "release:" + sym + ":" + sha1Short(strings.Join(ids, "|"), 12)
}

func releaseNoticeWire(sym string, rows []store.WireItem) (string, string) {
	newest := append([]store.WireItem{}, rows...)
	sort.SliceStable(newest, func(i, j int) bool { return newest[i].PublishedAt > newest[j].PublishedAt })
	head := newest[0].Headline
	if head == "" {
		head = "A new release."
	}
	title := "Press release · "
	if len(rows) != 1 {
		title = strconv.Itoa(len(rows)) + " press releases · "
	}
	return title + sym, head
}

func releaseKeyWire(sym string, rows []store.WireItem) string {
	ids := make([]string, 0, len(rows))
	for _, r := range rows {
		ids = append(ids, r.ID)
	}
	sort.Strings(ids)
	return "release:" + sym + ":" + sha1Short(strings.Join(ids, "|"), 12)
}

func (a *App) noteWireReleases(symbol, exchange string, rows []store.WireItem, newIDs map[string]bool) {
	scopes := a.notify.ReleaseScopes()
	sym := market.TMXSymbol(symbol)
	if sym == "" {
		sym = symbol
	}
	sym = strings.ToUpper(py.Strip(sym))
	if len(scopes) == 0 || !a.inReleaseScope(sym, scopes) {
		return
	}
	var rel []store.WireItem
	for _, r := range rows {
		if r.Kind == "release" {
			rel = append(rel, r)
		}
	}
	fresh := notify.FreshSince(a.st, "news:"+store.NewsKey(symbol, exchange), rel, func(r store.WireItem) string { return r.PublishedAt }, func(r store.WireItem) string { return r.ID }, func(r store.WireItem) bool { return !newIDs[r.ID] })
	if len(fresh) == 0 {
		return
	}
	title, body := releaseNoticeWire(sym, fresh)
	a.notify.Emit("releases", releaseKeyWire(sym, fresh), title, body, map[string]any{"symbol": sym})
}

func (a *App) filingsNotice(sym string, fresh []store.Filing) (string, string) {
	var named []string
	limit := len(fresh)
	if limit > 3 {
		limit = 3
	}
	for _, r := range fresh[:limit] {
		title := py.Strip(r.Subject)
		if title == "" && r.ID != "" {
			title = py.Strip(py.S(a.filingsEnrich(sym, r.ID)["subject"]))
		}
		if title == "" {
			title = py.Strip(r.Type)
		}
		if title != "" && !slices.Contains(named, title) {
			named = append(named, title)
		}
	}
	var sources []string
	for _, r := range fresh {
		src := py.Strip(r.Source)
		if src != "" && !slices.Contains(sources, src) {
			sources = append(sources, src)
		}
	}
	names := map[string]string{"sedar": "SEDAR+", "sedar+": "SEDAR+", "sec": "SEC EDGAR", "sec edgar": "SEC EDGAR"}
	var tails []string
	for _, x := range sources {
		if n, ok := names[strings.ToLower(x)]; ok {
			tails = append(tails, n)
		} else {
			tails = append(tails, x)
		}
	}
	tail := strings.Join(tails, ", ")
	head := strings.Join(named, ", ")
	if len(fresh) > 3 {
		head += " and more"
	}
	body := head
	if head != "" && tail != "" {
		body += " · "
	}
	body += tail
	if body == "" {
		body = "A new filing."
	}
	title := "New disclosure · "
	if len(fresh) != 1 {
		title = strconv.Itoa(len(fresh)) + " new disclosures · "
	}
	return title + sym, body
}

func (a *App) filingsSweepLoop() {
	for {
		if a.wait(FilingsSweepEverySec * time.Second) {
			return
		}
		a.sweepFilings(time.Now().UTC())
	}
}

func (a *App) refreshFilings(symbol, name string, exchange, currency *string) int {
	sym := strings.ToUpper(py.Strip(symbol))
	if sym == "" {
		return 0
	}
	if !a.singleFlightStart("filings:" + sym) {
		return 0
	}
	defer a.singleFlightEnd("filings:" + sym)
	iname, ex, cur := a.instrumentMeta(sym)
	if name != "" {
		iname = name
	}
	if exchange != nil {
		ex = *exchange
	}
	if currency != nil {
		cur = *currency
	}
	known := a.st.SedarProfile(sym)
	result := a.pipeline.Fetch(sym, iname, ex, cur, 200, known)
	total := 0
	anyReached := false
	profileNo := ""
	bySource := map[string][]store.FilingItem{}
	for _, it := range result.Items {
		bySource[it.Source] = append(bySource[it.Source], store.FilingItem{ID: it.ID, Source: it.Source, Category: it.Category, Date: it.Date, DateText: it.DateText, Type: it.Type, Title: it.Title, Size: it.Size, URL: it.URL, Issuer: it.Issuer, ProfileNo: it.ProfileNo})
		if it.Source == disclosures.SedarSource && it.ProfileNo != "" {
			profileNo = it.ProfileNo
		}
	}
	held := map[string]bool{}
	for _, r := range a.st.Filings(sym) {
		held[r.Source] = true
	}
	srcs := make([]string, 0, len(result.Sources))
	for src := range result.Sources {
		srcs = append(srcs, src)
	}
	sort.Strings(srcs)
	for _, src := range srcs {
		status := result.Sources[src]
		if status.Available {
			anyReached = true
		}
		rows := bySource[src]
		if len(rows) == 0 && held[src] {
			a.logf("bagholder disclosures: %s: %s answered empty; the stored rows stand\n", sym, src)
			continue
		}
		if status.Matched || status.Available {
			total += a.st.ReplaceFilings(sym, src, rows, "")
		}
	}
	raw, _ := json.Marshal(result.Sources)
	a.st.SetMeta("filings_sources:"+sym, string(raw))
	if !anyReached {
		return -1
	}
	a.st.MarkFilingsFetched(sym, profileNo, "")
	return total
}

func (a *App) sourceStatus(sym string) map[string]map[string]bool {
	stored := map[string]disclosures.SourceStatus{}
	if raw := a.st.GetMeta("filings_sources:" + sym); raw != "" {
		_ = json.Unmarshal([]byte(raw), &stored)
	}
	have := map[string]bool{}
	for _, r := range a.st.Filings(sym) {
		have[r.Source] = true
	}
	out := map[string]map[string]bool{}
	for _, p := range a.pipeline.Providers {
		src := p.Source()
		st := stored[src]
		out[src] = map[string]bool{"available": p.Available(), "matched": have[src], "filer": st.Filer || have[src]}
	}
	return out
}

func (a *App) filingsPayload(symbol string, refresh bool, name string, exchange, currency *string) map[string]any {
	sym := strings.ToUpper(py.Strip(symbol))
	if sym == "" {
		return map[string]any{"ok": false, "error": "symbol required"}
	}
	wrote := 0
	touched := false
	if refresh || a.filingsStale(sym, time.Now().UTC(), FilingsStaleHours) {
		wrote = a.refreshFilings(sym, name, exchange, currency)
		touched = true
	}
	return map[string]any{"ok": true, "symbol": sym, "available": a.pipeline.Available(), "sources": a.sourceStatus(sym), "categories": disclosures.Categories,
		"profileNo": a.st.SedarProfile(sym), "fetchedAt": a.st.FilingsFetchedAt(sym), "refreshed": touched && wrote > 0, "sourceUnavailable": touched && wrote == -1, "filings": a.freshFilings(sym)}
}

func rowOf(r store.Filing) disclosures.Row {
	return disclosures.Row{ID: r.ID, Source: r.Source, Category: r.Category, Type: r.Type, URL: r.URL, ProfileNo: r.ProfileNo, Issuer: r.Issuer}
}

func (a *App) freshFilings(sym string) []store.Filing {
	rows := a.st.Filings(sym)
	for i := range rows {
		if rows[i].EnrichVersion < EnrichVersion {
			rows[i].Subject, rows[i].Summary = "", ""
		}
		rows[i].Category = a.pipeline.Categorize(rowOf(rows[i]))
	}
	return rows
}

func (a *App) filingsDocument(symbol, docID string) ([]byte, string, string) {
	sym := strings.ToUpper(py.Strip(symbol))
	row := a.st.Filing(sym, docID)
	if row == nil {
		a.refreshFilings(sym, "", nil, nil)
		row = a.st.Filing(sym, docID)
	}
	if row == nil {
		return nil, "", "no such document for " + sym
	}
	data, ct, err := a.pipeline.Document(rowOf(*row))
	if err != nil {
		return nil, "", errText(err)
	}
	if len(data) == 0 {
		return nil, "", "empty document"
	}
	if ct == "" {
		ct = "application/octet-stream"
	}
	return data, ct, ""
}

func (a *App) enrichAnswer(docID, subject, summary string, model bool) map[string]any {
	return map[string]any{"ok": true, "id": docID, "subject": subject, "summary": summary, "summaryAvailable": model, "summaryStatus": a.enricher.SummaryStatus()}
}

func strPtr(s string) *string { return &s }
func intPtr(n int) *int       { return &n }
func boolPtr(b bool) *bool    { return &b }

func (a *App) filingsEnrich(symbol, docID string) map[string]any {
	sym := strings.ToUpper(py.Strip(symbol))
	row := a.st.Filing(sym, docID)
	if row == nil {
		return map[string]any{"ok": false, "error": "no such document"}
	}
	subject, summary := row.Subject, row.Summary
	model := a.enricher.SummaryAvailable()
	fresh := row.EnrichVersion >= EnrichVersion
	attempted := row.EnrichedAt != "" && fresh
	if attempted && (row.EnrichFinal || (subject != "" && summary != "") || !model) {
		return a.enrichAnswer(docID, subject, summary, model)
	}
	if !a.pipeline.Available() {
		return a.enrichAnswer(docID, subject, summary, model)
	}
	if exact := a.pipeline.Enrichment(rowOf(*row)); exact != nil && (exact.Subject != "" || exact.Summary != "") {
		subject, summary = exact.Subject, exact.Summary
		a.st.SetFilingEnrichment(sym, docID, strPtr(subject), strPtr(summary), intPtr(EnrichVersion), nil)
		return a.enrichAnswer(docID, subject, summary, model)
	}
	data, ct, err := a.pipeline.Content(rowOf(*row))
	if err != nil {
		return map[string]any{"ok": false, "error": errText(err)}
	}
	if len(data) == 0 {
		return map[string]any{"ok": false, "error": "the document could not be read"}
	}
	if !model {
		model = a.enricher.WaitForSummary(enrich.SummaryWaitSec)
	}
	info := a.enricher.EnrichDocument(row.Source, data, ct)
	newSubject, gotSummary := info.Subject, info.Summary
	if info.Final {
		a.st.SetFilingEnrichment(sym, docID, strPtr(newSubject), strPtr(gotSummary), intPtr(EnrichVersion), boolPtr(true))
		return a.enrichAnswer(docID, newSubject, gotSummary, model)
	}
	if model {
		if fresh {
			if newSubject != "" {
				subject = newSubject
			}
			if gotSummary != "" {
				summary = gotSummary
			}
		} else {
			subject, summary = newSubject, gotSummary
		}
		a.st.SetFilingEnrichment(sym, docID, strPtr(subject), strPtr(summary), intPtr(EnrichVersion), nil)
	} else {
		if newSubject != "" {
			subject = newSubject
		}
		var subj *string
		if newSubject != "" {
			subj = strPtr(newSubject)
		}
		a.st.SetFilingEnrichment(sym, docID, subj, nil, nil, nil)
	}
	return a.enrichAnswer(docID, subject, summary, a.enricher.SummaryAvailable())
}

func (a *App) refreshUniverses() []string {
	if !a.singleFlightStart("universes") {
		return nil
	}
	defer a.singleFlightEnd("universes")
	done := universes.Refresh(a.mk)
	if len(done) > 0 {
		a.invalidate(false)
	}
	return done
}

func (a *App) universeLoop() {
	for !a.stopped() {
		select {
		case <-a.universeKick:
		default:
		}
		a.refreshUniverses()
		t := time.NewTimer(1800 * time.Second)
		select {
		case <-a.universeKick:
		case <-t.C:
		case <-a.stopCh:
		}
		t.Stop()
		if a.stopped() {
			return
		}
	}
}

func (a *App) kickUniverses() map[string]any {
	select {
	case a.universeKick <- struct{}{}:
	default:
	}
	return map[string]any{"ok": true}
}
