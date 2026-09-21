package market

import "testing"

func TestTMXQuoteSymbolMapping(t *testing.T) {
	eq(t, TMXQuoteSymbol("CCHI", "TSX", "CAD"), "CCHI", "")
	eq(t, TMXQuoteSymbol("CH", "TSX-V", "CAD"), "CH", "")
	eq(t, TMXQuoteSymbol("LUNR", "NASDAQ", "USD"), "LUNR:US", "")
	eq(t, TMXQuoteSymbol("ASTS", "", "USD"), "ASTS:US", "")
	eq(t, TMXQuoteSymbol("HBIX", "Cboe Canada", "CAD"), "HBIX:AQL", "")
	eq(t, TMXQuoteSymbol("QIMC", "CSE", "CAD"), "QIMC:CNX", "TMX names CSE listings with :CNX")
	eq(t, TMXQuoteSymbol("VOD", "LSE", "GBP"), "", "a currency TMX does not carry")
	eq(t, TMXQuoteSymbol("ONE", "ALPHA EXCHANGE", "CAD"), "ONE", "an ATS venue: the currency's usual form, settled by tmx_lookup")
	eq(t, TMXQuoteSymbol("QNC 20NOV26 3.00 CALL", "", "USD"), "", "")
}

func TestQuoteSourcesCoverEveryHeldKind(t *testing.T) {
	src := func(r Rec) any {
		source, key, ok := QuoteSource(r)
		if !ok {
			return nil
		}
		return [2]string{source, key}
	}
	eq(t, src(rec("VEQT", "TSX", "CAD", "Shares")), [2]string{"tmx", "VEQT"}, "")
	eq(t, src(rec("LUNR", "NASDAQ", "USD", "Shares")), [2]string{"yahoo_quote", "LUNR"}, "a US listing is quoted where its quote is live: TMX stamps one fifteen minutes behind")
	eq(t, src(rec("HBIX", "Cboe Canada", "CAD", "Shares")), [2]string{"cboe_ca", "HBIX"}, "")
	eq(t, src(rec("BTC", "Crypto", "CAD", "Crypto")), [2]string{"coinbase", "BTC-CAD"}, "")
	eq(t, src(rec("BTC", "Crypto", "USD", "Crypto")), [2]string{"coinbase", "BTC-USD"}, "")
	eq(t, src(rec("QNC 20NOV26 3.00 CALL", "NYSE", "USD", "Options")), [2]string{"cboe_options", "QNC261120C00003000"}, "")
	eq(t, src(rec("SHOP 17OCT25 100.00 PUT", "TSX", "CAD", "Options")), nil, "")
	eq(t, OccCode("LUNR 29AUG25 11.50 CALL"), "LUNR250829C00011500", "")
	eq(t, OccCode("SPY 251219P00450000"), "SPY251219P00450000", "")
	eq(t, OccCode("VEQT"), "", "")
	eq(t, OccRoot("QNC261120C00003000"), "QNC", "")
}

func TestTMXSymbolFormIsResolvedByVenueAndRemembered(t *testing.T) {
	c, s := newTestClient(t, utc(2026, 9, 7, 12, 0, 0))
	venues := map[string]string{"QIMC:CNX": "Canadian Securities Exchange", "HBIX:AQL": "NEO-L (Cboe Canada Listed)", "HG:US": "New York Stock Exchange"}
	s.set(func(call stubCall) (int, string, error) {
		p, ok := parseTMX(call.Body)
		if !ok {
			return 0, "", errNoRoute
		}
		sym := p.Sym()
		venue, listed := venues[sym]
		if p.Op == "getQuoteBySymbol" {
			if listed {
				return 200, jsonText(obj("data", obj("getQuoteBySymbol", obj("symbol", sym, "exchangeName", venue, "price", 1.0)))), nil
			}
			return 200, jsonText(obj("data", obj("getQuoteBySymbol", nil))), nil
		}
		if listed {
			return 200, tmxSeries(obj("dateTime", "2026-02-02T16:00:00-05:00", "open", 1, "high", 1, "low", 1, "close", 1, "volume", 1)), nil
		}
		return 200, tmxSeries(), nil
	})
	r := rec("QIMC", "", "CAD", "Shares")
	bars, _ := c.FetchHistory(r, "2026-02-01", "2026-02-03")
	eq(t, len(bars), 1, "")
	eq(t, s.tmxSymbols("getTimeSeriesData"), []string{"QIMC", "QIMC:CNX"}, "")
	eq(t, s.tmxSymbols("getQuoteBySymbol"), []string{"QIMC", "QIMC:CNX"}, "the bare form is probed first, the CSE form answers")
	eq(t, c.TMXRemembered("QIMC"), "QIMC:CNX", "")
	s.reset()
	c.FetchHistory(r, "2026-02-01", "2026-02-03")
	eq(t, s.tmxOps(), [][2]string{{"getTimeSeriesData", "QIMC:CNX"}}, "remembered: no probing, straight to the right form")
	s.reset()
	hg := rec("HG", "CSE", "CAD", "Shares")
	first, _ := c.FetchHistory(hg, "2026-02-01", "2026-02-03")
	eq(t, len(first), 0, "")
	second, _ := c.FetchHistory(hg, "2026-02-01", "2026-02-03")
	eq(t, len(second), 0, "")
	eq(t, s.tmxSymbols("getQuoteBySymbol"), []string{"HG:CNX", "HG", "HG:AQL"}, "only the forms for the record's currency, once")
	eq(t, c.TMXRemembered("HG"), "HG", "")
	s.reset()
	q := c.FetchTMXQuote("QIMC")
	if q == nil {
		t.Fatal("no quote")
	}
	eq(t, q.Exchange, "Canadian Securities Exchange", "")
	eq(t, s.tmxOps(), [][2]string{{"getQuoteBySymbol", "QIMC:CNX"}}, "")
}
