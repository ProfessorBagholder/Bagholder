package app

import (
	"crypto/md5"
	"encoding/hex"
	"math"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
	"github.com/ProfessorBagholder/Bagholder/internal/ws"
)

const (
	BracketPollSec     = 5
	TrailMinMove       = 0.005
	TargetBackOff      = 0.01
	BracketRollSec     = 7 * 86400
	BracketRollLastSec = 2 * 86400
	GTCDays            = 90
	BracketTif         = "UNTIL_CANCEL"
)

var (
	BracketRetrySec      = []int{60, 300, 900, 3600}
	BracketLive          = []string{"waiting", "armed", "firing", "target_placed", "stopping", "closing"}
	BracketResting       = []string{"sent", "pending"}
	BracketInflight      = []string{"sent", "pending", "cancelling"}
	BracketEndedQuietly  = []string{"stopped", "target", "cancelled by the user", "both legs removed", "sold from the ticket"}
	watchStatuses        = []string{"armed", "firing", "target_placed", "stopping"}
)

func fstr(p *float64) string {
	if p == nil {
		return "None"
	}
	return py.Repr(*p)
}

func (a *App) releaseShares(b store.Bracket, sold float64) {
	remaining := py.Round(py.Deref(b.Quantity, 0)-sold, 6)
	for _, oid := range []string{b.SlOrderID, b.TpOrderID} {
		if err := a.cancelExit(oid); err != "" {
			a.logf("bagholder bracket: %s for %s: cancel of %s refused: %s\n", b.ID, b.Symbol, oid, err)
		}
	}
	a.st.UpdateBracket(b.ID, store.BracketPatch{"quantity": remaining, "slOrderId": "", "tpOrderId": "", "status": "armed", "error": "", "attempts": 0})
	a.logf("bagholder bracket: %s for %s: %s of its shares sold from the ticket; the stop is placed again on the %s left\n", b.ID, b.Symbol, qtyText(sold), qtyText(remaining))
}

func (a *App) awaitCancels(b store.Bracket, seconds int) {
	if !a.cfg.OrdersLive {
		return
	}
	for i := 0; i < seconds; i++ {
		var open []store.Order
		for _, o := range a.ownExitRows(b) {
			if py.Contains(BracketInflight, o.Status) {
				open = append(open, o)
			}
		}
		if len(open) == 0 {
			return
		}
		for _, o := range open {
			a.refreshOrders(o.ID)
		}
		time.Sleep(time.Second)
	}
}

func (a *App) createBracket(row *store.Order) *store.Bracket {
	b := store.Bracket{ID: "bracket-" + py.UUID4(), OrderID: row.ID, AccountID: row.AccountID, SecurityID: row.SecurityID, Symbol: row.Symbol, Currency: row.Currency, Tif: BracketTif, SlTrailUnit: "pct", Status: "waiting"}
	q := row.Quantity
	b.Quantity = &q
	if sl := row.StopLoss; sl != nil {
		b.SlKind = sl.Kind
		b.SlPrice = sl.Price
		b.SlTrail = sl.Trail
		if sl.TrailUnit != "" {
			b.SlTrailUnit = sl.TrailUnit
		}
	}
	if tp := row.TakeProfit; tp != nil {
		b.TpPrice = tp.Price
	}
	a.st.InsertBracket(b)
	return a.st.GetBracket(b.ID)
}

func (a *App) sayOnce(key, line string) {
	a.bracketSaidMu.Lock()
	defer a.bracketSaidMu.Unlock()
	if a.bracketSaid[key] {
		return
	}
	a.bracketSaid[key] = true
	a.logf("%s\n", line)
}

func trailDistance(b store.Bracket, price float64) *float64 {
	if b.SlKind != "trail" || b.SlTrail == nil || *b.SlTrail == 0 {
		return nil
	}
	var d float64
	if b.SlTrailUnit == "pct" {
		d = price * *b.SlTrail / 100.0
	} else {
		d = *b.SlTrail
	}
	return &d
}

func (a *App) exitBody(b store.Bracket, execType string, price *float64, role string) (*store.Order, map[string]any, string) {
	body := map[string]any{"symbol": b.Symbol, "securityId": b.SecurityID, "accountId": b.AccountID, "side": "SELL", "type": execType, "tif": BracketTif, "quantity": py.Deref(b.Quantity, 0), "currency": b.Currency}
	if b.Quantity == nil {
		body["quantity"] = nil
	}
	if execType == "LIMIT" {
		body["limitPrice"] = anyFloat(price)
	}
	if execType == "STOP" {
		body["stopPrice"] = anyFloat(price)
	}
	row, req, err := a.orderRequest(body)
	if err != "" {
		return nil, nil, err
	}
	row.Role, row.ParentID = role, b.OrderID
	return row, req, ""
}

