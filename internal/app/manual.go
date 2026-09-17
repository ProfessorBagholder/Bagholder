package app

import (
	"math"
	"strings"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
	"github.com/ProfessorBagholder/Bagholder/internal/ws"
)

func upper(v any) string { return strings.ToUpper(py.Strip(py.S(v))) }

func pick(body map[string]any, keys ...string) any {
	for _, k := range keys {
		if v, ok := body[k]; ok && v != nil {
			return v
		}
	}
	return nil
}

func firstS(body map[string]any, keys ...string) string {
	for _, k := range keys {
		if v := py.S(body[k]); v != "" {
			return v
		}
	}
	return ""
}

func manualFromFields(body map[string]any) store.Activity {
	side := upper(pick(body, "side"))
	if side == "" {
		side = "BUY"
	}
	if side != "BUY" && side != "SELL" {
		side = "BUY"
	}
	qty := math.Abs(py.Num(pick(body, "qty", "quantity"), 0))
	px := math.Abs(py.Num(pick(body, "price", "unitPrice"), 0))
	date := ws.DateOnly(firstS(body, "date", "transactionDate", "occurredAt"))
	if date == "" {
		date = today()
	}
	symbol := upper(body["symbol"])
	currency := upper(body["currency"])
	if currency == "" {
		currency = "CAD"
	}
	if currency != "CAD" && currency != "USD" {
		currency = "CAD"
	}
	accountID := firstS(body, "accountId", "account")
	if accountID == "" {
		accountID = "manual"
	}
	signedQty := qty
	cash := -(qty * px)
	direction := "DEBIT"
	desc := "Buy"
	if side == "SELL" {
		signedQty = -qty
		cash = qty * px
		direction = "CREDIT"
		desc = "Sell"
	}
	if symbol != "" {
		desc += " " + py.G(qty) + " " + symbol + " @ " + py.G(px)
	}
	accountType := py.S(body["accountType"])
	if accountType == "" && accountID == "manual" {
		accountType = "Manual"
	}
	return store.Activity{ID: py.UUID4(), OccurredAt: date, TransactionDate: date, SettlementDate: date, AccountID: accountID, BookID: accountID, AccountType: accountType,
		ActivityType: "Trade", ActivitySubType: side, Description: desc, Direction: direction, Symbol: symbol, Name: symbol, Currency: currency, Quantity: signedQty,
		UnitPrice: px, Commission: math.Abs(py.Num(body["commission"], 0)), NetCashAmount: cash, Category: "trade", Source: "manual"}
}

func activityFromMap(m map[string]any) store.Activity {
	a := store.Activity{ID: py.S(m["id"]), CanonicalID: firstS(m, "canonicalId", "canonical_id"), OccurredAt: py.S(m["occurredAt"]), TransactionDate: py.S(m["transactionDate"]),
		SettlementDate: py.S(m["settlementDate"]), AccountID: py.S(m["accountId"]), BookID: py.S(m["bookId"]), FifoID: py.S(m["fifoId"]), AccountType: py.S(m["accountType"]),
		ActivityType: py.S(m["activityType"]), ActivitySubType: py.S(m["activitySubType"]), Description: py.S(m["description"]), Direction: py.S(m["direction"]),
		Symbol: py.S(m["symbol"]), Name: py.S(m["name"]), Currency: py.S(m["currency"]), Quantity: py.Num(m["quantity"], 0), UnitPrice: py.Num(m["unitPrice"], 0),
		Commission: py.Num(m["commission"], 0), NetCashAmount: py.Num(m["netCashAmount"], 0), Category: py.S(m["category"]), Source: py.S(m["source"]),
		RawType: py.S(m["rawType"]), AftType: py.S(m["aftType"]), CounterSymbol: py.S(m["counterSymbol"]), SecurityID: py.S(m["securityId"])}
	if v, ok := py.NumOK(m["balance"]); ok {
		a.Balance = &v
	}
	return a
}

func normalizeLocalRow(act store.Activity) store.Activity {
	source := act.Source
	if source == "" {
		source = "manual"
	}
	if source == "wealthsimple" {
		if cid := store.CanonicalFromRow(&act, "wealthsimple"); cid != "" {
			act.CanonicalID = cid
			act.Source = "wealthsimple"
			return act
		}
		source = "manual"
	}
	act.Source = source
	act.CanonicalID = ""
	if act.AccountID == "" {
		act.AccountID = "manual"
	}
	if act.BookID == "" {
		act.BookID = act.AccountID
	}
	if act.ID == "" || store.LooksLikeHomemadeID(act.ID) {
		act.ID = py.UUID4()
	}
	if act.OccurredAt == "" {
		act.OccurredAt = act.TransactionDate
	}
	return act
}

func (a *App) appendManual(body map[string]any) map[string]any {
	if body == nil {
		body = map[string]any{}
	}
	var rows []store.Activity
	if list, ok := body["activities"].([]any); ok {
		for _, r := range list {
			if m, ok := r.(map[string]any); ok {
				rows = append(rows, activityFromMap(m))
			}
		}
	} else if m, ok := body["activity"].(map[string]any); ok && len(m) > 0 {
		rows = append(rows, activityFromMap(m))
	} else {
		rows = append(rows, manualFromFields(body))
	}
	for i := range rows {
		rows[i] = normalizeLocalRow(rows[i])
	}
	result := a.st.MergeLocalRows(rows)
	snap := a.st.Snapshot(false)
	if snap.SyncedAt == "" {
		a.st.SetMeta("synced_at", nowStamp())
		snap = a.st.Snapshot(false)
	}
	a.mu.Lock()
	if snap.SyncedAt != "" {
		a.state.lastSync = snap.SyncedAt
	}
	a.mu.Unlock()
	saved := result.Activities
	out := map[string]any{"ok": true, "added": result.Added, "duplicates": result.Duplicates}
	switch {
	case len(saved) == 1:
		out["activity"] = saved[0]
	case len(saved) > 0:
		out["activities"] = saved
	case len(rows) == 1:
		out["activity"] = rows[0]
	}
	return out
}
