package ws

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

type gqlCall struct {
	operation string
	variables map[string]any
	query     string
}

func gqlClient(t *testing.T, answer func(call gqlCall) map[string]any) (*Client, *[]gqlCall) {
	t.Helper()
	calls := &[]gqlCall{}
	c := NewClient(t.TempDir())
	c.HTTP.Transport = roundTripFunc(func(req *http.Request) (*http.Response, error) {
		if req.Method != http.MethodPost || req.URL.String() != GraphQLURL {
			t.Errorf("unexpected request %s %s", req.Method, req.URL)
			return httpResponse(req, 500, nil, nil), nil
		}
		raw, _ := io.ReadAll(req.Body)
		var body map[string]any
		if err := json.Unmarshal(raw, &body); err != nil {
			t.Errorf("graphql body is not JSON: %v", err)
			return httpResponse(req, 500, nil, nil), nil
		}
		vars, _ := body["variables"].(map[string]any)
		call := gqlCall{operation: py.S(body["operationName"]), variables: vars, query: py.S(body["query"])}
		*calls = append(*calls, call)
		out, err := json.Marshal(map[string]any{"data": answer(call)})
		if err != nil {
			t.Errorf("answer is not JSON: %v", err)
			return httpResponse(req, 500, nil, nil), nil
		}
		return httpResponse(req, 200, out, map[string]string{"Content-Type": "application/json"}), nil
	})
	return c, calls
}

func strList(v any) []string {
	raw, _ := v.([]any)
	out := make([]string, 0, len(raw))
	for _, x := range raw {
		out = append(out, py.S(x))
	}
	return out
}

func operations(calls []gqlCall) []string {
	out := make([]string, 0, len(calls))
	for _, c := range calls {
		out = append(out, c.operation)
	}
	return out
}

func marginTuple(r store.Margin) string {
	bp := "None"
	if r.BuyingPower != nil {
		bp = py.Repr(*r.BuyingPower)
	}
	return fmt.Sprintf("(%s, %s, %q)", r.AccountID, bp, r.Unavailable)
}

func marginTuples(rows []store.Margin) string {
	parts := make([]string, 0, len(rows))
	for _, r := range rows {
		parts = append(parts, marginTuple(r))
	}
	return "[" + strings.Join(parts, ", ") + "]"
}

func emptyNavHistory(call gqlCall) map[string]any {
	return map[string]any{"identity": map[string]any{"financials": map[string]any{"historicalDaily": map[string]any{"edges": []any{}, "pageInfo": map[string]any{}}}}}
}

func TestDailyPathDoesNotPageWholeHistoryWhenRowsExist(t *testing.T) {
	n := 0
	c, calls := gqlClient(t, func(call gqlCall) map[string]any {
		n++
		return map[string]any{
			"activityFeedItems": map[string]any{
				"edges": []any{
					map[string]any{
						"node": wsItem(Item{
							"canonicalId": "ws-cid-aaa-001",
							"occurredAt":  "2024-06-15T13:45:22.123Z",
						}),
					},
				},
				"pageInfo": map[string]any{"hasNextPage": n < 2, "endCursor": "cursor-page-2"},
			},
		}
	})
	if _, err := c.FetchActivitiesForAccount(Session{"access_token": "x"}, "acct-1", "2024-06-01"); err != nil {
		t.Fatal(err)
	}
	if len(*calls) != 2 {
		t.Fatalf("calls: got %d, want 2: a page of known rows does not end the walk", len(*calls))
	}
	cond, _ := (*calls)[0].variables["condition"].(map[string]any)
	if _, ok := cond["startDate"]; !ok {
		t.Fatalf("condition %v lacks startDate", cond)
	}
	if !strings.HasPrefix(py.S(cond["startDate"]), "2024-06-01") {
		t.Errorf("startDate: got %q, want a 2024-06-01 prefix", py.S(cond["startDate"]))
	}
}

func TestDailyWindowReachesBackPastRowsFiledUnderALaterDay(t *testing.T) {
	cond := ActivityFetchCondition("acct-1", "2026-08-26", time.Now().UTC())
	if got := py.S(cond["startDate"]); !(got < "2026-09-08T04:00:00.000Z") {
		t.Errorf("startDate: got %q, want less than 2026-09-08T04:00:00.000Z: a dividend filed under the 8th is inside the window", got)
	}
}

