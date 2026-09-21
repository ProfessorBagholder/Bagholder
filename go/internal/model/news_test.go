package model

import (
	"encoding/json"
	"reflect"
	"sort"
	"testing"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/news"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

type newsTagView struct {
	Symbol     string
	Held       bool
	Watched    bool
	Pct        *float64
	PositionID *string
}

type taggedNews struct {
	ID   string
	Tags []newsTagView
}

func tagViews(r *NewsRow) []newsTagView {
	out := []newsTagView{}
	for _, tg := range r.Tags {
		out = append(out, newsTagView{tg.Symbol, tg.Held, tg.Watched, tg.PercentChange, tg.PositionID})
	}
	return out
}

func tagSymbols(r *NewsRow) []string {
	out := []string{}
	for _, tg := range r.Tags {
		out = append(out, tg.Symbol)
	}
	return out
}

func strPtr(s string) *string { return &s }

func newsJSON(t *testing.T, text string) map[string]any {
	t.Helper()
	var out map[string]any
	if err := json.Unmarshal([]byte(text), &out); err != nil {
		t.Fatal(err)
	}
	return out
}

func TestRowsAreTaggedWithWhatTheBookHoldsOrWatches(t *testing.T) {
	base := &Base{News: []store.NewsItem{
		{ID: "tmx:1", Symbol: "SHOP", Exchange: "TSX", Wire: "GlobeNewswire", Headline: "One", URL: "u1", PublishedAt: "2026-09-11T14:00:00Z"},
		{ID: "tmx:1", Symbol: "HHIS", Exchange: "TSX", Wire: "GlobeNewswire", Headline: "One", URL: "u1", PublishedAt: "2026-09-11T14:00:00Z"},
		{ID: "nasdaq:9", Symbol: "NVDA", Exchange: "NASDAQ", Wire: "Zacks", Headline: "Nine", URL: "u9", PublishedAt: "2026-09-11T15:00:00Z"},
	}}
	positions := []*Position{{PositionCore: PositionCore{ID: "p1", Symbol: "HHIS", Exchange: "TSX", PercentChange: py.Ptr(0.6)}}}
	watch := []WatchRow{{Symbol: "SHOP", Exchange: "TSX", PercentChange: py.Ptr(3.28)}, {Symbol: "NVDA", Exchange: "NASDAQ"}}
	got := []taggedNews{}
	for _, r := range newsRows(base, positions, watch) {
		got = append(got, taggedNews{r.ID, tagViews(r)})
	}
	want := []taggedNews{
		{"nasdaq:9", []newsTagView{{"NVDA", false, true, nil, nil}}},
		{"tmx:1", []newsTagView{{"SHOP", false, true, py.Ptr(3.28), nil}, {"HHIS", true, false, py.Ptr(0.6), strPtr("p1")}}},
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("newest first; an item two listings share is one row with both tags: got %+v, want %+v", got, want)
	}
}

func TestTheMarketFeedIsAListingOfItsOwnWithNoTag(t *testing.T) {
	if got := news.SourceFor(news.Market[0], news.Market[1], news.Market[2]); got != "nasdaq" {
		t.Errorf("SourceFor(market) = %q, want nasdaq", got)
	}
	data := newsJSON(t, `{"data": {"rows": [{"id": 1, "title": "Stocks Settle Lower", "publisher": "Barchart", "url": "/articles/a", "ago": "7 minutes ago", "related_symbols": ["ryam|stocks"]},
		{"id": 2, "title": "Value ETFs", "publisher": "Zacks", "url": "/articles/b", "ago": "2 hours ago", "related_symbols": ["mu|stocks"]}]}}`)
	now := time.Date(2026, 9, 11, 16, 0, 0, 0, time.UTC)
	ids := []string{}
	for _, r := range news.ParseNasdaqNews(data, now, "", "") {
		ids = append(ids, r.ID)
	}
	if want := []string{"nasdaq:1", "nasdaq:2"}; !reflect.DeepEqual(ids, want) {
		t.Errorf("asked without a symbol, the feed keeps every item: %v, want %v", ids, want)
	}
	base := &Base{News: []store.NewsItem{
		{ID: "nasdaq:1", Symbol: "*", Exchange: "MARKET", Wire: "Barchart", Headline: "Stocks Settle Lower", URL: "u1", PublishedAt: "2026-09-11T15:53:00Z"},
		{ID: "nasdaq:2", Symbol: "*", Exchange: "MARKET", Wire: "Zacks", Headline: "Value ETFs", URL: "u2", PublishedAt: "2026-09-11T14:00:00Z"},
		{ID: "nasdaq:2", Symbol: "MU", Exchange: "NASDAQ", Wire: "Zacks", Headline: "Value ETFs", URL: "u2", PublishedAt: "2026-09-11T14:00:00Z"},
	}}
	type view struct {
		ID     string
		Market bool
		Tags   []string
	}
	got := []view{}
	for _, r := range newsRows(base, nil, []WatchRow{{Symbol: "MU", Exchange: "NASDAQ", PercentChange: py.Ptr(-0.18)}}) {
		got = append(got, view{r.ID, r.Market, tagSymbols(r)})
	}
	want := []view{{"nasdaq:1", true, []string{}}, {"nasdaq:2", true, []string{"MU"}}}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("a market item carries no tag; the same story read for a watched listing is one row, tagged, and still the market's: got %+v, want %+v", got, want)
	}
}

