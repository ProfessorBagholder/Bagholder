package exposure

import (
	"encoding/json"
	"strings"
	"testing"
)

var isharesCSV = strings.Join([]string{
	"\ufeffFund Holdings as of,\"Sep 9, 2026\"",
	" ",
	"Ticker,Name,Sector,Asset Class,Market Value,Weight (%),Notional Value,Shares,Price,Location,Exchange,Currency,FX Rate,Market Currency",
	`"RY","ROYAL BANK OF CANADA","Financials","Equity","2,177,806,813.19","9.73","2,177,806,813.19","7,625,641.00","285.59","Canada","Toronto Stock Exchange","CAD","1.00","CAD"`,
	`"SHOP","SHOPIFY SUBORDINATE VOTING CLASS A","Information Technology","Equity","1,165,541,988.48","5.21","1,165,541,988.48","6,655,296.00","175.13","Canada","Toronto Stock Exchange","CAD","1.00","CAD"`,
	`"XEF","ISHARES MSCI EAFE IMI INDEX","Other","Equity","5,416,028,871.87","24.37","5,416,028,871.87","104,617,131.00","51.77","Canada","Toronto Stock Exchange","CAD","1.00","CAD"`,
	`"CAD","CAD CASH","Cash and/or Derivatives","Cash","31,678,794.26","0.14","31,678,794.26","31,678,794.00","100.00","Canada","-","CAD","1.00","CAD"`,
	" ",
	`Fund Holdings as of,"Sep 9, 2026"`,
	"",
}, "\n")

const evolveHTML = `<html><body><script>
var portfolioBreakdownData = {"data":{"geographic":[{"name":"BIGY","weight":"46.33%"}],"sector":[{"name":"Technology","weight":"27.86%"},{"name":"Financial","weight":"24.63%"},{"name":"Communications","weight":"15.72%"}]}};
var holdingsData = {"data":[{"ticker":"MSFT US EQUITY","weight_percent":"4.11%","position":"6301","security_name":"Microsoft Corp","gics_sector":"Technology","country":"BIGY","last_price":"491.65","value":"4,278,213"},{"ticker":"RY CN EQUITY","weight_percent":"2.00%","security_name":"Royal Bank of Canada","gics_sector":"Financial","country":"Canada"}]};
</script></body></html>`

const harvestTableHTML = `<table><tr><th>Name</th><th>Ticker</th><th>Weight</th><th>Sector</th><th>Country</th></tr>
<tr><td>iShares Bitcoin Trust ETF</td><td>IBIT US</td><td>130.3%</td><td>Bitcoin Holding</td><td>United States</td></tr>
<tr><td>Written Options</td><td></td><td>(4.5)%</td><td></td><td></td></tr>
<tr><td>Cash and other assets and liabilities</td><td></td><td>(25.8)%</td><td></td><td></td></tr></table>`

const harvestNamesHTML = `<table><tr><th>Fund Details</th><th>As at 2026/09/09</th></tr><tr><td>Ticker</td><td>PLTE</td></tr><tr><td>Reference Asset</td><td>PLTR</td></tr></table>
<table><tr><th>HOLDING</th><th>As at 2026/08/31</th></tr><tr><td>Palantir Technologies Inc.</td><td>128.2%</td></tr><tr><td>Written Options</td><td>(2.8)%</td></tr><tr><td>Cash and other assets and liabilities</td><td>(25.4)%</td></tr></table>`

const harvestFofHTML = `<table><tr><th>HOLDINGS</th><th>As at 2026/08/31</th></tr><tr><td>Harvest Apple Enhanced High Income Shares ETF</td><td>7.0%</td></tr><tr><td>Harvest NVIDIA Enhanced High Income Shares ETF</td><td>6.9%</td></tr></table>`

const ninepointHTML = `<div><table><tr><td>Facts</td></tr><tr><td>Ticker</td><td>CCHI:TSX</td></tr><tr><td>Underlying Stock**</td><td>Cameco Corp. (CCO:TSX)</td></tr></table></div>`

const yahooJSON = `{"quoteSummary": {"result": [{"topHoldings": {
	"holdings": [{"symbol": "AAPL", "holdingName": "Apple Inc", "holdingPercent": {"raw": 0.07}}, {"symbol": "RY.TO", "holdingName": "Royal Bank of Canada", "holdingPercent": {"raw": 0.03}}],
	"sectorWeightings": [{"realestate": {"raw": 0.02}}, {"technology": {"raw": 0.30}}, {"financial_services": {"raw": 0.20}}]
}}]}}`

type isharesRow struct {
	Ticker  string
	Weight  float64
	Sector  string
	Country string
	Fund    bool
}

type classifiedRow struct {
	Ticker  string
	Weight  float64
	Sector  string
	Country string
}

type namedRow struct {
	Name   string
	Weight float64
	Fund   bool
}

type venueRow struct {
	Ticker   string
	Weight   float64
	Exchange string
}

