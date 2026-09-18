package app

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"io/fs"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"regexp"
	"runtime"
	"strconv"
	"strings"
	"syscall"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/csvimport"
	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/model"
	"github.com/ProfessorBagholder/Bagholder/internal/notify"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

type serverHandle struct {
	srv  *http.Server
	port int
}

var containerHostRE = regexp.MustCompile(`^127\.0\.0\.1:\d{1,5}$`)

func (a *App) local(r *http.Request) bool {
	if a.cfg.BindHost != "127.0.0.1" {
		return true
	}
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil {
		host = r.RemoteAddr
	}
	return host == "127.0.0.1" || host == "::1"
}

func (a *App) hostOK(r *http.Request, port int) bool {
	raw := strings.ToLower(strings.TrimSpace(r.Host))
	if raw == "" || strings.Contains(raw, ",") {
		return false
	}
	if a.cfg.BindHost != "127.0.0.1" {
		return containerHostRE.MatchString(raw)
	}
	return raw == "127.0.0.1:"+strconv.Itoa(port)
}

func writeOK(r *http.Request) bool {
	if strings.ToLower(strings.TrimSpace(r.Header.Get("Sec-Fetch-Site"))) == "same-origin" {
		return true
	}
	return strings.TrimSpace(r.Header.Get("X-Bagholder")) != ""
}

func (a *App) announceReached(r *http.Request, port int) {
	if a.cfg.BindHost == "127.0.0.1" || !a.local(r) || !a.hostOK(r, port) {
		return
	}
	host := strings.ToLower(strings.TrimSpace(r.Host))
	if host == "127.0.0.1:"+strconv.Itoa(port) {
		return
	}
	a.reachedOnce.Do(func() {
		fmt.Printf("Bagholder  http://%s\n", host)
	})
}

func startupLine(bindHost string, port int) string {
	return "http://127.0.0.1:" + strconv.Itoa(port)
}

func (a *App) gate(r *http.Request, port int, write bool) bool {
	if !a.local(r) || !a.hostOK(r, port) {
		return false
	}
	if write && !writeOK(r) {
		return false
	}
	return true
}

type responder struct {
	w    http.ResponseWriter
	r    *http.Request
	code int
}

func (rs *responder) send(code int, body any, contentType string) {
	var raw []byte
	switch b := body.(type) {
	case []byte:
		raw = b
	case string:
		raw = []byte(b)
	case json.RawMessage:
		raw = b
	default:
		data, err := json.Marshal(body)
		if err != nil {
			data = []byte(`{"ok":false,"error":"encode failed"}`)
			code = 500
		}
		raw = data
	}
	if contentType == "" {
		contentType = "application/json; charset=utf-8"
	}
	h := rs.w.Header()
	h.Set("Content-Type", contentType)
	h.Set("Content-Length", strconv.Itoa(len(raw)))
	h.Set("Cache-Control", "no-store")
	h.Set("X-Content-Type-Options", "nosniff")
	rs.code = code
	rs.w.WriteHeader(code)
	_, _ = rs.w.Write(raw)
}

func (rs *responder) forbidden() { rs.send(403, map[string]any{"ok": false}, "") }

func readJSON(r *http.Request) map[string]any {
	n := r.ContentLength
	if n < 0 || n > 1_048_576 {
		return map[string]any{}
	}
	raw, err := io.ReadAll(io.LimitReader(r.Body, 1_048_576+1))
	if err != nil || len(raw) == 0 || len(raw) > 1_048_576 {
		return map[string]any{}
	}
	var out any
	if err := json.Unmarshal(raw, &out); err != nil {
		return map[string]any{}
	}
	m, _ := out.(map[string]any)
	if m == nil {
		return nil
	}
	return m
}

func dictOr(m map[string]any) map[string]any {
	if m == nil {
		return map[string]any{}
	}
	return m
}

type staticEntry struct {
	data []byte
	etag string
	mod  time.Time
	size int64
	disk bool
}

