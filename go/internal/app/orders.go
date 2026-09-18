package app

import (
	"math"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
	"github.com/ProfessorBagholder/Bagholder/internal/symbols"
	"github.com/ProfessorBagholder/Bagholder/internal/ws"
)

const OrdersRefreshSec = 30

var LiveStatuses = []string{"sent", "pending", "cancelling"}

func qtyWords(q float64) string {
	if py.IsInteger(q) {
		return strconv.FormatInt(int64(q), 10)
	}
	return py.G(q)
}

func priceWords(p *float64) string {
	if p == nil {
		return "—"
	}
	v := *p
	if math.Abs(v) < 1 && py.Round(v, 3) != py.Round(v, 2) {
		return py.Fixed(v, 3)
	}
	return py.Fixed(v, 2)
}

func qtyText(q float64) string {
	if py.IsInteger(q) {
		return strconv.FormatInt(int64(q), 10)
	}
	return strings.TrimRight(strings.TrimRight(py.Fixed(q, 6), "0"), ".")
}

func orderWords(o *store.Order) string {
	side := "Sell"
	if o.Side == "BUY" {
		side = "Buy"
	}
	var how string
	switch o.Type {
	case "MARKET":
		how = "at market"
	case "STOP":
		how = "stop " + priceWords(o.StopPrice)
	case "STOP_LIMIT":
		how = "stop " + priceWords(o.StopPrice) + " · limit " + priceWords(o.LimitPrice)
	default:
		how = "at " + priceWords(o.LimitPrice) + " limit"
	}
	return side + " " + qtyWords(o.Quantity) + " " + how
}

type notice struct {
	kind, key, title, body string
}

func nonZero(p *float64) *float64 {
	if p != nil && *p != 0 {
		return p
	}
	return nil
}

func orderNotice(before *store.Order, upd *ws.OrderUpdate) *notice {
	was, now := before.Status, upd.Status
	sym := before.Symbol
	if sym == "" {
		sym = "?"
	}
	role := before.Role
	if role == "" {
		role = "entry"
	}
	acct := before.Account
	tail := ""
	if acct != "" {
		tail = " · " + acct
	}
	oid := before.ID
	qty := before.Quantity
	if q := nonZero(upd.FilledQty); q != nil {
		qty = *q
	} else if q := nonZero(before.FilledQty); q != nil {
		qty = *q
	}
	px := nonZero(upd.AvgFill)
	if px == nil {
		px = nonZero(before.AvgFill)
	}
	at := ""
	if px != nil {
		at = " at " + priceWords(px)
	}
	if now == "filled" && was != "filled" {
		did := "Bought "
		if before.Side == "SELL" {
			did = "Sold "
		}
		title := "Order filled · "
		switch role {
		case "stop":
			title = "Stopped out · "
		case "target":
			title = "Target hit · "
		}
		return &notice{"fills", "order:" + oid + ":filled", title + sym, did + qtyWords(qty) + at + tail}
	}
	if (now == "rejected" || now == "failed") && was != "rejected" && was != "failed" {
		reason := upd.Error
		if reason == "" {
			reason = before.Error
		}
		title := "Order not sent · "
		if now == "rejected" {
			title = "Order rejected · "
		}
		body := orderWords(before)
		if reason != "" {
			body += " · " + reason
		} else {
			body += tail
		}
		return &notice{"problems", "order:" + oid + ":" + now, title + sym, body}
	}
	if role == "stop" || role == "target" {
		return nil
	}
	if now == "expired" && was != "expired" {
		return &notice{"problems", "order:" + oid + ":expired", "Order expired · " + sym, orderWords(before) + tail}
	}
	if now == "cancelled" && was != "cancelled" && was != "cancelling" {
		return &notice{"problems", "order:" + oid + ":cancelled", "Order cancelled · " + sym, orderWords(before) + tail}
	}
	filled := py.Deref(upd.FilledQty, 0)
	if slices.Contains(LiveStatuses, now) && filled > py.Deref(before.FilledQty, 0) && filled < before.Quantity {
		return &notice{"fills", "order:" + oid + ":partial:" + qtyWords(filled), "Partly filled · " + sym, qtyWords(filled) + " of " + qtyWords(before.Quantity) + at + tail}
	}
	return nil
}

