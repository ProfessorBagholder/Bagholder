package fixtures

import (
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

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

func Buy(id, symbol string, qty, px float64, day string, extra ...func(*store.Activity)) store.Activity {
	a := store.Activity{ID: id, Category: "trade", ActivityType: "Trade", ActivitySubType: "BUY", RawType: "DIY_BUY", Quantity: qty, UnitPrice: px, NetCashAmount: -qty * px, TransactionDate: day, Symbol: symbol, Currency: "CAD"}
	for _, f := range extra {
		f(&a)
	}
	return Act(a)
}

func Sell(id, symbol string, qty, px float64, day string, extra ...func(*store.Activity)) store.Activity {
	a := store.Activity{ID: id, Category: "trade", ActivityType: "Trade", ActivitySubType: "SELL", RawType: "DIY_SELL", Quantity: -qty, UnitPrice: px, NetCashAmount: qty * px, TransactionDate: day, Symbol: symbol, Currency: "CAD"}
	for _, f := range extra {
		f(&a)
	}
	return Act(a)
}

func STO(id, symbol string, qty, px float64, day string, extra ...func(*store.Activity)) store.Activity {
	a := store.Activity{ID: id, Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -qty, UnitPrice: px, NetCashAmount: qty * px * 100, TransactionDate: day, Symbol: symbol}
	for _, f := range extra {
		f(&a)
	}
	return Act(a)
}

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

func Multileg(id, symbol string, cash float64, day string) store.Activity {
	return Act(store.Activity{ID: id, ActivityType: "OPTIONS_MULTILEG", ActivitySubType: "FILLED", RawType: "OPTIONS_MULTILEG", Quantity: 0, NetCashAmount: cash, TransactionDate: day, Symbol: symbol})
}

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

func WithAccount(nick, id string) func(*store.Activity) {
	return func(a *store.Activity) {
		a.AccountType = nick
		if id != "" {
			a.AccountID = id
		}
	}
}

func WithCurrency(ccy string) func(*store.Activity) {
	return func(a *store.Activity) { a.Currency = ccy }
}

func WithSecurity(sid string) func(*store.Activity) {
	return func(a *store.Activity) { a.SecurityID = sid }
}

func WithSource(src string) func(*store.Activity) {
	return func(a *store.Activity) { a.Source = src }
}