func (a *App) staticFile(name string) (*staticEntry, bool) {
	a.staticMu.Lock()
	defer a.staticMu.Unlock()
	if a.staticCache == nil {
		a.staticCache = map[string]*staticEntry{}
	}
	cur := a.staticCache[name]
	if a.cfg.AppDir != "" {
		path := filepath.Join(a.cfg.AppDir, name)
		if info, err := os.Stat(path); err == nil {
			if cur != nil && cur.disk && cur.mod.Equal(info.ModTime()) && cur.size == info.Size() {
				return cur, true
			}
			if data, err := os.ReadFile(path); err == nil {
				e := &staticEntry{data: data, etag: etagOf(data), mod: info.ModTime(), size: info.Size(), disk: true}
				a.staticCache[name] = e
				return e, true
			}
		}
	}
	if cur != nil && !cur.disk {
		return cur, true
	}
	if a.cfg.Static != nil {
		if data, err := fs.ReadFile(a.cfg.Static, name); err == nil {
			e := &staticEntry{data: data, etag: etagOf(data), size: int64(len(data))}
			a.staticCache[name] = e
			return e, true
		}
	}
	return nil, false
}

func etagOf(data []byte) string {
	sum := sha256.Sum256(data)
	return `"` + hex.EncodeToString(sum[:8]) + `"`
}

func (rs *responder) sendStatic(e *staticEntry, contentType string) {
	h := rs.w.Header()
	h.Set("ETag", e.etag)
	h.Set("Cache-Control", "no-cache")
	h.Set("X-Content-Type-Options", "nosniff")
	if rs.r.Header.Get("If-None-Match") == e.etag {
		rs.code = 304
		rs.w.WriteHeader(304)
		return
	}
	h.Set("Content-Type", contentType)
	h.Set("Content-Length", strconv.Itoa(len(e.data)))
	rs.code = 200
	rs.w.WriteHeader(200)
	_, _ = rs.w.Write(e.data)
}

func (a *App) modelFilters(q url.Values) any {
	raw := q.Get("filters")
	if raw == "" {
		return nil
	}
	var out any
	if err := json.Unmarshal([]byte(raw), &out); err != nil {
		return nil
	}
	return out
}

func optParam(q url.Values, name string) *string {
	v := strings.TrimSpace(q.Get(name))
	if v == "" {
		return nil
	}
	return &v
}

func flag(v string) bool { return v == "1" || v == "true" || v == "yes" }

func (a *App) handle(port int) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		rs := &responder{w: w, r: r, code: 200}
		a.announceReached(r, port)
		defer func() {
			if rec := recover(); rec != nil {
				a.logf("bagholder %s - request failed: %v\n", clientIP(r), rec)
				if rs.code == 200 {
					rs.send(500, map[string]any{"ok": false, "error": "internal error"}, "")
				}
				return
			}
			a.logf("bagholder %s - \"%s %s %s\" %d -\n", clientIP(r), r.Method, r.RequestURI, r.Proto, rs.code)
		}()
		switch r.Method {
		case http.MethodGet, http.MethodHead:
			a.doGet(rs, port)
		case http.MethodPost:
			a.doPost(rs, port)
		case http.MethodOptions:
			rs.forbidden()
		default:
			rs.send(501, map[string]any{"ok": false, "error": "not found"}, "")
		}
	})
}

func clientIP(r *http.Request) string {
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil {
		return r.RemoteAddr
	}
	return host
}

