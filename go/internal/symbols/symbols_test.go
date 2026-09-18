package symbols

import "testing"

func TestOptions(t *testing.T) {
	if !IsOption("LUNR 15JAN27 12.00 CALL") || !IsOption("QNC 20NOV26 3.00 PUT") || !IsOption("AAPL 250117C00150000") || !IsOption("X 10JAN26 5.00 C") {
		t.Error("contracts")
	}
	if IsOption("AAPL") || IsOption("SHOP.TO") || IsOption("") || IsOption("CALLAWAY") {
		t.Error("listings")
	}
	if Underlying("LUNR 15JAN27 12.00 CALL") != "LUNR" || Underlying("AAPL 250117C00150000") != "AAPL" || Underlying("SHOP.TO") != "SHOP.TO" || Underlying("") != "—" {
		t.Error("underlying")
	}
	if Right("BBAI 26DEC25 5.50 PUT") != "PUT" || Right("LUNR 15JAN27 12.00 CALL") != "CALL" || Right("AAPL 250117P00150000") != "PUT" {
		t.Error("right")
	}
	if Expiry("LUNR 29AUG25 11.50 CALL") != "2025-08-29" || Expiry("AAPL") != "" {
		t.Errorf("expiry %q", Expiry("LUNR 29AUG25 11.50 CALL"))
	}
	if Multiplier("LUNR 15JAN27 12.00 CALL") != 100 || Multiplier("LUNR") != 1 {
		t.Error("mult")
	}
	if Strike("ASTS 07MAR25 31.00 CALL") != 31 {
		t.Error("strike")
	}
	if ListingTicker("QNC.TO") != "QNC" || ListingTicker("qnc.v") != "qnc" || ListingTicker("QNC") != "QNC" || TMXSymbol(" shop.to ") != "SHOP" {
		t.Error("ticker")
	}
	if Compact("Buy_To-Open ") != "BUYTOOPEN" || NormAccountName("TFSA  Fund ") != "TFSA Fund" {
		t.Errorf("compact %q norm %q", Compact("Buy_To-Open "), NormAccountName("TFSA  Fund "))
	}
}