func anyFloat(p *float64) any {
	if p == nil {
		return nil
	}
	return *p
}

func (a *App) placeExit(b store.Bracket, execType string, price *float64, role string) (string, string) {
	row, req, err := a.exitBody(b, execType, price, role)
	if err != "" {
		return "", err
	}
	if !a.cfg.OrdersLive {
		rounded := "None"
		if price != nil {
			rounded = py.Repr(py.Round(*price, 4))
		}
		a.sayOnce(b.ID+"|"+role+"|"+rounded, "bagholder bracket (orders are off, not placed): "+role+" "+execType+" for "+b.Symbol+": "+sortedJSON(req))
		return "", ""
	}
	r := a.submitOrder(row, req)
	if !truthy(r["ok"]) {
		msg := py.S(r["error"])
		if msg == "" {
			msg = "not sent"
		}
		return "", msg
	}
	return py.S(r["id"]), ""
}

func (a *App) cancelExit(orderID string) string {
	if orderID == "" {
		return ""
	}
	row := a.st.GetOrder(orderID)
	if row == nil || (row.Status != "sent" && row.Status != "pending") {
		return ""
	}
	r := a.cancelOrder(orderID)
	if truthy(r["ok"]) || strings.Contains(py.S(r["error"]), "not open") {
		return ""
	}
	if msg := py.S(r["error"]); msg != "" {
		return msg
	}
	return "cancel failed"
}

func (a *App) exitRow(b store.Bracket, role string) *store.Order {
	held := b.TpOrderID
	if role == "stop" {
		held = b.SlOrderID
	}
	if held != "" {
		if row := a.st.GetOrder(held); row != nil {
			return row
		}
	}
	for _, o := range a.st.ListOrders(0) {
		if o.ParentID == b.OrderID && o.Role == role {
			c := o
			return &c
		}
	}
	return nil
}

func retryWait(attempts int) int {
	i := attempts
	if i > len(BracketRetrySec) {
		i = len(BracketRetrySec)
	}
	return BracketRetrySec[i-1]
}

func (a *App) mayRetry(b store.Bracket) bool {
	if b.Attempts == 0 {
		return true
	}
	wait := retryWait(b.Attempts)
	t, err := time.Parse("2006-01-02T15:04:05Z", b.UpdatedAt)
	if err != nil {
		return true
	}
	return time.Since(t).Seconds() >= float64(wait)
}

func (a *App) fail(b store.Bracket, msg string) {
	attempts := b.Attempts + 1
	a.st.UpdateBracket(b.ID, store.BracketPatch{"error": msg, "attempts": attempts})
	a.logf("bagholder bracket: %s for %s: %s (attempt %d; next in %d s)\n", b.ID, b.Symbol, msg, attempts, retryWait(attempts))
	if attempts == 1 {
		acct := ""
		if entry := a.st.GetOrder(b.OrderID); entry != nil {
			acct = entry.Account
		}
		sum := md5.Sum([]byte(msg))
		tail := ""
		if acct != "" {
			tail = " · " + acct
		}
		a.notify.Emit("problems", "bracket:"+b.ID+":fail:"+hex.EncodeToString(sum[:])[:8], "Bracket · "+b.Symbol, py.Capitalize(msg)+" · trying again in a minute"+tail, nil)
	}
}