func (a *App) doGet(rs *responder, port int) {
	r := rs.r
	path := r.URL.Path
	q := r.URL.Query()
	query := r.URL.RawQuery
	if !a.gate(r, port, false) {
		rs.forbidden()
		return
	}
	switch path {
	case "/api/login/stream":
		h := rs.w.Header()
		h.Set("Content-Type", "multipart/x-mixed-replace; boundary=frame")
		h.Set("Cache-Control", "no-store")
		h.Set("X-Content-Type-Options", "nosniff")
		rs.w.WriteHeader(200)
		fl, _ := rs.w.(http.Flusher)
		ctx := r.Context()
		a.loginStream(func(chunk []byte) error {
			if _, err := rs.w.Write(chunk); err != nil {
				return err
			}
			if fl != nil {
				fl.Flush()
			}
			return nil
		}, func() bool { return ctx.Err() == nil })
		return
	case "/api/login/frame":
		if data := a.loginFrame(); data != nil {
			rs.send(200, data, "image/jpeg")
		} else {
			rs.send(204, []byte{}, "")
		}
		return
	case "/", "/index.html", "/ledger.html", "/v2", "/v2/":
		e, ok := a.staticFile("ledger.html")
		if !ok {
			rs.send(404, map[string]any{"ok": false, "error": "ledger.html missing"}, "")
			return
		}
		rs.sendStatic(e, "text/html; charset=utf-8")
		return
	case "/api/order/quote":
		rs.send(200, a.ticketQuote(q.Get("symbol"), q.Get("security"), q.Get("account"), q.Get("exchange")), "")
		return
	case "/api/symbols/search":
		rs.send(200, a.symbolSearch(q.Get("q")), "")
		return
	case "/api/symbols/quote":
		rec := market.Rec{Symbol: queryParam(q, "symbol"), Exchange: queryParam(q, "exchange"), Currency: queryParam(q, "currency"), Kind: "Shares"}
		out := map[string]any{"ok": true, "price": nil, "priceChange": nil, "percentChange": nil}
		if rec.Symbol != "" {
			if pq := a.mk.PeekQuote(rec, a.mk.Clock()); pq != nil {
				out["price"], out["priceChange"], out["percentChange"] = pq.Price, pq.PriceChange, pq.PercentChange
			}
		}
		rs.send(200, out, "")
		return
	case "/api/filings":
		symbol := queryParam(q, "symbol")
		if symbol == "" {
			rs.send(400, map[string]any{"ok": false, "error": "symbol required"}, "")
			return
		}
		rs.send(200, a.filingsPayload(symbol, flag(queryParam(q, "refresh")), queryParam(q, "name"), optParam(q, "exchange"), optParam(q, "currency")), "")
		return
	case "/api/listing":
		rs.send(200, a.listingPayload(queryParam(q, "symbol"), queryParam(q, "exchange"), queryParam(q, "currency"), queryParam(q, "name")), "")
		return
	case "/api/fear":
		index := queryParam(q, "index")
		if index == "" {
			index = "stocks"
		}
		rs.send(200, a.fearPayload(index), "")
		return
	case "/api/shorts/feed":
		rs.send(200, a.shortsFeed(), "")
		return
	case "/api/shorts":
		rs.send(200, a.shortsPayload(queryParam(q, "symbol"), queryParam(q, "exchange"), queryParam(q, "currency"), flag(queryParam(q, "trend"))), "")
		return
	case "/api/news/symbol":
		rs.send(200, a.newsSymbolPayload(queryParam(q, "symbol"), queryParam(q, "exchange"), queryParam(q, "currency")), "")
		return
	case "/api/filings/feed":
		rs.send(200, a.filingsFeed(queryParam(q, "scope"), 200), "")
		return
	case "/api/filings/doc":
		symbol, docID := queryParam(q, "symbol"), queryParam(q, "id")
		if symbol == "" || docID == "" {
			rs.send(400, map[string]any{"ok": false, "error": "symbol and id required"}, "")
			return
		}
		data, ct, errMsg := a.filingsDocument(symbol, docID)
		if data == nil {
			// a refused document opens as a page that says so and offers the retry, since the
			// tab was opened to read something and a raw error reads as the app being broken
			rs.send(502, a.documentErrorPage(symbol, docID, errMsg), "text/html; charset=utf-8")
			return
		}
		if ct == "" {
			ct = "application/pdf"
		}
		rs.send(200, data, ct)
		return
	case "/api/filings/enrich":
		symbol, docID := queryParam(q, "symbol"), queryParam(q, "id")
		if symbol == "" || docID == "" {
			rs.send(400, map[string]any{"ok": false, "error": "symbol and id required"}, "")
			return
		}
		rs.send(200, a.filingsEnrich(symbol, docID), "")
		return
	case "/api/orders":
		rs.send(200, a.ordersPayload(true), "")
		return
	case "/api/notifications":
		rs.send(200, map[string]any{"ok": true, "settings": a.notify.Status(), "kinds": notify.Kinds, "rows": a.st.ListNotifications(0, "", false, 50, true), "unread": a.st.UnreadNotifications()}, "")
		return
	case "/api/notifications/stream":
		a.streamNotifications(rs)
		return
	case "/api/status":
		rs.send(200, a.statusPayload(), "")
		return
	case "/api/history":
		rs.send(200, a.historyPayload(query), "")
		return
	case "/lightweight-charts.js":
		e, ok := a.staticFile("lightweight-charts.js")
		if !ok {
			rs.send(404, map[string]any{"ok": false, "error": "lightweight-charts.js missing"}, "")
			return
		}
		rs.sendStatic(e, "application/javascript; charset=utf-8")
		return
	case "/api/watch":
		rs.send(200, csvimport.Status(a.st), "")
		return
	case "/api/data":
		summary := a.st.DataSummary()
		summary["ok"] = true
		summary["sessionPresent"] = a.loadSession() != nil
		rs.send(200, summary, "")
		return
	case "/api/model":
		if a.mk.IsStale(a.mk.Clock(), a.payerSymbols()) {
			a.kick("market", func() { a.refreshMarketData() })
		} else {
			base := a.model.Base()
			if len(a.mk.QuoteSymbolsNeedingRefresh(append(heldRecs(model.HeldSymbols(base)), quoteRecs(model.QuoteSymbols(base))...), a.mk.Clock(), market.QuoteRefreshMinutes)) > 0 {
				a.kick("quotes", func() { a.refreshQuotes() })
			}
		}
		payload, err := a.viewJSON(a.modelFilters(q), queryParam(q, "trade"), queryParam(q, "page"))
		if err != nil {
			a.logf("model failed: %v\n", err)
			rs.send(500, map[string]any{"ok": false, "error": "model failed: " + err.Error()}, "")
			return
		}
		rs.send(200, payload, "")
		return
	case "/api/trade":
		detail := model.TradeDetail(a.model.Base(), queryParam(q, "id"))
		if detail == nil {
			rs.send(404, map[string]any{"ok": false, "error": "no such trade"}, "")
			return
		}
		rs.send(200, map[string]any{"ok": true, "id": detail.ID, "legs": detail.Legs, "fills": detail.Fills, "trade": detail.Trade}, "")
		return
	case "/favicon.png", "/favicon.ico":
		e, ok := a.staticFile("favicon.png")
		if !ok {
			rs.send(404, map[string]any{"ok": false, "error": "favicon missing"}, "")
			return
		}
		rs.sendStatic(e, "image/png")
		return
	case "/api/book":
		book := a.loadBook()
		rs.send(200, map[string]any{"ok": true, "activities": book.Activities, "accounts": book.Accounts, "balances": book.Balances, "navHistory": book.NavHistory, "navByAccount": book.NavByAccount,
			"syncedAt": book.SyncedAt, "tradeGroups": book.TradeGroups, "notes": book.Notes, "securities": book.Securities}, "")
		return
	}
	rs.send(404, map[string]any{"ok": false, "error": "not found"}, "")
}

