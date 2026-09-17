package app

import (
	"encoding/json"
	"fmt"
	"math"
	"net/url"
	"sort"
	"strings"
	"sync"

	"github.com/ProfessorBagholder/Bagholder/internal/instruments"
	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/model"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
	"github.com/ProfessorBagholder/Bagholder/internal/symbols"
	"github.com/ProfessorBagholder/Bagholder/internal/ws"
)

var (
	OrderExecTypes         = []string{"MARKET", "LIMIT", "STOP", "STOP_LIMIT"}
	OrderTifs              = []string{"DAY", "UNTIL_CANCEL"}
	OrderTradableTypes     = []string{"SELF_DIRECTED"}
	OrderUntradableMarkers = []string{"CRYPTO", "PREDICTIONS", "MANAGED"}
)

func (a *App) ticketSession() ws.Session {
	sess := a.loadSession()
	if sess == nil || sess.Str("access_token") == "" {
		return nil
	}
	a.ensureFreshToken(sess)
	if s := a.loadSession(); s != nil {
		return s
	}
	return sess
}

type OrderAccount struct {
	ID              string `json:"id"`
	Name            string `json:"name"`
	Type            string `json:"type"`
	Margin          bool   `json:"margin"`
	Currency        string `json:"currency"`
	MarginAccountID string `json:"marginAccountId"`
}

func (a *App) orderAccounts() []OrderAccount {
	out := []OrderAccount{}
	for _, acc := range a.st.Snapshot(false).Accounts {
		typ := strings.ToUpper(acc.UnifiedAccountType)
		status := strings.ToLower(acc.Status)
		if acc.ID == "" || status == "closed" || !startsAny(typ, OrderTradableTypes) {
			continue
		}
		if containsAny(typ, OrderUntradableMarkers) {
			continue
		}
		nickRaw := acc.Nickname
		if nickRaw == "" {
			nickRaw = typ
		}
		margin := strings.Contains(typ, "MARGIN")
		marginID := acc.MarginAccountID
		if margin {
			marginID = acc.ID
		}
		out = append(out, OrderAccount{ID: acc.ID, Name: symbols.NormAccountName(nickRaw), Type: typ, Margin: margin, Currency: acc.Currency, MarginAccountID: marginID})
	}
	return out
}

func startsAny(s string, prefixes []string) bool {
	for _, p := range prefixes {
		if strings.HasPrefix(s, p) {
			return true
		}
	}
	return false
}

func containsAny(s string, parts []string) bool {
	for _, p := range parts {
		if strings.Contains(s, p) {
			return true
		}
	}
	return false
}

func (a *App) findAccount(id string) *OrderAccount {
	for _, acc := range a.orderAccounts() {
		if acc.ID == id {
			c := acc
			return &c
		}
	}
	return nil
}

func (a *App) resolveSecurity(symbol, securityID string) *store.Security {
	rows := a.st.ListSecurities()
	sid := py.Strip(securityID)
	if sid != "" {
		for i := range rows {
			if rows[i].ID == sid {
				return &rows[i]
			}
		}
		return &store.Security{ID: sid, Symbol: strings.ToUpper(py.Strip(symbol))}
	}
	sym := strings.ToUpper(py.Strip(symbol))
	if sym == "" {
		return nil
	}
	var same []store.Security
	for _, r := range rows {
		if strings.ToUpper(r.Symbol) == sym {
			same = append(same, r)
		}
	}
	sort.SliceStable(same, func(i, j int) bool {
		ai, aj := 1, 1
		if strings.HasPrefix(same[i].ID, "sec-s-") {
			ai = 0
		}
		if strings.HasPrefix(same[j].ID, "sec-s-") {
			aj = 0
		}
		if ai != aj {
			return ai < aj
		}
		return same[i].ID < same[j].ID
	})
	if len(same) == 0 {
		return nil
	}
	return &same[0]
}

const (
	NasdaqSearchURL = "https://api.nasdaq.com/api/autocomplete/slookup/10?search=%s"
	TSXSearchURL    = "https://www.tsx.com/json/company-directory/search/%s/%s"
	SearchMax       = 12
)