func (a *App) armStep(b store.Bracket, entry *store.Order) {
	if b.Status == "waiting" {
		if entry == nil {
			a.st.UpdateBracket(b.ID, store.BracketPatch{"status": "cancelled", "outcome": "entry not found"})
			return
		}
		if py.Contains([]string{"pending", "sent", "cancelling", "dry"}, entry.Status) {
			return
		}
		filled := py.Deref(entry.FilledQty, 0)
		if entry.Status == "filled" && filled == 0 {
			filled = entry.Quantity
		}
		if filled <= 0 {
			a.st.UpdateBracket(b.ID, store.BracketPatch{"status": "cancelled", "outcome": "entry " + entry.Status})
			a.logf("bagholder bracket: %s for %s off: entry %s without a fill\n", b.ID, b.Symbol, entry.Status)
			return
		}
		armedAt := nowStamp()
		b.Quantity, b.Status, b.ArmedAt = &filled, "armed", armedAt
		patch := store.BracketPatch{"quantity": filled, "status": "armed", "armedAt": armedAt}
		if b.SlKind == "trail" {
			high := nonZero(entry.AvgFill)
			if high == nil {
				high = nonZero(entry.LimitPrice)
			}
			if high == nil {
				high = nonZero(b.SlPrice)
			}
			if high != nil {
				h := *high
				patch["highWater"] = h
				sl := py.Round(h-py.Deref(trailDistance(b, h), 0), 2)
				patch["slPrice"] = sl
				b.HighWater, b.SlPrice = &h, &sl
			}
		}
		a.st.UpdateBracket(b.ID, patch)
		a.logf("bagholder bracket: %s armed for %s x %s\n", b.ID, qtyText(filled), b.Symbol)
	}
	if b.Status != "armed" || b.SlKind == "" || b.SlOrderID != "" {
		return
	}
	if !a.nothingResting(b) {
		return
	}
	if !a.stopAllowedFor(b.SecurityID) {
		if b.SlMode != "watched" {
			a.st.UpdateBracket(b.ID, store.BracketPatch{"slMode": "watched", "slNative": false})
			a.logf("bagholder bracket: %s for %s: Wealthsimple takes no stop order for it; the stop is watched here\n", b.ID, b.Symbol)
		}
		return
	}
	if !a.mayRetry(b) {
		return
	}
	oid, err := a.placeExit(b, "STOP", b.SlPrice, "stop")
	if err != "" {
		a.fail(b, "stop not placed: "+err)
	} else if oid != "" {
		a.st.UpdateBracket(b.ID, store.BracketPatch{"slOrderId": oid, "slNative": true, "slMode": "native", "error": "", "attempts": 0, "movedAt": nowStamp()})
		a.logf("bagholder bracket: %s stop placed at %s for %s\n", b.ID, fstr(b.SlPrice), b.Symbol)
	}
}

func (a *App) stopAllowedFor(securityID string) bool {
	a.stopAllowedMu.Lock()
	if ok, cached := a.stopAllowed[securityID]; cached {
		a.stopAllowedMu.Unlock()
		return ok
	}
	a.stopAllowedMu.Unlock()
	sess := a.ticketSession()
	if sess == nil {
		return false
	}
	data, err := a.ws.GraphQL(sess, "FetchSecurityMarketData", map[string]any{"id": securityID}, "")
	if err != nil {
		a.logf("bagholder bracket: order types for %s unknown: %s\n", securityID, errText(err))
		return false
	}
	ok := py.Contains(ws.ParseMarketData(data).OrderTypes, "STOP")
	a.stopAllowedMu.Lock()
	a.stopAllowed[securityID] = ok
	a.stopAllowedMu.Unlock()
	return ok
}

func (a *App) ownExitRows(b store.Bracket) []store.Order {
	var out []store.Order
	for _, o := range a.st.ListOrders(0) {
		if o.ParentID == b.OrderID && (o.Role == "stop" || o.Role == "target") {
			out = append(out, o)
		}
	}
	return out
}

func (a *App) endBracket(b store.Bracket, outcome, note string) string {
	pending := false
	for _, o := range a.ownExitRows(b) {
		if py.Contains(BracketResting, o.Status) {
			if err := a.cancelExit(o.ID); err != "" {
				a.logf("bagholder bracket: %s for %s: cancel of %s refused: %s; tried again on the next check\n", b.ID, b.Symbol, o.ID, err)
			}
			pending = true
		} else if o.Status == "cancelling" {
			pending = true
		}
	}
	status := "done"
	if pending {
		status = "closing"
	}
	a.st.UpdateBracket(b.ID, store.BracketPatch{"status": status, "outcome": outcome, "error": note, "slOrderId": "", "tpOrderId": ""})
	tail := ""
	if pending {
		tail = "; its resting exit is being cancelled"
	}
	a.logf("bagholder bracket: %s for %s: %s%s\n", b.ID, b.Symbol, outcome, tail)
	if !py.Contains(BracketEndedQuietly, outcome) {
		acct := ""
		if entry := a.st.GetOrder(b.OrderID); entry != nil {
			acct = entry.Account
		}
		body := py.Capitalize(outcome)
		if acct != "" {
			body += " · " + acct
		}
		a.notify.Emit("problems", "bracket:"+b.ID+":off", "Bracket off · "+b.Symbol, body, nil)
	}
	return status
}

func (a *App) closingStep(b store.Bracket) {
	var open []store.Order
	for _, o := range a.ownExitRows(b) {
		if py.Contains(BracketInflight, o.Status) {
			open = append(open, o)
		}
	}
	for _, o := range open {
		if py.Contains(BracketResting, o.Status) {
			if err := a.cancelExit(o.ID); err != "" {
				a.logf("bagholder bracket: %s for %s: cancel of %s refused again: %s\n", b.ID, b.Symbol, o.ID, err)
			}
		}
	}
	if len(open) == 0 {
		a.st.UpdateBracket(b.ID, store.BracketPatch{"status": "done"})
		a.logf("bagholder bracket: %s for %s: nothing rests at Wealthsimple; done\n", b.ID, b.Symbol)
	}
}