func (a *App) viewJSON(filters any, detail, page string) (payload []byte, err error) {
	defer func() {
		if rec := recover(); rec != nil {
			err = fmt.Errorf("%v", rec)
		}
	}()
	view := a.model.View(filters, detail, page)
	status, marshalErr := json.Marshal(a.statusPayload())
	if marshalErr != nil {
		return nil, marshalErr
	}
	out := make([]byte, 0, len(view)+len(status)+12)
	out = append(out, view[:len(view)-1]...)
	out = append(out, `,"status":`...)
	out = append(out, status...)
	out = append(out, '}')
	return out, nil
}

func (a *App) streamNotifications(rs *responder) {
	r := rs.r
	q := r.URL.Query()
	after := strings.TrimSpace(q.Get("after"))
	if after == "" {
		after = strings.TrimSpace(r.Header.Get("Last-Event-ID"))
	}
	h := rs.w.Header()
	h.Set("Content-Type", "text/event-stream; charset=utf-8")
	h.Set("Cache-Control", "no-store")
	h.Set("X-Content-Type-Options", "nosniff")
	rs.w.WriteHeader(200)
	fl, _ := rs.w.(http.Flusher)
	var afterID *int64
	if n, err := strconv.ParseInt(after, 10, 64); err == nil && after != "" {
		afterID = &n
	}
	ctx := r.Context()
	a.notify.Stream(rs.w, func() {
		if fl != nil {
			fl.Flush()
		}
	}, afterID, func() bool { return ctx.Err() == nil }, 0)
}

