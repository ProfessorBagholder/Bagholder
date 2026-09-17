package store

import (
	"database/sql"
	"encoding/json"
	"strings"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

type StopLoss struct {
	Kind      string   `json:"kind"`
	Price     *float64 `json:"price"`
	Trail     *float64 `json:"trail"`
	TrailUnit string   `json:"trailUnit"`
}

type TakeProfit struct {
	Price *float64 `json:"price"`
}

type Order struct {
	ID            string         `json:"id"`
	CreatedAt     string         `json:"createdAt"`
	AccountID     string         `json:"accountId"`
	Account       string         `json:"account"`
	SecurityID    string         `json:"securityId"`
	Symbol        string         `json:"symbol"`
	Currency      string         `json:"currency"`
	Side          string         `json:"side"`
	Type          string         `json:"type"`
	Quantity      float64        `json:"quantity"`
	LimitPrice    *float64       `json:"limitPrice"`
	StopPrice     *float64       `json:"stopPrice"`
	Tif           string         `json:"tif"`
	StopLoss      *StopLoss      `json:"stopLoss"`
	TakeProfit    *TakeProfit    `json:"takeProfit"`
	Status        string         `json:"status"`
	WsOrderID     string         `json:"wsOrderId"`
	Error         string         `json:"error"`
	Request       map[string]any `json:"request"`
	UpdatedAt     string         `json:"updatedAt"`
	Source        string         `json:"source"`
	WsStatus      string         `json:"wsStatus"`
	FilledQty     *float64       `json:"filledQty"`
	AvgFill       *float64       `json:"avgFill"`
	SubmittedAt   string         `json:"submittedAt"`
	ExpiresAt     string         `json:"expiresAt"`
	ParentID      string         `json:"parentId"`
	Role          string         `json:"role"`
	FillBookedQty *float64       `json:"fillBookedQty"`
	Exchange      string         `json:"exchange"`
}

func orderFromRow(r map[string]any) Order {
	o := Order{
		ID: str(r["id"]), CreatedAt: str(r["created_at"]), AccountID: str(r["account_id"]), Account: str(r["account"]), SecurityID: str(r["security_id"]),
		Symbol: str(r["symbol"]), Currency: str(r["currency"]), Side: str(r["side"]), Type: str(r["type"]), Quantity: py.Deref(fnum(r["quantity"]), 0),
		LimitPrice: fnum(r["limit_price"]), StopPrice: fnum(r["stop_price"]), Tif: str(r["tif"]), Status: str(r["status"]), WsOrderID: str(r["ws_order_id"]),
		Error: str(r["error"]), UpdatedAt: str(r["updated_at"]), Source: str(r["source"]), WsStatus: str(r["ws_status"]), FilledQty: fnum(r["filled_qty"]),
		AvgFill: fnum(r["avg_fill"]), SubmittedAt: str(r["submitted_at"]), ExpiresAt: str(r["expires_at"]), ParentID: str(r["parent_id"]), Role: str(r["role"]),
		FillBookedQty: fnum(r["fill_booked_qty"]),
	}
	if o.Source == "" {
		o.Source = "bagholder"
	}
	if o.Role == "" {
		o.Role = "entry"
	}
	if v := str(r["stop_loss"]); v != "" {
		var sl StopLoss
		if json.Unmarshal([]byte(v), &sl) == nil {
			o.StopLoss = &sl
		}
	}
	if v := str(r["take_profit"]); v != "" {
		var tp TakeProfit
		if json.Unmarshal([]byte(v), &tp) == nil {
			o.TakeProfit = &tp
		}
	}
	if v := str(r["request"]); v != "" {
		var req map[string]any
		if json.Unmarshal([]byte(v), &req) == nil {
			o.Request = req
		}
	}
	return o
}

func jsonOrNil(v any, present bool) any {
	if !present {
		return nil
	}
	b, err := json.Marshal(v)
	if err != nil {
		return nil
	}
	return string(b)
}

func (s *Store) InsertOrder(o Order) {
	s.must()
	now := nowISO()
	created := o.CreatedAt
	if created == "" {
		created = now
	}
	source := o.Source
	if source == "" {
		source = "bagholder"
	}
	role := o.Role
	if role == "" {
		role = "entry"
	}
	_, _ = s.exec("INSERT INTO orders (id, created_at, account_id, account, security_id, symbol, currency, side, type, quantity, limit_price, stop_price, tif, stop_loss, take_profit, status, ws_order_id, error, request, updated_at, source, ws_status, filled_qty, avg_fill, submitted_at, expires_at, parent_id, role) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
		o.ID, created, o.AccountID, o.Account, o.SecurityID, o.Symbol, o.Currency, o.Side, o.Type, o.Quantity, nullable(o.LimitPrice), nullable(o.StopPrice), o.Tif,
		jsonOrNil(o.StopLoss, o.StopLoss != nil), jsonOrNil(o.TakeProfit, o.TakeProfit != nil), o.Status, o.WsOrderID, o.Error, jsonOrNil(o.Request, len(o.Request) > 0), now,
		source, o.WsStatus, nullable(o.FilledQty), nullable(o.AvgFill), o.SubmittedAt, o.ExpiresAt, o.ParentID, role)
}

type OrderPatch map[string]any

var orderTextCols = map[string]string{"status": "status", "wsOrderId": "ws_order_id", "error": "error", "wsStatus": "ws_status", "submittedAt": "submitted_at", "expiresAt": "expires_at", "tif": "tif", "currency": "currency", "symbol": "symbol"}
var orderNumCols = map[string]string{"filledQty": "filled_qty", "avgFill": "avg_fill", "quantity": "quantity", "limitPrice": "limit_price", "stopPrice": "stop_price"}

func (s *Store) UpdateOrder(orderID string, patch OrderPatch) {
	var sets []string
	var vals []any
	for _, k := range []string{"status", "wsOrderId", "error", "wsStatus", "submittedAt", "expiresAt", "tif", "currency", "symbol"} {
		if v, ok := patch[k]; ok {
			sets = append(sets, orderTextCols[k]+" = ?")
			vals = append(vals, py.S(v))
		}
	}
	for _, k := range []string{"filledQty", "avgFill", "quantity", "limitPrice", "stopPrice"} {
		if v, ok := patch[k]; ok {
			sets = append(sets, orderNumCols[k]+" = ?")
			if f, ok := py.NumOK(v); ok {
				vals = append(vals, f)
			} else {
				vals = append(vals, nil)
			}
		}
	}
	if len(sets) == 0 {
		return
	}
	sets = append(sets, "updated_at = ?")
	vals = append(vals, nowISO(), orderID)
	s.must()
	_, _ = s.exec("UPDATE orders SET "+strings.Join(sets, ", ")+" WHERE id = ?", vals...)
}

func (s *Store) ListOrders(limit int) []Order {
	s.must()
	if limit <= 0 {
		limit = 200
	}
	out := []Order{}
	for _, r := range mustRows(s.queryMaps("SELECT * FROM orders ORDER BY created_at DESC, rowid DESC LIMIT ?", limit)) {
		out = append(out, orderFromRow(r))
	}
	return out
}

func (s *Store) GetOrder(orderID string) *Order {
	s.must()
	r, _ := s.queryOne("SELECT * FROM orders WHERE id = ?", orderID)
	if r == nil {
		return nil
	}
	o := orderFromRow(r)
	return &o
}

func (s *Store) MarkOrderFillBooked(orderID string, qty float64) bool {
	s.must()
	res, err := s.exec("UPDATE orders SET fill_booked_qty = ?, updated_at = ? WHERE id = ? AND (fill_booked_qty IS NULL OR fill_booked_qty < ?)", qty, nowISO(), orderID, qty)
	if err != nil {
		return false
	}
	n, _ := res.RowsAffected()
	return n > 0
}

type Bracket struct {
	ID          string   `json:"id"`
	OrderID     string   `json:"orderId"`
	CreatedAt   string   `json:"createdAt"`
	AccountID   string   `json:"accountId"`
	SecurityID  string   `json:"securityId"`
	Symbol      string   `json:"symbol"`
	Currency    string   `json:"currency"`
	Quantity    *float64 `json:"quantity"`
	Tif         string   `json:"tif"`
	SlKind      string   `json:"slKind"`
	SlPrice     *float64 `json:"slPrice"`
	SlTrail     *float64 `json:"slTrail"`
	SlTrailUnit string   `json:"slTrailUnit"`
	SlOrderID   string   `json:"slOrderId"`
	SlNative    bool     `json:"slNative"`
	SlMode      string   `json:"slMode"`
	HighWater   *float64 `json:"highWater"`
	TpPrice     *float64 `json:"tpPrice"`
	TpOrderID   string   `json:"tpOrderId"`
	Status      string   `json:"status"`
	Outcome     string   `json:"outcome"`
	Error       string   `json:"error"`
	Attempts    int      `json:"attempts"`
	MovedAt     string   `json:"movedAt"`
	ArmedAt     string   `json:"armedAt"`
	SeenHeld    bool     `json:"seenHeld"`
	MissedAt    string   `json:"missedAt"`
	UpdatedAt   string   `json:"updatedAt"`
}

func bracketFromRow(r map[string]any) Bracket {
	b := Bracket{
		ID: str(r["id"]), OrderID: str(r["order_id"]), CreatedAt: str(r["created_at"]), AccountID: str(r["account_id"]), SecurityID: str(r["security_id"]),
		Symbol: str(r["symbol"]), Currency: str(r["currency"]), Quantity: fnum(r["quantity"]), Tif: str(r["tif"]), SlKind: str(r["sl_kind"]), SlPrice: fnum(r["sl_price"]),
		SlTrail: fnum(r["sl_trail"]), SlTrailUnit: str(r["sl_trail_unit"]), SlOrderID: str(r["sl_order_id"]), SlNative: inum(r["sl_native"]) != 0, SlMode: str(r["sl_mode"]),
		HighWater: fnum(r["high_water"]), TpPrice: fnum(r["tp_price"]), TpOrderID: str(r["tp_order_id"]), Status: str(r["status"]), Outcome: str(r["outcome"]), Error: str(r["error"]),
		Attempts: int(inum(r["attempts"])), MovedAt: str(r["moved_at"]), ArmedAt: str(r["armed_at"]), SeenHeld: inum(r["seen_held"]) != 0, MissedAt: str(r["missed_at"]), UpdatedAt: str(r["updated_at"]),
	}
	if b.Tif == "" {
		b.Tif = "DAY"
	}
	if b.SlTrailUnit == "" {
		b.SlTrailUnit = "pct"
	}
	return b
}

func (s *Store) InsertBracket(b Bracket) {
	s.must()
	now := nowISO()
	created := b.CreatedAt
	if created == "" {
		created = now
	}
	tif := b.Tif
	if tif == "" {
		tif = "DAY"
	}
	unit := b.SlTrailUnit
	if unit == "" {
		unit = "pct"
	}
	status := b.Status
	if status == "" {
		status = "waiting"
	}
	native := 0
	if b.SlNative {
		native = 1
	}
	_, _ = s.exec("INSERT INTO brackets (id, order_id, created_at, account_id, security_id, symbol, currency, quantity, tif, sl_kind, sl_price, sl_trail, sl_trail_unit, sl_order_id, sl_native, sl_mode, high_water, tp_price, tp_order_id, status, outcome, error, attempts, moved_at, armed_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
		b.ID, b.OrderID, created, b.AccountID, b.SecurityID, b.Symbol, b.Currency, nullable(b.Quantity), tif, b.SlKind, nullable(b.SlPrice), nullable(b.SlTrail), unit, b.SlOrderID, native, b.SlMode, nullable(b.HighWater), nullable(b.TpPrice), b.TpOrderID, status, b.Outcome, b.Error, b.Attempts, b.MovedAt, b.ArmedAt, now)
}

type BracketPatch map[string]any

var bracketText = []struct{ key, col string }{{"symbol", "symbol"}, {"currency", "currency"}, {"tif", "tif"}, {"slKind", "sl_kind"}, {"slTrailUnit", "sl_trail_unit"}, {"slOrderId", "sl_order_id"}, {"tpOrderId", "tp_order_id"}, {"status", "status"}, {"outcome", "outcome"}, {"error", "error"}, {"movedAt", "moved_at"}, {"armedAt", "armed_at"}, {"slMode", "sl_mode"}, {"missedAt", "missed_at"}}
var bracketNum = []struct{ key, col string }{{"quantity", "quantity"}, {"slPrice", "sl_price"}, {"slTrail", "sl_trail"}, {"highWater", "high_water"}, {"tpPrice", "tp_price"}, {"attempts", "attempts"}, {"slNative", "sl_native"}, {"seenHeld", "seen_held"}}

func (s *Store) UpdateBracket(bracketID string, patch BracketPatch) {
	var sets []string
	var vals []any
	for _, c := range bracketText {
		if v, ok := patch[c.key]; ok {
			sets = append(sets, c.col+" = ?")
			vals = append(vals, py.S(v))
		}
	}
	for _, c := range bracketNum {
		v, ok := patch[c.key]
		if !ok {
			continue
		}
		sets = append(sets, c.col+" = ?")
		if v == nil {
			vals = append(vals, nil)
			continue
		}
		switch c.key {
		case "slNative", "seenHeld":
			b, isBool := v.(bool)
			if (isBool && b) || (!isBool && py.Num(v, 0) != 0) {
				vals = append(vals, 1)
			} else {
				vals = append(vals, 0)
			}
		case "attempts":
			vals = append(vals, int(py.Num(v, 0)))
		default:
			if f, ok := py.NumOK(v); ok {
				vals = append(vals, f)
			} else {
				vals = append(vals, nil)
			}
		}
	}
	if len(sets) == 0 {
		return
	}
	sets = append(sets, "updated_at = ?")
	vals = append(vals, nowISO(), bracketID)
	s.must()
	_, _ = s.exec("UPDATE brackets SET "+strings.Join(sets, ", ")+" WHERE id = ?", vals...)
}

func (s *Store) ListBrackets(statuses []string) []Bracket {
	s.must()
	out := []Bracket{}
	var rows []map[string]any
	if len(statuses) > 0 {
		args := make([]any, len(statuses))
		marks := make([]string, len(statuses))
		for i, st := range statuses {
			args[i] = st
			marks[i] = "?"
		}
		rows = mustRows(s.queryMaps("SELECT * FROM brackets WHERE status IN ("+strings.Join(marks, ",")+") ORDER BY created_at", args...))
	} else {
		rows = mustRows(s.queryMaps("SELECT * FROM brackets ORDER BY created_at"))
	}
	for _, r := range rows {
		out = append(out, bracketFromRow(r))
	}
	return out
}

func (s *Store) GetBracket(bracketID string) *Bracket {
	s.must()
	r, _ := s.queryOne("SELECT * FROM brackets WHERE id = ?", bracketID)
	if r == nil {
		return nil
	}
	b := bracketFromRow(r)
	return &b
}

func (s *Store) BracketForOrder(orderID string) *Bracket {
	s.must()
	r, _ := s.queryOne("SELECT * FROM brackets WHERE order_id = ? ORDER BY created_at DESC LIMIT 1", orderID)
	if r == nil {
		return nil
	}
	b := bracketFromRow(r)
	return &b
}

var _ = sql.ErrNoRows