var (
	SearchHeaders            = map[string]string{"User-Agent": market.UA, "Accept": "application/json, text/plain, */*", "Accept-Language": "en-CA,en;q=0.9"}
	NasdaqExchanges          = map[string]string{"NYSE": "NYSE", "AMEX": "NYSE", "PSE": "NYSE", "NASDAQ-GS": "NASDAQ", "NASDAQ-GM": "NASDAQ", "NASDAQ-CM": "NASDAQ", "NASDAQ": "NASDAQ", "BAT": "BATS"}
	NasdaqAssets             = []string{"STOCKS", "ETF"}
	NasdaqDerivativeSuffixes = []string{"WS", "W", "U", "RT", "R"}
	NasdaqNameTails          = []string{" Common Stock", " Common Shares", " Ordinary Shares", " Class A Common Stock", " Class A Ordinary Shares"}
)

type SearchRow struct {
	Symbol   string `json:"symbol"`
	Name     string `json:"name"`
	Exchange string `json:"exchange"`
	Currency string `json:"currency"`
	Kind     string `json:"kind,omitempty"`
	Rank     *int   `json:"rank,omitempty"`
}

type SearchResult struct {
	OK      bool        `json:"ok"`
	Error   string      `json:"error,omitempty"`
	Matches []SearchRow `json:"matches"`
}

func parseNasdaqSearch(data map[string]any) []SearchRow {
	out := []SearchRow{}
	rows, _ := data["data"].([]any)
	for _, raw := range rows {
		q, ok := raw.(map[string]any)
		if !ok || !py.Contains(NasdaqAssets, strings.ToUpper(py.S(q["asset"]))) {
			continue
		}
		ex := NasdaqExchanges[strings.ToUpper(py.S(q["exchange"]))]
		sym := strings.ToUpper(py.S(q["symbol"]))
		tail := sym
		if i := strings.LastIndex(sym, "."); i >= 0 {
			tail = sym[i+1:]
		}
		if ex == "" || sym == "" || py.Contains(NasdaqDerivativeSuffixes, tail) {
			continue
		}
		name := py.S(q["name"])
		for _, t := range NasdaqNameTails {
			if strings.HasSuffix(name, t) {
				name = strings.TrimRight(name[:len(name)-len(t)], " ,")
				break
			}
		}
		out = append(out, SearchRow{Symbol: sym, Name: name, Exchange: ex, Currency: "USD"})
	}
	return out
}

func parseTSXSearch(data map[string]any, exchange string) []SearchRow {
	out := []SearchRow{}
	rows, _ := data["results"].([]any)
	for _, raw := range rows {
		r, ok := raw.(map[string]any)
		if !ok {
			continue
		}
		if sym := strings.ToUpper(py.S(r["symbol"])); sym != "" {
			out = append(out, SearchRow{Symbol: sym, Name: py.S(r["name"]), Exchange: exchange, Currency: "CAD"})
		}
	}
	return out
}

func rankSearch(text string, rows []SearchRow) []SearchRow {
	key := strings.ToUpper(py.Strip(text))
	seen := map[[2]string]bool{}
	out := []SearchRow{}
	for _, r := range rows {
		k := [2]string{r.Symbol, r.Exchange}
		if seen[k] {
			continue
		}
		seen[k] = true
		out = append(out, r)
	}
	rankOf := func(r SearchRow) int {
		if r.Rank != nil {
			return *r.Rank
		}
		if r.Symbol == key {
			return 0
		}
		if strings.HasPrefix(r.Symbol, key) {
			return 1
		}
		return 2
	}
	sort.SliceStable(out, func(i, j int) bool { return rankOf(out[i]) < rankOf(out[j]) })
	if len(out) > SearchMax {
		out = out[:SearchMax]
	}
	return out
}

