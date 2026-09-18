package ws

import (
	"math"
	"strings"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

func wsItem(overrides Item) Item {
	item := Item{
		"occurredAt":    "2024-06-15T13:45:22.123Z",
		"canonicalId":   "ws-cid-aaa-001",
		"status":        "POSTED",
		"type":          "DIY_BUY",
		"subType":       "BUY",
		"assetSymbol":   "AAA",
		"assetQuantity": 10,
		"amount":        100,
		"accountId":     "acct-1",
		"currency":      "CAD",
	}
	for k, v := range overrides {
		item[k] = v
	}
	return item
}

func mustMap(t *testing.T, item Item) *store.Activity {
	t.Helper()
	row := MapActivity(item, nil)
	if row == nil {
		t.Fatal("map_activity returned None")
	}
	return row
}

func almostEqual(t *testing.T, what string, got, want float64) {
	t.Helper()
	if py.Round(math.Abs(got-want), 7) != 0 {
		t.Errorf("%s: got %v, want %v", what, got, want)
	}
}

func TestOptionsSellMapsAsSellToOpen(t *testing.T) {
	item := wsItem(Item{
		"type":          "OPTIONS_SELL",
		"subType":       "LIMIT_ORDER",
		"assetSymbol":   "QNC",
		"contractType":  "CALL",
		"strikePrice":   3,
		"expiryDate":    "2027-02-19",
		"assetQuantity": 35,
		"amount":        1050,
		"amountSign":    "positive",
	})
	row := mustMap(t, item)
	if row.ActivitySubType != "SELLTOOPEN" {
		t.Errorf("activitySubType: got %q, want SELLTOOPEN", row.ActivitySubType)
	}
	if row.Category != "trade" {
		t.Errorf("category: got %q, want trade", row.Category)
	}
	if row.Quantity != -35 {
		t.Errorf("quantity: got %v, want -35", row.Quantity)
	}
	if row.NetCashAmount != 1050 {
		t.Errorf("netCashAmount: got %v, want 1050", row.NetCashAmount)
	}
	if row.Symbol != "QNC 19FEB27 3.00 CALL" {
		t.Errorf("symbol: got %q, want %q", row.Symbol, "QNC 19FEB27 3.00 CALL")
	}
}

func TestCashDividendWithoutStatusIsKept(t *testing.T) {
	item := Item{
		"type":          "DIVIDEND",
		"subType":       "CASH_DIVIDEND",
		"status":        nil,
		"amount":        "3660.00",
		"amountSign":    "positive",
		"assetQuantity": "18300.0",
		"assetSymbol":   "RDDY",
		"currency":      "CAD",
		"occurredAt":    "2026-06-05T14:53:21.630000+00:00",
		"accountId":     "non-registered-x",
		"canonicalId":   "div-1",
	}
	if SkipActivity(item) {
		t.Fatal("skip_activity: got True, want False")
	}
	rec := mustMap(t, item)
	if rec.Category != "dividend" {
		t.Errorf("category: got %q, want dividend", rec.Category)
	}
	if rec.Symbol != "RDDY" {
		t.Errorf("symbol: got %q, want RDDY", rec.Symbol)
	}
	almostEqual(t, "netCashAmount", rec.NetCashAmount, 3660.0)
	almostEqual(t, "unitPrice", rec.UnitPrice, 0.2)
	if rec.TransactionDate != "2026-06-05" {
		t.Errorf("transactionDate: got %q, want 2026-06-05", rec.TransactionDate)
	}
	item["type"] = "DIY_BUY"
	if !SkipActivity(item) {
		t.Error("skip_activity: got False, want True")
	}
}

func TestMarginInterestChargeWithoutStatusIsKept(t *testing.T) {
	item := Item{
		"type":        "INTEREST_CHARGE",
		"subType":     "MARGIN_INTEREST",
		"status":      nil,
		"amount":      "412.10",
		"amountSign":  "negative",
		"currency":    "CAD",
		"occurredAt":  "2026-06-01T04:00:00.000000+00:00",
		"accountId":   "non-registered-x",
		"canonicalId": "int-1",
	}
	if SkipActivity(item) {
		t.Fatal("skip_activity: got True, want False")
	}
	rec := mustMap(t, item)
	if rec.ActivityType != "INTEREST_CHARGE" {
		t.Errorf("activityType: got %q, want INTEREST_CHARGE", rec.ActivityType)
	}
	almostEqual(t, "netCashAmount", rec.NetCashAmount, -412.10)
	if rec.TransactionDate != "2026-06-01" {
		t.Errorf("transactionDate: got %q, want 2026-06-01", rec.TransactionDate)
	}
}

func TestOptionsBuyMapsAsBuyToOpen(t *testing.T) {
	item := wsItem(Item{
		"type":          "OPTIONS_BUY",
		"subType":       "LIMIT_ORDER",
		"assetSymbol":   "QNC",
		"contractType":  "CALL",
		"strikePrice":   3,
		"expiryDate":    "2027-02-19",
		"assetQuantity": 5,
		"amount":        150,
		"amountSign":    "negative",
	})
	row := mustMap(t, item)
	if row.ActivitySubType != "BUYTOOPEN" {
		t.Errorf("activitySubType: got %q, want BUYTOOPEN", row.ActivitySubType)
	}
	if row.Category != "trade" {
		t.Errorf("category: got %q, want trade", row.Category)
	}
	if row.Quantity != 5 {
		t.Errorf("quantity: got %v, want 5", row.Quantity)
	}
	if row.NetCashAmount != -150 {
		t.Errorf("netCashAmount: got %v, want -150", row.NetCashAmount)
	}
}

func TestMapActivityOptionsMultilegDebitIsBuyToClose(t *testing.T) {
	row := mustMap(t, wsItem(Item{
		"type":          "OPTIONS_MULTILEG",
		"subType":       "FILLED",
		"status":        "FILLED",
		"assetSymbol":   "LUNR",
		"contractType":  "CALL",
		"strikePrice":   12,
		"expiryDate":    "2027-01-15",
		"assetQuantity": nil,
		"amount":        128,
		"amountSign":    "negative",
		"currency":      "USD",
	}))
	if row.Category != "trade" {
		t.Errorf("category: got %q, want trade", row.Category)
	}
	if row.ActivityType != "OPTIONS_BUY" {
		t.Errorf("activityType: got %q, want OPTIONS_BUY", row.ActivityType)
	}
	if row.ActivitySubType != "BUYTOCLOSE" {
		t.Errorf("activitySubType: got %q, want BUYTOCLOSE", row.ActivitySubType)
	}
	if row.Quantity != 0 {
		t.Errorf("quantity: got %v, want 0", row.Quantity)
	}
	if row.UnitPrice != 0 {
		t.Errorf("unitPrice: got %v, want 0", row.UnitPrice)
	}
	if row.NetCashAmount != -128 {
		t.Errorf("netCashAmount: got %v, want -128", row.NetCashAmount)
	}
	if row.Symbol != "LUNR 15JAN27 12.00 CALL" {
		t.Errorf("symbol: got %q, want %q", row.Symbol, "LUNR 15JAN27 12.00 CALL")
	}
}

func TestMapActivityOptionsMultilegCreditIsSellToOpen(t *testing.T) {
	row := mustMap(t, wsItem(Item{
		"type":          "OPTIONS_MULTILEG",
		"subType":       "FILLED",
		"status":        "FILLED",
		"assetSymbol":   "BBAI",
		"contractType":  "CALL",
		"strikePrice":   10,
		"expiryDate":    "2028-01-21",
		"assetQuantity": nil,
		"amount":        56,
		"amountSign":    "positive",
		"currency":      "USD",
	}))
	if row.Category != "trade" {
		t.Errorf("category: got %q, want trade", row.Category)
	}
	if row.ActivityType != "OPTIONS_SELL" {
		t.Errorf("activityType: got %q, want OPTIONS_SELL", row.ActivityType)
	}
	if row.ActivitySubType != "SELLTOOPEN" {
		t.Errorf("activitySubType: got %q, want SELLTOOPEN", row.ActivitySubType)
	}
	if row.Quantity != 0 {
		t.Errorf("quantity: got %v, want 0", row.Quantity)
	}
	if row.UnitPrice != 0 {
		t.Errorf("unitPrice: got %v, want 0", row.UnitPrice)
	}
	if row.NetCashAmount != 56 {
		t.Errorf("netCashAmount: got %v, want 56", row.NetCashAmount)
	}
	if row.Symbol != "BBAI 21JAN28 10.00 CALL" {
		t.Errorf("symbol: got %q, want %q", row.Symbol, "BBAI 21JAN28 10.00 CALL")
	}
}

func TestMapActivityOptionsShortExpiryCoversShort(t *testing.T) {
	row := mustMap(t, wsItem(Item{
		"type":          "OPTIONS_SHORT_EXPIRY",
		"subType":       "EXPIRED",
		"status":        "POSTED",
		"assetSymbol":   "LUNR",
		"contractType":  "CALL",
		"strikePrice":   12,
		"expiryDate":    "2027-01-15",
		"assetQuantity": 16,
		"amount":        0,
		"amountSign":    "negative",
		"currency":      "USD",
	}))
	if row.Category != "option_event" {
		t.Errorf("category: got %q, want option_event", row.Category)
	}
	if row.ActivityType != "EXPIR" {
		t.Errorf("activityType: got %q, want EXPIR", row.ActivityType)
	}
	if row.ActivitySubType != "BUY" {
		t.Errorf("activitySubType: got %q, want BUY", row.ActivitySubType)
	}
	if row.Quantity != 16 {
		t.Errorf("quantity: got %v, want 16", row.Quantity)
	}
	if row.UnitPrice != 0 {
		t.Errorf("unitPrice: got %v, want 0", row.UnitPrice)
	}
	if row.NetCashAmount != 0 {
		t.Errorf("netCashAmount: got %v, want 0", row.NetCashAmount)
	}
}

func TestMapActivityOptionsExpirySellsLongAssignCoversShort(t *testing.T) {
	expiry := mustMap(t, wsItem(Item{
		"type":          "OPTIONS_EXPIRY",
		"subType":       "EXPIRED",
		"assetSymbol":   "LUNR",
		"contractType":  "CALL",
		"strikePrice":   12,
		"expiryDate":    "2025-08-22",
		"assetQuantity": 4,
		"amount":        0,
	}))
	if expiry.Category != "option_event" {
		t.Errorf("expiry category: got %q, want option_event", expiry.Category)
	}
	if expiry.ActivityType != "EXPIR" {
		t.Errorf("expiry activityType: got %q, want EXPIR", expiry.ActivityType)
	}
	if expiry.ActivitySubType != "SELL" {
		t.Errorf("expiry activitySubType: got %q, want SELL", expiry.ActivitySubType)
	}
	if expiry.Quantity != -4 {
		t.Errorf("expiry quantity: got %v, want -4", expiry.Quantity)
	}
	if expiry.UnitPrice != 0 {
		t.Errorf("expiry unitPrice: got %v, want 0", expiry.UnitPrice)
	}
	assign := mustMap(t, wsItem(Item{
		"type":          "OPTIONS_ASSIGN",
		"subType":       "ASSIGNED",
		"assetSymbol":   "ASTS",
		"contractType":  "CALL",
		"strikePrice":   31,
		"expiryDate":    "2025-03-07",
		"assetQuantity": 1,
		"amount":        3100,
		"amountSign":    "negative",
		"currency":      "USD",
	}))
	if assign.Category != "option_event" {
		t.Errorf("assign category: got %q, want option_event", assign.Category)
	}
	if assign.ActivityType != "ASSIGN" {
		t.Errorf("assign activityType: got %q, want ASSIGN", assign.ActivityType)
	}
	if assign.ActivitySubType != "BUYTOCLOSE" {
		t.Errorf("assign activitySubType: got %q, want BUYTOCLOSE", assign.ActivitySubType)
	}
	if assign.Quantity != 1 {
		t.Errorf("assign quantity: got %v, want 1", assign.Quantity)
	}
	if assign.UnitPrice != 0 {
		t.Errorf("assign unitPrice: got %v, want 0", assign.UnitPrice)
	}
	if assign.Symbol != "ASTS 07MAR25 31.00 CALL" {
		t.Errorf("assign symbol: got %q, want %q", assign.Symbol, "ASTS 07MAR25 31.00 CALL")
	}
}

func TestMapActivityOptionUnitPriceIsPerShare(t *testing.T) {
	cheap := mustMap(t, wsItem(Item{
		"type":          "OPTIONS_SELL",
		"subType":       "LIMIT_ORDER",
		"assetSymbol":   "DRAM",
		"contractType":  "CALL",
		"strikePrice":   1,
		"expiryDate":    "2027-02-19",
		"assetQuantity": 10,
		"amount":        112.5,
		"amountSign":    "positive",
	}))
	almostEqual(t, "cheap unitPrice", cheap.UnitPrice, 0.1125)
	pricey := mustMap(t, wsItem(Item{
		"type":          "OPTIONS_SELL",
		"subType":       "LIMIT_ORDER",
		"assetSymbol":   "SOXL",
		"contractType":  "CALL",
		"strikePrice":   20,
		"expiryDate":    "2027-02-19",
		"assetQuantity": 10,
		"amount":        13300,
		"amountSign":    "positive",
	}))
	almostEqual(t, "pricey unitPrice", pricey.UnitPrice, 13.3)
	shareItem := wsItem(Item{"amount": 100, "assetQuantity": 10})
	share := mustMap(t, shareItem)
	if isOption(shareItem) {
		t.Error("_is_option: got True, want False")
	}
	almostEqual(t, "share unitPrice", share.UnitPrice, 10.0)
}

func TestOccurredAtNotCutToDateForWealthsimpleRow(t *testing.T) {
	row := mustMap(t, wsItem(nil))
	if row.OccurredAt != "2024-06-15T13:45:22.123Z" {
		t.Errorf("occurredAt: got %q, want %q", row.OccurredAt, "2024-06-15T13:45:22.123Z")
	}
	if row.TransactionDate != "2024-06-15" {
		t.Errorf("transactionDate: got %q, want 2024-06-15", row.TransactionDate)
	}
	if row.ID != "" {
		t.Errorf("id: got %q, want none", row.ID)
	}
	st := store.MustOpen(t.TempDir())
	t.Cleanup(func() { st.Close() })
	if err := st.Ensure(); err != nil {
		t.Fatal(err)
	}
	st.ApplyWealthsimpleMapped([]store.Activity{*row})
	rows := st.Snapshot(true).Activities
	if len(rows) == 0 {
		t.Fatal("no stored activities")
	}
	stored := rows[0]
	if stored.OccurredAt != "2024-06-15T13:45:22.123Z" {
		t.Errorf("stored occurredAt: got %q, want %q", stored.OccurredAt, "2024-06-15T13:45:22.123Z")
	}
	if !strings.Contains(stored.OccurredAt, "T") {
		t.Errorf("stored occurredAt %q has no T", stored.OccurredAt)
	}
}

func TestMapActivityCopiesSecurityID(t *testing.T) {
	row := mustMap(t, wsItem(Item{"securityId": "sec-s-abc123"}))
	if row.SecurityID != "sec-s-abc123" {
		t.Errorf("securityId: got %q, want sec-s-abc123", row.SecurityID)
	}
}

func TestOnlyOpenMarginAccountsAreAskedForBuyingPower(t *testing.T) {
	accounts := []Item{
		{"id": "tfsa-1", "unifiedAccountType": "SELF_DIRECTED_TFSA", "status": "open"},
		{"id": "nr-1", "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN", "status": "open"},
		{"id": "nr-2", "unifiedAccountType": "SELF_DIRECTED_JOINT_NON_REGISTERED_MARGIN", "status": "closed"},
		{"id": "cash-1", "unifiedAccountType": "CASH", "status": "open"},
		{"id": "", "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN", "status": "open"},
	}
	got := MarginAccountIDs(accounts)
	if strings.Join(got, ",") != "nr-1" {
		t.Errorf("margin_account_ids: got %v, want [nr-1]: a TFSA's buying power is cash, not margin; a closed margin account holds nothing", got)
	}
}

func TestNavAccountGroupsJoinsSameNickname(t *testing.T) {
	groups, _ := NavAccountGroups([]Item{
		{"id": "cad-1", "nickname": "TFSA", "currency": "CAD"},
		{"id": "usd-1", "nickname": "TFSA", "currency": "USD"},
		{"id": "rrsp-1", "nickname": "", "unifiedAccountType": "RRSP"},
	})
	if strings.Join(groups["TFSA"], ",") != "cad-1,usd-1" {
		t.Errorf("TFSA: got %v, want [cad-1 usd-1]", groups["TFSA"])
	}
	if strings.Join(groups["RRSP"], ",") != "rrsp-1" {
		t.Errorf("RRSP: got %v, want [rrsp-1]", groups["RRSP"])
	}
}

func TestNavPointsFromPayloadAcceptsV2AndIdentity(t *testing.T) {
	identPts, _ := NavPointsFromPayload(map[string]any{
		"identity": map[string]any{
			"financials": map[string]any{
				"historicalDaily": map[string]any{
					"edges": []any{
						map[string]any{
							"node": map[string]any{
								"date":                "2024-02-01",
								"netLiquidationValue": map[string]any{"amount": 10, "currency": "CAD"},
								"netDeposits":         map[string]any{"amount": 1, "currency": "CAD"},
							},
						},
					},
					"pageInfo": map[string]any{},
				},
			},
		},
	})
	if len(identPts) == 0 {
		t.Fatal("identity payload: no points")
	}
	if identPts[0].Equity != 10.0 {
		t.Errorf("identity equity: got %v, want 10.0", identPts[0].Equity)
	}
	if identPts[0].NetDeposits == nil || *identPts[0].NetDeposits != 1.0 {
		t.Errorf("identity netDeposits: got %v, want 1.0", identPts[0].NetDeposits)
	}
	accPts, _ := NavPointsFromPayload(map[string]any{
		"account": map[string]any{
			"financials": map[string]any{
				"historicalDaily": map[string]any{
					"edges": []any{
						map[string]any{
							"node": map[string]any{
								"date":                  "2024-02-01",
								"netLiquidationValueV2": map[string]any{"amount": "20", "currency": "CAD"},
								"netDepositsV2":         map[string]any{"amount": "4", "currency": "CAD"},
							},
						},
					},
					"pageInfo": map[string]any{},
				},
			},
		},
	})
	if len(accPts) == 0 {
		t.Fatal("account payload: no points")
	}
	if accPts[0].Equity != 20.0 {
		t.Errorf("account equity: got %v, want 20.0", accPts[0].Equity)
	}
	if accPts[0].NetDeposits == nil || *accPts[0].NetDeposits != 4.0 {
		t.Errorf("account netDeposits: got %v, want 4.0", accPts[0].NetDeposits)
	}
}

func TestMergeNavPointsSumsEquityAndDeposits(t *testing.T) {
	merged := MergeNavPoints([][]store.NavPoint{
		{{Date: "2024-01-01", Equity: 10, Currency: "CAD", NetDeposits: py.Ptr(1)}},
		{
			{Date: "2024-01-01", Equity: 5, Currency: "CAD", NetDeposits: py.Ptr(2)},
			{Date: "2024-01-02", Equity: 6, Currency: "CAD"},
		},
	})
	if len(merged) != 2 {
		t.Fatalf("merged: got %d points, want 2", len(merged))
	}
	if merged[0].Date != "2024-01-01" {
		t.Errorf("merged[0].date: got %q, want 2024-01-01", merged[0].Date)
	}
	if merged[0].Equity != 15.0 {
		t.Errorf("merged[0].equity: got %v, want 15.0", merged[0].Equity)
	}
	if merged[0].NetDeposits == nil || *merged[0].NetDeposits != 3.0 {
		t.Errorf("merged[0].netDeposits: got %v, want 3.0", merged[0].NetDeposits)
	}
	if merged[1].Date != "2024-01-02" {
		t.Errorf("merged[1].date: got %q, want 2024-01-02", merged[1].Date)
	}
	if merged[1].Equity != 6.0 {
		t.Errorf("merged[1].equity: got %v, want 6.0", merged[1].Equity)
	}
	if merged[1].NetDeposits != nil {
		t.Errorf("merged[1].netDeposits: got %v, want none", *merged[1].NetDeposits)
	}
}

func TestCollateralAccountNamesTheMarginAccountItBacks(t *testing.T) {
	raw := []Item{
		{"id": "acct-margin", "nickname": "Trading", "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN", "currency": "CAD", "status": "open", "type": "non_registered",
			"custodianAccounts": []any{map[string]any{"id": "cust-margin-1"}}, "accountFeatures": []any{map[string]any{"name": "MARGIN", "enabled": true, "functional": true, "metadata": nil}}},
		{"id": "acct-tfsa", "nickname": "TFSA", "unifiedAccountType": "SELF_DIRECTED_TFSA", "currency": "CAD", "status": "open", "type": "tfsa",
			"custodianAccounts": []any{map[string]any{"id": "cust-tfsa-1"}}, "accountFeatures": []any{map[string]any{"name": "MARGIN_BOOST", "enabled": true, "functional": true, "metadata": map[string]any{"__typename": "MarginBoostFeatureMetadata", "targetMarginAccountId": "cust-margin-1"}}}},
		{"id": "acct-rrsp", "nickname": "RRSP", "unifiedAccountType": "SELF_DIRECTED_RRSP", "currency": "CAD", "status": "open", "type": "rrsp",
			"custodianAccounts": []any{map[string]any{"id": "cust-rrsp-1"}}, "accountFeatures": []any{map[string]any{"name": "MARGIN_BOOST", "enabled": false, "functional": false, "metadata": map[string]any{"__typename": "MarginBoostFeatureMetadata", "targetMarginAccountId": "cust-margin-1"}}}},
		{"id": "acct-lira", "nickname": "LIRA", "unifiedAccountType": "SELF_DIRECTED_LIRA", "currency": "CAD", "status": "open", "type": "lira", "custodianAccounts": []any{}, "accountFeatures": []any{}},
	}
	slim := map[string]store.Account{}
	for _, a := range SlimAccounts(raw) {
		slim[a.ID] = a
	}
	if slim["acct-tfsa"].MarginAccountID != "acct-margin" {
		t.Errorf("acct-tfsa marginAccountId: got %q, want acct-margin: the feature's custodian id resolved to the margin account", slim["acct-tfsa"].MarginAccountID)
	}
	if slim["acct-rrsp"].MarginAccountID != "" {
		t.Errorf("acct-rrsp marginAccountId: got %q, want \"\": a feature that is not enabled links nothing", slim["acct-rrsp"].MarginAccountID)
	}
	if slim["acct-margin"].MarginAccountID != "" || slim["acct-lira"].MarginAccountID != "" {
		t.Errorf("(acct-margin, acct-lira) marginAccountId: got (%q, %q), want (\"\", \"\")", slim["acct-margin"].MarginAccountID, slim["acct-lira"].MarginAccountID)
	}
}