func (a *App) doPost(rs *responder, port int) {
	r := rs.r
	path := r.URL.Path
	if !a.gate(r, port, true) {
		rs.forbidden()
		return
	}
	switch path {
	case "/api/login/start":
		readJSON(r)
		rs.send(200, a.startLoginBrowser(), "")
	case "/api/login/cancel":
		readJSON(r)
		rs.send(200, a.cancelLogin(), "")
	case "/api/login/input":
		rs.send(200, a.loginInput(dictOr(readJSON(r))), "")
	case "/api/update":
		readJSON(r)
		rs.send(200, a.startUpdate(), "")
	case "/api/capture":
		rs.send(200, a.captureTokens(readJSON(r)), "")
	case "/api/refresh":
		readJSON(r)
		rs.send(200, a.refreshNow(), "")
	case "/api/sync":
		readJSON(r)
		if a.loadSession() == nil {
			rs.send(200, map[string]any{"ok": false, "error": "not connected"}, "")
			return
		}
		a.setError("")
		go a.syncThenMarket()
		rs.send(200, map[string]any{"ok": true, "syncing": true}, "")
	case "/api/data/clear":
		body := dictOr(readJSON(r))
		a.mu.Lock()
		syncing := a.state.syncing
		a.mu.Unlock()
		if syncing {
			rs.send(409, map[string]any{"ok": false, "error": "A sync is running. Wait for it to finish."}, "")
			return
		}
		summary := a.st.ClearSyncedData(!truthy(body["journal"]), !truthy(body["market"]))
		if truthy(body["session"]) {
			a.deleteSessionAndBook()
		}
		a.mu.Lock()
		a.state.lastSync = ""
		a.state.err = ""
		a.mu.Unlock()
		a.invalidate(true)
		summary["ok"] = true
		summary["sessionPresent"] = a.loadSession() != nil
		rs.send(200, summary, "")
	case "/api/order":
		rs.send(200, a.placeOrder(readJSON(r)), "")
	case "/api/order/cancel":
		rs.send(200, a.cancelOrder(py.S(dictOr(readJSON(r))["id"])), "")
	case "/api/order/modify":
		body := dictOr(readJSON(r))
		rs.send(200, a.modifyOrder(py.S(body["id"]), body["quantity"], body["limitPrice"]), "")
	case "/api/bracket/adjust":
		body := dictOr(readJSON(r))
		rs.send(200, a.adjustBracket(py.S(body["id"]), py.S(body["leg"]), body["price"], body["trail"], truthy(body["remove"])), "")
	case "/api/bracket/cancel":
		rs.send(200, a.cancelBracket(py.S(dictOr(readJSON(r))["id"])), "")
	case "/api/notifications/settings":
		a.notify.SetSettings(dictOr(readJSON(r)))
		rs.send(200, map[string]any{"ok": true, "settings": a.notify.Status()}, "")
	case "/api/notifications/test":
		readJSON(r)
		row := a.notify.TestNotification()
		id := int64(0)
		if row != nil {
			id = row.ID
		}
		rs.send(200, map[string]any{"ok": row != nil, "id": id}, "")
	case "/api/notifications/read":
		body := dictOr(readJSON(r))
		ids, isList := body["ids"].([]any)
		rs.send(200, map[string]any{"ok": true, "read": a.st.MarkNotificationsRead(int64List(ids), !isList)}, "")
	case "/api/notifications/clear":
		readJSON(r)
		rs.send(200, map[string]any{"ok": true, "cleared": a.st.ClearNotifications()}, "")
	case "/api/notifications/seen":
		body := dictOr(readJSON(r))
		ids, _ := body["ids"].([]any)
		rs.send(200, map[string]any{"ok": true, "seen": a.st.MarkNotificationsSeen(int64List(ids))}, "")
	case "/api/orders/refresh":
		readJSON(r)
		out := a.refreshOrders("")
		for k, v := range a.ordersPayload(false) {
			out[k] = v
		}
		rs.send(200, out, "")
	case "/api/markets/refresh":
		rs.send(200, a.kickUniverses(), "")
	case "/api/watchlist/add":
		rs.send(200, a.watchAdd(readJSON(r)), "")
	case "/api/watchlist/remove":
		rs.send(200, a.watchRemove(readJSON(r)), "")
	case "/api/tiles/set":
		rs.send(200, a.tilesSet(readJSON(r)), "")
	case "/api/journal":
		body := readJSON(r)
		if body == nil || py.Strip(py.S(body["id"])) == "" {
			rs.send(400, map[string]any{"ok": false, "error": "id required"}, "")
			return
		}
		entries := a.st.SaveJournalEntry(py.S(body["id"]), map[string]any{"thesis": body["thesis"], "tags": body["tags"], "grade": body["grade"]})
		a.model.ApplyJournal(entries)
		rs.send(200, map[string]any{"ok": true, "journal": entries}, "")
	case "/api/disconnect":
		readJSON(r)
		a.deleteSessionAndBook()
		rs.send(200, map[string]any{"ok": true}, "")
	case "/api/book/append":
		result := a.appendManual(readJSON(r))
		a.invalidate(true)
		rs.send(200, result, "")
	case "/api/import":
		body := dictOr(readJSON(r))
		text, ok := body["text"].(string)
		if !ok || strings.TrimSpace(text) == "" {
			rs.send(400, map[string]any{"ok": false, "error": "text required"}, "")
			return
		}
		name := py.S(body["name"])
		if name == "" {
			name = "upload.csv"
		}
		report := csvimport.ImportText(a.st, name, text)
		if report.Added > 0 {
			a.invalidate(true)
		}
		rs.send(200, report, "")
	case "/api/watch":
		body := dictOr(readJSON(r))
		setResult := csvimport.SetWatchFolder(a.st, py.S(body["path"]))
		if !truthy(setResult["ok"]) {
			rs.send(400, setResult, "")
			return
		}
		result := csvimport.ScanFolder(a.st, csvimport.WatchFolder(a.st), true)
		if truthy(result["added"]) {
			a.invalidate(true)
		}
		result["status"] = csvimport.Status(a.st)
		rs.send(200, result, "")
	case "/api/watch/scan":
		readJSON(r)
		result := csvimport.ScanFolder(a.st, csvimport.WatchFolder(a.st), true)
		if truthy(result["ok"]) && truthy(result["added"]) {
			a.invalidate(true)
		}
		result["status"] = csvimport.Status(a.st)
		code := 400
		if truthy(result["ok"]) {
			code = 200
		}
		rs.send(code, result, "")
	case "/api/watch/clear":
		readJSON(r)
		csvimport.ClearWatchFolder(a.st)
		rs.send(200, csvimport.Status(a.st), "")
	case "/api/groups":
		body := readJSON(r)
		var groups any = []any{}
		if body != nil {
			groups = body["groups"]
		}
		rs.send(200, map[string]any{"ok": true, "groups": a.st.SaveTradeGroups(groups)}, "")
	case "/api/notes":
		body := readJSON(r)
		var notes any = map[string]any{}
		if body != nil {
			notes = body["notes"]
		}
		rs.send(200, map[string]any{"ok": true, "notes": a.st.SaveTradeNotes(notes)}, "")
	default:
		rs.send(404, map[string]any{"ok": false, "error": "not found"}, "")
	}
}

