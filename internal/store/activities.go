package store

import (
	"database/sql"
	"encoding/json"
	"sort"
	"strings"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

type Activity struct {
	ID              string   `json:"id"`
	CanonicalID     string   `json:"canonicalId"`
	OccurredAt      string   `json:"occurredAt"`
	TransactionDate string   `json:"transactionDate"`
	SettlementDate  string   `json:"settlementDate"`
	AccountID       string   `json:"accountId"`
	BookID          string   `json:"bookId"`
	FifoID          string   `json:"fifoId"`
	AccountType     string   `json:"accountType"`
	ActivityType    string   `json:"activityType"`
	ActivitySubType string   `json:"activitySubType"`
	Description     string   `json:"description"`
	Direction       string   `json:"direction"`
	Symbol          string   `json:"symbol"`
	Name            string   `json:"name"`
	Currency        string   `json:"currency"`
	Quantity        float64  `json:"quantity"`
	UnitPrice       float64  `json:"unitPrice"`
	Commission      float64  `json:"commission"`
	NetCashAmount   float64  `json:"netCashAmount"`
	Category        string   `json:"category"`
	Balance         *float64 `json:"balance"`
	Source          string   `json:"source"`
	RawType         string   `json:"rawType"`
	AftType         string   `json:"aftType"`
	CounterSymbol   string   `json:"counterSymbol"`
	SecurityID      string   `json:"securityId"`

	Kind       string   `json:"kind,omitempty"`
	Flags      []string `json:"flags,omitempty"`
	Normalized bool     `json:"-"`
}

func (a Activity) MarshalJSON() ([]byte, error) {
	type plain Activity
	type out struct {
		plain
		CanonicalID *string `json:"canonicalId"`
		SecurityID  *string `json:"securityId"`
	}
	o := out{plain: plain(a)}
	if a.CanonicalID != "" {
		o.CanonicalID = &a.CanonicalID
	}
	if a.SecurityID != "" {
		o.SecurityID = &a.SecurityID
	}
	return json.Marshal(o)
}

func (a Activity) Clone() Activity {
	b := a
	if a.Flags != nil {
		b.Flags = append([]string{}, a.Flags...)
	}
	if a.Balance != nil {
		v := *a.Balance
		b.Balance = &v
	}
	return b
}

func (a *Activity) HasFlag(f string) bool {
	return py.Contains(a.Flags, f)
}

func LooksLikeHomemadeID(aid string) bool {
	s := strings.TrimSpace(aid)
	if s == "" || strings.Contains(s, "|") {
		return true
	}
	return strings.HasPrefix(strings.ToLower(s), "manual")
}

func IsRealAccount(accountID string) bool {
	s := strings.TrimSpace(accountID)
	if s == "" || strings.HasPrefix(s, "~") {
		return false
	}
	return !invented[strings.ToLower(s)]
}

func compactUpper(s string) string {
	r := strings.NewReplacer(" ", "", "_", "", "-", "")
	return r.Replace(strings.ToUpper(s))
}

func TradeSide(a *Activity) string {
	sub := compactUpper(a.ActivitySubType)
	switch sub {
	case "BUY", "BUYTOOPEN", "BTO", "BUYTOCLOSE", "BTC":
		return "BUY"
	case "SELL", "SELLTOOPEN", "STO", "SELLTOCLOSE", "STC":
		return "SELL"
	}
	if strings.HasPrefix(sub, "BUY") {
		return "BUY"
	}
	if strings.HasPrefix(sub, "SELL") {
		return "SELL"
	}
	typ := compactUpper(a.ActivityType)
	if strings.HasPrefix(typ, "BUY") {
		return "BUY"
	}
	if strings.HasPrefix(typ, "SELL") {
		return "SELL"
	}
	if a.Quantity > 0 {
		return "BUY"
	}
	if a.Quantity < 0 {
		return "SELL"
	}
	return ""
}

func roundQty(v float64) float64 { return py.Round(v, 8) }

func activityDate(a *Activity) string {
	d := a.TransactionDate
	if len(d) > 10 {
		d = d[:10]
	}
	if d == "" {
		d, _, _ = strings.Cut(a.OccurredAt, "T")
		if len(d) > 10 {
			d = d[:10]
		}
	}
	return d
}

type FieldMatchKey struct {
	Date, Account, Symbol string
	Qty, Price, Cash      float64
}

func FieldKey(a *Activity, includeAccount bool) FieldMatchKey {
	account := ""
	if includeAccount && IsRealAccount(a.AccountID) {
		account = a.AccountID
	}
	return FieldMatchKey{activityDate(a), account, strings.ToUpper(py.Strip(a.Symbol)), roundQty(a.Quantity), roundQty(a.UnitPrice), roundQty(a.NetCashAmount)}
}

type LinkMatchKey struct {
	Symbol, Side  string
	Qty, Price    float64
	Date, Account string
}

func LinkKey(a *Activity, includeAccount bool) LinkMatchKey {
	account := ""
	if includeAccount && IsRealAccount(a.AccountID) {
		account = a.AccountID
	}
	return LinkMatchKey{strings.ToUpper(py.Strip(a.Symbol)), TradeSide(a), roundQty(a.Quantity), roundQty(a.UnitPrice), activityDate(a), account}
}

func canonicalFromRow(a *Activity, source string) string {
	if source != "wealthsimple" {
		return ""
	}
	if cid := strings.TrimSpace(a.CanonicalID); cid != "" && !LooksLikeHomemadeID(cid) {
		return cid
	}
	if old := strings.TrimSpace(a.ID); old != "" && !LooksLikeHomemadeID(old) {
		return old
	}
	return ""
}

func CanonicalFromRow(a *Activity, source string) string { return canonicalFromRow(a, source) }

func rowToActivity(r map[string]any) Activity {
	settle := str(r["settlement_date"])
	if settle == "" {
		settle = str(r["transaction_date"])
	}
	book := str(r["book_id"])
	if book == "" {
		book = str(r["account_id"])
	}
	fifo := str(r["fifo_id"])
	if fifo == "" {
		fifo = str(r["account_id"])
	}
	return Activity{
		ID: str(r["id"]), CanonicalID: str(r["canonical_id"]), OccurredAt: str(r["occurred_at"]), TransactionDate: str(r["transaction_date"]),
		SettlementDate: settle, AccountID: str(r["account_id"]), BookID: book, FifoID: fifo, AccountType: str(r["account_type"]),
		ActivityType: str(r["activity_type"]), ActivitySubType: str(r["activity_sub_type"]), Description: str(r["description"]),
		Direction: str(r["direction"]), Symbol: str(r["symbol"]), Name: str(r["name"]), Currency: str(r["currency"]),
		Quantity: py.Deref(fnum(r["quantity"]), 0), UnitPrice: py.Deref(fnum(r["unit_price"]), 0), Commission: py.Deref(fnum(r["commission"]), 0),
		NetCashAmount: py.Deref(fnum(r["net_cash_amount"]), 0), Category: str(r["category"]), Balance: fnum(r["balance"]), Source: str(r["source"]),
		RawType: str(r["raw_type"]), AftType: str(r["aft_type"]), CounterSymbol: str(r["counter_symbol"]), SecurityID: str(r["security_id"]),
	}
}

const insertColumns = "id, canonical_id, occurred_at, transaction_date, settlement_date, account_id, book_id, fifo_id, account_type, activity_type, activity_sub_type, description, direction, symbol, name, currency, quantity, unit_price, commission, net_cash_amount, category, balance, source, raw_type, aft_type, counter_symbol, security_id"

const insertSQL = "INSERT INTO activities (" + insertColumns + ") VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"

var insertColumnList = strings.Split(insertColumns, ", ")

func insertParams(a *Activity, assignedID string, canonicalID any) []any {
	occurred := strings.TrimSpace(a.OccurredAt)
	date := strings.TrimSpace(a.TransactionDate)
	if date == "" && occurred != "" {
		date, _, _ = strings.Cut(occurred, "T")
		if len(date) > 10 {
			date = date[:10]
		}
	}
	if occurred != "" && !strings.Contains(occurred, "T") {
		if len(occurred) > 10 {
			occurred = occurred[:10]
		}
	}
	settle := strings.TrimSpace(a.SettlementDate)
	if settle == "" {
		settle = date
	}
	accountID := a.AccountID
	book := a.BookID
	if book == "" {
		book = accountID
	}
	fifo := a.FifoID
	if fifo == "" {
		fifo = accountID
	}
	return []any{assignedID, canonicalID, occurred, date, settle, accountID, book, fifo, a.AccountType, a.ActivityType, a.ActivitySubType,
		a.Description, a.Direction, a.Symbol, a.Name, a.Currency, a.Quantity, a.UnitPrice, a.Commission, a.NetCashAmount, a.Category,
		nullable(a.Balance), a.Source, a.RawType, a.AftType, a.CounterSymbol, nullStr(strings.TrimSpace(a.SecurityID))}
}

func (s *Store) InsertActivity(a Activity, canonicalID string, assignedID string) (Activity, error) {
	s.must()
	source := a.Source
	if source == "" {
		source = "wealthsimple"
		a.Source = source
	}
	if canonicalID == "" && source == "wealthsimple" {
		canonicalID = canonicalFromRow(&a, source)
	}
	if source != "wealthsimple" {
		canonicalID = ""
	}
	aid := assignedID
	if aid == "" {
		aid = strings.TrimSpace(a.ID)
	}
	if aid == "" || LooksLikeHomemadeID(aid) {
		aid = py.UUID4()
	}
	var out Activity
	err := s.tx(func(tx *sql.Tx) error {
		if _, err := tx.Exec(insertSQL, insertParams(&a, aid, nullStr(canonicalID))...); err != nil {
			return err
		}
		rows, err := tx.Query("SELECT * FROM activities WHERE id = ?", aid)
		if err != nil {
			return err
		}
		defer rows.Close()
		if rows.Next() {
			m, err := scanRow(rows)
			if err != nil {
				return err
			}
			out = rowToActivity(m)
		}
		return rows.Err()
	})
	return out, err
}

func (s *Store) InsertLocal(a Activity) (Activity, error) {
	source := a.Source
	if source == "" || source == "wealthsimple" {
		source = "manual"
	}
	a.Source = source
	a.CanonicalID = ""
	return s.InsertActivity(a, "", "")
}

func (s *Store) allActivities() ([]Activity, error) {
	rows, err := s.queryMaps("SELECT * FROM activities ORDER BY COALESCE(occurred_at, transaction_date) ASC, id ASC")
	if err != nil {
		return nil, err
	}
	out := make([]Activity, 0, len(rows))
	for _, r := range rows {
		out = append(out, rowToActivity(r))
	}
	return out, nil
}

func (s *Store) Activities() []Activity {
	s.must()
	out, _ := s.allActivities()
	return out
}

func (s *Store) ActivityCount() int {
	s.must()
	var n int
	_ = s.db.QueryRow("SELECT COUNT(*) AS n FROM activities").Scan(&n)
	return n
}

func (s *Store) CanonicalIDs() map[string]bool {
	s.must()
	out := map[string]bool{}
	rows, err := s.queryMaps("SELECT canonical_id FROM activities WHERE canonical_id IS NOT NULL AND canonical_id != ''")
	if err != nil {
		return out
	}
	for _, r := range rows {
		out[str(r["canonical_id"])] = true
	}
	return out
}

func (s *Store) NewestWSOccurredAt() string {
	s.must()
	row, _ := s.queryOne("SELECT occurred_at, transaction_date FROM activities WHERE source = 'wealthsimple' ORDER BY COALESCE(occurred_at, transaction_date) DESC LIMIT 1")
	if row == nil {
		row, _ = s.queryOne("SELECT occurred_at, transaction_date FROM activities ORDER BY COALESCE(occurred_at, transaction_date) DESC LIMIT 1")
	}
	if row == nil {
		return ""
	}
	v := str(row["occurred_at"])
	if v == "" {
		v = str(row["transaction_date"])
	}
	return strings.TrimSpace(v)
}

func (s *Store) IncrementalStartDate() string {
	newest := s.NewestWSOccurredAt()
	if newest == "" {
		return ""
	}
	day, _, _ := strings.Cut(newest, "T")
	if len(day) > 10 {
		day = day[:10]
	}
	t, ok := py.ParseDate(day)
	if !ok {
		return day
	}
	return py.DateStr(t.AddDate(0, 0, -PullOverlapDays))
}

func inPullTZ(t time.Time) time.Time { return t.In(ActivityPullTZ) }

func (s *Store) ActivityPullDue(now time.Time) bool {
	if now.IsZero() {
		now = time.Now()
	}
	local := inPullTZ(now)
	if local.Weekday() == time.Saturday || local.Weekday() == time.Sunday {
		return false
	}
	close := time.Date(local.Year(), local.Month(), local.Day(), ActivityPullHour, ActivityPullMinute, 0, 0, ActivityPullTZ)
	if local.Before(close) {
		return false
	}
	last := s.GetMeta("last_activity_pull")
	if last == "" {
		return true
	}
	then, ok := py.ParseStamp(last)
	if !ok {
		return true
	}
	return inPullTZ(then).Before(close)
}

func (s *Store) MarkActivityPulled(when string) {
	if when == "" {
		when = nowISO()
	}
	s.SetMeta("last_activity_pull", when)
}

func (s *Store) FindLinkCandidates(a *Activity) []Activity {
	s.must()
	includeAccount := IsRealAccount(a.AccountID)
	target := LinkKey(a, includeAccount)
	var matches []Activity
	rows, err := s.queryMaps("SELECT * FROM activities WHERE canonical_id IS NULL OR canonical_id = ''")
	if err != nil {
		return nil
	}
	for _, r := range rows {
		mapped := rowToActivity(r)
		if LinkKey(&mapped, includeAccount) == target {
			matches = append(matches, mapped)
		}
	}
	return matches
}

func (s *Store) StampCanonicalID(activityID, canonicalID string) bool {
	s.must()
	cid := strings.TrimSpace(canonicalID)
	if cid == "" || LooksLikeHomemadeID(cid) {
		return false
	}
	res, err := s.exec("UPDATE activities SET canonical_id = ? WHERE id = ? AND (canonical_id IS NULL OR canonical_id = '')", cid, activityID)
	if err != nil {
		return false
	}
	n, _ := res.RowsAffected()
	return n > 0
}

var revisableColumns = []string{"occurred_at", "transaction_date", "settlement_date", "activity_type", "activity_sub_type", "description", "direction", "symbol", "name", "currency", "quantity", "unit_price", "commission", "net_cash_amount", "category", "raw_type", "aft_type", "counter_symbol"}

func differs(a, b any) bool {
	af, aok := a.(float64)
	bf, bok := b.(float64)
	if aok || bok {
		var x, y float64
		if aok {
			x = af
		} else if p := fnum(a); p != nil {
			x = *p
		} else if a != nil {
			return true
		}
		if bok {
			y = bf
		} else if p := fnum(b); p != nil {
			y = *p
		} else if b != nil {
			return true
		}
		return abs(x-y) > 1e-9
	}
	return str(a) != str(b)
}

func (s *Store) reviseWealthsimpleRow(cid string, row *Activity) bool {
	params := insertParams(row, "", cid)
	incoming := map[string]any{}
	for i, c := range insertColumnList {
		incoming[c] = params[i]
	}
	changed := false
	_ = s.tx(func(tx *sql.Tx) error {
		rows, err := tx.Query("SELECT * FROM activities WHERE canonical_id = ?", cid)
		if err != nil {
			return err
		}
		var stored map[string]any
		if rows.Next() {
			stored, err = scanRow(rows)
		}
		rows.Close()
		if err != nil || stored == nil {
			return err
		}
		var cols []string
		for _, c := range revisableColumns {
			if differs(incoming[c], stored[c]) {
				cols = append(cols, c)
			}
		}
		if len(cols) == 0 {
			return nil
		}
		if incoming["security_id"] != nil && str(stored["security_id"]) == "" {
			cols = append(cols, "security_id")
		}
		sets := make([]string, 0, len(cols))
		args := make([]any, 0, len(cols)+1)
		for _, c := range cols {
			sets = append(sets, c+" = ?")
			args = append(args, incoming[c])
		}
		args = append(args, cid)
		if _, err := tx.Exec("UPDATE activities SET "+strings.Join(sets, ", ")+" WHERE canonical_id = ?", args...); err != nil {
			return err
		}
		changed = true
		return nil
	})
	return changed
}

type ApplyResult struct {
	Inserted, Linked, Skipped, Revised int
}

func (s *Store) ApplyWealthsimpleMapped(rows []Activity) ApplyResult {
	s.must()
	var out ApplyResult
	known := s.CanonicalIDs()
	for i := range rows {
		row := rows[i].Clone()
		row.Source = "wealthsimple"
		cid := strings.TrimSpace(row.CanonicalID)
		if cid == "" || LooksLikeHomemadeID(cid) {
			continue
		}
		if known[cid] {
			if s.reviseWealthsimpleRow(cid, &row) {
				out.Revised++
				continue
			}
			if sid := strings.TrimSpace(row.SecurityID); sid != "" {
				_, _ = s.exec("UPDATE activities SET security_id = ? WHERE canonical_id = ? AND (security_id IS NULL OR security_id = '')", sid, cid)
			}
			out.Skipped++
			continue
		}
		matches := s.FindLinkCandidates(&row)
		if len(matches) == 1 && s.StampCanonicalID(matches[0].ID, cid) {
			known[cid] = true
			out.Linked++
			continue
		}
		if _, err := s.InsertActivity(row, cid, ""); err == nil {
			known[cid] = true
			out.Inserted++
		}
	}
	return out
}

type MergeResult struct {
	Added      int        `json:"added"`
	Duplicates int        `json:"duplicates"`
	Activities []Activity `json:"activities"`
}

func (s *Store) MergeLocalRows(rows []Activity) MergeResult {
	s.must()
	out := MergeResult{Activities: []Activity{}}
	existing := map[FieldMatchKey]int{}
	all, _ := s.allActivities()
	for i := range all {
		existing[FieldKey(&all[i], true)]++
	}
	incoming := map[FieldMatchKey]int{}
	for i := range rows {
		row := rows[i].Clone()
		source := row.Source
		if source == "" {
			source = "csv"
		}
		if source == "wealthsimple" {
			if cid := canonicalFromRow(&row, "wealthsimple"); cid != "" {
				r := s.ApplyWealthsimpleMapped([]Activity{row})
				out.Added += r.Inserted + r.Linked
				out.Duplicates += r.Skipped
				continue
			}
			source = "csv"
		}
		row.Source = source
		row.CanonicalID = ""
		k := FieldKey(&row, true)
		incoming[k]++
		if incoming[k] <= existing[k] {
			out.Duplicates++
			continue
		}
		saved, err := s.InsertLocal(row)
		if err != nil {
			continue
		}
		existing[k]++
		out.Activities = append(out.Activities, saved)
		out.Added++
	}
	return out
}

type Account struct {
	ID                  string   `json:"id"`
	Nickname            string   `json:"nickname"`
	UnifiedAccountType  string   `json:"unifiedAccountType"`
	Currency            string   `json:"currency"`
	Status              string   `json:"status"`
	Type                string   `json:"type"`
	NetLiquidationValue *float64 `json:"netLiquidationValue"`
	MarginAccountID     string   `json:"marginAccountId"`
}

func (s *Store) ReplaceAccounts(accounts []Account) {
	s.must()
	_ = s.tx(func(tx *sql.Tx) error {
		if _, err := tx.Exec("DELETE FROM accounts"); err != nil {
			return err
		}
		for _, acc := range accounts {
			if acc.ID == "" {
				continue
			}
			if _, err := tx.Exec("INSERT INTO accounts (id, nickname, unified_account_type, currency, status, type, net_liquidation_value, margin_account_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
				acc.ID, acc.Nickname, acc.UnifiedAccountType, acc.Currency, acc.Status, acc.Type, nullable(acc.NetLiquidationValue), acc.MarginAccountID); err != nil {
				return err
			}
		}
		return nil
	})
}

type Balance struct {
	AccountID          string   `json:"accountId"`
	CustodianAccountID string   `json:"custodianAccountId"`
	SecurityID         string   `json:"securityId"`
	Quantity           *float64 `json:"quantity"`
}

func (s *Store) ReplaceBalances(balances []Balance) {
	s.must()
	_ = s.tx(func(tx *sql.Tx) error {
		if _, err := tx.Exec("DELETE FROM balances"); err != nil {
			return err
		}
		for _, b := range balances {
			if _, err := tx.Exec("INSERT INTO balances (account_id, custodian_account_id, security_id, quantity) VALUES (?, ?, ?, ?)", b.AccountID, b.CustodianAccountID, b.SecurityID, nullable(b.Quantity)); err != nil {
				return err
			}
		}
		return nil
	})
}

type Margin struct {
	AccountID   string   `json:"accountId"`
	BuyingPower *float64 `json:"buyingPower"`
	Currency    string   `json:"currency"`
	Unavailable string   `json:"unavailable"`
	FetchedAt   string   `json:"fetchedAt"`
}

func (s *Store) ReplaceMargin(rows []Margin) {
	s.must()
	now := nowISO()
	_ = s.tx(func(tx *sql.Tx) error {
		if _, err := tx.Exec("DELETE FROM margin"); err != nil {
			return err
		}
		for _, m := range rows {
			if m.AccountID == "" {
				continue
			}
			ccy := m.Currency
			if ccy == "" {
				ccy = "CAD"
			}
			at := m.FetchedAt
			if at == "" {
				at = now
			}
			if _, err := tx.Exec("INSERT INTO margin (account_id, buying_power, currency, unavailable, fetched_at) VALUES (?, ?, ?, ?, ?)", m.AccountID, nullable(m.BuyingPower), ccy, m.Unavailable, at); err != nil {
				return err
			}
		}
		return nil
	})
}

type NavPoint struct {
	Date        string   `json:"date"`
	Equity      float64  `json:"equity"`
	Currency    string   `json:"currency"`
	NetDeposits *float64 `json:"netDeposits,omitempty"`
	AccountID   string   `json:"accountId,omitempty"`
}

func navPointFromRow(r map[string]any) NavPoint {
	ccy := str(r["currency"])
	if ccy == "" {
		ccy = "CAD"
	}
	return NavPoint{Date: str(r["date"]), Equity: py.Deref(fnum(r["equity"]), 0), Currency: ccy, NetDeposits: fnum(r["net_deposits"])}
}

func (s *Store) NavLastDates() map[string]string {
	s.must()
	out := map[string]string{}
	rows, err := s.queryMaps("SELECT account_id, MAX(date) AS last FROM nav_history GROUP BY account_id")
	if err != nil {
		return out
	}
	for _, r := range rows {
		if last := str(r["last"]); last != "" {
			out[str(r["account_id"])] = last
		}
	}
	return out
}

func writeNavPoints(tx *sql.Tx, points []NavPoint) error {
	for _, rec := range points {
		day := rec.Date
		if len(day) > 10 {
			day = day[:10]
		}
		if day == "" {
			continue
		}
		ccy := rec.Currency
		if ccy == "" {
			ccy = "CAD"
		}
		if _, err := tx.Exec("INSERT INTO nav_history (account_id, date, equity, currency, net_deposits) VALUES (?, ?, ?, ?, ?) ON CONFLICT(account_id, date) DO UPDATE SET equity = excluded.equity, currency = excluded.currency, net_deposits = excluded.net_deposits",
			rec.AccountID, day, rec.Equity, ccy, nullable(rec.NetDeposits)); err != nil {
			return err
		}
	}
	return nil
}

func (s *Store) UpsertNav(points []NavPoint) {
	s.must()
	_ = s.tx(func(tx *sql.Tx) error { return writeNavPoints(tx, points) })
}

func (s *Store) ReplaceNav(points []NavPoint) {
	s.must()
	_ = s.tx(func(tx *sql.Tx) error {
		if _, err := tx.Exec("DELETE FROM nav_history"); err != nil {
			return err
		}
		return writeNavPoints(tx, points)
	})
}

type Security struct {
	ID              string `json:"id"`
	Symbol          string `json:"symbol"`
	Name            string `json:"name"`
	PrimaryExchange string `json:"primaryExchange"`
	PrimaryMic      string `json:"primaryMic"`
	Currency        string `json:"currency"`
	UnderlyingID    string `json:"underlyingId"`
	FetchedAt       string `json:"-"`
}

func (sec Security) MarshalJSON() ([]byte, error) {
	type plain Security
	type out struct {
		plain
		UnderlyingID *string `json:"underlyingId"`
	}
	o := out{plain: plain(sec)}
	if sec.UnderlyingID != "" {
		o.UnderlyingID = &sec.UnderlyingID
	}
	return json.Marshal(o)
}

func securityFromRow(r map[string]any) Security {
	return Security{ID: str(r["id"]), Symbol: str(r["symbol"]), Name: str(r["name"]), PrimaryExchange: str(r["primary_exchange"]), PrimaryMic: str(r["primary_mic"]), Currency: str(r["currency"]), UnderlyingID: str(r["underlying_id"])}
}

func (s *Store) UpsertSecurities(rows []Security) {
	s.must()
	now := nowISO()
	_ = s.tx(func(tx *sql.Tx) error {
		for _, raw := range rows {
			sid := strings.TrimSpace(raw.ID)
			if sid == "" {
				continue
			}
			at := raw.FetchedAt
			if at == "" {
				at = now
			}
			if _, err := tx.Exec("INSERT INTO securities (id, symbol, name, primary_exchange, primary_mic, currency, underlying_id, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET symbol = excluded.symbol, name = excluded.name, primary_exchange = excluded.primary_exchange, primary_mic = excluded.primary_mic, currency = excluded.currency, underlying_id = excluded.underlying_id, fetched_at = excluded.fetched_at",
				sid, raw.Symbol, raw.Name, raw.PrimaryExchange, raw.PrimaryMic, raw.Currency, nullStr(strings.TrimSpace(raw.UnderlyingID)), at); err != nil {
				return err
			}
		}
		return nil
	})
}

func (s *Store) ListSecurities() []Security {
	s.must()
	rows, _ := s.queryMaps("SELECT * FROM securities ORDER BY id")
	out := make([]Security, 0, len(rows))
	for _, r := range rows {
		out = append(out, securityFromRow(r))
	}
	return out
}

func (s *Store) MissingSecurityIDs(ids []string) []string {
	s.must()
	var wanted []string
	seen := map[string]bool{}
	for _, raw := range ids {
		sid := strings.TrimSpace(raw)
		if sid == "" || seen[sid] {
			continue
		}
		seen[sid] = true
		wanted = append(wanted, sid)
	}
	if len(wanted) == 0 {
		return []string{}
	}
	have := map[string]bool{}
	for i := 0; i < len(wanted); i += 400 {
		chunk := wanted[i:min(i+400, len(wanted))]
		args := make([]any, len(chunk))
		marks := make([]string, len(chunk))
		for j, c := range chunk {
			args[j] = c
			marks[j] = "?"
		}
		rows, _ := s.queryMaps("SELECT id FROM securities WHERE id IN ("+strings.Join(marks, ",")+")", args...)
		for _, r := range rows {
			have[str(r["id"])] = true
		}
	}
	out := []string{}
	for _, sid := range wanted {
		if !have[sid] {
			out = append(out, sid)
		}
	}
	return out
}

func (s *Store) NeedsSecurityIDBackfill() bool {
	s.must()
	var one int
	err := s.db.QueryRow("SELECT 1 AS n FROM activities WHERE source = 'wealthsimple' AND IFNULL(symbol, '') != '' AND (security_id IS NULL OR security_id = '') LIMIT 1").Scan(&one)
	return err == nil
}

func (s *Store) SymbolForSecurity(securityID string) string {
	s.must()
	row, _ := s.queryOne("SELECT symbol FROM activities WHERE security_id = ? AND symbol IS NOT NULL AND symbol != '' ORDER BY occurred_at DESC LIMIT 1", securityID)
	if row == nil {
		return ""
	}
	return str(row["symbol"])
}

func (s *Store) SoldSince(accountID, securityID, sinceISO, symbol string) float64 {
	s.must()
	var row map[string]any
	if securityID != "" {
		row, _ = s.queryOne("SELECT SUM(quantity) AS q FROM activities WHERE account_id = ? AND security_id = ? AND activity_type = 'Trade' AND activity_sub_type = 'SELL' AND occurred_at > ?", accountID, securityID, sinceISO)
	} else {
		row, _ = s.queryOne("SELECT SUM(quantity) AS q FROM activities WHERE account_id = ? AND symbol = ? AND activity_type = 'Trade' AND activity_sub_type = 'SELL' AND occurred_at > ?", accountID, symbol, sinceISO)
	}
	if row == nil {
		return 0
	}
	return py.Deref(fnum(row["q"]), 0)
}

func (s *Store) PositionQuantity(accountID, securityID string) *float64 {
	s.must()
	row, _ := s.queryOne("SELECT SUM(quantity) AS q FROM balances WHERE account_id = ? AND security_id = ?", accountID, securityID)
	if row == nil {
		return nil
	}
	return fnum(row["q"])
}

func (s *Store) BalancesCount() int {
	s.must()
	var n int
	_ = s.db.QueryRow("SELECT COUNT(*) FROM balances").Scan(&n)
	return n
}

type DividendSymbol struct {
	Symbol, Currency, Exchange string
}

func (s *Store) DividendSymbols() []DividendSymbol {
	s.must()
	rows, _ := s.queryMaps("SELECT DISTINCT a.symbol AS symbol, a.currency AS currency, s.primary_exchange AS exchange FROM activities a LEFT JOIN securities s ON s.id = a.security_id WHERE a.category = 'dividend' AND IFNULL(a.symbol, '') != ''")
	out := []DividendSymbol{}
	seen := map[string]bool{}
	for _, r := range rows {
		sym := strings.ToUpper(strings.TrimSpace(str(r["symbol"])))
		if sym == "" || seen[sym] {
			continue
		}
		seen[sym] = true
		out = append(out, DividendSymbol{sym, str(r["currency"]), strings.TrimSpace(str(r["exchange"]))})
	}
	return out
}

type TradeGroup struct {
	ID      string   `json:"id"`
	Locked  bool     `json:"locked"`
	Members []string `json:"members"`
}

func cleanTradeGroups(raw any) []TradeGroup {
	out := []TradeGroup{}
	list, ok := raw.([]any)
	if !ok {
		return out
	}
	seen := map[string]bool{}
	for _, item := range list {
		m, ok := item.(map[string]any)
		if !ok {
			continue
		}
		gid := strings.TrimSpace(py.OrStr(m["id"]))
		members, ok := m["members"].([]any)
		if gid == "" || seen[gid] || !ok {
			continue
		}
		var keys []string
		used := map[string]bool{}
		for _, x := range members {
			k := strings.TrimSpace(py.OrStr(x))
			if k == "" || used[k] {
				continue
			}
			used[k] = true
			keys = append(keys, k)
		}
		if len(keys) == 0 {
			continue
		}
		seen[gid] = true
		out = append(out, TradeGroup{ID: gid, Locked: py.Truthy(m["locked"]), Members: keys})
	}
	return out
}

func (s *Store) TradeGroups() []TradeGroup {
	raw := s.GetMeta("trade_groups")
	if raw == "" {
		return []TradeGroup{}
	}
	var data any
	if json.Unmarshal([]byte(raw), &data) != nil {
		return []TradeGroup{}
	}
	return cleanTradeGroups(data)
}

func (s *Store) SaveTradeGroups(groups any) []TradeGroup {
	clean := cleanTradeGroups(groups)
	b, _ := json.Marshal(clean)
	s.SetMeta("trade_groups", string(b))
	return clean
}

type TradeNote struct {
	Thesis  string `json:"thesis"`
	Tag     string `json:"tag"`
	Grade   string `json:"grade"`
	TradeID string `json:"tradeId"`
}

func cleanTradeNotes(raw any) map[string]TradeNote {
	out := map[string]TradeNote{}
	m, ok := raw.(map[string]any)
	if !ok {
		return out
	}
	for key, val := range m {
		kid := strings.TrimSpace(key)
		v, ok := val.(map[string]any)
		if kid == "" || !ok {
			continue
		}
		thesis, tag, grade := py.OrStr(v["thesis"]), py.OrStr(v["tag"]), py.OrStr(v["grade"])
		if grade != "A" && grade != "B" && grade != "C" && grade != "F" {
			grade = ""
		}
		if thesis == "" && tag == "" && grade == "" {
			continue
		}
		out[kid] = TradeNote{thesis, tag, grade, kid}
	}
	return out
}

func (s *Store) TradeNotes() map[string]TradeNote {
	raw := s.GetMeta("trade_notes")
	if raw == "" {
		return map[string]TradeNote{}
	}
	var data any
	if json.Unmarshal([]byte(raw), &data) != nil {
		return map[string]TradeNote{}
	}
	return cleanTradeNotes(data)
}

func (s *Store) SaveTradeNotes(notes any) map[string]TradeNote {
	clean := cleanTradeNotes(notes)
	b, _ := json.Marshal(clean)
	s.SetMeta("trade_notes", string(b))
	return clean
}

type Snapshot struct {
	Activities   []Activity            `json:"activities"`
	Accounts     []Account             `json:"accounts"`
	Balances     []Balance             `json:"balances"`
	Margin       []Margin              `json:"margin"`
	Exposures    map[string]Exposure   `json:"exposures"`
	Watchlist    []Watch               `json:"watchlist"`
	News         []NewsItem            `json:"news"`
	Universes    map[string][]Universe `json:"universes"`
	NavHistory   []NavPoint            `json:"navHistory"`
	NavByAccount map[string][]NavPoint `json:"navByAccount"`
	SyncedAt     string                `json:"syncedAt"`
	TradeGroups  []TradeGroup          `json:"tradeGroups"`
	Notes        map[string]TradeNote  `json:"notes"`
	Tiles        []Tile                `json:"tiles"`
	TilesSaved   bool                  `json:"-"`
	Securities   []Security            `json:"securities"`
}

func (s *Store) Snapshot(withActivities bool) Snapshot {
	_ = s.Ensure()
	snap := Snapshot{Activities: []Activity{}, Accounts: []Account{}, Balances: []Balance{}, Margin: []Margin{}, NavHistory: []NavPoint{}, NavByAccount: map[string][]NavPoint{}, Securities: []Security{}}
	if withActivities {
		snap.Activities, _ = s.allActivities()
		if snap.Activities == nil {
			snap.Activities = []Activity{}
		}
	}
	for _, r := range mustRows(s.queryMaps("SELECT * FROM accounts ORDER BY id")) {
		snap.Accounts = append(snap.Accounts, Account{ID: str(r["id"]), Nickname: str(r["nickname"]), UnifiedAccountType: str(r["unified_account_type"]), Currency: str(r["currency"]), Status: str(r["status"]), Type: str(r["type"]), NetLiquidationValue: fnum(r["net_liquidation_value"]), MarginAccountID: str(r["margin_account_id"])})
	}
	for _, r := range mustRows(s.queryMaps("SELECT * FROM balances")) {
		snap.Balances = append(snap.Balances, Balance{AccountID: str(r["account_id"]), CustodianAccountID: str(r["custodian_account_id"]), SecurityID: str(r["security_id"]), Quantity: fnum(r["quantity"])})
	}
	for _, r := range mustRows(s.queryMaps("SELECT * FROM margin ORDER BY account_id")) {
		ccy := str(r["currency"])
		if ccy == "" {
			ccy = "CAD"
		}
		snap.Margin = append(snap.Margin, Margin{AccountID: str(r["account_id"]), BuyingPower: fnum(r["buying_power"]), Currency: ccy, Unavailable: str(r["unavailable"]), FetchedAt: str(r["fetched_at"])})
	}
	for _, r := range mustRows(s.queryMaps("SELECT * FROM nav_history ORDER BY account_id, date")) {
		rec := navPointFromRow(r)
		aid := str(r["account_id"])
		if aid == "" {
			snap.NavHistory = append(snap.NavHistory, rec)
		} else {
			snap.NavByAccount[aid] = append(snap.NavByAccount[aid], rec)
		}
	}
	snap.SyncedAt = s.GetMeta("synced_at")
	snap.TradeGroups = s.TradeGroups()
	snap.Notes = s.TradeNotes()
	for _, r := range mustRows(s.queryMaps("SELECT * FROM securities ORDER BY id")) {
		snap.Securities = append(snap.Securities, securityFromRow(r))
	}
	snap.Exposures = map[string]Exposure{}
	for _, r := range mustRows(s.queryMaps("SELECT * FROM exposures")) {
		snap.Exposures[str(r["key"])] = exposureFromRow(r)
	}
	snap.Watchlist = []Watch{}
	for _, r := range mustRows(s.queryMaps("SELECT * FROM watchlist ORDER BY added_at, symbol")) {
		snap.Watchlist = append(snap.Watchlist, watchFromRow(r))
	}
	snap.News = []NewsItem{}
	for _, r := range mustRows(s.queryMaps("SELECT * FROM news ORDER BY published_at DESC, id")) {
		snap.News = append(snap.News, newsFromRow(r))
	}
	snap.Universes = s.universes()
	snap.Tiles, snap.TilesSaved = s.Tiles()
	return snap
}

func mustRows(rows []map[string]any, err error) []map[string]any {
	if err != nil {
		return nil
	}
	return rows
}

type Tile struct {
	Symbol   string `json:"symbol"`
	Exchange string `json:"exchange"`
}

func tilesFrom(raw string) ([]Tile, bool) {
	if raw == "" {
		return nil, false
	}
	var rows any
	if json.Unmarshal([]byte(raw), &rows) != nil {
		return nil, false
	}
	out := []Tile{}
	list, _ := rows.([]any)
	for _, r := range list {
		m, ok := r.(map[string]any)
		if !ok {
			continue
		}
		sym := strings.TrimSpace(py.S(m["symbol"]))
		if sym == "" {
			continue
		}
		out = append(out, Tile{Symbol: strings.ToUpper(sym), Exchange: strings.ToUpper(strings.TrimSpace(py.S(m["exchange"])))})
	}
	return out, true
}

func (s *Store) Tiles() ([]Tile, bool) {
	return tilesFrom(s.GetMeta(TilesMeta))
}

func (s *Store) SaveTiles(rows []Tile) []Tile {
	clean := []Tile{}
	for _, r := range rows {
		sym := strings.TrimSpace(r.Symbol)
		if sym == "" {
			continue
		}
		clean = append(clean, Tile{Symbol: strings.ToUpper(sym), Exchange: strings.ToUpper(strings.TrimSpace(r.Exchange))})
	}
	b, _ := json.Marshal(clean)
	s.SetMeta(TilesMeta, string(b))
	return clean
}

func SortedKeys[V any](m map[string]V) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	return keys
}
