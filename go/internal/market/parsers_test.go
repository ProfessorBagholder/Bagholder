package market

import (
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

func TestParsers(t *testing.T) {
	boc := jsonText(obj("observations", []any{obj("d", "2026-08-28", "FXUSDCAD", obj("v", "1.3888")), obj("d", "x")}))
	eq(t, ParseBocJSON(boc), map[string]float64{"2026-08-28": 1.3888}, "")
	fred := "observation_date,SP500\n2026-08-28,6500.12\n2026-08-29,.\nbad\n"
	eq(t, ParseFredCSV(fred), map[string]float64{"2026-08-28": 6500.12}, "")
	stooq := "Date,Open,High,Low,Close,Volume\n2026-08-28,1,2,0,6501.5,0\n"
	eq(t, ParseStooqCSV(stooq), map[string]float64{"2026-08-28": 6501.5}, "")
}

func TestPublicQuoteParsers(t *testing.T) {
	closed := jsonText(obj("data", obj("last", "0.0", "prev_close", "6.7600", "change", "0.0", "change_pct", "0.0", "company_name", "HARVEST BITCOIN ENHANCED INCOME ETF")))
	eq(t, price(ParseCboeCaQuote(closed)), 6.76, "")
	open := jsonText(obj("data", obj("last", "6.81", "prev_close", "6.7600", "change", "0.05", "change_pct", "0.74")))
	q := ParseCboeCaQuote(open)
	if q == nil {
		t.Fatal("no quote")
	}
	eq(t, []any{fv(q.Price), fv(q.PrevClose), fv(q.PriceChange)}, []any{6.81, 6.76, 0.05}, "")
	eq(t, ParseCboeCaQuote(jsonText(obj("data", obj("last", "0", "prev_close", "0")))), (*store.Quote)(nil), "")
	coin := ParseCoinbase(jsonText(obj("data", obj("amount", "109300.3", "base", "BTC", "currency", "CAD"))), "BTC-CAD")
	if coin == nil {
		t.Fatal("no coinbase quote")
	}
	eq(t, *coin, store.Quote{Price: ptr(109300.3), Currency: "CAD"}, "")
	eq(t, ParseCoinbase(jsonText(obj("errors", []any{obj("id", "not_found")})), "XYZ-CAD"), (*store.Quote)(nil), "")
	chain := jsonText(obj("data", obj("options", []any{
		obj("option", "QNC261120C00003000", "bid", 0.0, "ask", 0.25, "last_trade_price", 0.15, "prev_day_close", 0.15),
		obj("option", "QNC261120C00005000", "bid", 0.1, "ask", 0.2, "last_trade_price", 0.05, "prev_day_close", 0.12),
	})))
	rows := ParseCboeOptions(chain)
	eq(t, price(OptionMark(rows["QNC261120C00003000"])), 0.15, "")
	near(t, price(OptionMark(rows["QNC261120C00005000"])), 0.15, "")
	eq(t, OptionMark(rows["QNC261120C00009000"]), (*store.Quote)(nil), "")
	eq(t, OptionMark(obj("bid", 0, "ask", 0, "last_trade_price", 0, "prev_day_close", 0)), (*store.Quote)(nil), "")
}

func TestTMXParsers(t *testing.T) {
	q := obj("data", obj("getQuoteBySymbol", obj("symbol", "CCHI", "name", "Ninepoint Cameco HighShares ETF", "price", 10.95, "dividendFrequency", nil, "dividendYield", 27.5, "dividendAmount", 0.135, "exDividendDate", "2026-09-15 00:00:00.0")))
	r := ParseTMXQuote(q)
	if r == nil {
		t.Fatal("no quote")
	}
	eq(t, fv(r.Price), 10.95, "")
	eq(t, r.ExDividendDate, "2026-09-15", "")
	d := obj("data", obj("dividends", obj("dividends", []any{
		obj("exDate", "2026-09-15", "payableDate", "2026-09-21", "amount", 0.135, "currency", "CAD"),
		obj("exDate", "bad", "amount", 1),
		obj("exDate", "2026-08-31", "payableDate", "2026-09-04", "amount", "0.135"),
	})))
	rows := ParseTMXDividends(d)
	ex := []string{}
	for _, row := range rows {
		ex = append(ex, row.ExDate)
	}
	eq(t, ex, []string{"2026-09-15", "2026-08-31"}, "")
	eq(t, TMXSymbol("cchi.to"), "CCHI", "")
	eq(t, IsCanadianListing("TSX", "CAD"), true, "")
	eq(t, IsCanadianListing("NASDAQ", "USD"), false, "")
	eq(t, IsCanadianListing("", "CAD"), true, "")
}

func TestHistoryParsersAndSources(t *testing.T) {
	tmx := obj("data", obj("getTimeSeriesData", []any{
		obj("dateTime", "2026-09-04T16:00:00-04:00", "open", 4.8, "high", 4.8, "low", 4.68, "close", 4.75, "volume", 50972),
		obj("dateTime", "2026-09-03T16:00:00-04:00", "open", 4.83, "high", 4.95, "low", 4.73, "close", 4.75, "volume", 115702),
	}))
	bars := ParseTMXHistory(tmx)
	eq(t, dates(bars), []string{"2026-09-03", "2026-09-04"}, "")
	eq(t, bars[1].Close, 4.75, "")
	candles := jsonText([]any{[]any{1787097600, 63000.5, 65341.83, 64848.68, 63911.88, 6197.03}, []any{1787011200, 62000, 64000, 63000, 63500, 100}, []any{"bad"}})
	eq(t, barRows(ParseCoinbaseCandles(candles)), [][]any{{int64(1787011200), 63000.0, 64000.0, 62000.0, 63500.0, 100.0}, {int64(1787097600), 64848.68, 65341.83, 63000.5, 63911.88, 6197.03}}, "")
	src := func(r Rec) any {
		c := HistorySource(r)
		if c == nil {
			return nil
		}
		return [2]string{c.Source, c.Key}
	}
	eq(t, src(rec("RDDY", "TSX", "CAD", "Shares")), [2]string{"tmx", "RDDY"}, "")
	eq(t, src(rec("LUNR", "NASDAQ", "USD", "Shares")), [2]string{"tmx", "LUNR:US"}, "bars are a closed record, so a US listing's history stays TMX's; only the live quote moved")
	eq(t, src(rec("HBIX", "Cboe Canada", "CAD", "Shares")), [2]string{"tmx", "HBIX:AQL"}, "history from TMX even where the quote comes from Cboe")
	eq(t, src(rec("ONE", "Alpha Exchange", "CAD", "Shares")), [2]string{"tmx", "ONE"}, "an unknown venue starts from the currency's usual form")
	eq(t, src(rec("ASTS", "", "USD", "Shares")), [2]string{"tmx", "ASTS:US"}, "")
	eq(t, src(rec("BTC", "Crypto", "CAD", "Crypto")), [2]string{"coinbase", "BTC-CAD"}, "")
	eq(t, src(rec("QNC 20NOV26 3.00 CALL", "NYSE", "USD", "Options")), nil, "")
}