func (a *App) feedOrderRow(node ws.Item) store.Order {
	sec, _ := node["security"].(map[string]any)
	stock, _ := sec["stock"].(map[string]any)
	acct := a.findAccount(py.S(node["canonicalAccountId"]))
	side := strings.ToUpper(py.S(node["side"]))
	secID := py.S(node["securityId"])
	if secID == "" {
		secID = py.S(sec["id"])
	}
	symbol := a.st.SymbolForSecurity(secID)
	if symbol == "" {
		symbol = py.S(node["symbol"])
		if symbol == "" {
			symbol = py.S(stock["symbol"])
		}
	}
	account := ""
	if acct != nil {
		account = acct.Name
	}
	typ := strings.ToUpper(py.S(node["executionType"]))
	if typ == "" {
		typ = "LIMIT"
	}
	sideOut := "BUY"
	if strings.HasPrefix(side, "SELL") {
		sideOut = "SELL"
	}
	return store.Order{ID: py.S(node["id"]), CreatedAt: py.S(node["createdAtUtc"]), AccountID: py.S(node["canonicalAccountId"]), Account: account, SecurityID: secID, Symbol: symbol,
		Currency: strings.ToUpper(py.S(node["securityCurrency"])), Side: sideOut, Type: typ, Quantity: py.Num(node["submittedQuantity"], 0), LimitPrice: numPtr(node["limitPrice"]),
		StopPrice: numPtr(node["stopPrice"]), Status: ws.AppStatus(py.S(node["status"])), WsStatus: strings.ToUpper(py.S(node["status"])), WsOrderID: py.S(node["orderId"]),
		AvgFill: numPtr(node["averageFillPrice"]), Source: "wealthsimple"}
}

func (a *App) ordersRefreshedAtValue() string {
	a.ordersMu.Lock()
	defer a.ordersMu.Unlock()
	return a.ordersRefreshedAt
}

func (a *App) kickOrdersRefresh() bool {
	if at := a.ordersRefreshedAtValue(); at != "" {
		age := float64(OrdersRefreshSec)
		if t, err := time.Parse("2006-01-02T15:04:05Z", at); err == nil {
			age = time.Since(t).Seconds()
		}
		if age < OrdersRefreshSec {
			return false
		}
	}
	if !a.connectedIdle() || !a.ordersRefreshing.TryLock() {
		return false
	}
	go func() {
		defer a.ordersRefreshing.Unlock()
		a.refreshOrders("")
	}()
	return true
}

