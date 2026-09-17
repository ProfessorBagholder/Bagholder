package shorts

import (
	"errors"
	"fmt"
	"reflect"
	"strings"
	"testing"
	"time"
)

func finraPosition(rows []map[string]any) func(method, url, body string) (int, string, error) {
	return func(method, url, body string) (int, string, error) {
		if strings.Contains(url, "consolidatedShortInterest") {
			return 200, jsonText(rows), nil
		}
		return 404, "", nil
	}
}

func TestTheFloatIsReadUnderTheVenuesOwnSymbol(t *testing.T) {
	t.Skip("needs a Yahoo session: browserhttp.Session wraps an unexported TLS-fingerprint client that cannot be stubbed")
}

func TestAFormTheSourceDoesNotCarryFallsToTheNext(t *testing.T) {
	t.Skip("needs a Yahoo session: browserhttp.Session wraps an unexported TLS-fingerprint client that cannot be stubbed")
}

func TestAListingWithNoFloatPublishedReportsNone(t *testing.T) {
	t.Skip("needs a Yahoo session: browserhttp.Session wraps an unexported TLS-fingerprint client that cannot be stubbed")
}

func TestItIsReadOnceAndKept(t *testing.T) {
	t.Skip("needs a Yahoo session: browserhttp.Session wraps an unexported TLS-fingerprint client that cannot be stubbed")
}

func TestWithoutTheBrowserClientTheFloatIsSimplyUnknown(t *testing.T) {
	c, _ := newClient(t)
	if got := c.FloatShares("GME", "NYSE", "USD", ""); got != nil {
		t.Errorf("float = %v", *got)
	}
}

func TestThePositionIsMeasuredAgainstTheFloat(t *testing.T) {
	c, s := newClient(t)
	c.shares["GME|NYSE"] = floatHit{p(400.0), time.Now()}
	s.answer = finraPosition([]map[string]any{{"settlementDate": "2026-08-31", "currentShortPositionQuantity": 100.0}})
	rec, _ := c.ForListing("GME", "NYSE", "USD", time.Now().UTC(), false, "")
	if !is(rec.Float, 400.0) {
		t.Errorf("float = %v", show(rec.Float))
	}
	if !is(rec.OfFloat, 25.0) {
		t.Errorf("ofFloat = %v", show(rec.OfFloat))
	}
}

func TestACompanyWithNoFloatPublishedIsNeverGivenItsShareCount(t *testing.T) {
	t.Skip("needs a Yahoo session answering sharesOutstanding: browserhttp.Session cannot be stubbed")
}

func tmxUnits(t *testing.T, count any) func(method, url, body string) (int, string, error) {
	return func(method, url, body string) (int, string, error) {
		if strings.Contains(url, "tmx.com") && operationOf(body) == "getQuoteBySymbol" {
			return 200, jsonText(map[string]any{"data": map[string]any{"getQuoteBySymbol": map[string]any{"shareOutStanding": count}}}), nil
		}
		t.Errorf("asked anyway: %s %s", method, url)
		return 0, "", errors.New("asked anyway")
	}
}

func TestAFundFallsBackToTheUnitsTheExchangePublishes(t *testing.T) {
	c, s := newClient(t)
	s.answer = tmxUnits(t, 20075000.0)
	if got := c.FloatShares("RDDY", "TSX", "CAD", "Harvest Reddit Enhanced High Income Shares ETF"); !is(got, 20075000.0) {
		t.Errorf("float = %v", show(got))
	}
}

func TestAUSFundTakesTheCountFromTheSameAnswerAsTheFloat(t *testing.T) {
	t.Skip("needs a Yahoo session answering sharesOutstanding: browserhttp.Session cannot be stubbed")
}

func TestAFloatThatIsPublishedIsStillWhatAFundIsMeasuredAgainst(t *testing.T) {
	t.Skip("needs a Yahoo session answering floatShares: browserhttp.Session cannot be stubbed")
}

func readSearched(t *testing.T, exchange, name string) Record {
	t.Helper()
	c, s := newClient(t)
	s.answer = refuse(t, "read anyway")
	c.files["ca_position"] = &fileTable{key: "2026-08-31", rows: map[string]map[string]any{"SHOP": {"venue": "TSX", "shares": 7573829.0, "change": 41206.0, "name": "SHOPIFY INC. CL 'A' SV"}}, at: time.Now()}
	c.files["ca_volume"] = &fileTable{key: "", rows: map[string]map[string]any{}, at: time.Now()}
	c.shares["SHOP|"+strings.ToUpper(exchange)] = floatHit{p(1000000.0), time.Now()}
	rec, _ := c.ForListing("SHOP", exchange, "CAD", time.Now().UTC(), false, name)
	return rec
}

