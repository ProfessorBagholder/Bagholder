package store

import (
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

func fillLong(s *Store, cid string, qty, price float64) {
	s.ApplyWealthsimpleMapped([]Activity{{CanonicalID: cid, OccurredAt: "2026-09-01T14:00:00Z", TransactionDate: "2026-09-01", SettlementDate: "2026-09-01", AccountID: "acct-tfsa", BookID: "acct-tfsa", FifoID: "acct-tfsa", AccountType: "TFSA",
		ActivityType: "Trade", ActivitySubType: "BUY", Symbol: "QNC", Name: "QNC", Currency: "USD", Quantity: qty, UnitPrice: price, Commission: 0, NetCashAmount: -(qty * price), Category: "trade", Source: "wealthsimple"}})
}

func wsSell(cid string, qty, price float64, symbol, sub, atype string) Activity {
	return Activity{CanonicalID: cid, OccurredAt: "2026-09-10T20:47:00Z", TransactionDate: "2026-09-10", SettlementDate: "2026-09-10", AccountID: "acct-tfsa", BookID: "acct-tfsa", FifoID: "acct-tfsa", AccountType: "TFSA",
		ActivityType: atype, ActivitySubType: sub, Symbol: symbol, Name: symbol, Currency: "USD", Quantity: -qty, UnitPrice: price, Commission: 0, NetCashAmount: qty * price, Category: "trade", Source: "wealthsimple"}
}

func bookFill(t *testing.T, s *Store, symbol, securityID string, filled, price, mult float64) Activity {
	t.Helper()
	act, err := s.InsertLocal(Activity{ID: py.UUID4(), OccurredAt: "2026-09-10", TransactionDate: "2026-09-10", SettlementDate: "2026-09-10", AccountID: "acct-tfsa", BookID: "acct-tfsa", FifoID: "acct-tfsa", AccountType: "TFSA",
		ActivityType: "Trade", ActivitySubType: "SELL", Description: "Sell " + py.G(filled) + " " + symbol + " @ " + py.G(price), Direction: "CREDIT", Symbol: symbol, Name: symbol, Currency: "USD",
		Quantity: -filled, UnitPrice: price, Commission: 0, NetCashAmount: filled * price * mult, Category: "trade", SecurityID: securityID, Source: "bagholder-fill"})
	if err != nil {
		t.Fatal(err)
	}
	return act
}

func bookedFills(s *Store, symbol string) []Activity {
	var out []Activity
	for _, a := range s.Snapshot(true).Activities {
		if a.Source == "bagholder-fill" && a.Symbol == symbol {
			out = append(out, a)
		}
	}
	return out
}

func TestTheRealWealthsimpleSellCollapsesWithTheBookedRow(t *testing.T) {
	s := ordersBase(t)
	fillLong(s, "ws-buy-1", 5, 1.40)
	bookFill(t, s, "QNC", "sec-s-us", 5, 1.6374, 1)
	before := s.ActivityCount()
	if len(bookedFills(s, "QNC")) != 1 {
		t.Fatal("exactly one local activity for the fill")
	}
	result := s.ApplyWealthsimpleMapped([]Activity{wsSell("ws-sell-9", 5, 1.6374, "QNC", "SELL", "Trade")})
	if result.Linked != 1 || result.Inserted != 0 {
		t.Fatalf("the synced sell links to the booked row, none inserted: %+v", result)
	}
	if s.ActivityCount() != before {
		t.Error("no second sell: the position is not double-counted")
	}
	var sells []Activity
	for _, a := range s.Snapshot(true).Activities {
		if TradeSide(&a) == "SELL" && a.Symbol == "QNC" {
			sells = append(sells, a)
		}
	}
	if len(sells) != 1 {
		t.Fatalf("%+v", sells)
	}
	if sells[0].CanonicalID != "ws-sell-9" {
		t.Errorf("the booked row adopted Wealthsimple's canonical id: %+v", sells[0])
	}
	s.ApplyWealthsimpleMapped([]Activity{wsSell("ws-sell-9", 5, 1.6374, "QNC", "SELL", "Trade")})
	if s.ActivityCount() != before {
		t.Error("the linked row is known by its canonical id on the next sync too")
	}
}

func TestAnOptionFillNetsWithTheHundredTimesMultiplier(t *testing.T) {
	s := ordersBase(t)
	sym := "QNC 16JAN26 5.00 CALL"
	s.ApplyWealthsimpleMapped([]Activity{{CanonicalID: "ws-opt-buy", OccurredAt: "2026-09-01T14:00:00Z", TransactionDate: "2026-09-01", SettlementDate: "2026-09-01", AccountID: "acct-tfsa", BookID: "acct-tfsa", FifoID: "acct-tfsa",
		AccountType: "TFSA", ActivityType: "OPTIONS_BUY", ActivitySubType: "BUYTOOPEN", Symbol: sym, Name: sym, Currency: "USD", Quantity: 2, UnitPrice: 1.00, Commission: 0, NetCashAmount: -200.0, Category: "trade", Source: "wealthsimple"}})
	bookFill(t, s, sym, "sec-o-1", 2, 1.50, 100)
	booked := bookedFills(s, sym)
	if len(booked) != 1 {
		t.Fatalf("%+v", booked)
	}
	b := booked[0]
	if b.Quantity != -2.0 || b.UnitPrice != 1.50 {
		t.Errorf("contracts and per-share premium, as Wealthsimple stores them: %+v", b)
	}
	if !near(b.NetCashAmount, 2*1.50*100) {
		t.Errorf("the 100x multiplier is in the cash: %v", b.NetCashAmount)
	}
	before := s.ActivityCount()
	result := s.ApplyWealthsimpleMapped([]Activity{wsSell("ws-opt-sell", 2, 1.50, sym, "SELLTOCLOSE", "OPTIONS_SELL")})
	if result.Linked != 1 || result.Inserted != 0 {
		t.Fatalf("%+v", result)
	}
	if s.ActivityCount() != before {
		t.Error("the option position is not double-counted")
	}
}