func int64List(ids []any) []int64 {
	if ids == nil {
		return nil
	}
	out := make([]int64, 0, len(ids))
	for _, v := range ids {
		if f, ok := py.NumOK(v); ok {
			out = append(out, int64(f))
		}
	}
	return out
}

func (a *App) bindServer() (*serverHandle, error) {
	var last error
	for _, port := range a.cfg.Ports {
		ln, err := net.Listen("tcp", net.JoinHostPort(a.cfg.BindHost, strconv.Itoa(port)))
		if err != nil {
			last = err
			continue
		}
		srv := &http.Server{Handler: a.handle(port), ReadHeaderTimeout: 30 * time.Second, IdleTimeout: 2 * time.Minute}
		h := &serverHandle{srv: srv, port: port}
		a.serverMu.Lock()
		a.server = h
		a.serverMu.Unlock()
		a.notify.Configure("http://127.0.0.1:"+strconv.Itoa(port)+"/", a.iconPath())
		go func() { _ = srv.Serve(ln) }()
		return h, nil
	}
	ports := make([]string, 0, len(a.cfg.Ports))
	for _, p := range a.cfg.Ports {
		ports = append(ports, strconv.Itoa(p))
	}
	return nil, fmt.Errorf("Could not bind %s:%s (%v)", a.cfg.BindHost, strings.Join(ports, "-"), last)
}