func (a *App) bookOrderFill(order *store.Order, upd *ws.OrderUpdate) bool {
	if order == nil {
		return false
	}
	if order.Source == "wealthsimple" {
		return false
	}
	side := strings.ToUpper(order.Side)
	if side != "BUY" && side != "SELL" {
		return false
	}
	accountID := order.AccountID
	if !store.IsRealAccount(accountID) {
		return false
	}
	if order.SecurityID == "" {
		return false
	}
	symbol := py.Strip(upd.Symbol)
	if symbol == "" {
		symbol = py.Strip(order.Symbol)
	}
	if symbol == "" {
		return false
	}
	filled := py.Deref(upd.FilledQty, py.Deref(order.FilledQty, 0))
	if upd.FilledQty != nil {
		filled = *upd.FilledQty
	}
	price := py.Deref(order.AvgFill, 0)
	if upd.AvgFill != nil {
		price = *upd.AvgFill
	}
	if filled <= 0 || price <= 0 {
		return false
	}
	already := py.Deref(order.FillBookedQty, 0)
	if already+1e-9 >= filled {
		return false
	}
	fillTime := upd.LastFilledAt
	if fillTime == "" {
		fillTime = upd.FirstFilledAt
	}
	if fillTime == "" {
		fillTime = order.SubmittedAt
	}
	date := ws.DateOnly(fillTime)
	if date == "" {
		date = today()
	}
	currency := strings.ToUpper(py.Strip(order.Currency))
	if currency == "" {
		currency = strings.ToUpper(py.Strip(upd.Currency))
	}
	if currency == "" {
		currency = "CAD"
	}
	if currency != "CAD" && currency != "USD" {
		currency = "CAD"
	}
	accounts := accountItemsByID(a.st.Snapshot(false).Accounts)
	mult := symbols.Multiplier(symbol)
	signedQty := filled
	cash := -(filled * price * mult)
	direction := "DEBIT"
	desc := "Buy"
	if side == "SELL" {
		signedQty = -filled
		cash = filled * price * mult
		direction = "CREDIT"
		desc = "Sell"
	}
	fifo := accountID
	if p, ok := ws.FifoPoolIDs(accounts)[accountID]; ok {
		fifo = p
	}
	act := store.Activity{ID: py.UUID4(), OccurredAt: date, TransactionDate: date, SettlementDate: date, AccountID: accountID, BookID: accountID, FifoID: fifo,
		AccountType: ws.AccountType(accountID, accounts), ActivityType: "Trade", ActivitySubType: side, Description: desc + " " + qtyText(filled) + " " + symbol + " @ " + py.Repr(price),
		Direction: direction, Symbol: symbol, Name: symbol, Currency: currency, Quantity: signedQty, UnitPrice: price, Commission: 0, NetCashAmount: cash, Category: "trade",
		SecurityID: order.SecurityID, Source: "bagholder-fill"}
	a.st.InsertLocal(act)
	a.st.MarkOrderFillBooked(order.ID, filled)
	a.invalidate(false)
	a.logf("bagholder orders: %s filled %s %s @ %s booked as a local trade until the next sync\n", order.ID, qtyText(filled), symbol, py.Repr(price))
	return true
}

func (a *App) refreshOrders(onlyID string) map[string]any {
	sess := a.ticketSession()
	if sess == nil {
		return map[string]any{"ok": false, "skipped": "no session"}
	}
	var live []store.Order
	var rows []store.Order
	if onlyID != "" {
		if o := a.st.GetOrder(onlyID); o != nil {
			rows = []store.Order{*o}
		}
	} else {
		rows = a.st.ListOrders(0)
	}
	for _, o := range rows {
		if slices.Contains(LiveStatuses, o.Status) && (onlyID == "" || o.ID == onlyID) {
			live = append(live, o)
		}
	}
	read, failed := 0, 0
	for i := range live {
		o := live[i]
		upd, err := a.ws.FetchExtendedOrder(sess, o.ID)
		if err != nil {
			if notAuthorized(err) {
				a.logf("bagholder orders: Wealthsimple refused the session\n")
				return map[string]any{"ok": false, "skipped": "refused"}
			}
			failed++
			a.logf("bagholder orders: %s status failed: %s\n", o.ID, errText(err))
			continue
		}
		if upd == nil {
			continue
		}
		patch := store.OrderPatch{"wsStatus": upd.WsStatus, "status": upd.Status, "submittedAt": upd.SubmittedAt, "expiresAt": upd.ExpiresAt}
		if upd.FilledQty != nil {
			patch["filledQty"] = *upd.FilledQty
		}
		if upd.AvgFill != nil {
			patch["avgFill"] = *upd.AvgFill
		}
		if upd.Error != "" {
			patch["error"] = upd.Error
		}
		if o.Source == "wealthsimple" || o.Role == "stop" || o.Role == "target" {
			if upd.Tif != "" {
				patch["tif"] = upd.Tif
			}
			if upd.Quantity != nil {
				patch["quantity"] = *upd.Quantity
			}
			if upd.LimitPrice != nil {
				patch["limitPrice"] = *upd.LimitPrice
			}
			if upd.StopPrice != nil {
				patch["stopPrice"] = *upd.StopPrice
			}
			if upd.Currency != "" {
				patch["currency"] = upd.Currency
			}
			if name := a.st.SymbolForSecurity(o.SecurityID); name != "" && name != o.Symbol {
				patch["symbol"] = name
			}
		}
		n := orderNotice(&o, upd)
		a.st.UpdateOrder(o.ID, patch)
		read++
		if n != nil {
			a.notify.Emit(n.kind, n.key, n.title, n.body, nil)
		}
		if upd.Status == "filled" {
			cur := a.st.GetOrder(o.ID)
			if cur == nil {
				cur = &o
			}
			a.bookOrderFill(cur, upd)
		}
	}
	added := 0
	if onlyID == "" {
		if identity := ws.IdentityFrom(sess); identity != "" {
			known := map[string]bool{}
			for _, o := range rows {
				known[o.ID] = true
				if o.WsOrderID != "" {
					known[o.WsOrderID] = true
				}
			}
			nodes, err := a.ws.FetchOrderFeed(sess, identity, ws.WSPending)
			if err != nil {
				if notAuthorized(err) {
					return map[string]any{"ok": false, "skipped": "refused"}
				}
				failed++
				a.logf("bagholder orders: pending-order feed failed: %s\n", errText(err))
			} else {
				for _, node := range nodes {
					if known[py.S(node["id"])] || known[py.S(node["orderId"])] {
						continue
					}
					a.st.InsertOrder(a.feedOrderRow(node))
					added++
				}
			}
		}
	}
	if onlyID == "" {
		a.ordersMu.Lock()
		a.ordersRefreshedAt = nowStamp()
		a.ordersMu.Unlock()
	}
	if read > 0 || added > 0 || failed > 0 {
		a.logf("bagholder orders: %d read, %d found pending at Wealthsimple, %d failed\n", read, added, failed)
	}
	return map[string]any{"ok": failed == 0, "read": read, "added": added, "failed": failed}
}

