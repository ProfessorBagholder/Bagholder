package app

import (
	"net/url"
	"slices"
	"strings"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/csvimport"
	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/model"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

func (a *App) statusPayload() map[string]any {
	counts := a.st.StatusCounts()
	update := a.updateStatus()
	sess := a.loadSession()
	a.mu.Lock()
	defer a.mu.Unlock()
	connected := a.state.connected && sess != nil && sess.Str("access_token") != ""
	email := a.state.email
	if email == "" && sess != nil {
		email = sess.Str("email")
	}
	lastSync := a.state.lastSync
	if lastSync == "" {
		lastSync = counts.SyncedAt
	}
	updateURL := py.S(update["url"])
	if updateURL == "" {
		updateURL = RepoURL
	}
	if a.cfg.UpdatesOff {
		updateURL = ImagePage
	}
	updateBy := "app"
	if a.cfg.UpdatesOff {
		updateBy = "image"
	}
	return map[string]any{
		"ok":              true,
		"connected":       connected,
		"email":           email,
		"lastSync":        lastSync,
		"activityCount":   counts.ActivityCount,
		"accountCount":    counts.AccountCount,
		"capturing":       a.state.capturing,
		"syncing":         a.state.syncing,
		"listingsFilling": a.state.listingsFilling,
		"syncStep":        a.state.syncStep,
		"error":           a.state.err,
		"dataVersion":     a.st.DataVersion() + "|" + model.TodayLocal(),
		"summaryReady":    a.enricher.SummaryStatus() == "ready",
		"protocol":        Protocol,
		"startedAt":       StartedAt,
		"version":         AppVersion,
		"latestVersion":   py.S(update["latest"]),
		"updateAvailable": truthy(update["updateAvailable"]),
		"updateUrl":       updateURL,
		"canUpdate":       a.canUpdate(update),
		"updateBy":        updateBy,
		"loginView":       a.cfg.LoginView,
		"ordersLive":      a.cfg.OrdersLive,
		"openOrders":      a.openOrdersCount(),
		"updating":        a.state.updating,
		"updateError":     a.state.updateError,
		"notify":          a.notify.Status(),
		"newsReading":     a.newsReading(),
	}
}

func (a *App) payerSymbols() []market.Rec {
	rows := model.PayerSymbols(a.model.Base())
	out := make([]market.Rec, 0, len(rows))
	for _, r := range rows {
		out = append(out, market.Rec{Symbol: r.Symbol, Exchange: r.Exchange, Currency: r.Currency})
	}
	return out
}

func (a *App) refreshMarketData() map[string]any {
	if !a.singleFlightStart("market") {
		return map[string]any{}
	}
	defer a.singleFlightEnd("market")
	out := a.mk.RefreshAll(a.payerSymbols())
	quotes := a.refreshQuotes()
	if out.Distributions > 0 || quotes > 0 {
		a.invalidate(false)
	}
	return map[string]any{"fx": out.FX, "benchmark": out.Benchmark, "distributions": out.Distributions, "skipped": out.Skipped, "quotes": quotes}
}

func (a *App) refreshQuotes() int {
	if !a.singleFlightStart("quotes") {
		return 0
	}
	defer a.singleFlightEnd("quotes")
	base := a.model.Base()
	n := a.mk.RefreshQuotes(append(heldRecs(model.HeldSymbols(base)), quoteRecs(model.QuoteSymbols(base))...), a.mk.Clock())
	if n > 0 {
		a.invalidate(false)
	}
	return n
}

func (a *App) refreshPeriodicMarket() market.RefreshResult {
	if !a.singleFlightStart("periodic") {
		return market.RefreshResult{}
	}
	defer a.singleFlightEnd("periodic")
	out := a.mk.RefreshPeriodic(a.payerSymbols(), a.mk.Clock())
	if out.FX > 0 || out.Benchmark > 0 || out.Distributions > 0 {
		a.invalidate(false)
	}
	return out
}

func (a *App) archiveIntradayBars(limit int) []string {
	if !a.singleFlightStart("archive") {
		return nil
	}
	defer a.singleFlightEnd("archive")
	rows := model.IntradayArchiveSymbols(a.model.Base(), "")
	recs := make([]market.Rec, 0, len(rows))
	for _, r := range rows {
		recs = append(recs, market.Rec{Symbol: r.Symbol, Exchange: r.Exchange, Currency: r.Currency, Kind: r.Kind, Start: r.Start})
	}
	if limit < 1 {
		limit = 1
	}
	now := a.mk.Clock()
	return append(a.mk.ArchiveDaily(recs, now, limit), a.mk.ArchiveIntraday(recs, now, limit)...)
}

const (
	ArchiveDuty    = 1.0
	ArchivePassSec = 1.0
	ArchiveMinSec  = 0.5
	ArchiveIdleSec = 5 * 60
)

func (a *App) archiveLoop() {
	delay := 20 * time.Second
	batch := market.ArchiveBatch
	for !a.wait(delay) {
		started := cpuClock()
		worked := a.archiveIntradayBars(batch)
		spent := cpuClock() - started
		if spent < 0 {
			spent = 0
		}
		if len(worked) == 0 {
			delay = ArchiveIdleSec * time.Second
			batch = market.ArchiveBatch
			continue
		}
		if spent > ArchivePassSec && batch > 1 {
			batch = batch / 2
			if batch < 1 {
				batch = 1
			}
		} else if spent < ArchivePassSec/3 && batch < market.ArchiveBatch {
			batch = batch * 2
			if batch > market.ArchiveBatch {
				batch = market.ArchiveBatch
			}
		}
		rest := spent * ArchiveDuty
		if rest < ArchiveMinSec {
			rest = ArchiveMinSec
		}
		delay = secs(rest)
	}
}

func (a *App) quoteLoop() {
	for !a.wait(60 * market.QuoteRefreshMinutes * time.Second) {
		a.refreshQuotes()
	}
}

func (a *App) marketLoop() {
	for !a.wait(60 * market.MarketCheckMinutes * time.Second) {
		a.refreshPeriodicMarket()
		a.checkForUpdateIfDue(time.Time{})
	}
}

const WatchScanSec = 10 * 60

func (a *App) scanWatchedFolder() map[string]any {
	folder := csvimport.WatchFolder(a.st)
	if folder == "" {
		return nil
	}
	result := csvimport.ScanFolder(a.st, folder, false)
	if truthy(result["ok"]) && truthy(result["added"]) {
		a.invalidate(true)
	}
	return result
}

func (a *App) watchLoop() {
	a.scanWatchedFolder()
	for !a.wait(WatchScanSec * time.Second) {
		a.scanWatchedFolder()
	}
}

func queryParam(q url.Values, name string) string {
	return strings.TrimSpace(q.Get(name))
}

func (a *App) historyPayload(query string) map[string]any {
	q, _ := url.ParseQuery(query)
	one := func(k string) string { return queryParam(q, k) }
	rec := market.Rec{Symbol: one("symbol"), Exchange: one("exchange"), Currency: one("currency"), Kind: one("kind")}
	if rec.Currency == "" {
		rec.Currency = "CAD"
	}
	if rec.Kind == "" {
		rec.Kind = "Shares"
	}
	start, end := cutStr(one("from"), 10), cutStr(one("to"), 10)
	tf := one("tf")
	if tf == "" {
		tf = "1d"
	}
	if rec.Symbol == "" || len(start) != 10 || len(end) != 10 || !slices.Contains(market.Timeframes, tf) {
		return map[string]any{"ok": false, "error": "symbol, from, to and a known tf are required"}
	}
	inst := market.ChartInstrument(rec)
	src := market.HistorySource(inst)
	now := a.mk.Clock()
	available := a.mk.OfferedTimeframes(inst, start, now)
	pending := false
	var bars any = []any{}
	count := 0
	_, intraday := market.IntradaySeconds[tf]
	switch {
	case src == nil || !slices.Contains(available, tf):
	case intraday && !a.mk.IntradayReady(inst, tf, start, now):
		a.mk.EnsureIntradayInBackground(inst, tf, start, end)
		pending = true
	default:
		daily, hourly := a.mk.EnsureBars(inst, tf, start, end, now)
		if intraday {
			if hourly == nil {
				hourly = []market.Bar{}
			}
			bars, count = hourly, len(hourly)
		} else {
			if daily == nil {
				daily = []market.Daily{}
			}
			bars, count = daily, len(daily)
		}
	}
	source := ""
	if src != nil {
		source = src.Source
	}
	reason := ""
	if count == 0 && !pending {
		reason = a.mk.ChartReason(inst, tf)
	}
	return map[string]any{"ok": true, "symbol": rec.Symbol, "chartSymbol": inst.Symbol, "source": source, "tf": tf, "available": available, "bars": bars, "pending": pending, "reason": reason}
}

func cutStr(s string, n int) string {
	r := []rune(s)
	if len(r) > n {
		return string(r[:n])
	}
	return s
}