func TestEmptyTableFullHistoryOmitsStartDate(t *testing.T) {
	cond := ActivityFetchCondition("acct-1", "", time.Now().UTC())
	if _, ok := cond["startDate"]; ok {
		t.Errorf("condition %v carries startDate", cond)
	}
}

func TestParseMarginAndFetchMargin(t *testing.T) {
	available := map[string]any{"account": map[string]any{"financials": map[string]any{"current": map[string]any{"marginV3": map[string]any{"trading": map[string]any{"buyingPower": map[string]any{"__typename": "BuyingPowerMetricAvailable", "total": map[string]any{"amount": "6817.33", "currency": "CAD"}, "restrictions": []any{}}}}}}}}
	unavailable := map[string]any{"account": map[string]any{"financials": map[string]any{"current": map[string]any{"marginV3": map[string]any{"trading": map[string]any{"buyingPower": map[string]any{"__typename": "BuyingPowerMetricUnavailable", "reason": map[string]any{"__typename": "UnavailableSecurities", "securities": []any{map[string]any{"securityId": "s1", "status": "x"}, map[string]any{"securityId": "s2", "status": "x"}}}}}}}}}}
	none := map[string]any{"account": map[string]any{"financials": map[string]any{"current": map[string]any{"marginV3": nil}}}}
	if got := ParseMargin(available); got == nil || got.BuyingPower == nil || *got.BuyingPower != 6817.33 || got.Currency != "CAD" || got.Unavailable != "" {
		t.Errorf("parse_margin(available): got %+v, want {buyingPower: 6817.33, currency: CAD, unavailable: \"\"}", got)
	}
	if got := ParseMargin(unavailable); got == nil || got.BuyingPower != nil || got.Currency != "CAD" || got.Unavailable != "UnavailableSecurities (2 securities)" {
		t.Errorf("parse_margin(unavailable): got %+v, want {buyingPower: None, currency: CAD, unavailable: \"UnavailableSecurities (2 securities)\"}", got)
	}
	if got := ParseMargin(none); got != nil {
		t.Errorf("parse_margin(none): got %+v, want None", got)
	}
	if got := ParseMargin(map[string]any{}); got != nil {
		t.Errorf("parse_margin({}): got %+v, want None", got)
	}
	answers := map[string]map[string]any{"acct-1": available, "acct-2": none, "acct-3": unavailable}
	c, calls := gqlClient(t, func(call gqlCall) map[string]any {
		return answers[py.S(call.variables["accountId"])]
	})
	rows := c.FetchMargin(Session{"access_token": "x"}, []string{"acct-1", "acct-2", "acct-3", ""})
	if got := strings.Join(operations(*calls), ","); got != "FetchAccountCurrentMarginBuyingPowerV2,FetchAccountCurrentMarginBuyingPowerV2,FetchAccountCurrentMarginBuyingPowerV2" {
		t.Errorf("operations: got %q", got)
	}
	for i, call := range *calls {
		if py.S(call.variables["currency"]) != "CAD" {
			t.Errorf("call %d currency: got %v, want CAD", i, call.variables["currency"])
		}
	}
	want := `[(acct-1, 6817.33, ""), (acct-3, None, "UnavailableSecurities (2 securities)")]`
	if got := marginTuples(rows); got != want {
		t.Errorf("rows: got %s, want %s: an account without margin figures is not a row", got, want)
	}
	for _, r := range rows {
		if r.FetchedAt == "" {
			t.Errorf("row %s has no fetchedAt", r.AccountID)
		}
	}
}