func (a *App) sweepExits() {
	for _, o := range a.st.ListOrders(0) {
		if (o.Role != "stop" && o.Role != "target") || !py.Contains(BracketResting, o.Status) {
			continue
		}
		b := a.st.BracketForOrder(o.ParentID)
		heldBy := b != nil && py.Contains(BracketLive, b.Status) && (b.Status == "closing" || o.ID == b.SlOrderID || o.ID == b.TpOrderID)
		if heldBy {
			continue
		}
		err := a.cancelExit(o.ID)
		tail := ""
		if err != "" {
			tail = " (refused: " + err + ")"
		}
		a.sayOnce(o.ID+"|orphan", "bagholder bracket: "+o.ID+" for "+o.Symbol+" rests at Wealthsimple with no bracket holding it; cancelled"+tail+"\n")
	}
}

func (a *App) nothingResting(b store.Bracket) bool {
	for _, o := range a.ownExitRows(b) {
		if py.Contains(BracketInflight, o.Status) {
			return false
		}
	}
	return true
}

func (a *App) closedElsewhere(b store.Bracket) string {
	if !py.Contains(watchStatuses, b.Status) || b.ArmedAt == "" || !a.nothingResting(b) {
		return ""
	}
	sold := a.st.SoldSince(b.AccountID, b.SecurityID, b.ArmedAt, b.Symbol)
	if sold != 0 && sold >= py.Deref(b.Quantity, 0) {
		return "sold: " + qtyText(sold) + " shares in the activity feed"
	}
	readAt := a.st.GetMeta("balances_read_at")
	if readAt == "" || readAt <= b.ArmedAt {
		return ""
	}
	held := a.st.PositionQuantity(b.AccountID, b.SecurityID)
	if held != nil && *held > 0 {
		if !b.SeenHeld || b.MissedAt != "" {
			a.st.UpdateBracket(b.ID, store.BracketPatch{"seenHeld": true, "missedAt": ""})
		}
		return ""
	}
	if !b.SeenHeld {
		return ""
	}
	missed := b.MissedAt
	if missed == "" {
		a.st.UpdateBracket(b.ID, store.BracketPatch{"missedAt": readAt})
		a.logf("bagholder bracket: %s for %s: the balances read at %s does not list the position; a second read decides\n", b.ID, b.Symbol, readAt)
		return ""
	}
	if readAt > missed {
		return "position gone: two balance reads without it (" + missed + ", " + readAt + ")"
	}
	return ""
}

func parseUTC(text string) (time.Time, bool) {
	text = py.Strip(text)
	if text == "" {
		return time.Time{}, false
	}
	if len(text) > 19 {
		text = text[:19]
	}
	t, err := time.Parse("2006-01-02T15:04:05", text)
	if err != nil {
		return time.Time{}, false
	}
	return t, true
}

func expiresIn(row *store.Order, now time.Time) *float64 {
	exp, ok := parseUTC(row.ExpiresAt)
	if !ok && strings.ToUpper(row.Tif) == "UNTIL_CANCEL" {
		text := row.SubmittedAt
		if text == "" {
			text = row.CreatedAt
		}
		if sub, ok2 := parseUTC(text); ok2 {
			exp, ok = sub.Add(GTCDays*24*time.Hour), true
		}
	}
	if !ok {
		return nil
	}
	d := exp.Sub(now).Seconds()
	return &d
}

func rollDue(row *store.Order, quote *ws.Quote, now time.Time) bool {
	if row == nil || (row.Status != "sent" && row.Status != "pending") {
		return false
	}
	left := expiresIn(row, now)
	if left == nil || *left > BracketRollSec {
		return false
	}
	if *left <= BracketRollLastSec {
		return true
	}
	status := ""
	if quote != nil {
		status = strings.ToUpper(quote.MarketStatus)
	}
	return status != "OPEN"
}

func (a *App) rollStep(b store.Bracket, quote *ws.Quote) {
	now := time.Now().UTC()
	nowS := py.Stamp(now)
	if b.Status == "armed" && b.SlMode == "native" && b.SlOrderID != "" {
		row := a.st.GetOrder(b.SlOrderID)
		if rollDue(row, quote, now) {
			if err := a.cancelExit(b.SlOrderID); err != "" {
				a.fail(b, "stop not rolled: "+err)
				return
			}
			a.st.UpdateBracket(b.ID, store.BracketPatch{"slOrderId": "", "movedAt": nowS, "error": ""})
			a.logf("bagholder bracket: %s for %s: stop at %s nears Wealthsimple's ninety days; cancelled, placed again at the same level\n", b.ID, b.Symbol, fstr(b.SlPrice))
		}
	} else if b.Status == "target_placed" {
		if b.TpOrderID != "" {
			row := a.st.GetOrder(b.TpOrderID)
			if rollDue(row, quote, now) {
				if err := a.cancelExit(b.TpOrderID); err != "" {
					a.fail(b, "target not rolled: "+err)
					return
				}
				a.st.UpdateBracket(b.ID, store.BracketPatch{"tpOrderId": "", "movedAt": nowS, "error": ""})
				a.logf("bagholder bracket: %s for %s: target at %s nears Wealthsimple's ninety days; cancelled, placed again\n", b.ID, b.Symbol, fstr(b.TpPrice))
			}
		} else {
			tpRow := a.exitRow(b, "target")
			if tpRow != nil && (tpRow.Status == "cancelled" || tpRow.Status == "expired") {
				a.fireTarget(b)
			}
		}
	}
}