func (a *App) symbolSearch(text string) SearchResult {
	text = py.Strip(text)
	if text == "" {
		return SearchResult{OK: true, Matches: []SearchRow{}}
	}
	key := strings.ToUpper(text)
	a.searchMu.Lock()
	if rows, ok := a.searchCache[key]; ok {
		a.searchMu.Unlock()
		return SearchResult{OK: true, Matches: rows}
	}
	a.searchMu.Unlock()
	text, venues := market.YahooSplit(text)
	q := url.QueryEscape(text)
	q = strings.ReplaceAll(q, "+", "%20")
	type searchJob struct {
		name  string
		url   string
		parse func(map[string]any) []SearchRow
	}
	jobs := []searchJob{
		{"nasdaq", fmt.Sprintf(NasdaqSearchURL, q), parseNasdaqSearch},
		{"tsx", fmt.Sprintf(TSXSearchURL, "tsx", q), func(d map[string]any) []SearchRow { return parseTSXSearch(d, "TSX") }},
		{"tsxv", fmt.Sprintf(TSXSearchURL, "tsxv", q), func(d map[string]any) []SearchRow { return parseTSXSearch(d, "TSX-V") }},
	}
	answers := map[string][]SearchRow{}
	errs := map[string]string{}
	var errOrder []string
	var mu sync.Mutex
	var wg sync.WaitGroup
	for _, j := range jobs {
		wg.Add(1)
		go func(j searchJob) {
			defer wg.Done()
			text, err := a.mk.GetText(j.url, SearchHeaders)
			var data map[string]any
			if err == nil {
				err = json.Unmarshal([]byte(text), &data)
			}
			mu.Lock()
			defer mu.Unlock()
			if err != nil {
				errs[j.name] = errText(err)
				errOrder = append(errOrder, j.name)
				return
			}
			answers[j.name] = j.parse(data)
		}(j)
	}
	wg.Wait()
	if len(answers) == 0 {
		var parts []string
		for _, n := range errOrder {
			parts = append(parts, errs[n])
		}
		return SearchResult{OK: false, Error: "Search failed: " + strings.Join(parts, "; "), Matches: []SearchRow{}}
	}
	var found []SearchRow
	for _, j := range jobs {
		found = append(found, answers[j.name]...)
	}
	if len(found) == 0 && len([]rune(text)) <= 6 && !strings.Contains(text, " ") {
		if hit := a.mk.TMXListing(text); hit != nil {
			found = []SearchRow{{Symbol: hit.Symbol, Name: hit.Name, Exchange: hit.Exchange, Currency: hit.Currency}}
		}
	}
	var rows []SearchRow
	for _, m := range instruments.Search(text) {
		rank := m.Rank
		rows = append(rows, SearchRow{Symbol: m.Symbol, Name: m.Name, Exchange: m.Exchange, Currency: m.Currency, Kind: m.Kind, Rank: &rank})
	}
	rows = rankSearch(text, append(rows, found...))
	if len(venues) > 0 {
		var kept []SearchRow
		for _, r := range rows {
			if py.Contains(venues, strings.ToUpper(r.Exchange)) {
				kept = append(kept, r)
			}
		}
		if len(kept) > 0 {
			rows = kept
		}
	}
	if len(errs) == 0 {
		a.searchMu.Lock()
		a.searchCache[key] = rows
		a.searchMu.Unlock()
	}
	return SearchResult{OK: true, Matches: rows}
}

func (a *App) lookupListing(sess ws.Session, symbol, exchange string) *store.Security {
	sec := a.ws.LookupListing(sess, symbol, exchange)
	if sec != nil {
		a.st.UpsertSecurities([]store.Security{*sec})
	}
	return sec
}

