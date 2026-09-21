package market

import (
	"errors"
	"strings"
	"testing"
	"time"
)

func TestTimeframesAggregateAndReportAvailability(t *testing.T) {
	daily := []Daily{
		{Date: "2026-08-31", Open: ptr(1), High: ptr(3), Low: ptr(0.5), Close: 2, Volume: ptr(10)},
		{Date: "2026-09-01", Open: ptr(2), High: ptr(4), Low: ptr(1.5), Close: 3, Volume: ptr(10)},
		{Date: "2026-09-04", Open: ptr(3), High: ptr(3.5), Low: ptr(2), Close: 2.5, Volume: ptr(10)},
		{Date: "2026-09-08", Open: ptr(2.5), High: ptr(5), Low: ptr(2), Close: 4.5, Volume: ptr(10)},
	}
	weeks := AggregateDaily(daily, "1w")
	eq(t, dailyRows(weeks), [][]any{{"2026-08-31", 1.0, 4.0, 0.5, 2.5, 30.0}, {"2026-09-07", 2.5, 5.0, 2.0, 4.5, 10.0}}, "")
	months := AggregateDaily(daily, "1M")
	short := [][]any{}
	for _, m := range months {
		short = append(short, []any{m.Date, fv(m.Open), m.Close})
	}
	eq(t, short, [][]any{{"2026-08-01", 1.0, 2.0}, {"2026-09-01", 2.0, 4.5}}, "")
	hourly := []Bar{}
	for h := 1; h < 10; h++ {
		f := float64(h)
		hourly = append(hourly, Bar{Time: int64(3600 * h), Open: ptr(f), High: ptr(f + 0.5), Low: ptr(f - 0.5), Close: f, Volume: ptr(1)})
	}
	four := AggregateHourly(hourly, 14400)
	eq(t, barRows(four), [][]any{{int64(0), 1.0, 3.5, 0.5, 3.0, 3.0}, {int64(14400), 4.0, 7.5, 3.5, 7.0, 4.0}, {int64(28800), 8.0, 9.5, 7.5, 9.0, 2.0}}, "")
	now := utc(2026, 9, 7, 12, 0, 0)
	share := rec("RDDY", "TSX", "CAD", "Shares")
	coin := rec("BTC", "Crypto", "CAD", "Crypto")
	opt := rec("QNC 20NOV26 3.00 CALL", "NYSE", "USD", "Options")
	eq(t, AvailableTimeframes(share, "2024-01-01", now), []string{"1d", "1w", "1M"}, "")
	eq(t, AvailableTimeframes(coin, "2026-08-01", now), []string{"1h", "4h", "1d", "1w", "1M"}, "")
	eq(t, AvailableTimeframes(coin, "2019-01-01", now), []string{"1h", "4h", "1d", "1w", "1M"}, "Coinbase keeps hourly candles for good")
	eq(t, AvailableTimeframes(opt, "2026-08-01", now), []string{}, "")
}