func priceMoved(a, b *float64) bool {
	return a != nil && *a != 0 && b != nil && *b != 0 && math.Abs(*a-*b) > 0.005
}

func (a *App) reconcileStep(b store.Bracket, entry *store.Order) string {
	if b.Status == "closing" {
		a.closingStep(b)
		return "done"
	}
	stopRow, tpRow := a.exitRow(b, "stop"), a.exitRow(b, "target")
	if stopRow != nil && stopRow.Status == "filled" {
		a.endBracket(b, "stopped", "")
		return "done"
	}
	if tpRow != nil && tpRow.Status == "filled" {
		a.endBracket(b, "target", "")
		return "done"
	}
	if b.SlOrderID != "" && stopRow != nil && py.Contains(BracketResting, stopRow.Status) && priceMoved(stopRow.StopPrice, b.SlPrice) {
		a.st.UpdateBracket(b.ID, store.BracketPatch{"slPrice": *stopRow.StopPrice})
		a.logf("bagholder bracket: %s for %s: stop moved by hand to %s; the bracket follows\n", b.ID, b.Symbol, fstr(stopRow.StopPrice))
		v := *stopRow.StopPrice
		b.SlPrice = &v
	}
	if b.TpOrderID != "" && tpRow != nil && py.Contains(BracketResting, tpRow.Status) && priceMoved(tpRow.LimitPrice, b.TpPrice) {
		a.st.UpdateBracket(b.ID, store.BracketPatch{"tpPrice": *tpRow.LimitPrice})
		a.logf("bagholder bracket: %s for %s: target moved by hand to %s; the bracket follows\n", b.ID, b.Symbol, fstr(tpRow.LimitPrice))
	}
	nativeStop := b.Status == "armed" && b.SlMode == "native" && b.SlOrderID != "" && stopRow != nil
	if nativeStop && stopRow.Status == "expired" {
		a.st.UpdateBracket(b.ID, store.BracketPatch{"slOrderId": "", "error": ""})
		a.logf("bagholder bracket: %s for %s: stop expired at Wealthsimple; placed again\n", b.ID, b.Symbol)
	} else if nativeStop && py.Contains([]string{"cancelled", "rejected", "failed"}, stopRow.Status) {
		why := "stop cancelled at Wealthsimple by hand"
		if stopRow.Status != "cancelled" {
			why = "stop " + stopRow.Status + " at Wealthsimple"
			if stopRow.Error != "" {
				why += ": " + stopRow.Error
			}
		}
		a.endBracket(b, why, "")
		return "done"
	}
	placedTarget := b.Status == "target_placed" && b.TpOrderID != "" && tpRow != nil
	if placedTarget && tpRow.Status == "expired" {
		a.st.UpdateBracket(b.ID, store.BracketPatch{"tpOrderId": "", "error": ""})
		a.logf("bagholder bracket: %s for %s: target expired at Wealthsimple; placed again\n", b.ID, b.Symbol)
	} else if placedTarget && py.Contains([]string{"cancelled", "rejected", "failed"}, tpRow.Status) {
		why := "target cancelled at Wealthsimple by hand"
		if tpRow.Status != "cancelled" {
			why = "target " + tpRow.Status + " at Wealthsimple"
			if tpRow.Error != "" {
				why += ": " + tpRow.Error
			}
		}
		a.endBracket(b, why, "")
		return "done"
	}
	if why := a.closedElsewhere(b); why != "" {
		a.endBracket(b, why, "")
		return "done"
	}
	return ""
}