func TestTheSameHeadlineUnderOtherIDsIsOneRow(t *testing.T) {
	base := &Base{News: []store.NewsItem{
		{ID: "tmx:1", Symbol: "ENB", Exchange: "TSX", Wire: "PR Newswire", Headline: "Enbridge Announces Retirement of Greg Ebel", URL: "u1", PublishedAt: "2026-09-08T12:00:00Z"},
		{ID: "tmx:2", Symbol: "ENB", Exchange: "TSX", Wire: "Canada Newswire", Headline: "Enbridge Announces Retirement of Greg Ebel", URL: "u2", PublishedAt: "2026-09-08T12:01:00Z"},
		{ID: "nasdaq:7", Symbol: "AAPL", Exchange: "NASDAQ", Wire: "Barchart", Headline: "Stocks Shake Off CPI Report", URL: "u7", PublishedAt: "2026-09-11T18:07:00Z"},
		{ID: "nasdaq:8", Symbol: "MSFT", Exchange: "NASDAQ", Wire: "Barchart", Headline: "Stocks Shake Off CPI Report", URL: "u8", PublishedAt: "2026-09-11T18:07:00Z"},
		{ID: "nasdaq:9", Symbol: "*", Exchange: "MARKET", Wire: "Barchart", Headline: "Stocks shake off CPI report.", URL: "u9", PublishedAt: "2026-09-11T18:07:00Z"},
	}}
	positions := []*Position{{PositionCore: PositionCore{ID: "p1", Symbol: "AAPL", Exchange: "NASDAQ"}}, {PositionCore: PositionCore{ID: "p2", Symbol: "MSFT", Exchange: "NASDAQ"}}}
	type view struct {
		ID          string
		Market      bool
		Tags        []string
		PublishedAt string
	}
	got := []view{}
	for _, r := range newsRows(base, positions, []WatchRow{{Symbol: "ENB", Exchange: "TSX"}}) {
		syms := tagSymbols(r)
		sort.Strings(syms)
		got = append(got, view{r.ID, r.Market, syms, r.PublishedAt})
	}
	want := []view{{"nasdaq:7", true, []string{"AAPL", "MSFT"}, "2026-09-11T18:07:00Z"}, {"tmx:2", false, []string{"ENB"}, "2026-09-08T12:01:00Z"}}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("a release on two wires is one row (the newest kept); a story per symbol feed and in the market feed is one row, tagged, the market's: got %+v, want %+v", got, want)
	}
}

func TestAFrenchReleaseBesideItsEnglishOriginalIsOneStory(t *testing.T) {
	base := &Base{News: []store.NewsItem{
		{ID: "tmx:1", Symbol: "CH", Exchange: "TSX-V", Wire: "TheNewsWire", Headline: "CHARBONE annonce la clôture du tirage de 1,5 M$ auprès de RiverFort pour accélérer sa croissance", URL: "u1", PublishedAt: "2026-09-08T12:05:00Z"},
		{ID: "tmx:2", Symbol: "CH", Exchange: "TSX-V", Wire: "TheNewsWire", Headline: "CHARBONE Announces Closing of $1.5M Drawdown with RiverFort to Accelerate Growth", URL: "u2", PublishedAt: "2026-09-08T12:00:00Z"},
		{ID: "tmx:3", Symbol: "CH", Exchange: "TSX-V", Wire: "TheNewsWire", Headline: "Charbone annonce un tirage de 1,5 M$ du prêt convertible de 10 M$, accélérant sa croissance", URL: "u3", PublishedAt: "2026-09-02T12:00:00Z"},
		{ID: "tmx:4", Symbol: "ENB", Exchange: "TSX", Wire: "Canada Newswire", Headline: "Enbridge annonce ses résultats du deuxième trimestre", URL: "u4", PublishedAt: "2026-08-01T12:00:00Z"},
	}}
	ids := []string{}
	for _, r := range newsRows(base, []*Position{{PositionCore: PositionCore{ID: "p1", Symbol: "CH", Exchange: "TSX-V"}}}, nil) {
		ids = append(ids, r.ID)
	}
	if want := []string{"tmx:2", "tmx:3", "tmx:4"}; !reflect.DeepEqual(ids, want) {
		t.Errorf("the French twin of an English release goes; a French release with no English twin within three hours stays: %v, want %v", ids, want)
	}
}

func TestTheBooksFormAndTheBareTickerAreOneListing(t *testing.T) {
	base := &Base{News: []store.NewsItem{
		{ID: "tmx:7", Symbol: "QNC.TO", Exchange: "TSX-V", Wire: "TMX Newsfile", Headline: "Seven", URL: "u7", PublishedAt: "2026-09-08T13:00:00Z"},
		{ID: "tmx:7", Symbol: "QNC", Exchange: "TSX-V", Wire: "TMX Newsfile", Headline: "Seven", URL: "u7", PublishedAt: "2026-09-08T13:00:00Z"},
	}}
	positions := []*Position{{PositionCore: PositionCore{ID: "p2", Symbol: "QNC.TO", Exchange: "TSX-V", PercentChange: py.Ptr(-1.67)}}}
	watch := []WatchRow{{Symbol: "QNC", Exchange: "TSX-V", PercentChange: py.Ptr(-1.67)}}
	rows := newsRows(base, positions, watch)
	if len(rows) == 0 {
		t.Fatal("no rows")
	}
	if want := []newsTagView{{"QNC", true, true, py.Ptr(-1.67), strPtr("p2")}}; !reflect.DeepEqual(tagViews(rows[0]), want) {
		t.Errorf("one tag, held and watched, the bare ticker: %+v, want %+v", tagViews(rows[0]), want)
	}
}