func (a *App) ticketQuote(symbol, securityID, accountID, exchange string) map[string]any {
	sec := a.resolveSecurity(symbol, securityID)
	noListing := func() map[string]any {
		name := py.S(symbol)
		if name == "" {
			name = py.S(securityID)
		}
		return map[string]any{"ok": false, "error": "No listing stored for " + name + "."}
	}
	if sec == nil && py.S(exchange) == "" {
		return noListing()
	}
	sess := a.ticketSession()
	if sess == nil {
		return map[string]any{"ok": false, "error": "Not connected."}
	}
	if sec == nil {
		sec = a.lookupListing(sess, strings.ToUpper(py.Strip(symbol)), exchange)
	}
	if sec == nil {
		return noListing()
	}
	quotes, err := a.ws.FetchQuotes(sess, []string{sec.ID})
	if err != nil {
		if notAuthorized(err) {
			return map[string]any{"ok": false, "error": "Wealthsimple refused the session. Connect Wealthsimple again."}
		}
		return map[string]any{"ok": false, "error": "Quote failed: " + errText(err)}
	}
	quote := quotes[sec.ID]
	if quote == nil {
		name := sec.Symbol
		if name == "" {
			name = sec.ID
		}
		return map[string]any{"ok": false, "error": "Wealthsimple has no quote for " + name + "."}
	}
	if quote.Symbol == "" {
		quote.Symbol = sec.Symbol
	}
	if quote.Name == "" {
		quote.Name = sec.Name
	}
	if quote.Exchange == "" {
		quote.Exchange = sec.PrimaryExchange
	}
	if quote.Currency == "" {
		quote.Currency = strings.ToUpper(sec.Currency)
	}
	md := ws.MarketData{OrderTypes: append([]string{}, OrderExecTypes...)}
	if data, err := a.ws.GraphQL(sess, "FetchSecurityMarketData", map[string]any{"id": sec.ID}, ""); err != nil {
		a.logf("bagholder ticket: market data for %s failed: %s\n", sec.ID, errText(err))
	} else {
		md = ws.ParseMarketData(data)
	}
	accounts := a.orderAccounts()
	var acct *OrderAccount
	for i := range accounts {
		if accounts[i].ID == py.S(accountID) {
			acct = &accounts[i]
			break
		}
	}
	balance := ws.BuyingPower{}
	if acct != nil {
		ccy := quote.Currency
		if ccy == "" {
			ccy = "CAD"
		}
		if data, err := a.ws.GraphQL(sess, "FetchTradingBalanceBuyingPower", map[string]any{"accountCanonicalId": acct.ID, "currency": ccy, "securityId": sec.ID}, ""); err != nil {
			a.logf("bagholder ticket: buying power for %s failed: %s\n", acct.ID, errText(err))
		} else {
			balance = ws.ParseBuyingPower(data)
		}
	}
	var marginAvailable *float64
	if acct != nil && acct.MarginAccountID != "" {
		for _, m := range a.st.Snapshot(false).Margin {
			if m.AccountID == acct.MarginAccountID && m.BuyingPower != nil {
				v := *m.BuyingPower
				marginAvailable = &v
			}
		}
	}
	fx := a.st.FXRates()
	var fxRate *float64
	if len(fx) > 0 {
		r := model.RateOn(fx, model.TodayLocal())
		fxRate = &r
	}
	orderTypes := md.OrderTypes
	if len(orderTypes) == 0 {
		orderTypes = append([]string{}, OrderExecTypes...)
	}
	var acctOut any
	if acct != nil {
		acctOut = *acct
	}
	return map[string]any{"ok": true, "quote": quote, "orderTypes": orderTypes, "marginRate": md.MarginRate, "accounts": accounts, "account": acctOut,
		"buyingPower": balance.BuyingPower, "cash": balance.Cash, "marginAvailable": marginAvailable, "fxUsdCad": fxRate, "live": a.cfg.OrdersLive}
}

func orderTick(price *float64) *float64 {
	if price == nil {
		return nil
	}
	n := 4
	if *price >= 1 {
		n = 2
	}
	v := py.Round(*price, n)
	return &v
}

func numPtr(v any) *float64 {
	if f, ok := py.NumOK(v); ok {
		return &f
	}
	return nil
}

func positive(p *float64) bool { return p != nil && *p != 0 && *p > 0 }

