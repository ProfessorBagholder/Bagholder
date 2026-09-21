package store

import (
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

func ordersBase(t *testing.T) *Store {
	t.Helper()
	s := temp(t)
	s.ReplaceAccounts([]Account{
		{ID: "acct-margin", Nickname: "Trading", UnifiedAccountType: "SELF_DIRECTED_NON_REGISTERED_MARGIN", Currency: "CAD", Status: "open", Type: "non_registered"},
		{ID: "acct-tfsa", Nickname: "TFSA", UnifiedAccountType: "SELF_DIRECTED_TFSA", Currency: "CAD", Status: "open", Type: "tfsa", MarginAccountID: "acct-margin"},
		{ID: "acct-crypto", Nickname: "Crypto", UnifiedAccountType: "SELF_DIRECTED_CRYPTO", Currency: "CAD", Status: "open", Type: "crypto"},
		{ID: "acct-old", Nickname: "Old", UnifiedAccountType: "SELF_DIRECTED_RRSP", Currency: "CAD", Status: "closed", Type: "rrsp"},
		{ID: "acct-managed", Nickname: "Managed", UnifiedAccountType: "MANAGED_TFSA", Currency: "CAD", Status: "open", Type: "tfsa"},
	})
	s.UpsertSecurities([]Security{
		{ID: "sec-o-1", Symbol: "QNC", Currency: "USD", UnderlyingID: "sec-s-us"},
		{ID: "sec-s-us", Symbol: "QNC", Name: "Quantum Emotion Corp", PrimaryExchange: "NYSE", PrimaryMic: "XNYS", Currency: "USD"},
		{ID: "sec-s-ca", Symbol: "QNC.TO", Name: "Quantum Emotion Corp", PrimaryExchange: "TSX-V", PrimaryMic: "XTSX", Currency: "CAD"},
	})
	s.ReplaceMargin([]Margin{{AccountID: "acct-margin", BuyingPower: py.Ptr(12680.45), Currency: "CAD"}})
	return s
}

func ticketOrder(id string) Order {
	return Order{ID: id, AccountID: "acct-margin", Account: "Trading", SecurityID: "sec-s-us", Symbol: "QNC", Currency: "USD", Side: "BUY", Type: "LIMIT", Tif: "DAY", Quantity: 25, LimitPrice: py.Ptr(165.4),
		StopLoss: &StopLoss{Kind: "stop", Price: py.Ptr(157.13), TrailUnit: "pct"}, TakeProfit: &TakeProfit{Price: py.Ptr(181.94)}, Status: "dry",
		Request: map[string]any{"canonicalAccountId": "acct-margin", "executionType": "LIMIT", "orderType": "BUY_QUANTITY", "quantity": 25.0, "securityId": "sec-s-us", "timeInForce": "DAY", "limitPrice": 165.4, "externalId": id}}
}

func TestCollateralAccountNamesTheMarginAccountItBacks(t *testing.T) {
	s := ordersBase(t)
	s.ReplaceAccounts([]Account{
		{ID: "acct-margin", Nickname: "Trading", UnifiedAccountType: "SELF_DIRECTED_NON_REGISTERED_MARGIN", Currency: "CAD", Status: "open", Type: "non_registered", MarginAccountID: ""},
		{ID: "acct-tfsa", Nickname: "TFSA", UnifiedAccountType: "SELF_DIRECTED_TFSA", Currency: "CAD", Status: "open", Type: "tfsa", MarginAccountID: "acct-margin"},
		{ID: "acct-rrsp", Nickname: "RRSP", UnifiedAccountType: "SELF_DIRECTED_RRSP", Currency: "CAD", Status: "open", Type: "rrsp", MarginAccountID: ""},
		{ID: "acct-lira", Nickname: "LIRA", UnifiedAccountType: "SELF_DIRECTED_LIRA", Currency: "CAD", Status: "open", Type: "lira", MarginAccountID: ""},
	})
	kept := map[string]Account{}
	for _, a := range s.Snapshot(false).Accounts {
		kept[a.ID] = a
	}
	if kept["acct-tfsa"].MarginAccountID != "acct-margin" {
		t.Error("the link survives the store")
	}
	if kept["acct-rrsp"].MarginAccountID != "" || kept["acct-margin"].MarginAccountID != "" || kept["acct-lira"].MarginAccountID != "" {
		t.Errorf("%+v", kept)
	}
}

func TestTheOrdersTableSurvivesClearSyncedData(t *testing.T) {
	s := ordersBase(t)
	s.InsertOrder(ticketOrder("order-dry-1"))
	s.ClearSyncedData(false, false)
	rows := s.ListOrders(0)
	if len(rows) != 1 {
		t.Fatal("what was submitted is a record of the user's own actions, never cleared with the synced rows")
	}
	if rows[0].ID != "order-dry-1" {
		t.Errorf("%+v", rows[0])
	}
}