func TestTMXMinutesBecomeSessionAlignedHourlyAndFourHourBars(t *testing.T) {
	row := func(hhmm string, o, h, l, c float64) map[string]any {
		return minuteRow("2025-11-10", hhmm, "-05:00", o, h, l, c, 1)
	}
	data := obj("data", obj("intraday", []any{
		row("09:30", 10, 11, 9, 10.5), row("09:31", 10.5, 12, 10, 11), row("10:29", 11, 11.5, 10.8, 11.2),
		row("10:30", 11.2, 11.3, 11.1, 11.25), row("13:29", 11.25, 11.4, 11.0, 11.3),
		row("13:30", 11.3, 11.6, 11.2, 11.5), row("15:59", 11.5, 11.7, 11.4, 11.6),
		minuteRow("2025-11-11", "09:30", "-05:00", 20, 21, 19, 20.5, 1),
	}))
	minutes := ParseTMXMinutes(data)
	eq(t, len(minutes), 8, "")
	eq(t, minutes[0].Minute, 9*60+30, "")
	eq(t, minutes[0].Time, utc(2025, 11, 10, 14, 30, 0).Unix(), "09:30 Eastern is 14:30 UTC")
	starts := func(bars []Bar) []string {
		out := []string{}
		for _, b := range bars {
			out = append(out, time.Unix(b.Time, 0).UTC().Format("01-02 15:04"))
		}
		return out
	}
	hourly := AggregateSession(minutes, 60)
	eq(t, starts(hourly), []string{"11-10 14:30", "11-10 15:30", "11-10 17:30", "11-10 18:30", "11-10 20:30", "11-11 14:30"}, "")
	first := hourly[0]
	eq(t, []any{fv(first.Open), fv(first.High), fv(first.Low), first.Close, fv(first.Volume)}, []any{10.0, 12.0, 9.0, 11.2, 3.0}, "")
	four := AggregateSession(minutes, 240)
	eq(t, starts(four), []string{"11-10 14:30", "11-10 18:30", "11-11 14:30"}, "9:30-13:29 and 13:30-16:00")
	eq(t, []any{fv(four[0].Open), fv(four[0].High), fv(four[0].Low), four[0].Close}, []any{10.0, 12.0, 9.0, 11.3}, "")
	eq(t, []any{fv(four[1].Open), four[1].Close}, []any{11.3, 11.6}, "")
}

func TestOptionTradesAreChartedOnTheirUnderlying(t *testing.T) {
	opt := rec("QNC 20NOV26 3.00 CALL", "NYSE", "USD", "Options")
	inst := ChartInstrument(opt)
	eq(t, inst, rec("QNC", "NYSE", "USD", "Shares"), "")
	eq(t, *HistorySource(inst), Candidate{"tmx", "QNC:US"}, "")
	now := utc(2026, 9, 7, 12, 0, 0)
	eq(t, AvailableTimeframes(inst, "2026-06-01", now), []string{"1h", "4h", "1d", "1w", "1M"}, "")
	share := rec("RDDY", "TSX", "CAD", "Shares")
	eq(t, ChartInstrument(share), share, "")
}

func TestIntradayAvailableForTMXListingsWithinAYear(t *testing.T) {
	now := utc(2026, 9, 7, 12, 0, 0)
	tsla := rec("TSLA", "NASDAQ", "USD", "Shares")
	eq(t, AvailableTimeframes(tsla, "2025-11-01", now), []string{"1h", "4h", "1d", "1w", "1M"}, "")
	eq(t, AvailableTimeframes(tsla, "2024-08-01", now), []string{"1d", "1w", "1M"}, "beyond every source's intraday reach")
	eq(t, AvailableTimeframes(tsla, "2025-08-01", now), []string{"1h", "4h", "1d", "1w", "1M"}, "past TMX's year, within Yahoo's two")
	eq(t, IntradayReach(tsla, now), "2024-09-08", "")
	hbix := rec("HBIX", "Cboe Canada", "CAD", "Shares")
	eq(t, AvailableTimeframes(hbix, "2026-08-01", now), []string{"1h", "4h", "1d", "1w", "1M"}, "Cboe Canada listings have TMX's minute bars under :AQL")
}