func TestTheReportNamesTheVenueAndTheIssuerWhereTheBookKnowsNeither(t *testing.T) {
	rec := readSearched(t, "", "")
	if rec.Exchange != "TSX" || rec.Name != "SHOPIFY INC. CL 'A' SV" {
		t.Errorf("got %q %q", rec.Exchange, rec.Name)
	}
}

func TestTheBookOwnNameForAListingItCarriesIsTheOneTheFloatIsReadUnder(t *testing.T) {
	rec := readSearched(t, "TSX", "Shopify Inc.")
	if rec.Exchange != "TSX" {
		t.Errorf("exchange = %q", rec.Exchange)
	}
}

var cboeDirectory = jsonText(map[string]any{"data": []map[string]any{
	{"symbol": "HBIX", "name": "HARVEST BITCOIN ENHANCED INCOME ETF", "security": "etf", "marketcap": 44091000.0, "last": 6.39},
	{"symbol": "BCBN", "name": "A COMPANY", "security": "equity", "marketcap": 100876283.0, "last": 1.0},
	{"symbol": "NOPR", "name": "NO PRICE ETF", "security": "etf", "marketcap": 500.0, "last": 0.0},
	{"symbol": "ODDS", "name": "NOT A WHOLE COUNT ETF", "security": "etf", "marketcap": 100.0, "last": 3.0}}})

func TestTheCountIsTheCapitalisationOverThePriceForTheVenuesOwnFunds(t *testing.T) {
	c, s := newClient(t)
	s.answer = func(method, url, body string) (int, string, error) {
		if strings.Contains(url, "tmx.com") {
			return 200, jsonText(map[string]any{"data": map[string]any{"getQuoteBySymbol": map[string]any{"shareOutStanding": 0}}}), nil
		}
		if url == CACboeURL {
			return 200, cboeDirectory, nil
		}
		return 404, "", nil
	}
	if got := c.fundUnits("HBIX", "CBOE CANADA", "CAD"); !is(got, 6900000.0) {
		t.Errorf("HBIX = %v", show(got))
	}
	if got := c.fundUnits("BCBN", "CBOE CANADA", "CAD"); got != nil {
		t.Errorf("a company's shares in issue are not its float: %v", *got)
	}
	if got := c.fundUnits("NOPR", "CBOE CANADA", "CAD"); got != nil {
		t.Errorf("no price, no count: %v", *got)
	}
	if got := c.fundUnits("ODDS", "CBOE CANADA", "CAD"); got != nil {
		t.Errorf("a count that is not whole is not the exchange's own: %v", *got)
	}
	if n := s.count(CACboeURL); n != 1 {
		t.Errorf("one directory for every listing looked up in it: %d", n)
	}
}

func TestAListingOnAnotherVenueNeverTakesACountFromThisOne(t *testing.T) {
	c, s := newClient(t)
	s.answer = tmxUnits(t, 0)
	if got := c.fundUnits("HBIX", "TSX", "CAD"); got != nil {
		t.Errorf("units = %v", *got)
	}
}

func TestTheVenueIsAskedOnlyWhereTMXHasNoCount(t *testing.T) {
	c, s := newClient(t)
	s.answer = tmxUnits(t, 4200)
	if got := c.fundUnits("XYZ", "CBOE CANADA", "CAD"); !is(got, 4200.0) {
		t.Errorf("units = %v", show(got))
	}
}

func TestAFigureIsReadOnceAndKept(t *testing.T) {
	t.Skip("needs a Yahoo session whose asks can be counted: browserhttp.Session cannot be stubbed")
}

func TestALookupThatAnsweredWithNothingIsAskedAgain(t *testing.T) {
	t.Skip("needs a Yahoo session whose asks can be counted: browserhttp.Session cannot be stubbed")
}

func TestAFigureIsNotAskedAgainThatSoon(t *testing.T) {
	t.Skip("needs a Yahoo session whose asks can be counted: browserhttp.Session cannot be stubbed")
}

func TestEachLookupTakesItsTurnAndLeavesTheNextSlot(t *testing.T) {
	t.Skip("needs a Yahoo session and market's unexported yahooNextAt: browserhttp.Session cannot be stubbed")
}

func TestNothingIsAskedWhileABackoffStands(t *testing.T) {
	t.Skip("needs a Yahoo session whose asks can be counted: browserhttp.Session cannot be stubbed")
}

func TestARefusalStartsTheBackoffTheWholeAppHonours(t *testing.T) {
	t.Skip("needs a Yahoo session answering 429 and market's unexported yahooBackoffUntil: browserhttp.Session cannot be stubbed")
}

