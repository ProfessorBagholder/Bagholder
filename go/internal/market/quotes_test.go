package market

import (
	"strings"
	"testing"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

func TestRefreshQuotesPricesCryptoAndOptions(t *testing.T) {
	now := utc(2026, 9, 6, 14, 0, 0)
	c, s := newTestClient(t, now)
	syms := []Rec{
		rec("BTC", "Crypto", "CAD", "Crypto"),
		rec("QNC 20NOV26 3.00 CALL", "NYSE", "USD", "Options"),
		rec("QNC 19FEB27 3.00 CALL", "NYSE", "USD", "Options"),
		rec("SHOP 17OCT25 100.00 PUT", "TSX", "CAD", "Options"),
	}
	chain := jsonText(obj("data", obj("options", []any{
		obj("option", "QNC261120C00003000", "bid", 0.1, "ask", 0.2, "prev_day_close", 0.15),
		obj("option", "QNC270219C00003000", "bid", 0, "ask", 0.5, "last_trade_price", 0.3, "prev_day_close", 0.3),
	})))
	s.set(func(call stubCall) (int, string, error) {
		switch {
		case strings.Contains(call.URL, "/v2/prices/BTC-CAD/spot"):
			return 200, jsonText(obj("data", obj("amount", "109300.3", "base", "BTC", "currency", "CAD"))), nil
		case strings.Contains(call.URL, "delayed_quotes/options/QNC.json"):
			return 200, chain, nil
		}
		return 0, "", errNoRoute
	})
	eq(t, c.RefreshQuotes(syms, now), 3, "")
	eq(t, s.urls("/v2/prices/"), []string{"https://api.coinbase.com/v2/prices/BTC-CAD/spot"}, "")
	eq(t, len(s.urls("delayed_quotes/options/")), 1, "one chain fetch serves every contract on the underlying")
	q := c.Store.Quotes()
	eq(t, fv(q["BTC"].Price), 109300.3, "")
	near(t, fv(q["QNC 20NOV26 3.00 CALL"].Price), 0.15, "")
	eq(t, fv(q["QNC 19FEB27 3.00 CALL"].Price), 0.3, "")
	_, has := q["SHOP 17OCT25 100.00 PUT"]
	eq(t, has, false, "")
}

func TestRefreshQuotesRespectsTheInterval(t *testing.T) {
	c, s := newTestClient(t, time.Time{})
	now := time.Now().UTC()
	syms := []Rec{rec("VEQT", "TSX", "CAD", ""), rec("LUNR", "NASDAQ", "USD", ""), rec("HBIX", "Cboe Canada", "CAD", "")}
	s.set(func(call stubCall) (int, string, error) {
		switch {
		case strings.Contains(call.URL, "finance.yahoo.com"):
			return 200, jsonText(obj("chart", obj("result", []any{obj("meta", obj("regularMarketPrice", 10.0, "chartPreviousClose", 9.9))}))), nil
		case call.URL == TMXURL:
			return 200, jsonText(obj("data", obj("getQuoteBySymbol", obj("symbol", "VEQT", "price", 10.0, "priceChange", 0.1, "percentChange", 1.0, "prevClose", 9.9)))), nil
		case strings.Contains(call.URL, "cboe.com/ca/equities"):
			return 200, jsonText(obj("data", obj("last", "6.76", "prev_close", "6.76"))), nil
		}
		return 0, "", errNoRoute
	})
	eq(t, c.RefreshQuotes(syms, now), 3, "")
	eq(t, s.tmxSymbols("getQuoteBySymbol"), []string{"VEQT"}, "the Canadian listing through TMX; the US one is Yahoo's")
	eq(t, s.urls("cboe.com/ca/equities"), []string{"https://www-api.cboe.com/ca/equities/securities-1/HBIX/quote/"}, "")
	eq(t, c.RefreshQuotes(syms, now.Add(30*time.Second)), 0, "")
	eq(t, c.RefreshQuotes(syms, now.Add(2*time.Minute)), 3, "")
	eq(t, fv(c.Store.Quotes()["HBIX"].Price), 6.76, "")
	q := c.Store.Quotes()["LUNR"]
	eq(t, fv(q.Price), 10.0, "")
	eq(t, fv(q.PrevClose), 9.9, "")
	c.Store.UpsertQuote("LUNR", store.Quote{Price: ptr(11.0), FetchedAt: "2026-09-06T15:00:00Z"}, "")
	eq(t, fv(c.Store.Quotes()["LUNR"].Price), 11.0, "")
}

func TestStoreRoundtripAndStaleDetection(t *testing.T) {
	old := utc(2026, 9, 8, 5, 0, 0)
	c, s := newTestClient(t, old)
	eq(t, c.Store.UpsertDistributions("cchi", []store.Distribution{{ExDate: "2026-08-31", PayDate: "2026-09-04", Amount: 0.135, Currency: "CAD"}, {ExDate: "x", Amount: 1}}, ""), 1, "")
	eq(t, c.Store.Distributions()["CCHI"][0].Amount, 0.135, "")
	c.Store.UpsertQuote("CCHI", store.Quote{Price: ptr(10.95), DividendAmount: ptr(0.135), FetchedAt: "2026-09-06T00:00:00Z"}, "")
	eq(t, fv(c.Store.Quotes()["CCHI"].Price), 10.95, "")
	syms := []Rec{rec("CCHI", "TSX", "CAD", ""), rec("LUNR", "NASDAQ", "USD", ""), rec("NEW", "", "CAD", "")}
	fresh := utc(2026, 9, 6, 5, 0, 0)
	eq(t, c.StaleSymbols(syms, fresh), []string{"CCHI", "NEW"}, "")
	c.Store.MarkDistributionsFetched("CCHI", "2026-09-06T00:00:00Z")
	eq(t, c.StaleSymbols(syms, fresh), []string{"NEW"}, "")
	eq(t, c.StaleSymbols(syms, old), []string{"CCHI", "NEW"}, "")
	c.Store.UpsertQuote("CCHI", store.Quote{Price: ptr(11.0), FetchedAt: "2026-09-08T04:55:00Z"}, "")
	eq(t, c.StaleSymbols(syms, old), []string{"CCHI", "NEW"}, "")
	s.set(func(call stubCall) (int, string, error) {
		p, ok := parseTMX(call.Body)
		if !ok {
			return 0, "", errNoRoute
		}
		if p.Op == "getQuoteBySymbol" {
			return 200, jsonText(obj("data", obj("getQuoteBySymbol", obj("price", 1.0, "dividendAmount", 0.1, "dividendFrequency", "Monthly", "exDividendDate", "2026-09-01")))), nil
		}
		return 200, jsonText(obj("data", obj("dividends", obj("dividends", []any{obj("exDate", "2026-09-01", "payableDate", "2026-09-05", "amount", 0.1, "currency", "CAD")})))), nil
	})
	eq(t, c.RefreshDistributions(syms, false, old), 2, "")
	eq(t, keysOf(c.Store.Quotes()), []string{"CCHI", "NEW"}, "")
	eq(t, keysOf(c.Store.DistributionsFetchedAt()), []string{"CCHI", "NEW"}, "")
	eq(t, len(s.tmxSymbols("getDividendsForSymbol")), 2, "")
	eq(t, c.StaleSymbols(syms, old), []string{}, "")
	eq(t, TMXRecordSymbol("HBIX", "CBOE CANADA"), "HBIX:AQL", "")
	eq(t, TMXRecordSymbol("HBIX", "NEO"), "HBIX:AQL", "")
	eq(t, TMXRecordSymbol("CCHI", "TSX"), "CCHI", "")
	cboe := []Rec{rec("HBIX", "CBOE CANADA", "CAD", "")}
	c.Store.UpsertQuote("HBIX", store.Quote{Price: ptr(6.76)}, "cboe_ca")
	s.reset()
	s.set(func(call stubCall) (int, string, error) {
		p, ok := parseTMX(call.Body)
		if !ok {
			return 0, "", errNoRoute
		}
		if p.Op == "getQuoteBySymbol" {
			return 200, jsonText(obj("data", obj("getQuoteBySymbol", obj("symbol", "HBIX:AQL", "price", 6.70, "exDividendDate", "2026-08-31 00:00:00.0", "dividendFrequency", "Monthly", "dividendAmount", 0.12)))), nil
		}
		return 200, jsonText(obj("data", obj("dividends", obj("dividends", []any{obj("exDate", "2026-08-31", "payableDate", "2026-09-04", "amount", 0.12, "currency", "CAD")})))), nil
	})
	eq(t, c.RefreshDistributions(cboe, false, old), 1, "")
	asked := []string{}
	for _, p := range s.tmxPosts() {
		asked = append(asked, p.Sym())
	}
	eq(t, uniqSorted(asked), []string{"HBIX:AQL"}, "")
	ex := []string{}
	for _, d := range c.Store.Distributions()["HBIX"] {
		ex = append(ex, d.ExDate)
	}
	eq(t, ex, []string{"2026-08-31"}, "")
	eq(t, fv(c.Store.Quotes()["HBIX"].Price), 6.76, "")
	_, has := c.Store.DistributionsFetchedAt()["HBIX"]
	eq(t, has, true, "")
}