func TestCryptoCandlesComeFromCoinbaseInThePositionCurrency(t *testing.T) {
	now := utc(2026, 9, 7, 12, 0, 0)
	c, s := newTestClient(t, now)
	c.Store.UpsertFXRates(map[string]float64{"2026-02-05": 1.40, "2026-02-06": 1.50})
	s.set(func(call stubCall) (int, string, error) {
		url := call.URL
		switch {
		case strings.Contains(url, "finance.yahoo.com"):
			return 0, "", errors.New("404")
		case strings.Contains(url, "/products/PEPE-CAD") || strings.Contains(url, "/products/NOPE-"):
			return 0, "", errors.New("404")
		case strings.HasSuffix(url, "/products/PEPE-USD"):
			return 200, jsonText(obj("id", "PEPE-USD", "status", "online")), nil
		case strings.HasSuffix(url, "/products/USDC-CAD"):
			return 200, jsonText(obj("id", "USDC-CAD", "status", "online")), nil
		case strings.Contains(url, "/candles?"):
			return 200, jsonText([]any{[]any{1770336000, 1.0, 3.0, 2.0, 2.5, 10}, []any{1770508800, 1.0, 3.0, 2.0, 2.5, 10}, []any{1769040000, 1.0, 3.0, 2.0, 2.5, 10}}), nil
		}
		return 0, "", errors.New("unexpected " + url)
	})
	pepe := rec("PEPE", "Crypto", "CAD", "Crypto")
	eq(t, HistoryCandidates(pepe), []Candidate{{"coinbase", "PEPE-CAD"}, {"yahoo", "PEPE-CAD"}, {"coinbase", "PEPE-USD"}, {"yahoo", "PEPE-USD"}}, "")
	eq(t, c.CoinbaseMarket("PEPE-CAD", now), "", "")
	eq(t, c.CoinbaseMarket("USDC-CAD", now), "USDC-CAD", "")
	n := s.count()
	eq(t, c.CoinbaseMarket("PEPE-CAD", now), "", "")
	eq(t, c.CoinbaseMarket("USDC-CAD", now), "USDC-CAD", "")
	eq(t, s.count(), n, "markets are remembered, misses for a day")
	bars, source := c.FetchHistory(pepe, "2026-01-20", "2026-02-09")
	eq(t, source, "coinbase", "")
	eq(t, c.Store.GetMeta("bars_source:PEPE"), "coinbase|PEPE-USD", "the candidate that answered is remembered")
	eq(t, dailyRows(bars), [][]any{{"2026-02-06", 3.0, 4.5, 1.5, 3.75, 10.0}, {"2026-02-08", 3.0, 4.5, 1.5, 3.75, 10.0}}, "USD candles at the Bank of Canada rate of the day (Sunday takes Friday's); the day with no rate within a week is dropped")
	one := []Bar{{Time: 1770336000, Open: ptr(1), High: ptr(1), Low: ptr(1), Close: 1, Volume: ptr(0)}}
	eq(t, c.InPositionCurrencyBars(one, "CAD", "CAD")[0].Close, 1.0, "a CAD market is used as is")
	eq(t, c.InPositionCurrencyBars(one, "EUR", "CAD"), []Bar{}, "nothing else is converted")
	eq(t, IntradayReach(pepe, now), CoinbaseExchangeStart, "")
}

func TestYahooIsAskedGently(t *testing.T) {
	c, s := newTestClient(t, utc(2026, 9, 7, 12, 0, 0))
	c.SetYahooBackoff(time.Time{})
	s.set(func(call stubCall) (int, string, error) {
		if strings.Contains(call.URL, "/GONE.CN?") {
			return 404, "Not Found", nil
		}
		return 429, "Too Many Requests", nil
	})
	bars, err := c.FetchYahoo("GONE.CN", 0, 1, "1d")
	eq(t, err, nil, "")
	eq(t, len(bars), 0, "")
	sent := map[string]string{}
	for k, v := range s.all()[0].Header {
		sent[k] = v[0]
	}
	eq(t, sent, YahooHeaders, "Yahoo is asked with its own headers, not the app's usual ones")
	bars, err = c.FetchYahoo("GONE.CN", 0, 1, "1d")
	eq(t, err, nil, "")
	eq(t, len(bars), 0, "")
	eq(t, len(s.urls("yahoo")), 1, "a symbol Yahoo does not carry is not asked again today")
	_, err = c.FetchYahoo("BUSY.TO", 0, 1, "1d")
	eq(t, StatusOf(err), 429, "")
	_, err = c.FetchYahoo("BUSY.TO", 0, 1, "1d")
	eq(t, errors.Is(err, ErrBackingOff), true, "")
	_, err = c.FetchYahoo("OTHER.TO", 0, 1, "60m")
	eq(t, errors.Is(err, ErrBackingOff), true, "")
	eq(t, len(s.urls("yahoo")), 2, "after a 429 nothing is asked for a while")
	c.SetYahooBackoff(time.Time{})
}