func TestFetchSecuritiesBatchesIDs(t *testing.T) {
	c, calls := gqlClient(t, func(call gqlCall) map[string]any {
		rows := []any{}
		for _, sid := range strList(call.variables["ids"]) {
			switch {
			case sid == "sec-s-missing":
				rows = append(rows, nil)
			case strings.HasPrefix(sid, "sec-o-"):
				rows = append(rows, map[string]any{"id": sid, "currency": "USD", "stock": map[string]any{"symbol": "LUNR", "name": ""},
					"optionDetails": map[string]any{"underlyingSecurity": map[string]any{"id": "sec-s-under", "currency": "USD"}}})
			default:
				rows = append(rows, map[string]any{"id": sid, "currency": "CAD", "stock": map[string]any{"symbol": "NSAV", "name": "Ninepoint", "primaryExchange": "TSX", "primaryMic": "XTSE"}, "optionDetails": nil})
			}
		}
		return map[string]any{"securities": rows}
	})
	ids := []string{}
	for i := 0; i < 60; i++ {
		ids = append(ids, fmt.Sprintf("sec-s-%d", i))
	}
	ids = append(ids, "sec-o-1", "sec-s-missing", "sec-s-1")
	recs := c.FetchSecurities(Session{"access_token": "t"}, ids)
	if got := strings.Join(operations(*calls), ","); got != "FetchSecurities,FetchSecurities" {
		t.Fatalf("operations: got %q, want FetchSecurities,FetchSecurities", got)
	}
	if n := len(strList((*calls)[0].variables["ids"])); n != 50 {
		t.Errorf("first batch: got %d ids, want 50", n)
	}
	if n := len(strList((*calls)[1].variables["ids"])); n != 12 {
		t.Errorf("second batch: got %d ids, want 12", n)
	}
	if len(recs) != 61 {
		t.Fatalf("records: got %d, want 61", len(recs))
	}
	var opt *store.Security
	for i := range recs {
		if recs[i].ID == "sec-o-1" {
			opt = &recs[i]
			break
		}
	}
	if opt == nil {
		t.Fatal("sec-o-1 missing from the records")
	}
	if opt.UnderlyingID != "sec-s-under" {
		t.Errorf("sec-o-1 underlyingId: got %q, want sec-s-under", opt.UnderlyingID)
	}
	if recs[0].PrimaryExchange != "TSX" {
		t.Errorf("recs[0].primaryExchange: got %q, want TSX", recs[0].PrimaryExchange)
	}
}

func TestFetchSecurityReadsStockFields(t *testing.T) {
	c, _ := gqlClient(t, func(call gqlCall) map[string]any {
		if call.operation != "FetchSecurity" {
			t.Errorf("operation: got %q, want FetchSecurity", call.operation)
		}
		if py.S(call.variables["securityId"]) != "sec-s-ch" {
			t.Errorf("securityId: got %v, want sec-s-ch", call.variables["securityId"])
		}
		return map[string]any{
			"security": map[string]any{
				"id":       "sec-s-ch",
				"currency": "CAD",
				"stock": map[string]any{
					"name":            "Charbone Corporation",
					"primaryExchange": "TSX Venture Exchange",
					"primaryMic":      "XTSV",
					"symbol":          "CH",
				},
				"optionDetails": map[string]any{},
			},
		}
	})
	rec := c.FetchSecurity(Session{"access_token": "t"}, "sec-s-ch")
	if rec == nil {
		t.Fatal("fetch_security returned None")
	}
	if rec.Name != "Charbone Corporation" {
		t.Errorf("name: got %q, want Charbone Corporation", rec.Name)
	}
	if rec.Symbol != "CH" {
		t.Errorf("symbol: got %q, want CH", rec.Symbol)
	}
	if rec.PrimaryMic != "XTSV" {
		t.Errorf("primaryMic: got %q, want XTSV", rec.PrimaryMic)
	}
	if rec.Currency != "CAD" {
		t.Errorf("currency: got %q, want CAD", rec.Currency)
	}
}

func TestFetchNavHistorySinceDateSkipsOlderYears(t *testing.T) {
	c, calls := gqlClient(t, emptyNavHistory)
	if _, err := c.FetchNavHistory(Session{"access_token": "t"}, "ident-1", "2026-08-30"); err != nil {
		t.Fatal(err)
	}
	if len(*calls) == 0 {
		t.Fatal("no graphql calls")
	}
	for _, call := range *calls {
		start := py.S(call.variables["startDate"])
		if start < "2026-08-30" {
			t.Errorf("startDate %q is before 2026-08-30", start)
		}
		if !strings.HasPrefix(start, "2026") {
			t.Errorf("startDate %q is not in 2026", start)
		}
	}
}