func (a *App) orderRequest(b map[string]any) (*store.Order, map[string]any, string) {
	if b == nil {
		b = map[string]any{}
	}
	side := strings.ToUpper(py.S(b["side"]))
	if side != "BUY" && side != "SELL" {
		return nil, nil, "Side must be Buy or Sell."
	}
	execType := strings.ToUpper(py.S(b["type"]))
	if !py.Contains(OrderExecTypes, execType) {
		return nil, nil, "Order type must be Market, Limit, Stop or Stop limit."
	}
	tif := strings.ToUpper(py.S(b["tif"]))
	if tif == "" {
		tif = "DAY"
	}
	if !py.Contains(OrderTifs, tif) {
		return nil, nil, "Time in force must be Day or Good till cancelled."
	}
	qty := py.Num(b["quantity"], 0)
	if qty <= 0 {
		return nil, nil, "Quantity must be more than zero."
	}
	limitPrice := orderTick(numPtr(b["limitPrice"]))
	stopPrice := orderTick(numPtr(b["stopPrice"]))
	if (execType == "LIMIT" || execType == "STOP_LIMIT") && !positive(limitPrice) {
		return nil, nil, "A limit price is required."
	}
	if (execType == "STOP" || execType == "STOP_LIMIT") && !positive(stopPrice) {
		return nil, nil, "A stop price is required."
	}
	acct := a.findAccount(py.S(b["accountId"]))
	if acct == nil {
		return nil, nil, "Choose an account."
	}
	sec := a.resolveSecurity(py.S(b["symbol"]), py.S(b["securityId"]))
	if sec == nil {
		return nil, nil, "No listing stored for " + py.S(b["symbol"]) + "."
	}
	slRaw, _ := b["stopLoss"].(map[string]any)
	tpRaw, _ := b["takeProfit"].(map[string]any)
	if side == "SELL" {
		slRaw, tpRaw = nil, nil
	}
	var sl *store.StopLoss
	if slRaw != nil {
		kind := strings.ToLower(py.S(slRaw["kind"]))
		if kind == "" {
			kind = "stop"
		}
		if kind != "stop" && kind != "trail" {
			return nil, nil, "Stop loss type must be Stop or Trailing stop."
		}
		if kind == "stop" && !(py.Num(slRaw["price"], 0) > 0) {
			return nil, nil, "A stop loss price is required."
		}
		if kind == "trail" && !(py.Num(slRaw["trail"], 0) > 0) {
			return nil, nil, "A trail is required."
		}
		unit := "pct"
		if strings.ToLower(py.S(slRaw["trailUnit"])) == "amt" {
			unit = "amt"
		}
		sl = &store.StopLoss{Kind: kind, Price: orderTick(numPtr(slRaw["price"])), Trail: numPtr(slRaw["trail"]), TrailUnit: unit}
	}
	var tp *store.TakeProfit
	if tpRaw != nil {
		if !(py.Num(tpRaw["price"], 0) > 0) {
			return nil, nil, "A take profit price is required."
		}
		tp = &store.TakeProfit{Price: orderTick(numPtr(tpRaw["price"]))}
	}
	oid := "order-" + py.UUID4()
	req := map[string]any{"canonicalAccountId": acct.ID, "externalId": oid, "executionType": execType, "orderType": side + "_QUANTITY", "quantity": qty, "securityId": sec.ID, "timeInForce": tif}
	var rowLimit, rowStop *float64
	if execType == "LIMIT" || execType == "STOP_LIMIT" {
		req["limitPrice"] = *limitPrice
		rowLimit = limitPrice
	}
	if execType == "STOP" || execType == "STOP_LIMIT" {
		req["stopPrice"] = *stopPrice
		rowStop = stopPrice
	}
	currency := strings.ToUpper(py.S(b["currency"]))
	if currency == "" {
		currency = strings.ToUpper(sec.Currency)
	}
	row := &store.Order{ID: oid, CreatedAt: nowStamp(), AccountID: acct.ID, Account: acct.Name, SecurityID: sec.ID, Symbol: sec.Symbol, Currency: currency, Side: side, Type: execType,
		Quantity: qty, LimitPrice: rowLimit, StopPrice: rowStop, Tif: tif, StopLoss: sl, TakeProfit: tp, Request: req}
	return row, req, ""
}

func sortedJSON(v any) string {
	raw, _ := json.Marshal(v)
	return string(raw)
}

func graphqlErrors(result map[string]any) string {
	errs, _ := result["errors"].([]any)
	if len(errs) == 0 {
		return ""
	}
	first, ok := errs[0].(map[string]any)
	if !ok {
		return py.S(errs[0])
	}
	msg := py.S(first["message"])
	if msg == "" {
		msg = py.S(first["code"])
	}
	return msg
}