func yahooDaily() string {
	return jsonText(obj("chart", obj("result", []any{obj(
		"meta", obj("exchangeTimezoneName", "America/Toronto", "gmtoffset", -14400),
		"timestamp", []any{1770042600, 1770129000},
		"indicators", obj("quote", []any{obj("open", []any{1.0, 1.1}, "high", []any{1.2, 1.3}, "low", []any{0.9, 1.0}, "close", []any{1.1, 1.2}, "volume", []any{10, 20})}),
	)})))
}

func TestHistoryChainFallsThroughToYahooAndRemembersTheWinner(t *testing.T) {
	c, s := newTestClient(t, utc(2026, 9, 7, 12, 0, 0))
	yahoo := yahooDaily()
	s.set(func(call stubCall) (int, string, error) {
		if call.URL == TMXURL {
			return 200, jsonText(obj("data", obj("getTimeSeriesData", []any{}, "getQuoteBySymbol", nil))), nil
		}
		if strings.Contains(call.URL, "/QMET.CN?") {
			return 200, yahoo, nil
		}
		return 0, "", errors.New("404")
	})
	r := rec("QMET", "CSE", "CAD", "Shares")
	eq(t, HistoryCandidates(r), []Candidate{{"tmx", "QMET:CNX"}, {"yahoo", "QMET.CN"}, {"yahoo", "QMET.TO"}, {"yahoo", "QMET.V"}, {"yahoo", "QMET.NE"}}, "TMX first, then Yahoo with the venue's suffix first")
	bars, source := c.FetchHistory(r, "2026-02-01", "2026-02-05")
	eq(t, source, "yahoo", "")
	short := [][]any{}
	for _, b := range bars {
		short = append(short, []any{b.Date, fv(b.Open), b.Close})
	}
	eq(t, short, [][]any{{"2026-02-02", 1.0, 1.1}, {"2026-02-03", 1.1, 1.2}}, "Yahoo's daily stamps fall on the exchange's local day (February: standard time, not the offset Yahoo reports today)")
	if len(s.tmxPosts()) == 0 {
		t.Fatal("TMX was asked first")
	}
	eq(t, c.Store.GetMeta("bars_source:QMET"), "yahoo|QMET.CN", "")
	s.reset()
	c.FetchHistory(r, "2026-02-01", "2026-02-05")
	eq(t, len(s.tmxPosts()), 0, "the remembered winner is tried first; TMX is not asked again")
	eq(t, s.gets(), 1, "")
	usdc := rec("USDC", "Crypto", "CAD", "Crypto")
	s.set(func(call stubCall) (int, string, error) {
		switch {
		case strings.HasSuffix(call.URL, "/products/USDC-CAD"):
			return 200, jsonText(obj("id", "USDC-CAD")), nil
		case strings.Contains(call.URL, "/candles?"):
			return 200, "[]", nil
		case strings.Contains(call.URL, "/USDC-CAD?"):
			return 200, yahoo, nil
		}
		return 0, "", errors.New("404")
	})
	bars, source = c.FetchHistory(usdc, "2026-02-01", "2026-02-05")
	eq(t, []any{source, len(bars)}, []any{"yahoo", 2}, "")
	eq(t, c.Store.GetMeta("bars_source:USDC"), "yahoo|USDC-CAD", "")
	c.Store.SetMeta("bars_source:USDC", "")
	late := jsonText([]any{[]any{1770508800, 1.0, 1.0, 1.0, 1.4, 1}})
	s.set(func(call stubCall) (int, string, error) {
		switch {
		case strings.HasSuffix(call.URL, "/products/USDC-CAD"):
			return 200, jsonText(obj("id", "USDC-CAD")), nil
		case strings.Contains(call.URL, "/candles?"):
			return 200, late, nil
		case strings.Contains(call.URL, "/USDC-CAD?"):
			return 200, yahoo, nil
		}
		return 0, "", errors.New("404")
	})
	bars, source = c.FetchHistory(usdc, "2026-01-20", "2026-02-09")
	eq(t, []any{source, bars[0].Date}, []any{"yahoo", "2026-02-02"}, "Yahoo reaches further back than Coinbase for this span")
	_, source = c.FetchHistory(usdc, "2026-02-01", "2026-02-09")
	eq(t, source, "yahoo", "remembered, and it covers the span")
	c.Store.SetMeta("bars_source:USDC", "")
	_, source = c.FetchHistory(usdc, "2026-02-06", "2026-02-09")
	eq(t, source, "coinbase", "a span Coinbase covers (within the slack) is answered by the first link")
}

