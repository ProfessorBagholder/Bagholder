package store

import (
	"reflect"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

func mapRowToActivity(r map[string]any) Activity {
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

func mapNavPoint(r map[string]any) (string, NavPoint) {
	ccy := str(r["currency"])
	if ccy == "" {
		ccy = "CAD"
	}
	return str(r["account_id"]), NavPoint{Date: str(r["date"]), Equity: py.Deref(fnum(r["equity"]), 0), Currency: ccy, NetDeposits: fnum(r["net_deposits"])}
}

func TestTypedScanMatchesMapPath(t *testing.T) {
	s := temp(t)
	inserts := []string{
		"INSERT INTO activities (id, canonical_id, occurred_at, transaction_date, settlement_date, account_id, book_id, fifo_id, account_type, activity_type, activity_sub_type, description, direction, symbol, name, currency, quantity, unit_price, commission, net_cash_amount, category, balance, source, raw_type, aft_type, counter_symbol, security_id) VALUES ('a-null', NULL, NULL, '2024-01-02', NULL, 'acct', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
		"INSERT INTO activities (id, canonical_id, occurred_at, transaction_date, settlement_date, account_id, book_id, fifo_id, account_type, activity_type, activity_sub_type, description, direction, symbol, name, currency, quantity, unit_price, commission, net_cash_amount, category, balance, source, raw_type, aft_type, counter_symbol, security_id) VALUES ('a-empty', '', '', '2024-01-03', '', '', '', '', '', '', '', '', '', '', '', '', 0, 0, 0, 0, '', 0, '', '', '', '', '')",
		"INSERT INTO activities (id, canonical_id, occurred_at, transaction_date, settlement_date, account_id, book_id, fifo_id, account_type, activity_type, activity_sub_type, description, direction, symbol, name, currency, quantity, unit_price, commission, net_cash_amount, category, balance, source, raw_type, aft_type, counter_symbol, security_id) VALUES ('a-full', 'cid-1', '2024-01-04T10:00:00Z', '2024-01-04', '2024-01-06', 'acct', 'book', 'fifo', 'Trading', 'Trade', 'BUY', 'desc', 'DEBIT', 'AAA', 'Aaa Inc', 'USD', 10.5, 1.25, 0.99, -14.115, 'trade', 1234.5678, 'wealthsimple', 'DIY_BUY', 'aft', 'BBB', 'sec-1')",
		"INSERT INTO activities (id, canonical_id, occurred_at, transaction_date, settlement_date, account_id, book_id, fifo_id, account_type, activity_type, activity_sub_type, description, direction, symbol, name, currency, quantity, unit_price, commission, net_cash_amount, category, balance, source, raw_type, aft_type, counter_symbol, security_id) VALUES ('a-int', 'cid-2', '2024-01-05T10:00:00Z', '2024-01-05', '2024-01-05', 'acct', 'acct', 'acct', 'Trading', 'Trade', 'SELL', 'desc', 'CREDIT', 'AAA', 'Aaa Inc', 'CAD', -3, 2, 0, 6, 'trade', 7, 'manual', '', '', '', NULL)",
		"INSERT INTO nav_history (account_id, date, equity, currency, net_deposits) VALUES ('', '2024-01-02', NULL, NULL, NULL)",
		"INSERT INTO nav_history (account_id, date, equity, currency, net_deposits) VALUES ('acct', '2024-01-02', 0, '', 0)",
		"INSERT INTO nav_history (account_id, date, equity, currency, net_deposits) VALUES ('acct', '2024-01-03', 1500.25, 'USD', 1000)",
	}
	for _, q := range inserts {
		if _, err := s.exec(q); err != nil {
			t.Fatal(err)
		}
	}
	maps, err := s.queryMaps("SELECT * FROM activities ORDER BY COALESCE(occurred_at, transaction_date) ASC, id ASC")
	if err != nil {
		t.Fatal(err)
	}
	want := make([]Activity, 0, len(maps))
	for _, r := range maps {
		want = append(want, mapRowToActivity(r))
	}
	got, err := s.allActivities()
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 4 || !reflect.DeepEqual(got, want) {
		t.Fatalf("typed activities differ from map path\n got %+v\nwant %+v", got, want)
	}
	byID := map[string]Activity{}
	for _, a := range got {
		byID[a.ID] = a
	}
	nulls, empties, full := byID["a-null"], byID["a-empty"], byID["a-full"]
	if nulls.Balance != nil || empties.Balance == nil || *empties.Balance != 0 || *full.Balance != 1234.5678 || nulls.SettlementDate != "2024-01-02" || nulls.BookID != "acct" || empties.BookID != "" {
		t.Fatalf("null and empty handling: %+v %+v", nulls, empties)
	}
	navMaps, err := s.queryMaps("SELECT * FROM nav_history ORDER BY account_id, date")
	if err != nil {
		t.Fatal(err)
	}
	snap := s.Snapshot(false)
	wantHistory := []NavPoint{}
	wantByAccount := map[string][]NavPoint{}
	for _, r := range navMaps {
		aid, rec := mapNavPoint(r)
		if aid == "" {
			wantHistory = append(wantHistory, rec)
		} else {
			wantByAccount[aid] = append(wantByAccount[aid], rec)
		}
	}
	if !reflect.DeepEqual(snap.NavHistory, wantHistory) || !reflect.DeepEqual(snap.NavByAccount, wantByAccount) {
		t.Fatalf("typed nav differs from map path\n got %+v %+v\nwant %+v %+v", snap.NavHistory, snap.NavByAccount, wantHistory, wantByAccount)
	}
	if snap.NavHistory[0].NetDeposits != nil || snap.NavHistory[0].Currency != "CAD" || *snap.NavByAccount["acct"][0].NetDeposits != 0 {
		t.Fatalf("nav null handling: %+v %+v", snap.NavHistory, snap.NavByAccount)
	}
}