func (a *App) iconPath() string {
	if p := filepath.Join(a.cfg.AppDir, "favicon.png"); isFile(p) {
		return p
	}
	p := filepath.Join(a.cfg.Home, "favicon.png")
	if !isFile(p) {
		if e, ok := a.staticFile("favicon.png"); ok {
			_ = os.WriteFile(p, e.data, 0o644)
		}
	}
	return p
}

func (a *App) shutdownServer() {
	a.serverMu.Lock()
	h := a.server
	a.serverMu.Unlock()
	if h == nil {
		return
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	_ = h.srv.Shutdown(ctx)
}

func openBrowser(target string) {
	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "darwin":
		cmd = exec.Command("open", target)
	case "windows":
		cmd = exec.Command("rundll32", "url.dll,FileProtocolHandler", target)
	default:
		if p := which("xdg-open"); p != "" {
			cmd = exec.Command(p, target)
		} else if p := which("sensible-browser", "x-www-browser", "gnome-open"); p != "" {
			cmd = exec.Command(p, target)
		}
	}
	if cmd != nil {
		_ = cmd.Start()
	}
}

func (a *App) Run() int {
	_ = ensureHome(a.cfg.Home)
	_ = a.st.EnsureChanged()
	a.bootSession()
	h, err := a.bindServer()
	if err != nil {
		fmt.Fprintln(os.Stderr, err.Error())
		return 1
	}
	go a.autoSyncLoop()
	go a.refreshMarketData()
	go a.checkForUpdate(time.Time{})
	go a.quoteLoop()
	go a.portfolioLoop()
	go a.ordersLoop()
	go a.bracketLoop()
	go a.exposureLoop()
	for _, w := range a.st.ListWatchlist() {
		bare := market.TMXSymbol(w.Symbol)
		if bare != "" && bare != w.Symbol {
			a.st.RemoveWatch(w.Symbol, w.Exchange)
			a.st.AddWatch(bare, w.Exchange, w.Name, w.Currency, w.SecurityID, w.AddedAt)
		}
	}
	go a.newsLoop()
	go a.universeLoop()
	go a.marketLoop()
	go a.archiveLoop()
	go a.watchLoop()
	go a.filingsSweepLoop()
	go a.shortsSweepLoop()
	go a.fearSweepLoop()
	url := "http://127.0.0.1:" + strconv.Itoa(h.port)
	fmt.Printf("Bagholder  %s\n", startupLine(a.cfg.BindHost, h.port))
	if !a.cfg.NoBrowser {
		openBrowser(url)
	}
	a.mu.Lock()
	connected := a.state.connected
	a.mu.Unlock()
	if connected {
		if a.st.ActivityPullDue(time.Now()) {
			go a.runSync(true, true)
		} else {
			go a.fillListings(a.loadSession(), false)
		}
	}
	sigs := make(chan os.Signal, 2)
	signal.Notify(sigs, os.Interrupt, syscall.SIGTERM)
	select {
	case <-sigs:
		fmt.Println("\nBagholder stopped.")
	case <-a.stopCh:
	}
	signal.Stop(sigs)
	a.setStop()
	a.shutdownServer()
	a.closeLoginBrowser(nil)
	a.enricher.Model.Shutdown()
	a.mu.Lock()
	code := a.exitCode
	a.mu.Unlock()
	return code
}