func TestFetchNavHistoryIdentityWideOmitsAccountIDs(t *testing.T) {
	c, calls := gqlClient(t, emptyNavHistory)
	if _, err := c.FetchNavHistory(Session{"access_token": "t"}, "ident-1", ""); err != nil {
		t.Fatal(err)
	}
	if len(*calls) == 0 {
		t.Fatal("no graphql calls")
	}
	for _, call := range *calls {
		if call.operation != "IdentityHistoricalFinancialsQuery" {
			t.Errorf("operation: got %q, want IdentityHistoricalFinancialsQuery", call.operation)
		}
		if _, ok := call.variables["accountIds"]; ok {
			t.Errorf("variables %v carry accountIds", call.variables)
		}
		if call.query != QIdentityHistoricalFinancials {
			t.Error("query is not Q_IDENTITY_HISTORICAL_FINANCIALS")
		}
		if py.Num(call.variables["limit"], -1) != 400 {
			t.Errorf("limit: got %v, want 400", call.variables["limit"])
		}
	}
	if strings.Contains(QIdentityHistoricalFinancials, "$accountIds") {
		t.Error("Q_IDENTITY_HISTORICAL_FINANCIALS carries $accountIds")
	}
	if strings.Contains(QIdentityHistoricalFinancials, "accounts: $accountIds") {
		t.Error("Q_IDENTITY_HISTORICAL_FINANCIALS carries accounts: $accountIds")
	}
}

func TestFetchAccountNavHistoryUsesAccountQuery(t *testing.T) {
	c, calls := gqlClient(t, func(call gqlCall) map[string]any {
		return map[string]any{
			"account": map[string]any{
				"financials": map[string]any{
					"historicalDaily": map[string]any{
						"edges": []any{
							map[string]any{
								"node": map[string]any{
									"date":                  "2024-01-02",
									"netLiquidationValueV2": map[string]any{"amount": "12.5", "currency": "CAD"},
									"netDepositsV2":         map[string]any{"amount": "3", "currency": "CAD"},
								},
							},
						},
						"pageInfo": map[string]any{},
					},
				},
			},
		}
	})
	pts, err := c.FetchAccountNavHistory(Session{"access_token": "t"}, "acct-1", "")
	if err != nil {
		t.Fatal(err)
	}
	if len(*calls) == 0 {
		t.Fatal("no graphql calls")
	}
	if len(pts) == 0 {
		t.Fatal("no points")
	}
	if pts[0].Date != "2024-01-02" {
		t.Errorf("date: got %q, want 2024-01-02", pts[0].Date)
	}
	if pts[0].Equity != 12.5 {
		t.Errorf("equity: got %v, want 12.5", pts[0].Equity)
	}
	if pts[0].NetDeposits == nil || *pts[0].NetDeposits != 3.0 {
		t.Errorf("netDeposits: got %v, want 3.0", pts[0].NetDeposits)
	}
	for _, call := range *calls {
		if call.operation != "FetchAccountHistoricalFinancials" {
			t.Errorf("operation: got %q, want FetchAccountHistoricalFinancials", call.operation)
		}
		if py.S(call.variables["id"]) != "acct-1" {
			t.Errorf("id: got %v, want acct-1", call.variables["id"])
		}
		if py.Num(call.variables["first"], -1) != 400 {
			t.Errorf("first: got %v, want 400", call.variables["first"])
		}
		if py.S(call.variables["resolution"]) != "DAILY" {
			t.Errorf("resolution: got %v, want DAILY", call.variables["resolution"])
		}
		if _, ok := call.variables["accountIds"]; ok {
			t.Errorf("variables %v carry accountIds", call.variables)
		}
		if _, ok := call.variables["identityId"]; ok {
			t.Errorf("variables %v carry identityId", call.variables)
		}
		if call.query != QFetchAccountHistoricalFinancials {
			t.Error("query is not Q_FETCH_ACCOUNT_HISTORICAL_FINANCIALS")
		}
	}
	if !strings.Contains(QFetchAccountHistoricalFinancials, "account(id: $id)") {
		t.Error("Q_FETCH_ACCOUNT_HISTORICAL_FINANCIALS lacks account(id: $id)")
	}
	if !strings.Contains(QFetchAccountHistoricalFinancials, "$resolution: DateResolution!") {
		t.Error("Q_FETCH_ACCOUNT_HISTORICAL_FINANCIALS lacks $resolution: DateResolution!")
	}
	if _, ok := Queries["FetchAccountHistoricalFinancials"]; !ok {
		t.Error("QUERIES lacks FetchAccountHistoricalFinancials")
	}
}