func (a *App) anyLiveOrders() bool {
	return a.st.LiveOrderCount(LiveStatuses, nil) > 0
}

func (a *App) ordersLoop() {
	for !a.wait(OrdersRefreshSec * time.Second) {
		if !a.connectedIdle() {
			continue
		}
		if !a.anyLiveOrders() && a.ordersRefreshedAtValue() != "" {
			if (time.Now().Unix()/OrdersRefreshSec)%10 != 0 {
				continue
			}
		}
		a.refreshOrders("")
	}
}

func (a *App) cancelOrder(orderID string) map[string]any {
	row := a.st.GetOrder(orderID)
	if row == nil {
		return map[string]any{"ok": false, "error": "No such order."}
	}
	if !slices.Contains(LiveStatuses, row.Status) {
		return map[string]any{"ok": false, "error": "That order is not open."}
	}
	if !a.cfg.OrdersLive {
		return map[string]any{"ok": false, "error": "Orders are off (BAGHOLDER_DRY_ORDERS): nothing is sent to Wealthsimple."}
	}
	sess := a.ticketSession()
	if sess == nil {
		return map[string]any{"ok": false, "error": "Not connected."}
	}
	data, err := a.ws.GraphQL(sess, "SoOrdersOrderCancel", map[string]any{"cancelOrderRequest": map[string]any{"externalId": row.ID}}, "")
	if err != nil {
		if notAuthorized(err) {
			return map[string]any{"ok": false, "error": "Wealthsimple refused the session. Connect Wealthsimple again."}
		}
		msg := errText(err)
		a.logf("bagholder orders: cancel %s failed: %s\n", row.ID, msg)
		return map[string]any{"ok": false, "error": "Cancel failed: " + msg}
	}
	result, _ := data["orderServiceCancelOrder"].(map[string]any)
	if errs, _ := result["errors"].([]any); len(errs) > 0 {
		msg := graphqlErrors(result)
		a.logf("bagholder orders: cancel %s refused: %s\n", row.ID, msg)
		return map[string]any{"ok": false, "error": "Wealthsimple refused the cancel: " + msg}
	}
	a.st.UpdateOrder(row.ID, store.OrderPatch{"status": "cancelling", "wsStatus": "CANCEL_PENDING"})
	a.logf("bagholder orders: cancel %s accepted\n", row.ID)
	go a.refreshOrders(row.ID)
	return map[string]any{"ok": true, "id": row.ID, "status": "cancelling"}
}