func (a *App) watchStep(b store.Bracket, quote *ws.Quote) {
	if quote == nil || strings.ToUpper(quote.MarketStatus) != "OPEN" {
		return
	}
	if quote.Last == nil {
		return
	}
	last := *quote.Last
	nowS := nowStamp()
	trigger := last
	if quote.Bid != nil {
		trigger = *quote.Bid
	}
	tp := py.Deref(b.TpPrice, 0)
	atTarget := tp != 0 && trigger >= tp
	if b.SlKind == "trail" && (b.Status == "target_placed" || (b.Status == "armed" && !atTarget)) {
		high := math.Max(py.Deref(b.HighWater, 0), last)
		if b.HighWater == nil || high != *b.HighWater {
			a.st.UpdateBracket(b.ID, store.BracketPatch{"highWater": high})
		}
		newStop := py.Round(high-py.Deref(trailDistance(b, high), 0), 2)
		cur := py.Deref(b.SlPrice, 0)
		if newStop > cur+math.Max(0.01, cur*TrailMinMove) {
			if b.SlOrderID != "" {
				if err := a.cancelExit(b.SlOrderID); err != "" {
					a.fail(b, "stop not moved: "+err)
					return
				}
			}
			a.st.UpdateBracket(b.ID, store.BracketPatch{"slPrice": newStop, "slOrderId": "", "movedAt": nowS})
			a.logf("bagholder bracket: %s for %s: stop moves to %s (high %s)\n", b.ID, b.Symbol, py.Repr(newStop), py.Repr(high))
			ns := newStop
			b.SlPrice, b.SlOrderID = &ns, ""
		}
	}
	sl := py.Deref(b.SlPrice, 0)
	if b.Status == "armed" && b.SlKind != "" && b.SlMode == "watched" && b.SlOrderID == "" && sl != 0 {
		if trigger <= sl {
			if !a.mayRetry(b) {
				return
			}
			oid, err := a.placeExit(b, "MARKET", b.SlPrice, "stop")
			if err != "" {
				a.fail(b, "stop not placed: "+err)
			} else if oid != "" {
				a.st.UpdateBracket(b.ID, store.BracketPatch{"slOrderId": oid, "status": "firing", "error": ""})
				a.logf("bagholder bracket: %s for %s: stop hit at %s, market sell placed\n", b.ID, b.Symbol, py.Repr(trigger))
			}
			return
		}
	}
	if b.Status == "armed" && tp != 0 {
		if trigger >= tp {
			if b.SlOrderID != "" {
				if err := a.cancelExit(b.SlOrderID); err != "" {
					a.fail(b, "stop not cancelled for the target: "+err)
					return
				}
				a.st.UpdateBracket(b.ID, store.BracketPatch{"status": "firing", "error": ""})
				a.logf("bagholder bracket: %s for %s: target reached at %s, stop cancel sent\n", b.ID, b.Symbol, py.Repr(trigger))
				return
			}
			a.fireTarget(b)
		}
	}
	if b.Status == "firing" && tp != 0 && b.TpOrderID == "" {
		if stopRow := a.exitRow(b, "stop"); stopRow != nil && stopRow.Status == "cancelled" {
			a.fireTarget(b)
		}
	}
	if b.Status == "target_placed" && b.SlKind != "" && sl != 0 && b.TpOrderID != "" {
		if trigger <= sl {
			if err := a.cancelExit(b.TpOrderID); err != "" {
				a.fail(b, "target not cancelled for the stop: "+err)
				return
			}
			a.st.UpdateBracket(b.ID, store.BracketPatch{"status": "stopping", "tpOrderId": "", "error": "", "attempts": 0})
			a.logf("bagholder bracket: %s for %s: stop level %s reached at %s while the limit sell rested; its cancel sent, market sell follows\n", b.ID, b.Symbol, py.Repr(sl), py.Repr(trigger))
			return
		}
		if tp != 0 && trigger < tp*(1-TargetBackOff) {
			if err := a.cancelExit(b.TpOrderID); err != "" {
				a.fail(b, "target not cancelled for the stop: "+err)
				return
			}
			a.st.UpdateBracket(b.ID, store.BracketPatch{"status": "armed", "tpOrderId": "", "slOrderId": "", "error": "", "attempts": 0})
			a.logf("bagholder bracket: %s for %s: target out of reach at %s; the limit sell's cancel sent, the stop order goes back\n", b.ID, b.Symbol, py.Repr(trigger))
			return
		}
	}
	if b.Status == "stopping" {
		tpRow := a.exitRow(b, "target")
		if tpRow != nil && (tpRow.Status == "cancelled" || tpRow.Status == "expired") {
			if !a.mayRetry(b) || !a.nothingResting(b) {
				return
			}
			oid, err := a.placeExit(b, "MARKET", b.SlPrice, "stop")
			if err != "" {
				a.fail(b, "stop not placed: "+err)
			} else if oid != "" {
				a.st.UpdateBracket(b.ID, store.BracketPatch{"slOrderId": oid, "status": "firing", "error": "", "attempts": 0})
				a.logf("bagholder bracket: %s for %s: market sell placed at the stop\n", b.ID, b.Symbol)
			}
		}
	}
}

