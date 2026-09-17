package exposure

import "testing"

func TestSectorNamesFoldOntoOneSet(t *testing.T) {
	equal(t, NormSector("Technology"), "Information Technology", "")
	equal(t, NormSector("Financial"), "Financials", "")
	equal(t, NormSector("Communication"), "Communication Services", "")
	equal(t, NormSector("Consumer, Non-cyclical"), "Consumer Staples", "")
	equal(t, NormSector("Basic Materials"), "Materials", "")
	equal(t, NormSector("Bitcoin Holding"), "Digital assets", "")
	equal(t, NormSector("Cash and/or Derivatives"), "", "cash is no sector")
	equal(t, NormSector("Aerospace"), "Aerospace", "a name outside the set passes through")
}

func TestCountryNamesAndVenues(t *testing.T) {
	equal(t, NormCountry("USA"), "United States", "")
	equal(t, NormCountry("Korea, Republic of"), "South Korea", "")
	equal(t, VenueCountryOf("TSX-V"), "Canada", "")
	equal(t, VenueCountryOf("NASDAQ"), "United States", "")
	equal(t, VenueCountryOf("OPRA"), "", "")
}

func TestIssuerOfAFundName(t *testing.T) {
	equal(t, IssuerOf("Vanguard All-Equity ETF Portfolio - ETF"), "vanguard", "")
	equal(t, IssuerOf("iShares Core Equity ETF Portfolio"), "ishares", "")
	equal(t, IssuerOf("Harvest Diversified High Income Shares ETF - Class A"), "harvest", "")
	equal(t, IssuerOf("Ninepoint Partners LP - Cameco Highshares ETF"), "ninepoint", "")
	equal(t, IssuerOf("Evolve All-in-One UltraYield ETF"), "evolve", "")
	equal(t, IssuerOf("Shopify Inc."), "", "")
	equal(t, IsFund("Global X High Interest Savings ETF"), true, "")
	equal(t, IsFund("Shopify Inc."), false, "")
}
