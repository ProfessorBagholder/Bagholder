// Package fixtures builds the activity rows the tests share, the way tests/test_model.py did.
package fixtures

import (
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

// Act is one activity with the test defaults; fields set on the argument override them.
func Act(o store.Activity) store.Activity {
	base := store.Activity{
		ID: o.ID, AccountID: "acct-1", AccountType: "Trading", Symbol: "LUNR 15JAN27 12.00 CALL", Name: "LUNR", Currency: "USD",
		Category: "other", TransactionDate: "2026-01-01",
	}
	if o.AccountID != "" {
		base.AccountID = o.AccountID
	}
	if o.AccountType != "" {
		base.AccountType = o.AccountType
	}
	if o.Symbol != "" {
		base.Symbol = o.Symbol
	}
	if o.Name != "" {
		base.Name = o.Name
	}
	if o.Currency != "" {
		base.Currency = o.Currency
	}
	if o.Category != "" {
		base.Category = o.Category
	}
	if o.TransactionDate != "" {
		base.TransactionDate = o.TransactionDate
	}
	base.ActivityType, base.ActivitySubType, base.RawType = o.ActivityType, o.ActivitySubType, o.RawType
	base.Quantity, base.UnitPrice, base.NetCashAmount, base.Commission = o.Quantity, o.UnitPrice, o.NetCashAmount, o.Commission
	base.OccurredAt, base.SecurityID, base.Direction, base.Source, base.Description = o.OccurredAt, o.SecurityID, o.Direction, o.Source, o.Description
	base.CanonicalID, base.BookID, base.FifoID, base.AftType, base.CounterSymbol, base.SettlementDate = o.CanonicalID, o.BookID, o.FifoID, o.AftType, o.CounterSymbol, o.SettlementDate
	base.Balance, base.Kind, base.Flags = o.Balance, o.Kind, o.Flags
	if base.OccurredAt == "" {
		base.OccurredAt = base.TransactionDate + "T15:00:00+00:00"
	}
	return base
}

// Buy is a share buy.
func Buy(id, symbol string, qty, px float64, day string, extra ...func(*store.Activity)) store.Activity {
	a := store.Activity{ID: id, Category: "trade", ActivityType: "Trade", ActivitySubType: "BUY", RawType: "DIY_BUY", Quantity: qty, UnitPrice: px, NetCashAmount: -qty * px, TransactionDate: day, Symbol: symbol, Currency: "CAD"}
	for _, f := range extra {
		f(&a)
	}
	return Act(a)
}

// Sell is a share sell.
func Sell(id, symbol string, qty, px float64, day string, extra ...func(*store.Activity)) store.Activity {
	a := store.Activity{ID: id, Category: "trade", ActivityType: "Trade", ActivitySubType: "SELL", RawType: "DIY_SELL", Quantity: -qty, UnitPrice: px, NetCashAmount: qty * px, TransactionDate: day, Symbol: symbol, Currency: "CAD"}
	for _, f := range extra {
		f(&a)
	}
	return Act(a)
}

// STO is an option sold to open.
func STO(id, symbol string, qty, px float64, day string, extra ...func(*store.Activity)) store.Activity {
	a := store.Activity{ID: id, Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -qty, UnitPrice: px, NetCashAmount: qty * px * 100, TransactionDate: day, Symbol: symbol}
	for _, f := range extra {
		f(&a)
	}
	return Act(a)
}

// BTC is an option bought (to close by default).
func BTC(id, symbol string, qty, px float64, day, sub string, extra ...func(*store.Activity)) store.Activity {
	if sub == "" {
		sub = "BUYTOCLOSE"
	}
	a := store.Activity{ID: id, Category: "trade", ActivityType: "OPTIONS_BUY", ActivitySubType: sub, RawType: "OPTIONS_BUY", Quantity: qty, UnitPrice: px, NetCashAmount: -qty * px * 100, TransactionDate: day, Symbol: symbol}
	for _, f := range extra {
		f(&a)
	}
	return Act(a)
}

// Multileg is a Wealthsimple multileg fill as posted: quantity 0, only the cash.
func Multileg(id, symbol string, cash float64, day string) store.Activity {
	return Act(store.Activity{ID: id, ActivityType: "OPTIONS_MULTILEG", ActivitySubType: "FILLED", RawType: "OPTIONS_MULTILEG", Quantity: 0, NetCashAmount: cash, TransactionDate: day, Symbol: symbol})
}

// Dividend is a cash dividend row.
func Dividend(id, symbol string, qty, per float64, day, account string) store.Activity {
	if account == "" {
		account = "Cashflow"
	}
	cash := float64(int64(qty*per*100+0.5)) / 100
	if qty*per < 0 {
		cash = -float64(int64(-qty*per*100+0.5)) / 100
	}
	return Act(store.Activity{ID: id, Category: "dividend", ActivityType: "Dividend", RawType: "DIVIDEND", Quantity: qty, UnitPrice: per, NetCashAmount: cash, TransactionDate: day, Symbol: symbol, Currency: "CAD", AccountType: account})
}

// Crypto is a coin bought, sold or rewarded.
func Crypto(id, kind, symbol string, qty, px float64, day, account string) store.Activity {
	if account == "" {
		account = "Crypto"
	}
	raw := map[string]string{"buy": "CRYPTO_BUY", "sell": "CRYPTO_SELL", "reward": "CRYPTO_STAKING_REWARD"}[kind]
	sub := "MARKET_ORDER"
	if kind == "reward" {
		sub = "other"
	}
	return Act(store.Activity{ID: id, ActivityType: raw, ActivitySubType: sub, RawType: raw, Quantity: qty, UnitPrice: px, NetCashAmount: qty * px, TransactionDate: day, Symbol: symbol, Currency: "CAD", AccountType: account})
}

// CryptoTransfer is a coin moved in or out.
func CryptoTransfer(id, symbol string, qty, value float64, day string, out bool, account string) store.Activity {
	if account == "" {
		account = "Crypto"
	}
	sub, dir, cash := "TRANSFER_IN", "CREDIT", value
	if out {
		sub, dir, cash = "TRANSFER_OUT", "DEBIT", -value
	}
	return Act(store.Activity{ID: id, ActivityType: "CRYPTO_TRANSFER", ActivitySubType: sub, RawType: "CRYPTO_TRANSFER", Direction: dir, Quantity: qty, UnitPrice: value / qty, NetCashAmount: cash, TransactionDate: day, Symbol: symbol, Currency: "CAD", AccountType: account})
}

// WithAccount sets the nickname (and optionally the id) of a row.
func WithAccount(nick, id string) func(*store.Activity) {
	return func(a *store.Activity) {
		a.AccountType = nick
		if id != "" {
			a.AccountID = id
		}
	}
}

// WithCurrency sets a row's currency.
func WithCurrency(ccy string) func(*store.Activity) {
	return func(a *store.Activity) { a.Currency = ccy }
}

// WithSecurity sets a row's security id.
func WithSecurity(sid string) func(*store.Activity) {
	return func(a *store.Activity) { a.SecurityID = sid }
}

// WithSource sets a row's source.
func WithSource(src string) func(*store.Activity) {
	return func(a *store.Activity) { a.Source = src }
}