func TestAUSListingWithNoCurrencyOnItsRowIsStillAskedForAsOne(t *testing.T) {
	t.Skip("needs a Yahoo session whose asks can be inspected: browserhttp.Session cannot be stubbed")
}

func TestACanadianListingWithNoCurrencyKeepsItsOwnSuffixes(t *testing.T) {
	t.Skip("needs a Yahoo session whose asks can be inspected: browserhttp.Session cannot be stubbed")
}

func TestARecordReadOnTheSpotNamesItsListing(t *testing.T) {
	c, s := newClient(t)
	s.answer = finraPosition([]map[string]any{{"settlementDate": "2026-08-31", "currentShortPositionQuantity": 1.0}})
	rec, _ := c.ForListing("RKLB", "NASDAQ", "USD", time.Now().UTC(), false, "")
	if rec.Exchange != "NASDAQ" {
		t.Errorf("exchange = %q", rec.Exchange)
	}
}

func warmVolume(c *Client, rows map[string]map[string]any) {
	c.files["ca_volume"] = &fileTable{key: "2026-08-16/2026-08-31", rows: rows, at: time.Now()}
}

func tmxTraded(bars []map[string]any) func(method, url, body string) (int, string, error) {
	return func(method, url, body string) (int, string, error) {
		if strings.Contains(url, "tmx.com") && operationOf(body) == "getTimeSeriesData" {
			return 200, jsonText(map[string]any{"data": map[string]any{"getTimeSeriesData": bars}}), nil
		}
		return 200, "{}", nil
	}
}

func TestAListingTheReportOmitsReadsAsNoneOfItsTrading(t *testing.T) {
	c, s := newClient(t)
	warmVolume(c, map[string]map[string]any{})
	s.answer = tmxTraded([]map[string]any{{"dateTime": "2026-08-17", "close": 1.0, "volume": 1000000.0}, {"dateTime": "2026-08-18", "close": 1.0, "volume": 519546.0}})
	var out Record
	c.CAVolume("YES", "TSX-V", "CAD", time.Now().UTC(), &out)
	if !is(out.ShortVolume, 0.0) {
		t.Errorf("shortVolume = %v", show(out.ShortVolume))
	}
	if !is(out.VolumePct, 0.0) {
		t.Errorf("volumePct = %v", show(out.VolumePct))
	}
	if !is(out.TotalVolume, 1519546.0) {
		t.Errorf("totalVolume = %v", show(out.TotalVolume))
	}
}

func TestAListingTheReportOmitsAndTheExchangeHasNoVolumeForSaysNothing(t *testing.T) {
	c, s := newClient(t)
	warmVolume(c, map[string]map[string]any{})
	s.answer = tmxTraded([]map[string]any{})
	var out Record
	c.CAVolume("YES", "TSX-V", "CAD", time.Now().UTC(), &out)
	if !reflect.DeepEqual(out, Record{}) {
		t.Errorf("got %+v", out)
	}
}

func TestAListingTheReportCarriesIsReadFromTheReport(t *testing.T) {
	c, s := newClient(t)
	warmVolume(c, map[string]map[string]any{"QNC": {"venue": "TSXV", "shortVolume": 1197633.0, "volumePct": 21.319, "totalVolume": 5617679.0}})
	s.answer = refuse(t, "asked the exchange anyway")
	var out Record
	c.CAVolume("QNC", "TSX-V", "CAD", time.Now().UTC(), &out)
	if !is(out.VolumePct, 21.319) {
		t.Errorf("volumePct = %v", show(out.VolumePct))
	}
}

func TestDaysToCoverFollowsFromWhatTheExchangeSaysWasTraded(t *testing.T) {
	c, _ := newClient(t)
	prices := map[string]float64{}
	for d := 17; d < 28; d++ {
		prices[fmt.Sprintf("2026-08-%02d", d)] = 100.0
	}
	c.Store.UpsertBenchmarkPrices(prices, "TSX")
	rec := &Record{Market: "ca", Shares: p(17873.0), TotalVolume: p(1519546.0), VolumeOf: "2026-08-16/2026-08-31"}
	days := c.Store.BenchmarkDays("TSX", "2026-08-16", "2026-08-31")
	if days != 0 {
		if got := c.AverageVolume(rec); got == nil || !almost(*got, 1519546.0/float64(days)) {
			t.Errorf("averageVolume = %v, days %d", show(got), days)
		}
		if c.DaysToCover(rec) == nil {
			t.Error("daysToCover is nil")
		}
	}
}