func (a *App) submitOrder(row *store.Order, req map[string]any) map[string]any {
	if !a.cfg.OrdersLive {
		row.Status = "dry"
		a.st.InsertOrder(*row)
		a.logf("bagholder order (dry run, not sent): %s\n", sortedJSON(req))
		return map[string]any{"ok": true, "id": row.ID, "status": "dry", "order": orderRowMap(row)}
	}
	sess := a.ticketSession()
	if sess == nil {
		return map[string]any{"ok": false, "error": "Not connected."}
	}
	row.Status = "sending"
	a.st.InsertOrder(*row)
	data, err := a.ws.GraphQL(sess, "SoOrdersOrderCreate", map[string]any{"input": req}, "")
	if err != nil {
		if notAuthorized(err) {
			a.st.UpdateOrder(row.ID, store.OrderPatch{"status": "failed", "error": "Wealthsimple refused the session."})
			return map[string]any{"ok": false, "error": "Wealthsimple refused the session. Connect Wealthsimple again.", "id": row.ID}
		}
		msg := errText(err)
		a.st.UpdateOrder(row.ID, store.OrderPatch{"status": "failed", "error": msg})
		a.logf("bagholder order: %s failed: %s\n", row.ID, msg)
		return map[string]any{"ok": false, "error": "Order failed: " + msg, "id": row.ID}
	}
	result, _ := data["soOrdersCreateOrder"].(map[string]any)
	if errs, _ := result["errors"].([]any); len(errs) > 0 {
		msg := graphqlErrors(result)
		a.st.UpdateOrder(row.ID, store.OrderPatch{"status": "rejected", "error": msg})
		a.logf("bagholder order: %s rejected: %s\n", row.ID, msg)
		return map[string]any{"ok": false, "error": "Wealthsimple rejected the order: " + msg, "id": row.ID}
	}
	order, _ := result["order"].(map[string]any)
	wsID := py.S(order["orderId"])
	a.st.UpdateOrder(row.ID, store.OrderPatch{"status": "sent", "wsOrderId": wsID})
	a.logf("bagholder order: %s sent, Wealthsimple order %s\n", row.ID, wsID)
	go a.refreshOrders(row.ID)
	return map[string]any{"ok": true, "id": row.ID, "status": "sent", "wsOrderId": wsID}
}

func (a *App) placeOrder(body map[string]any) map[string]any {
	row, req, errMsg := a.orderRequest(body)
	if errMsg != "" {
		return map[string]any{"ok": false, "error": errMsg}
	}
	if row.Side == "SELL" {
		left := row.Quantity
		for _, b := range a.st.ListBrackets(BracketLive) {
			if b.AccountID != row.AccountID || b.SecurityID != row.SecurityID || b.Status == "waiting" || b.Status == "closing" {
				continue
			}
			held := py.Deref(b.Quantity, 0)
			if left >= held {
				a.endBracket(b, "sold from the ticket", "")
				a.awaitCancels(b, 8)
				left -= held
			} else if left > 0 {
				a.releaseShares(b, left)
				a.awaitCancels(b, 8)
				left = 0
			}
		}
	}
	r := a.submitOrder(row, req)
	if truthy(r["ok"]) && (row.StopLoss != nil || row.TakeProfit != nil) {
		b := a.createBracket(row)
		r["bracketId"] = b.ID
	}
	return r
}

func absf(x float64) float64 { return math.Abs(x) }

func orderRowMap(row *store.Order) map[string]any {
	var sl, tp any
	if row.StopLoss != nil {
		sl = row.StopLoss
	}
	if row.TakeProfit != nil {
		tp = row.TakeProfit
	}
	return map[string]any{"id": row.ID, "createdAt": row.CreatedAt, "accountId": row.AccountID, "account": row.Account, "securityId": row.SecurityID, "symbol": row.Symbol,
		"currency": row.Currency, "side": row.Side, "type": row.Type, "quantity": row.Quantity, "limitPrice": row.LimitPrice, "stopPrice": row.StopPrice, "tif": row.Tif,
		"stopLoss": sl, "takeProfit": tp, "status": row.Status, "wsOrderId": row.WsOrderID, "error": row.Error, "request": row.Request}
}
