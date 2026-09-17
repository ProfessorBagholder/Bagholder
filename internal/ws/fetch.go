package ws

import (
	"fmt"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

const SecurityBatch = 50

func (c *Client) FetchAllAccounts(sess Session, identityID string) ([]Item, error) {
	var accounts []Item
	var cursor any
	for {
		variables := map[string]any{"identityId": identityID, "pageSize": 25, "startDate": "2015-01-01", "cursor": cursor}
		data, err := c.GraphQL(sess, "FetchAllAccountFinancials", variables, "")
		if err != nil {
			return nil, err
		}
		ident, _ := data["identity"].(map[string]any)
		conn, _ := ident["accounts"].(map[string]any)
		edges, _ := conn["edges"].([]any)
		for _, raw := range edges {
			edge, _ := raw.(map[string]any)
			if node, ok := edge["node"].(map[string]any); ok && node != nil {
				accounts = append(accounts, node)
			}
		}
		page, _ := conn["pageInfo"].(map[string]any)
		if next, _ := page["hasNextPage"].(bool); !next {
			break
		}
		cursor = page["endCursor"]
		if py.S(cursor) == "" {
			break
		}
	}
	return accounts, nil
}

func ActivityFetchCondition(accountID, startDate string, now time.Time) map[string]any {
	end := now.Add(24 * time.Hour)
	cond := map[string]any{"endDate": end.UTC().Format("2006-01-02T15:04:05.999Z"), "accountIds": []string{accountID}}
	if raw := py.Strip(startDate); raw != "" {
		if !strings.Contains(raw, "T") {
			if len(raw) > 10 {
				raw = raw[:10]
			}
			raw += "T00:00:00.000Z"
		}
		cond["startDate"] = raw
	}
	return cond
}

func (c *Client) FetchActivitiesForAccount(sess Session, accountID, startDate string) ([]Item, error) {
	var items []Item
	var cursor string
	for {
		variables := map[string]any{"first": 100, "orderBy": "OCCURRED_AT_DESC", "condition": ActivityFetchCondition(accountID, startDate, time.Now().UTC())}
		if cursor != "" {
			variables["cursor"] = cursor
		}
		data, err := c.GraphQL(sess, "FetchActivityFeedItems", variables, "")
		if err != nil {
			return nil, err
		}
		feed, _ := data["activityFeedItems"].(map[string]any)
		edges, _ := feed["edges"].([]any)
		for _, raw := range edges {
			edge, _ := raw.(map[string]any)
			if node, ok := edge["node"].(map[string]any); ok && node != nil {
				items = append(items, node)
			}
		}
		page, _ := feed["pageInfo"].(map[string]any)
		if next, _ := page["hasNextPage"].(bool); !next {
			break
		}
		cursor = py.S(page["endCursor"])
		if cursor == "" {
			break
		}
	}
	return items, nil
}

func (c *Client) FetchBalances(sess Session, accountIDs []string) ([]store.Balance, error) {
	balances := []store.Balance{}
	var ids []string
	for _, id := range accountIDs {
		if id != "" {
			ids = append(ids, id)
		}
	}
	for i := 0; i < len(ids); i += 20 {
		end := i + 20
		if end > len(ids) {
			end = len(ids)
		}
		data, err := c.GraphQL(sess, "FetchAccountsWithBalance", map[string]any{"ids": ids[i:end], "type": "TRADING"}, "")
		if err != nil {
			return nil, err
		}
		accs, _ := data["accounts"].([]any)
		for _, raw := range accs {
			acc, _ := raw.(map[string]any)
			aid := py.S(acc["id"])
			cas, _ := acc["custodianAccounts"].([]any)
			for _, craw := range cas {
				ca, _ := craw.(map[string]any)
				fin, _ := ca["financials"].(map[string]any)
				var bals []any
				switch v := fin["balance"].(type) {
				case []any:
					bals = v
				case map[string]any:
					bals = []any{v}
				}
				for _, braw := range bals {
					b, _ := braw.(map[string]any)
					balances = append(balances, store.Balance{AccountID: aid, CustodianAccountID: py.S(ca["id"]), SecurityID: py.S(b["securityId"]), Quantity: numPtr(b["quantity"])})
				}
			}
		}
	}
	return balances, nil
}

func (c *Client) FetchMargin(sess Session, accountIDs []string) []store.Margin {
	rows := []store.Margin{}
	now := time.Now().UTC().Format("2006-01-02T15:04:05Z")
	failed := 0
	firstError := ""
	total := 0
	for _, aid := range accountIDs {
		if aid == "" {
			continue
		}
		total++
		data, err := c.GraphQL(sess, "FetchAccountCurrentMarginBuyingPowerV2", map[string]any{"accountId": aid, "currency": "CAD"}, "")
		if err != nil {
			failed++
			if firstError == "" {
				firstError = errorName(err)
			}
			continue
		}
		parsed := ParseMargin(data)
		if parsed == nil {
			continue
		}
		parsed.AccountID = aid
		parsed.FetchedAt = now
		rows = append(rows, *parsed)
	}
	if failed > 0 {
		fmt.Fprintf(os.Stderr, "bagholder portfolio: buying power request failed for %d of %d accounts (%s)\n", failed, total, firstError)
	}
	return rows
}

func errorName(err error) string {
	if err == nil {
		return ""
	}
	if err == ErrNotAuthorized {
		return "PermissionError: not authorized"
	}
	return "RuntimeError: " + err.Error()
}

func (c *Client) paginateNav(sess Session, operation string, extra map[string]any, query, sinceDate string) ([]store.NavPoint, error) {
	today := time.Now().UTC().Format("2006-01-02")
	since := sinceDate
	if len(since) > 10 {
		since = since[:10]
	}
	if since != "" && since > today {
		return []store.NavPoint{}, nil
	}
	year0 := 2020
	if since != "" {
		if y, err := strconv.Atoi(since[:4]); err == nil {
			year0 = y
		}
	}
	year1, _ := strconv.Atoi(today[:4])
	var points []store.NavPoint
	for year := year0; year <= year1; year++ {
		start := fmt.Sprintf("%d-01-01", year)
		if since != "" && start < since {
			start = since
		}
		end := fmt.Sprintf("%d-12-31", year)
		if year == year1 {
			end = today
		}
		if start > end {
			continue
		}
		var cursor any
		for i := 0; i < 8; i++ {
			variables := map[string]any{}
			for k, v := range extra {
				variables[k] = v
			}
			variables["startDate"] = start
			variables["endDate"] = end
			variables["cursor"] = cursor
			data, err := c.GraphQL(sess, operation, variables, query)
			if err != nil {
				return nil, err
			}
			chunk, page := NavPointsFromPayload(data)
			points = append(points, chunk...)
			if next, _ := page["hasNextPage"].(bool); !next {
				break
			}
			cursor = page["endCursor"]
			if py.S(cursor) == "" {
				break
			}
		}
	}
	byDate := map[string]store.NavPoint{}
	for _, rec := range points {
		byDate[rec.Date] = rec
	}
	out := make([]store.NavPoint, 0, len(byDate))
	for _, d := range store.SortedKeys(byDate) {
		out = append(out, byDate[d])
	}
	return out, nil
}

func (c *Client) FetchNavHistory(sess Session, identityID, sinceDate string) ([]store.NavPoint, error) {
	return c.paginateNav(sess, "IdentityHistoricalFinancialsQuery", map[string]any{"identityId": identityID, "currency": "CAD", "limit": 400, "includeNetDeposits": true}, QIdentityHistoricalFinancials, sinceDate)
}

func (c *Client) FetchAccountNavHistory(sess Session, accountID, sinceDate string) ([]store.NavPoint, error) {
	aid := py.Strip(accountID)
	if aid == "" {
		return []store.NavPoint{}, nil
	}
	return c.paginateNav(sess, "FetchAccountHistoricalFinancials", map[string]any{"id": aid, "currency": "CAD", "resolution": "DAILY", "first": 400}, QFetchAccountHistoricalFinancials, sinceDate)
}

func (c *Client) FetchSecurity(sess Session, securityID string) *store.Security {
	sid := py.Strip(securityID)
	if sid == "" {
		return nil
	}
	data, err := c.GraphQL(sess, "FetchSecurity", map[string]any{"securityId": sid}, "")
	if err != nil {
		return nil
	}
	sec, _ := data["security"].(map[string]any)
	return SecurityRecord(sec, sid)
}

func (c *Client) FetchSecurities(sess Session, securityIDs []string) []store.Security {
	var ids []string
	seen := map[string]bool{}
	for _, raw := range securityIDs {
		sid := py.Strip(raw)
		if sid != "" && !seen[sid] {
			seen[sid] = true
			ids = append(ids, sid)
		}
	}
	out := []store.Security{}
	for i := 0; i < len(ids); i += SecurityBatch {
		end := i + SecurityBatch
		if end > len(ids) {
			end = len(ids)
		}
		chunk := ids[i:end]
		data, err := c.GraphQL(sess, "FetchSecurities", map[string]any{"ids": chunk}, "")
		rows, ok := data["securities"].([]any)
		if err != nil || !ok {
			for _, sid := range chunk {
				if rec := c.FetchSecurity(sess, sid); rec != nil {
					out = append(out, *rec)
				}
			}
			continue
		}
		for _, raw := range rows {
			sec, _ := raw.(map[string]any)
			if rec := SecurityRecord(sec, ""); rec != nil {
				out = append(out, *rec)
			}
		}
	}
	return out
}

func (c *Client) FetchQuotes(sess Session, securityIDs []string) (map[string]*Quote, error) {
	var ids []string
	for _, x := range securityIDs {
		if s := py.Strip(x); s != "" {
			ids = append(ids, s)
		}
	}
	out := map[string]*Quote{}
	if len(ids) == 0 {
		return out, nil
	}
	data, err := c.GraphQL(sess, "FetchSecuritiesSummary", map[string]any{"ids": ids}, "")
	if err != nil {
		return nil, err
	}
	nodes, _ := data["securities"].([]any)
	for _, raw := range nodes {
		node, _ := raw.(map[string]any)
		if q := ParseQuote(node); q != nil {
			out[q.SecurityID] = q
		}
	}
	return out, nil
}

func (c *Client) LookupListing(sess Session, symbol, exchange string) *store.Security {
	data, err := c.GraphQL(sess, "FetchSecuritySearchResult", map[string]any{"query": py.Strip(symbol)}, "")
	if err != nil {
		fmt.Fprintf(os.Stderr, "bagholder ticket: listing search for %s failed: %s\n", symbol, err)
		return nil
	}
	return ParseListingSearch(data, symbol, exchange)
}

const OrderBranch = "TR"

func (c *Client) FetchExtendedOrder(sess Session, externalID string) (*OrderUpdate, error) {
	data, err := c.GraphQL(sess, "FetchSoOrdersExtendedOrder", map[string]any{"branchId": OrderBranch, "externalId": externalID}, "")
	if err != nil {
		return nil, err
	}
	return ParseExtendedOrder(data), nil
}

func (c *Client) FetchOrderFeed(sess Session, identity string, statuses []string) ([]Item, error) {
	var out []Item
	var cursor any
	for {
		data, err := c.GraphQL(sess, "OrderServiceExtendedOrderFeed", map[string]any{"identityId": identity, "statuses": statuses, "first": 25, "cursor": cursor}, "")
		if err != nil {
			return nil, err
		}
		ident, _ := data["identity"].(map[string]any)
		feed, _ := ident["orderServiceExtendedOrderFeed"].(map[string]any)
		edges, _ := feed["edges"].([]any)
		for _, raw := range edges {
			edge, _ := raw.(map[string]any)
			if node, ok := edge["node"].(map[string]any); ok && py.S(node["id"]) != "" {
				out = append(out, node)
			}
		}
		page, _ := feed["pageInfo"].(map[string]any)
		cursor = page["endCursor"]
		if next, _ := page["hasNextPage"].(bool); !next || py.S(cursor) == "" {
			break
		}
	}
	return out, nil
}