func (a *App) fireTarget(b store.Bracket) {
	if !a.mayRetry(b) || !a.nothingResting(b) {
		return
	}
	oid, err := a.placeExit(b, "LIMIT", b.TpPrice, "target")
	if err != "" {
		a.fail(b, "target not placed: "+err)
	} else if oid != "" {
		a.st.UpdateBracket(b.ID, store.BracketPatch{"tpOrderId": oid, "status": "target_placed", "error": "", "attempts": 0})
		a.logf("bagholder bracket: %s for %s: limit sell at %s placed\n", b.ID, b.Symbol, fstr(b.TpPrice))
	}
}

func (a *App) bracketTick(quotes map[string]*ws.Quote) map[string]any {
	a.bracketMu.Lock()
	if a.bracketBusy {
		a.bracketMu.Unlock()
		return map[string]any{"ok": false, "skipped": "running"}
	}
	a.bracketBusy = true
	a.bracketMu.Unlock()
	defer func() {
		a.bracketMu.Lock()
		a.bracketBusy = false
		a.bracketMu.Unlock()
	}()
	live := a.st.ListBrackets(BracketLive)
	if len(live) == 0 {
		a.sweepExits()
		return map[string]any{"ok": true, "brackets": 0}
	}
	if a.cfg.OrdersLive {
		for _, b := range live {
			switch {
			case b.Status == "waiting":
				a.refreshOrders(b.OrderID)
			case b.Status == "firing" && b.SlOrderID != "" && b.TpOrderID == "":
				a.refreshOrders(b.SlOrderID)
			case b.Status == "armed" && b.SlKind != "" && b.SlOrderID == "":
				if prev := a.exitRow(b, "stop"); prev != nil && py.Contains(BracketInflight, prev.Status) {
					a.refreshOrders(prev.ID)
				}
			case b.Status == "target_placed" && b.TpOrderID == "":
				if prev := a.exitRow(b, "target"); prev != nil && py.Contains(BracketInflight, prev.Status) {
					a.refreshOrders(prev.ID)
				}
			case b.Status == "stopping":
				if prev := a.exitRow(b, "target"); prev != nil && py.Contains(BracketInflight, prev.Status) {
					a.refreshOrders(prev.ID)
				}
			case b.Status == "closing":
				for _, o := range a.ownExitRows(b) {
					if o.Status == "cancelling" {
						a.refreshOrders(o.ID)
					}
				}
			}
		}
	}
	orders := map[string]*store.Order{}
	for _, o := range a.st.ListOrders(0) {
		c := o
		orders[o.ID] = &c
	}
	if quotes == nil {
		idSet := map[string]bool{}
		for _, b := range live {
			if py.Contains(watchStatuses, b.Status) {
				idSet[b.SecurityID] = true
			}
		}
		ids := make([]string, 0, len(idSet))
		for id := range idSet {
			ids = append(ids, id)
		}
		sort.Strings(ids)
		quotes = map[string]*ws.Quote{}
		if len(ids) > 0 {
			if sess := a.ticketSession(); sess != nil {
				q, err := a.ws.FetchQuotes(sess, ids)
				if err != nil {
					a.logf("bagholder bracket: quotes failed: %s\n", errText(err))
				} else {
					quotes = q
				}
			}
		}
	}
	for _, b := range live {
		entry := orders[b.OrderID]
		if a.reconcileStep(b, entry) == "done" {
			continue
		}
		cur := a.st.GetBracket(b.ID)
		if cur == nil {
			continue
		}
		a.rollStep(*cur, quotes[cur.SecurityID])
		if cur = a.st.GetBracket(b.ID); cur == nil {
			continue
		}
		a.armStep(*cur, entry)
		if cur = a.st.GetBracket(b.ID); cur == nil {
			continue
		}
		if py.Contains(watchStatuses, cur.Status) {
			a.watchStep(*cur, quotes[cur.SecurityID])
		}
	}
	a.sweepExits()
	return map[string]any{"ok": true, "brackets": len(live)}
}

func (a *App) bracketLoop() {
	for !a.wait(BracketPollSec * time.Second) {
		if !a.connectedIdle() {
			continue
		}
		a.bracketTick(nil)
	}
}

func (a *App) cancelBracket(bracketID string) map[string]any {
	b := a.st.GetBracket(bracketID)
	if b == nil {
		return map[string]any{"ok": false, "error": "No such bracket."}
	}
	if !py.Contains(BracketLive, b.Status) {
		return map[string]any{"ok": false, "error": "That bracket is not live."}
	}
	a.endBracket(*b, "cancelled by the user", "")
	return map[string]any{"ok": true, "id": b.ID}
}