func TestPartialHistoryDoesNotClaimTheEarlierDays(t *testing.T) {
	now := utc(2026, 9, 7, 12, 0, 0)
	c, s := newTestClient(t, now)
	r := rec("USDC", "Crypto", "CAD", "Crypto")
	late := jsonText([]any{[]any{utc(2026, 2, 25, 0, 0, 0).Unix(), 1, 1, 1, 1, 1}})
	s.set(func(call stubCall) (int, string, error) {
		switch {
		case strings.HasSuffix(call.URL, "/products/USDC-CAD"):
			return 200, jsonText(obj("id", "USDC-CAD")), nil
		case strings.Contains(call.URL, "/candles?"):
			return 200, late, nil
		}
		return 0, "", errNoRoute
	})
	c.EnsureHistory(r, "2026-01-10", "2026-01-30", now)
	eq(t, c.Store.HistoryFetch("USDC").Start, "2026-02-25", "covered from the first bar, not from the day asked")
	c.EnsureHistory(r, "2026-01-10", "2026-01-30", now)
	eq(t, len(s.urls("/candles?")), 2, "the earlier span is asked for again")
	c.EnsureHistory(r, "2026-03-01", "2026-03-10", now)
	eq(t, len(s.urls("/candles?")), 2, "a span the bars do cover is served from the store")
}

func TestHistoryIsCachedAndClosedDaysNeverRewritten(t *testing.T) {
	now := utc(2026, 9, 5, 12, 0, 0)
	c, s := newTestClient(t, now)
	r := rec("RDDY", "TSX", "CAD", "Shares")
	answer := tmxSeries(tmxRow("2026-09-03", 4.83, 4.95, 4.73, 4.75, 1), tmxRow("2026-09-04", 4.8, 4.8, 4.68, 4.75, 1))
	s.set(func(call stubCall) (int, string, error) {
		if call.URL == TMXURL {
			return 200, answer, nil
		}
		return 0, "", errNoRoute
	})
	spans := func() [][2]string {
		out := [][2]string{}
		for _, p := range s.tmxPosts() {
			if p.Op == "getTimeSeriesData" {
				out = append(out, [2]string{p.Var("start"), p.Var("end")})
			}
		}
		return out
	}
	out := c.EnsureHistory(r, "2026-08-28", "2026-09-05", now)
	eq(t, dates(out), []string{"2026-09-03", "2026-09-04"}, "")
	eq(t, spans()[0], [2]string{"2026-08-28", "2026-09-05"}, "")
	s.reset()
	c.EnsureHistory(r, "2026-08-28", "2026-09-05", now.Add(5*time.Minute))
	eq(t, len(spans()), 0, "")
	s.reset()
	c.EnsureHistory(r, "2026-06-01", "2026-06-30", now.Add(5*time.Minute))
	eq(t, spans()[0][0], "2026-06-01", "")
	answer = tmxSeries(tmxRow("2026-09-03", 1, 1, 1, 1, 1), tmxRow("2026-09-04", 4.8, 4.9, 4.68, 4.85, 2))
	s.reset()
	out = c.EnsureHistory(r, "2026-08-25", "2026-09-05", now.Add(25*time.Hour))
	eq(t, len(spans()), 1, "")
	got := [][2]any{}
	for _, b := range out {
		got = append(got, [2]any{b.Date, b.Close})
	}
	eq(t, got, [][2]any{{"2026-09-03", 4.75}, {"2026-09-04", 4.85}}, "")
}