func (a *App) ordersPayload(kick bool) map[string]any {
	if kick {
		a.kickOrdersRefresh()
	}
	exchanges := map[string]string{}
	for _, s := range a.st.ListSecurities() {
		exchanges[s.ID] = s.PrimaryExchange
	}
	orders := a.st.ListOrders(0)
	for i := range orders {
		orders[i].Exchange = exchanges[orders[i].SecurityID]
	}
	return map[string]any{"ok": true, "orders": orders, "brackets": a.st.ListBrackets(nil), "live": a.cfg.OrdersLive, "refreshedAt": a.ordersRefreshedAtValue()}
}

func (a *App) modifyOrder(orderID string, quantity, limitPrice any) map[string]any {
	row := a.st.GetOrder(orderID)
	if row == nil {
		return map[string]any{"ok": false, "error": "No such order."}
	}
	if row.Status != "sent" && row.Status != "pending" {
		return map[string]any{"ok": false, "error": "That order is not open."}
	}
	if row.Type == "STOP" {
		return map[string]any{"ok": false, "error": "A stop order cannot be changed; cancel it and place another."}
	}
	q := numPtr(quantity)
	lp := orderTick(numPtr(limitPrice))
	if q != nil && *q <= 0 {
		return map[string]any{"ok": false, "error": "Shares must be more than zero."}
	}
	if lp != nil && *lp <= 0 {
		return map[string]any{"ok": false, "error": "A limit price must be more than zero."}
	}
	limitType := row.Type == "LIMIT" || row.Type == "STOP_LIMIT"
	if limitType && lp == nil && q == nil {
		return map[string]any{"ok": false, "error": "Nothing to change."}
	}
	inp := map[string]any{"externalId": row.ID}
	if lp != nil && limitType && (row.LimitPrice == nil || *lp != *row.LimitPrice) {
		inp["newLimitPrice"] = *lp
	}
	if q != nil && *q != row.Quantity {
		inp["newQuantity"] = *q
	}
	if len(inp) == 1 {
		return map[string]any{"ok": true, "id": row.ID, "unchanged": true}
	}
	if !a.cfg.OrdersLive {
		return map[string]any{"ok": false, "error": "Orders are off (BAGHOLDER_DRY_ORDERS): nothing is sent to Wealthsimple."}
	}
	sess := a.ticketSession()
	if sess == nil {
		return map[string]any{"ok": false, "error": "Not connected."}
	}
	data, err := a.ws.GraphQL(sess, "SoOrdersOrderModify", map[string]any{"input": inp}, "")
	if err != nil {
		if notAuthorized(err) {
			return map[string]any{"ok": false, "error": "Wealthsimple refused the session. Connect Wealthsimple again."}
		}
		msg := errText(err)
		a.logf("bagholder orders: modify %s failed: %s\n", row.ID, msg)
		return map[string]any{"ok": false, "error": "Change failed: " + msg}
	}
	result, _ := data["soOrdersModifyOrder"].(map[string]any)
	if errs, _ := result["errors"].([]any); len(errs) > 0 {
		msg := graphqlErrors(result)
		a.logf("bagholder orders: modify %s refused: %s\n", row.ID, msg)
		return map[string]any{"ok": false, "error": "Wealthsimple refused the change: " + msg}
	}
	patch := store.OrderPatch{}
	if _, ok := inp["newLimitPrice"]; ok {
		patch["limitPrice"] = *lp
	}
	if _, ok := inp["newQuantity"]; ok {
		patch["quantity"] = *q
	}
	a.st.UpdateOrder(row.ID, patch)
	if b := a.st.BracketForOrder(row.ID); b != nil && b.Status == "waiting" {
		if _, ok := inp["newQuantity"]; ok {
			a.st.UpdateBracket(b.ID, store.BracketPatch{"quantity": *q})
		}
	}
	shown := map[string]any{}
	for k, v := range inp {
		if k != "externalId" {
			shown[k] = v
		}
	}
	a.logf("bagholder orders: modify %s accepted: %s\n", row.ID, sortedJSON(shown))
	go a.refreshOrders(row.ID)
	return map[string]any{"ok": true, "id": row.ID}
}
