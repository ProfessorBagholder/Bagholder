package news

import (
	"database/sql"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"reflect"
	"sort"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

var testMu sync.Mutex

type stubTransport func(*http.Request) (*http.Response, error)

func (f stubTransport) RoundTrip(r *http.Request) (*http.Response, error) { return f(r) }

func reply(status int, body string) *http.Response {
	return &http.Response{StatusCode: status, Status: fmt.Sprintf("%d %s", status, http.StatusText(status)), Header: http.Header{}, Body: io.NopCloser(strings.NewReader(body))}
}

func jsonMap(t *testing.T, text string) map[string]any {
	t.Helper()
	var out map[string]any
	if err := json.Unmarshal([]byte(text), &out); err != nil {
		t.Fatal(err)
	}
	return out
}

func tempStore(t *testing.T) *store.Store {
	t.Helper()
	st := store.MustOpen(t.TempDir())
	if err := st.Ensure(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	return st
}

func newClient(t *testing.T, now time.Time, handle func(*http.Request) (*http.Response, error)) *market.Client {
	t.Helper()
	c := market.NewClient(tempStore(t))
	c.HTTP.Transport = stubTransport(handle)
	c.Now = func() time.Time { return now }
	saved := pace
	pace = func(string, float64) {}
	t.Cleanup(func() { pace = saved })
	return c
}

type graphQL struct {
	OperationName string         `json:"operationName"`
	Variables     map[string]any `json:"variables"`
}

func readGraphQL(r *http.Request) graphQL {
	var q graphQL
	raw, _ := io.ReadAll(r.Body)
	_ = json.Unmarshal(raw, &q)
	return q
}

func (q graphQL) symbol() string {
	s, _ := q.Variables["symbol"].(string)
	return s
}

func tmxNews(items string) string { return `{"data": {"news": [` + items + `]}}` }

func tmxQuote(symbol, venue string) string {
	return fmt.Sprintf(`{"data": {"getQuoteBySymbol": {"symbol": %q, "exchangeName": %q}}}`, symbol, venue)
}

func TestTMXItemsCarryAnExactTimeAndAPageLink(t *testing.T) {
	data := jsonMap(t, `{"data": {"news": [{"headline": "Shopify Delivers Big: 30%+ Growth Across&#xA0;GMV", "datetime": "2026-08-05T07:00:00-04:00", "source": "GlobeNewswire via QuoteMedia", "newsid": 4883675477075330},
		{"headline": "no id", "datetime": "2026-08-05T07:00:00-04:00"},
		{"headline": "bad time", "datetime": "yesterday", "newsid": 5}]}}`)
	rows := ParseTMXNews(data, "SHOP", false)
	want := []store.WireItem{{ID: "tmx:4883675477075330", Headline: "Shopify Delivers Big: 30%+ Growth Across GMV", Source: "GlobeNewswire", URL: "https://money.tmx.com/en/quote/SHOP/news/4883675477075330", PublishedAt: "2026-08-05T11:00:00Z", Kind: "release", Via: "tmx"}}
	if !reflect.DeepEqual(rows, want) {
		t.Errorf("rows = %+v, want %+v", rows, want)
	}
}

func TestNasdaqItemsTakeTheirTimeFromTheAgeGiven(t *testing.T) {
	now := time.Date(2026, 9, 11, 15, 30, 0, 0, time.UTC)
	data := jsonMap(t, `{"data": {"rows": [
		{"id": 28351741, "title": "Forget AMD. Here&#39;s Who Nvidia Really Needs to Be Worried About.", "publisher": "The Motley Fool", "created": "Sep 11, 2026", "ago": "17 minutes ago", "url": "/articles/forget-amd", "primarysymbol": "avgo", "related_symbols": ["avgo|stocks", "nvda|stocks"]},
		{"id": 2, "title": "Two hours", "publisher": "Zacks", "created": "Sep 11, 2026", "ago": "2 hours ago", "url": "https://www.nasdaq.com/articles/two", "related_symbols": ["NVDA|stocks"]},
		{"id": 3, "title": "Old", "publisher": "Barchart", "created": "Sep 3, 2026", "ago": "", "url": "/articles/old", "primarysymbol": "nvda"},
		{"id": 4, "publisher": "no title", "related_symbols": ["nvda|stocks"]},
		{"id": 5, "title": "Market wrap that never names it", "publisher": "Barchart", "created": "Sep 11, 2026", "ago": "3 minutes ago", "url": "/articles/wrap", "related_symbols": ["spy|etf", "aapl|stocks"]}]}}`)
	rows := ParseNasdaqNews(data, now, "NVDA", "")
	got := [][5]string{}
	for _, r := range rows {
		got = append(got, [5]string{r.ID, r.Headline, r.Source, r.URL, r.PublishedAt})
	}
	want := [][5]string{
		{"nasdaq:28351741", "Forget AMD. Here's Who Nvidia Really Needs to Be Worried About.", "The Motley Fool", "https://www.nasdaq.com/articles/forget-amd", "2026-09-11T15:13:00Z"},
		{"nasdaq:2", "Two hours", "Zacks", "https://www.nasdaq.com/articles/two", "2026-09-11T13:30:00Z"},
		{"nasdaq:3", "Old", "Barchart", "https://www.nasdaq.com/articles/old", "2026-09-03T00:00:00Z"},
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("an item Nasdaq does not tag with the symbol is left out: got %v, want %v", got, want)
	}
}

func TestTheWireFollowsTheVenue(t *testing.T) {
	cases := []struct{ symbol, exchange, currency, want string }{
		{"SHOP", "TSX", "CAD", "tmx"},
		{"NVDA", "NASDAQ", "USD", "nasdaq"},
		{"AAPL", "", "USD", "nasdaq"},
		{"QBTC", "NEO", "CAD", "tmx"},
	}
	for _, c := range cases {
		if got := SourceFor(c.symbol, c.exchange, c.currency); got != c.want {
			t.Errorf("SourceFor(%q, %q, %q) = %q, want %q", c.symbol, c.exchange, c.currency, got, c.want)
		}
	}
}

func TestAWiresItemIsAReleaseAndAPublishersAStory(t *testing.T) {
	for _, wire := range []string{"GlobeNewswire", "Business Wire", "PR Newswire", "ACCESS Newswire", "Accesswire", "TheNewsWire", "Canada Newswire", "TMX Newsfile", "Marketwired", "CNW Group", "NewMediaWire"} {
		if got := KindOf(wire); got != "release" {
			t.Errorf("%s: KindOf = %q, want release", wire, got)
		}
	}
	for _, pub := range []string{"The Motley Fool", "Zacks", "Barchart", "RTTNews", "MarketBeat", "BNK Invest", "Fintel", "", "WIRED", "MT Newswires", "Dow Jones Newswires"} {
		if got := KindOf(pub); got != "story" {
			t.Errorf("%q: KindOf = %q, want story", pub, got)
		}
	}
	tmx := ParseTMXNews(jsonMap(t, `{"data": {"news": [{"newsid": "1", "headline": "Closing", "source": "GlobeNewswire via QuoteMedia", "datetime": "2026-09-14T08:00:00-04:00"}]}}`), "CH", false)
	if len(tmx) != 1 || tmx[0].Kind != "release" || tmx[0].Source != "GlobeNewswire" {
		t.Errorf("tmx = %+v, want one release from GlobeNewswire", tmx)
	}
	now := time.Date(2026, 9, 15, 12, 0, 0, 0, time.UTC)
	press := ParseNasdaqNews(jsonMap(t, `{"data": {"rows": [{"id": 9, "title": "Shopify Delivers Big", "publisher": "", "created": "Aug 5, 2026", "ago": "Aug 5, 2026", "url": "/press-release/x", "related_symbols": ["shop|stocks"]}]}}`), now, "SHOP", "release")
	if len(press) != 1 || press[0].Kind != "release" || press[0].Source != "Nasdaq" || press[0].PublishedAt != "2026-08-05T00:00:00Z" {
		t.Errorf("a release Nasdaq names no wire for reads as Nasdaq's: %+v", press)
	}
	story := ParseNasdaqNews(jsonMap(t, `{"data": {"rows": [{"id": 8, "title": "Why SHOP", "publisher": "The Motley Fool", "created": "Sep 14, 2026", "ago": "1 day ago", "url": "/articles/y", "related_symbols": ["shop|stocks"]}]}}`), now, "SHOP", "")
	if len(story) != 1 || story[0].Kind != "story" {
		t.Errorf("story = %+v, want one story", story)
	}
}

func TestTMXIsAskedUnderTheCodeTheQuoteUsesAndResolvesAWrongVenue(t *testing.T) {
	item := `{"newsid": "7", "headline": "QIMC Engages Echo Seismic", "source": "TMX Newsfile via QuoteMedia", "datetime": "2026-09-14T09:13:00-04:00"}`
	asked := []string{}
	now := time.Date(2026, 9, 15, 12, 0, 0, 0, time.UTC)
	c := newClient(t, now, func(r *http.Request) (*http.Response, error) {
		q := readGraphQL(r)
		form := q.symbol()
		switch q.OperationName {
		case "getNewsForSymbol":
			testMu.Lock()
			asked = append(asked, form)
			testMu.Unlock()
			if form == "QIMC:CNX" || form == "CH" {
				return reply(200, tmxNews(item)), nil
			}
			return reply(200, tmxNews("")), nil
		case "getQuoteBySymbol":
			if form == "QIMC:CNX" {
				return reply(200, tmxQuote(form, "Canadian Securities Exchange")), nil
			}
			return reply(200, `{"data": {"getQuoteBySymbol": null}}`), nil
		}
		return reply(404, ""), nil
	})
	src, rows, ok := FetchSymbol(c, "QIMC", "CSE", "CAD", now)
	FetchSymbol(c, "CH", "TSX-V", "CAD", now)
	if !ok || src != "tmx" || !reflect.DeepEqual(asked, []string{"QIMC:CNX", "QIMC:CNX", "CH", "CH"}) {
		t.Errorf("each listing under the code its quote uses, once for each of TMX's two tabs: src %q, asked %v", src, asked)
	}
	if len(rows) != 1 || rows[0].Kind != "release" || rows[0].URL != "https://money.tmx.com/en/quote/QIMC:CNX/news/7" {
		t.Errorf("rows = %+v", rows)
	}
	asked = asked[:0]
	_, found, _ := FetchSymbol(c, "QIMC", "TSX-V", "CAD", now)
	ids := []string{}
	for _, r := range found {
		ids = append(ids, r.ID)
	}
	if !reflect.DeepEqual(asked, []string{"QIMC", "QIMC", "QIMC:CNX", "QIMC:CNX"}) || !reflect.DeepEqual(ids, []string{"tmx:7"}) {
		t.Errorf("asked %v ids %v, want [QIMC QIMC QIMC:CNX QIMC:CNX] [tmx:7]", asked, ids)
	}
}

func TestATickerWithNoVenueIsNeverAskedOfTMX(t *testing.T) {
	seen := map[string]bool{}
	now := time.Date(2026, 9, 15, 12, 0, 0, 0, time.UTC)
	c := newClient(t, now, func(r *http.Request) (*http.Response, error) {
		if r.Method == http.MethodPost {
			seen["tmx"] = true
			return reply(200, tmxNews(`{"newsid": "9", "headline": "IIROC Trading Halt - F", "source": "TMX Newsfile", "datetime": "2026-09-14T09:13:00-04:00"}`)), nil
		}
		seen["nasdaq"] = true
		return reply(200, `{"data": {"rows": [{"id": 5, "title": "Ford declares dividend", "publisher": "PR Newswire", "created": "Sep 14, 2026", "ago": "1 day ago", "url": "/a", "related_symbols": ["f|stocks"]}]}}`), nil
	})
	src, rows, _ := FetchSymbol(c, "F", "", "", now)
	if src != "nasdaq" || seen["tmx"] {
		t.Errorf("no venue: TMX is never asked: src %q, tmx asked %v", src, seen["tmx"])
	}
	got := [][2]string{}
	for _, r := range rows {
		got = append(got, [2]string{r.Kind, r.Headline})
	}
	if want := [][2]string{{"release", "Ford declares dividend"}}; !reflect.DeepEqual(got, want) {
		t.Errorf("rows = %v, want %v", got, want)
	}
}

func TestAUSListingReadsItsReleasesBesideItsNewsEachOnce(t *testing.T) {
	now := time.Date(2026, 9, 15, 12, 0, 0, 0, time.UTC)
	feeds := map[string]string{
		"articlebysymbol": `{"data": {"rows": [{"id": 1, "title": "Why SHOP", "publisher": "Zacks", "created": "Sep 14, 2026", "ago": "1 day ago", "url": "/articles/a", "related_symbols": ["shop|stocks"]}, {"id": 2, "title": "Shopify Delivers Big", "publisher": "GlobeNewswire", "created": "Aug 5, 2026", "ago": "Aug 5, 2026", "url": "/articles/b", "related_symbols": ["shop|stocks"]}]}}`,
		"press_release":   `{"data": {"rows": [{"id": 2, "title": "Shopify Delivers Big", "publisher": "", "created": "Aug 5, 2026", "ago": "Aug 5, 2026", "url": "/press-release/b", "related_symbols": ["shop|stocks"]}, {"id": 3, "title": "Shopify to Announce", "publisher": "", "created": "Jul 8, 2026", "ago": "Jul 8, 2026", "url": "/press-release/c", "related_symbols": ["shop|stocks"]}]}}`,
	}
	asked := []string{}
	c := newClient(t, now, func(r *http.Request) (*http.Response, error) {
		u := r.URL.String()
		testMu.Lock()
		asked = append(asked, u)
		testMu.Unlock()
		if strings.Contains(u, "press_release") {
			return reply(200, feeds["press_release"]), nil
		}
		return reply(200, feeds["articlebysymbol"]), nil
	})
	src, rows, _ := FetchSymbol(c, "SHOP", "NASDAQ", "USD", now)
	if src != "nasdaq" {
		t.Errorf("src = %q, want nasdaq", src)
	}
	got := [][3]string{}
	for _, r := range rows {
		got = append(got, [3]string{r.ID, r.Kind, r.Source})
	}
	want := [][3]string{{"nasdaq:1", "story", "Zacks"}, {"nasdaq:2", "release", "GlobeNewswire"}, {"nasdaq:3", "release", "Nasdaq"}}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("the news feed's own wire item is a release; the press feed adds what the news feed lacks, each once: got %v, want %v", got, want)
	}
	if len(asked) != 2 {
		t.Errorf("asked %d feeds, want 2: %v", len(asked), asked)
	}
}

func TestRefreshReadsOnlyStaleListingsAndReplacesTheirRows(t *testing.T) {
	now := time.Date(2026, 9, 11, 15, 30, 0, 0, time.UTC)
	shop := `{"newsid": "1", "headline": "One", "source": "GlobeNewswire", "datetime": "2026-09-11T14:00:00+00:00"}`
	calls := []string{}
	c := newClient(t, now, func(r *http.Request) (*http.Response, error) {
		if r.Method == http.MethodPost {
			q := readGraphQL(r)
			if q.OperationName != "getNewsForSymbol" {
				return reply(200, `{"data": {"getQuoteBySymbol": null}}`), nil
			}
			sym := q.symbol()
			if media, _ := q.Variables["companyInNews"].(bool); !media {
				testMu.Lock()
				calls = append(calls, sym)
				testMu.Unlock()
			}
			if sym == "SHOP" {
				return reply(200, tmxNews(shop)), nil
			}
			return reply(500, "boom"), nil
		}
		if strings.Contains(r.URL.String(), "press_release") {
			return reply(200, `{"data": {"rows": []}}`), nil
		}
		sym, _, _ := strings.Cut(r.URL.Query().Get("q"), "|")
		testMu.Lock()
		calls = append(calls, sym)
		testMu.Unlock()
		if sym == "NVDA" {
			return reply(200, `{"data": {"rows": [{"id": "9", "title": "Nine", "publisher": "Zacks", "created": "Sep 11, 2026", "ago": "30 minutes ago", "url": "u9", "related_symbols": ["nvda|stocks"]}]}}`), nil
		}
		return reply(500, "boom"), nil
	})
	listings := []Listing{{Symbol: "SHOP", Exchange: "TSX", Currency: "CAD"}, {Symbol: "NVDA", Exchange: "NASDAQ", Currency: "USD"}, {Symbol: "BROKEN", Exchange: "TSX", Currency: "CAD"}}
	saved := readExtra
	readExtra = func(c *market.Client, key, symbol, exchange, currency, name string) ([]store.WireItem, bool, error) {
		if symbol == "BROKEN" {
			return nil, true, fmt.Errorf("down")
		}
		return []store.WireItem{}, true, nil
	}
	defer func() { readExtra = saved }()
	if n := Refresh(c, listings, now, nil, nil, nil); n != 2 {
		t.Errorf("a listing no source answers for leaves nothing behind and is asked again next time: refresh = %d, want 2", n)
	}
	sort.Strings(calls)
	if want := []string{"BROKEN", "NVDA", "SHOP"}; !reflect.DeepEqual(calls, want) {
		t.Errorf("listings are read side by side: calls = %v, want %v", calls, want)
	}
	calls = calls[:0]
	if n := Refresh(c, listings, now, nil, nil, nil); n != 0 {
		t.Errorf("refresh = %d, want 0", n)
	}
	if want := []string{"BROKEN"}; !reflect.DeepEqual(calls, want) {
		t.Errorf("fresh listings are not asked again within fifteen minutes: calls = %v, want %v", calls, want)
	}
	shop = `{"newsid": "2", "headline": "Two", "source": "CNW", "datetime": "2026-09-11T16:00:00+00:00"}`
	later := time.Date(2026, 9, 11, 16, 0, 0, 0, time.UTC)
	Refresh(c, listings, later, nil, nil, nil)
	got := [][3]string{}
	for _, r := range c.Store.Snapshot(false).News {
		got = append(got, [3]string{r.ID, r.Symbol, r.Wire})
	}
	if want := [][3]string{{"tmx:2", "SHOP", "CNW"}, {"nasdaq:9", "NVDA", "Zacks"}}; !reflect.DeepEqual(got, want) {
		t.Errorf("newest first; a listing's rows are replaced by its wire's latest: got %v, want %v", got, want)
	}
	c.Store.ForgetNews("SHOP", "TSX")
	ids := []string{}
	for _, r := range c.Store.Snapshot(false).News {
		ids = append(ids, r.ID)
	}
	if want := []string{"nasdaq:9"}; !reflect.DeepEqual(ids, want) {
		t.Errorf("ids = %v, want %v", ids, want)
	}
	stale := Stale(c, []Listing{{Symbol: "SHOP", Exchange: "TSX", Currency: "CAD"}}, later, FreshMinutes)
	if want := []Listing{{Symbol: "SHOP", Exchange: "TSX", Currency: "CAD"}}; !reflect.DeepEqual(stale, want) {
		t.Errorf("forgotten means stale: %v, want %v", stale, want)
	}
}

func TestRowsKeepTheirKindAndAnOldTableIsToldByItsWires(t *testing.T) {
	st := tempStore(t)
	st.ReplaceNews("SHOP", "NASDAQ", "nasdaq", []store.WireItem{
		{ID: "nasdaq:1", Headline: "a", Source: "Zacks", URL: "u", PublishedAt: "2026-09-14T00:00:00Z", Kind: "story"},
		{ID: "nasdaq:2", Headline: "b", Source: "GlobeNewswire", URL: "u", PublishedAt: "2026-08-05T00:00:00Z", Kind: "release"},
	}, "")
	if _, err := st.DB().Exec("UPDATE news SET kind = NULL"); err != nil {
		t.Fatal(err)
	}
	st.DeleteMeta("schema_version")
	if err := st.Ensure(); err != nil {
		t.Fatal(err)
	}
	rows, err := st.DB().Query("SELECT id, kind FROM news")
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	told := map[string]string{}
	for rows.Next() {
		var id string
		var kind sql.NullString
		if err := rows.Scan(&id, &kind); err != nil {
			t.Fatal(err)
		}
		told[id] = kind.String
	}
	if want := map[string]string{"nasdaq:1": "story", "nasdaq:2": "release"}; !reflect.DeepEqual(told, want) {
		t.Errorf("a table from before releases were told apart is told by the wire names it holds: %v, want %v", told, want)
	}
}

func TestTrimKeepsTheNewest(t *testing.T) {
	st := tempStore(t)
	items := []store.WireItem{}
	for i := 1; i <= 5; i++ {
		items = append(items, store.WireItem{ID: "tmx:" + strconv.Itoa(i), Headline: strconv.Itoa(i), PublishedAt: fmt.Sprintf("2026-09-%02dT00:00:00Z", i)})
	}
	st.ReplaceNews("A", "TSX", "tmx", items, "")
	st.TrimNews(2)
	ids := []string{}
	for _, r := range st.Snapshot(false).News {
		ids = append(ids, r.ID)
	}
	if want := []string{"tmx:5", "tmx:4"}; !reflect.DeepEqual(ids, want) {
		t.Errorf("ids = %v, want %v", ids, want)
	}
}