func (a *App) adjustBracket(bracketID, leg string, price, trail any, remove bool) map[string]any {
	b := a.st.GetBracket(bracketID)
	if b == nil {
		return map[string]any{"ok": false, "error": "No such bracket."}
	}
	if !py.Contains(BracketLive, b.Status) {
		return map[string]any{"ok": false, "error": "That bracket is not live."}
	}
	leg = strings.ToLower(leg)
	if leg != "sl" && leg != "tp" {
		return map[string]any{"ok": false, "error": "Which leg?"}
	}
	if remove {
		if leg == "sl" {
			if err := a.cancelExit(b.SlOrderID); err != "" {
				return map[string]any{"ok": false, "error": err}
			}
			patch := store.BracketPatch{"slKind": "", "slOrderId": "", "slMode": "", "error": ""}
			if py.Deref(b.TpPrice, 0) == 0 {
				patch["status"], patch["outcome"] = "cancelled", "both legs removed"
			}
			a.st.UpdateBracket(b.ID, patch)
		} else {
			if err := a.cancelExit(b.TpOrderID); err != "" {
				return map[string]any{"ok": false, "error": err}
			}
			patch := store.BracketPatch{"tpPrice": nil, "tpOrderId": "", "error": ""}
			if b.Status == "target_placed" {
				patch["status"] = "armed"
			}
			if b.SlKind == "" {
				patch["status"], patch["outcome"] = "cancelled", "both legs removed"
			}
			a.st.UpdateBracket(b.ID, patch)
		}
		what := "take profit"
		if leg == "sl" {
			what = "stop loss"
		}
		a.logf("bagholder bracket: %s for %s: %s removed by the user\n", b.ID, b.Symbol, what)
		return map[string]any{"ok": true, "id": b.ID}
	}
	if leg == "sl" {
		if b.SlKind == "" {
			return map[string]any{"ok": false, "error": "This bracket has no stop loss."}
		}
		var patch store.BracketPatch
		if b.SlKind == "trail" {
			t := numPtr(trail)
			if t == nil || *t <= 0 {
				return map[string]any{"ok": false, "error": "A trail is required."}
			}
			high := py.Deref(b.HighWater, 0)
			if high == 0 {
				high = py.Deref(b.SlPrice, 0)
			}
			nb := *b
			tv := *t
			nb.SlTrail = &tv
			patch = store.BracketPatch{"slTrail": tv}
			if high != 0 {
				patch["slPrice"] = py.Round(high-py.Deref(trailDistance(nb, high), 0), 2)
			} else {
				patch["slPrice"] = anyFloat(b.SlPrice)
			}
		} else {
			p := numPtr(price)
			if p == nil || *p <= 0 {
				return map[string]any{"ok": false, "error": "A stop price is required."}
			}
			patch = store.BracketPatch{"slPrice": *p}
		}
		if b.SlOrderID != "" && b.Status == "armed" {
			if err := a.cancelExit(b.SlOrderID); err != "" {
				return map[string]any{"ok": false, "error": err}
			}
			patch["slOrderId"] = ""
			patch["movedAt"] = nowStamp()
		}
		patch["error"] = ""
		a.st.UpdateBracket(b.ID, patch)
		shown := "None"
		if v, ok := py.NumOK(patch["slPrice"]); ok {
			shown = py.Repr(v)
		}
		a.logf("bagholder bracket: %s for %s: stop moved to %s by the user\n", b.ID, b.Symbol, shown)
		return map[string]any{"ok": true, "id": b.ID}
	}
	p := numPtr(price)
	if p == nil || *p <= 0 {
		return map[string]any{"ok": false, "error": "A limit price is required."}
	}
	patch := store.BracketPatch{"tpPrice": *p, "error": ""}
	if b.Status == "target_placed" && b.TpOrderID != "" {
		if err := a.cancelExit(b.TpOrderID); err != "" {
			return map[string]any{"ok": false, "error": err}
		}
		patch["tpOrderId"], patch["status"] = "", "armed"
	}
	a.st.UpdateBracket(b.ID, patch)
	a.logf("bagholder bracket: %s for %s: target moved to %s by the user\n", b.ID, b.Symbol, py.Repr(*p))
	return map[string]any{"ok": true, "id": b.ID}
}

func (a *App) openOrdersCount() int {
	entries := 0
	for _, o := range a.st.ListOrders(0) {
		role := o.Role
		if role == "" {
			role = "entry"
		}
		if py.Contains(LiveStatuses, o.Status) && role == "entry" {
			entries++
		}
	}
	brackets := 0
	for _, b := range a.st.ListBrackets(nil) {
		if py.Contains(BracketLive, b.Status) && b.Status != "waiting" {
			brackets++
		}
	}
	return entries + brackets
}

var _ = strconv.Itoa