func TestISharesHoldingsCSV(t *testing.T) {
	rows, asOf := ParseISharesCSV(isharesCSV)
	equal(t, asOf, "Sep 9, 2026", "")
	got := []isharesRow{}
	for _, r := range rows {
		got = append(got, isharesRow{r.Ticker, r.Weight, r.Sector, r.Country, r.Fund})
	}
	equal(t, got, []isharesRow{{"RY", 9.73, "Financials", "Canada", false}, {"SHOP", 5.21, "Information Technology", "Canada", false}, {"XEF", 24.37, "", "Canada", true}},
		"cash is out; a fund row is marked to be looked through")
}

func TestEvolvePage(t *testing.T) {
	sectors, holdings := ParseEvolvePage(evolveHTML)
	equal(t, sectors, map[string]float64{"Information Technology": 27.86, "Financials": 24.63, "Communication Services": 15.72}, "")
	got := []classifiedRow{}
	for _, h := range holdings {
		got = append(got, classifiedRow{h.Ticker, h.Weight, h.Sector, h.Country})
	}
	equal(t, got, []classifiedRow{{"MSFT", 4.11, "Information Technology", "United States"}, {"RY", 2.0, "Financials", "Canada"}},
		"a sub-fund code in the country column is not a country; the Bloomberg market code gives it")
}

func TestHarvestTables(t *testing.T) {
	rows, ref := ParseHarvestTables(HTMLTables(harvestTableHTML))
	equal(t, ref, "", "")
	got := []classifiedRow{}
	for _, h := range rows {
		got = append(got, classifiedRow{h.Ticker, h.Weight, h.Sector, h.Country})
	}
	equal(t, got, []classifiedRow{{"IBIT", 130.3, "Digital assets", "United States"}}, "options and cash rows are out")
	rows, ref = ParseHarvestTables(HTMLTables(harvestNamesHTML))
	equal(t, ref, "PLTR", "")
	equal(t, namedRows(rows), []namedRow{{"Palantir Technologies Inc.", 128.2, false}}, "")
	rows, _ = ParseHarvestTables(HTMLTables(harvestFofHTML))
	equal(t, namedRows(rows), []namedRow{{"Harvest Apple Enhanced High Income Shares ETF", 7.0, true}, {"Harvest NVIDIA Enhanced High Income Shares ETF", 6.9, true}}, "")
}

func namedRows(rows []Holding) []namedRow {
	out := []namedRow{}
	for _, h := range rows {
		out = append(out, namedRow{h.Name, h.Weight, h.Fund})
	}
	return out
}

func TestNinepointPage(t *testing.T) {
	ticker, under, ex := ParseNinepointPage(ninepointHTML)
	equal(t, [3]string{ticker, under, ex}, [3]string{"CCHI", "CCO", "TSX"}, "")
}

func TestYahooSummary(t *testing.T) {
	var d YahooSummaryData
	if err := json.Unmarshal([]byte(yahooJSON), &d); err != nil {
		t.Fatal(err)
	}
	sectors, holdings := ParseYahooSummary(d)
	equal(t, sectors, map[string]float64{"Real Estate": 2.0, "Information Technology": 30.0, "Financials": 20.0}, "")
	got := []venueRow{}
	for _, h := range holdings {
		got = append(got, venueRow{h.Ticker, h.Weight, h.Exchange})
	}
	equal(t, got, []venueRow{{"AAPL", 7.0, ""}, {"RY.TO", 3.0, "TSX"}}, "")
}

func TestNasdaqSummaryAndTheISharesScreener(t *testing.T) {
	s := &stub{pages: map[string]page{
		strings.Replace(NasdaqSummaryURL, "%s", "AAPL", 1): {text: `{"data": {"symbol": "AAPL", "summaryData": {"Sector": {"label": "Sector", "value": "Technology"}, "Industry": {"label": "Industry", "value": "Computer Manufacturing"}}}, "status": {"rCode": 200}}`},
		strings.Replace(NasdaqSummaryURL, "%s", "NONE", 1): {text: `{"data": null, "status": {"rCode": 400}}`},
		ISharesScreener: {text: "\ufeff" + `{"239832": {"localExchangeTicker": "XIC", "productPageUrl": "/ca/investors/en/products/239832/"}, "1": {"localExchangeTicker": "", "productPageUrl": "/x"}, "2": "junk"}`},
	}}
	c := newClient(t, s)
	sector, industry := c.nasdaqSummary("AAPL")
	equal(t, [2]string{sector, industry}, [2]string{"Technology", "Computer Manufacturing"}, "")
	sector, industry = c.nasdaqSummary("NONE")
	equal(t, [2]string{sector, industry}, [2]string{"", ""}, "")
	page, err := c.isharesPage("XIC")
	if err != nil {
		t.Fatal(err)
	}
	equal(t, page, "/ca/investors/en/products/239832/", "")
	equal(t, c.isharesMap, map[string]string{"XIC": "/ca/investors/en/products/239832/"}, "")
}
