package news

import (
	"encoding/json"
	"fmt"
	"html"
	"os"
	"regexp"
	"strconv"
	"strings"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

const (
	TMXNewsQuery    = "query getNewsForSymbol($symbol: String!, $page: Int!, $limit: Int!, $locale: String!) { news: getNewsForSymbol(symbol: $symbol, page: $page, limit: $limit, locale: $locale) { headline datetime source newsid summary } }"
	TMXNewsURL      = "https://money.tmx.com/en/quote/%s/news/%s"
	NasdaqNewsURL   = "https://api.nasdaq.com/api/news/topic/articlebysymbol?q=%s|STOCKS&offset=0&limit=%d"
	NasdaqLatestURL = "https://api.nasdaq.com/api/news/topic/latestnews?offset=0&limit=%d"
	NasdaqPressURL  = "https://api.nasdaq.com/api/news/topic/press_release?q=symbol:%s|assetclass:stocks&offset=0&limit=%d"
	UA              = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0 Safari/537.36"
	PerSymbol       = 12
	PerMarket       = 50
	FreshMinutes    = 15
	Keep            = 400
)

var NasdaqHeaders = map[string]string{"User-Agent": UA, "Accept": "application/json, text/plain, */*", "Origin": "https://www.nasdaq.com", "Referer": "https://www.nasdaq.com/"}
var TMXHeaders = map[string]string{"User-Agent": UA, "locale": "en", "Origin": "https://money.tmx.com", "Referer": "https://money.tmx.com/"}

var pacer = market.NewPacer()

func CleanText(t string) string {
	return py.Strip(py.CollapseSpace(html.UnescapeString(t)))
}

func stamp(t time.Time) string { return t.UTC().Format("2006-01-02T15:04:05Z") }

func ParseTMXNews(data map[string]any, symbol string) []store.WireItem {
	d, _ := data["data"].(map[string]any)
	items, _ := d["news"].([]any)
	rows := []store.WireItem{}
	for _, raw := range items {
		it, ok := raw.(map[string]any)
		if !ok || py.S(it["newsid"]) == "" {
			continue
		}
		t, naive, ok := py.ParseISO(py.S(it["datetime"]))
		if !ok {
			continue
		}
		if naive {
			t = time.Date(t.Year(), t.Month(), t.Day(), t.Hour(), t.Minute(), t.Second(), t.Nanosecond(), time.Local)
		}
		source := strings.ReplaceAll(CleanText(py.S(it["source"])), " via QuoteMedia", "")
		id := py.S(it["newsid"])
		rows = append(rows, store.WireItem{ID: "tmx:" + id, Headline: CleanText(py.S(it["headline"])), Source: source, URL: fmt.Sprintf(TMXNewsURL, symbol, id), PublishedAt: stamp(t), Kind: KindOf(source)})
	}
	return rows
}

var agoRE = regexp.MustCompile(`(?i)(\d+)\s+(minute|hour|day)s?\s+ago`)

func NasdaqWhen(row map[string]any, now time.Time) string {
	if m := agoRE.FindStringSubmatch(py.S(row["ago"])); m != nil {
		n, _ := strconv.Atoi(m[1])
		var delta time.Duration
		switch strings.ToLower(m[2]) {
		case "minute":
			delta = time.Duration(n) * time.Minute
		case "hour":
			delta = time.Duration(n) * time.Hour
		default:
			delta = time.Duration(n) * 24 * time.Hour
		}
		return now.Add(-delta).UTC().Format("2006-01-02T15:04:00Z")
	}
	t, err := time.Parse("Jan 2, 2006", py.S(row["created"]))
	if err != nil {
		return ""
	}
	return t.Format("2006-01-02T00:00:00Z")
}

func ParseNasdaqNews(data map[string]any, now time.Time, symbol, kind string) []store.WireItem {
	want := strings.ToLower(strings.TrimSpace(symbol))
	d, _ := data["data"].(map[string]any)
	items, _ := d["rows"].([]any)
	rows := []store.WireItem{}
	for _, raw := range items {
		it, ok := raw.(map[string]any)
		if !ok || py.S(it["id"]) == "" || py.S(it["title"]) == "" {
			continue
		}
		if want != "" {
			named := map[string]bool{strings.ToLower(strings.TrimSpace(py.S(it["primarysymbol"]))): true}
			if related, ok := it["related_symbols"].([]any); ok {
				for _, x := range related {
					first, _, _ := strings.Cut(py.S(x), "|")
					named[strings.ToLower(strings.TrimSpace(first))] = true
				}
			}
			if !named[want] {
				continue
			}
		}
		when := NasdaqWhen(it, now)
		if when == "" {
			continue
		}
		u := py.S(it["url"])
		if !strings.HasPrefix(u, "http") {
			u = "https://www.nasdaq.com" + u
		}
		source := CleanText(py.S(it["publisher"]))
		if source == "" && kind == "release" {
			source = "Nasdaq"
		}
		k := kind
		if k == "" {
			k = KindOf(source)
		}
		rows = append(rows, store.WireItem{ID: "nasdaq:" + py.S(it["id"]), Headline: CleanText(py.S(it["title"])), Source: source, URL: u, PublishedAt: when, Kind: k})
	}
	return rows
}

func SourceFor(symbol, exchange, currency string) string {
	if symbol == Market[0] && strings.ToUpper(exchange) == Market[1] {
		return "nasdaq"
	}
	form := market.TMXForm(exchange, currency)
	if form == nil {
		return ""
	}
	if *form == ":US" {
		return "nasdaq"
	}
	return "tmx"
}

func getJSON(c *market.Client, url string, headers map[string]string) (map[string]any, error) {
	text, err := c.GetText(url, headers)
	if err != nil {
		return nil, err
	}
	var data map[string]any
	if err := json.Unmarshal([]byte(text), &data); err != nil {
		return nil, err
	}
	return data, nil
}

func FetchSymbol(c *market.Client, symbol, exchange, currency string, now time.Time) (string, []store.WireItem, bool) {
	src := SourceFor(symbol, exchange, currency)
	sym := market.TMXSymbol(symbol)
	if sym == "" {
		return src, []store.WireItem{}, true
	}
	if src == "" {
		src = "nasdaq"
	}
	if symbol == Market[0] {
		pacer.Pace("api.nasdaq.com", 0.6)
		data, err := getJSON(c, fmt.Sprintf(NasdaqLatestURL, PerMarket), NasdaqHeaders)
		if err != nil {
			fmt.Fprintf(os.Stderr, "bagholder news: %s from %s failed: %s\n", sym, src, err)
			return src, nil, false
		}
		return src, ParseNasdaqNews(data, now, "", ""), true
	}
	if src == "tmx" {
		code := market.TMXQuoteSymbol(symbol, exchange, currency)
		if code == "" {
			return src, []store.WireItem{}, true
		}
		ask := func(form string) ([]store.WireItem, error) {
			pacer.Pace("app-money.tmx.com", 0.6)
			data, err := c.PostJSON(market.TMXURL, map[string]any{"operationName": "getNewsForSymbol", "variables": map[string]any{"symbol": form, "page": 1, "limit": PerSymbol, "locale": "en"}, "query": TMXNewsQuery}, TMXHeaders)
			if err != nil {
				return nil, err
			}
			return ParseTMXNews(data, form), nil
		}
		first := c.TMXRemembered(code)
		rows, err := ask(first)
		if err != nil {
			fmt.Fprintf(os.Stderr, "bagholder news: %s from %s failed: %s\n", sym, src, err)
			return src, nil, false
		}
		if len(rows) == 0 {
			if alt := c.TMXResolve(code); alt != "" && alt != first {
				rows, err = ask(alt)
				if err != nil {
					fmt.Fprintf(os.Stderr, "bagholder news: %s from %s failed: %s\n", sym, src, err)
					return src, nil, false
				}
			}
		}
		return src, rows, true
	}
	pacer.Pace("api.nasdaq.com", 0.6)
	data, err := getJSON(c, fmt.Sprintf(NasdaqNewsURL, sym, PerSymbol), NasdaqHeaders)
	if err != nil {
		fmt.Fprintf(os.Stderr, "bagholder news: %s from %s failed: %s\n", sym, src, err)
		return src, nil, false
	}
	rows := ParseNasdaqNews(data, now, sym, "")
	pacer.Pace("api.nasdaq.com", 0.6)
	press, err := getJSON(c, fmt.Sprintf(NasdaqPressURL, sym, PerSymbol), NasdaqHeaders)
	if err != nil {
		fmt.Fprintf(os.Stderr, "bagholder news: %s releases from nasdaq failed: %s\n", sym, err)
	} else {
		seen := map[string]bool{}
		for _, r := range rows {
			seen[r.ID] = true
		}
		for _, r := range ParseNasdaqNews(press, now, sym, "release") {
			if !seen[r.ID] {
				rows = append(rows, r)
			}
		}
	}
	return src, rows, true
}

type Listing struct {
	Symbol, Exchange, Currency string
}

func Stale(st *store.Store, listings []Listing, now time.Time, minutes float64) []Listing {
	fetched := st.NewsFetchedAt()
	out := []Listing{}
	for _, l := range listings {
		last := fetched[store.NewsKey(l.Symbol, l.Exchange)]
		age, ok := market.AgeOf(last, now)
		if !ok || age > time.Duration(minutes*float64(time.Minute)) {
			out = append(out, l)
		}
	}
	return out
}

type OnNew func(symbol, exchange string, rows []store.WireItem, newIDs map[string]bool)

func Refresh(c *market.Client, listings []Listing, now time.Time, onNew OnNew) int {
	done := 0
	for _, l := range Stale(c.Store, listings, now, FreshMinutes) {
		src, rows, ok := FetchSymbol(c, l.Symbol, l.Exchange, l.Currency, now)
		if !ok {
			continue
		}
		var before map[string]bool
		if onNew != nil {
			before = c.Store.NewsIDs(l.Symbol, l.Exchange)
		}
		c.Store.ReplaceNews(l.Symbol, l.Exchange, src, rows, stamp(now))
		if onNew != nil {
			fresh := map[string]bool{}
			for _, r := range rows {
				if r.ID != "" && !before[r.ID] {
					fresh[r.ID] = true
				}
			}
			func() {
				defer func() {
					if e := recover(); e != nil {
						fmt.Fprintf(os.Stderr, "bagholder news: %s items not told: %v\n", l.Symbol, e)
					}
				}()
				onNew(l.Symbol, l.Exchange, rows, fresh)
			}()
		}
		done++
	}
	if done > 0 {
		c.Store.TrimNews(Keep)
	}
	return done
}
