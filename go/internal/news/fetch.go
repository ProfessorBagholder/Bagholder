package news

import (
	"crypto/sha1"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"html"
	"os"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"
	"unicode"

	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

const (
	TMXNewsQuery    = "query getNewsForSymbol($symbol: String!, $page: Int!, $limit: Int!, $locale: String!, $companyInNews: Boolean) { news: getNewsForSymbol(symbol: $symbol, page: $page, limit: $limit, locale: $locale, companyInNews: $companyInNews) { headline datetime source newsid summary topic } }"
	TMXNewsURL      = "https://money.tmx.com/en/quote/%s/news/%s"
	NasdaqNewsURL   = "https://api.nasdaq.com/api/news/topic/articlebysymbol?q=%s|STOCKS&offset=0&limit=%d"
	NasdaqLatestURL = "https://api.nasdaq.com/api/news/topic/latestnews?offset=0&limit=%d"
	NasdaqPressURL  = "https://api.nasdaq.com/api/news/topic/press_release?q=symbol:%s|assetclass:stocks&offset=0&limit=%d"
	UA              = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0 Safari/537.36"
	PerSymbol       = 12
	PerListing      = 60
	PerMarket       = 50
	FreshMinutes    = 15
	Keep            = 4000
	ListingsAtOnce  = 4
	SameStoryHours  = 26
	YahooGateway    = "https://nexus-gateway-prod.media.yahoo.com/"
	YahooNewsQuery  = "query FinancePolarisTickerNews($listInput:LightyearListInput!,$clientContext:ClientContext!,$mlRecsInput:MLRecsInput!,$gqlContext:[GqlContext]=[],$imageResize:[ImageResizeInput!]!=[],$first:Int,$mlRecsFirst:Int,$after:String,$offset:Int){lightyearList(list_input:$listInput,cc:$clientContext,first:$first){...HydratedLightyearListStoryVideoStreamPolarisWithPagination}}\nfragment ResizedResolutions on ImageResized{url height width transformLabel}\nfragment Image on Image{type:imgType originalUrl:url originalHeight:height originalWidth:width resolutions:resized(resizeInput:$imageResize){...ResizedResolutions}}\nfragment ContentAttributes on ContentAttributes{description summary pubDate:publishTime displayTime isHosted canonicalUrl clickthroughUrl(cc:$clientContext) provider{displayName url providerContentUrl providerId} thumbnail{...Image} mabMeta{mabLogString}}\nfragment FinanceStockTickers on Finance{stockTickers{symbol}}\nfragment StoryData on Story{id:uuid __typename title previewUrl(cc:$clientContext) isPremiumNews isLiveBlog embeddedLiveBlog{status} contentAttributes{...ContentAttributes} finance{...FinanceStockTickers}}\nfragment VideoData on Video{id:uuid __typename title duration previewUrl(cc:$clientContext) liveEventInfo{scheduledStartTime scheduledStopTime status} contentAttributes{...ContentAttributes} finance{...FinanceStockTickers}}\nfragment OutlinkData on Outlink{__typename uuid description displayTime headline url provider{displayName url providerContentUrl providerId} contentAttributes{thumbnail{...Image}}}\nfragment HydratedAssetRefStoryOrVideo on AssetRef{__typename asset(gqlContext:$gqlContext){__typename ... on Story{...StoryData} ... on Video{...VideoData} ... on Outlink{...OutlinkData}}}\nfragment HydratedLightyearListStoryVideoStreamPolarisWithPagination on LightyearList{main:mlRecsStream(mlRecsInput:$mlRecsInput,first:$mlRecsFirst,after:$after,offset:$offset){edges{node{...HydratedAssetRefStoryOrVideo}} pagination:pageInfo{nextPage:hasNextPage endCursor} totalCount}}"
	SANewsURL       = "https://seekingalpha.com/api/sa/combined/%s.xml"
	GNewsURL        = "https://news.google.com/rss/search?q=%s&hl=en-CA&gl=CA&ceid=CA:en"
)

var NasdaqHeaders = map[string]string{"User-Agent": UA, "Accept": "application/json, text/plain, */*", "Origin": "https://www.nasdaq.com", "Referer": "https://www.nasdaq.com/"}
var TMXHeaders = map[string]string{"User-Agent": UA, "locale": "en", "Origin": "https://money.tmx.com", "Referer": "https://money.tmx.com/"}
var YahooHeaders = map[string]string{"x-yahoo-cg-client-name": "finance", "Origin": "https://finance.yahoo.com", "Referer": "https://finance.yahoo.com/"}
var FeedHeaders = map[string]string{"User-Agent": UA, "Accept": "application/rss+xml, application/xml, text/xml, */*"}

var ExtraSources = []string{"yahoo", "sa", "gnews"}
var SourceMinutes = map[string]int{"yahoo": FreshMinutes, "sa": 30, "gnews": 30}

var pacer = market.NewPacer()

var pace = func(host string, seconds float64) { pacer.Pace(host, seconds) }

type Listing struct {
	Symbol, Exchange, Currency, Name string
}

func CleanText(t string) string {
	return py.Strip(py.CollapseSpace(html.UnescapeString(t)))
}

func stamp(t time.Time) string { return t.UTC().Format("2006-01-02T15:04:05Z") }

func TMXNames(topic, symbol string) bool {
	bare, suffix, _ := strings.Cut(strings.ToUpper(strings.TrimSpace(symbol)), ":")
	if bare == "" {
		return false
	}
	us := suffix == "US"
	for _, code := range strings.Split(strings.Trim(strings.TrimSpace(topic), "[]"), ",") {
		head, mkt, _ := strings.Cut(strings.ToUpper(strings.TrimSpace(code)), ":")
		if head != bare {
			continue
		}
		if us && (mkt == "" || mkt == "US") {
			return true
		}
		if !us && (mkt == "CA" || mkt == "CNX" || mkt == "AQL") {
			return true
		}
	}
	return false
}

func ParseTMXNews(data map[string]any, symbol string, media bool) []store.WireItem {
	d, _ := data["data"].(map[string]any)
	items, _ := d["news"].([]any)
	rows := []store.WireItem{}
	for _, raw := range items {
		it, ok := raw.(map[string]any)
		if !ok || py.JSONStr(it["newsid"]) == "" {
			continue
		}
		if media && !TMXNames(py.S(it["topic"]), symbol) {
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
		id := py.JSONStr(it["newsid"])
		kind, via := KindOf(source), "tmx"
		if media {
			kind, via = "story", "tmx-media"
		}
		rows = append(rows, store.WireItem{ID: "tmx:" + id, Headline: CleanText(py.S(it["headline"])), Source: source, URL: fmt.Sprintf(TMXNewsURL, symbol, id), PublishedAt: stamp(t), Summary: SummaryText(py.S(it["summary"]), py.S(it["headline"])), Kind: kind, Via: via})
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
		if !ok || py.JSONStr(it["id"]) == "" || py.S(it["title"]) == "" {
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
		rows = append(rows, store.WireItem{ID: "nasdaq:" + py.JSONStr(it["id"]), Headline: CleanText(py.S(it["title"])), Source: source, URL: u, PublishedAt: when, Kind: k})
	}
	return rows
}

var rfc822Layouts = []string{time.RFC1123Z, time.RFC1123, "Mon, 2 Jan 2006 15:04:05 -0700", "Mon, 2 Jan 2006 15:04:05 MST", "2 Jan 2006 15:04:05 -0700", "2 Jan 2006 15:04:05 MST", time.RFC822Z, time.RFC822}

func iso(value string) string {
	text := strings.TrimSpace(value)
	if text == "" {
		return ""
	}
	var when time.Time
	digits := len(text) >= 4
	for i := 0; digits && i < 4; i++ {
		if text[i] < '0' || text[i] > '9' {
			digits = false
		}
	}
	if digits {
		t, naive, ok := py.ParseISO(strings.Replace(text, "Z", "+00:00", 1))
		if !ok {
			return ""
		}
		if naive {
			t = time.Date(t.Year(), t.Month(), t.Day(), t.Hour(), t.Minute(), t.Second(), t.Nanosecond(), time.UTC)
		}
		when = t
	} else {
		parsed := false
		for _, layout := range rfc822Layouts {
			if t, err := time.Parse(layout, text); err == nil {
				when, parsed = t, true
				break
			}
		}
		if !parsed {
			return ""
		}
	}
	if when.Year() < 2000 {
		return ""
	}
	return stamp(when)
}

var cdataRE = regexp.MustCompile(`(?s)^\s*<!\[CDATA\[(.*?)\]\]>\s*$`)

func tag(xml, name string) string {
	re := py.RE(`(?s)<` + regexp.QuoteMeta(name) + `[^>]*>(.*?)</` + regexp.QuoteMeta(name) + `>`)
	m := re.FindStringSubmatch(xml)
	if m == nil {
		return ""
	}
	text := m[1]
	if c := cdataRE.FindStringSubmatch(text); c != nil {
		text = c[1]
	}
	return CleanText(text)
}

func YahooForm(c *market.Client, symbol, exchange, currency string) string {
	if market.TMXForm(exchange, currency) == nil {
		return ""
	}
	forms := market.YahooFormsFor(market.Rec{Symbol: symbol, Exchange: exchange, Currency: currency})
	if len(forms) > 0 && strings.TrimSpace(exchange) == "" && strings.HasSuffix(forms[0], ".TO") {
		remembered := c.TMXRemembered(market.TMXSymbol(symbol))
		suffix := map[string]string{":CNX": ".CN", ":AQL": ".NE"}[remembered[len(market.TMXBare(remembered)):]]
		if suffix != "" {
			return market.YahooRoot(symbol) + suffix
		}
	}
	if len(forms) == 0 {
		return ""
	}
	return strings.ToUpper(forms[0])
}

var otcTwinRE = regexp.MustCompile(`^[A-Z]{4}[FY]$`)

func ParseYahooNews(data map[string]any, form, symbol, name string) []store.WireItem {
	type asset struct {
		item map[string]any
		tags map[string]bool
	}
	d, _ := data["data"].(map[string]any)
	list, _ := d["lightyearList"].(map[string]any)
	main, _ := list["main"].(map[string]any)
	edges, _ := main["edges"].([]any)
	assets := []asset{}
	for _, e := range edges {
		edge, _ := e.(map[string]any)
		node, _ := edge["node"].(map[string]any)
		a, _ := node["asset"].(map[string]any)
		if py.S(a["id"]) == "" || py.S(a["title"]) == "" {
			continue
		}
		tags := map[string]bool{}
		fin, _ := a["finance"].(map[string]any)
		tickers, _ := fin["stockTickers"].([]any)
		for _, t := range tickers {
			tm, ok := t.(map[string]any)
			if !ok {
				continue
			}
			if s := strings.ToUpper(strings.TrimSpace(py.S(tm["symbol"]))); s != "" {
				tags[s] = true
			}
		}
		assets = append(assets, asset{a, tags})
	}
	twins := map[string]bool{}
	if strings.Contains(form, ".") {
		for _, a := range assets {
			if !a.tags[form] || len(a.tags) != 2 {
				continue
			}
			for t := range a.tags {
				if t != form && otcTwinRE.MatchString(t) {
					twins[t] = true
				}
			}
		}
	}
	us := !strings.Contains(form, ".")
	bare := symbol
	if bare == "" {
		bare, _, _ = strings.Cut(form, ".")
	}
	rows := []store.WireItem{}
	for _, a := range assets {
		title := CleanText(py.S(a.item["title"]))
		keep := a.tags[form]
		if !keep {
			for t := range a.tags {
				if twins[t] {
					keep = true
				}
			}
		}
		if !keep && len(a.tags) == 0 && NamesListing(title, bare, name, us) {
			keep = true
		}
		if !keep {
			continue
		}
		attrs, _ := a.item["contentAttributes"].(map[string]any)
		when := iso(py.S(attrs["pubDate"]))
		if when == "" {
			continue
		}
		provider, _ := attrs["provider"].(map[string]any)
		source := CleanText(py.S(provider["displayName"]))
		if source == "" {
			source = "Yahoo Finance"
		}
		u := py.S(attrs["canonicalUrl"])
		if u == "" {
			u = py.S(attrs["clickthroughUrl"])
		}
		summary := py.S(attrs["summary"])
		if summary == "" {
			summary = py.S(attrs["description"])
		}
		rows = append(rows, store.WireItem{ID: "yahoo:" + py.S(a.item["id"]), Headline: title, Source: source, URL: u, PublishedAt: when, Summary: SummaryText(summary, title), Kind: KindOf(source), Via: "yahoo"})
	}
	return rows
}

func fetchYahoo(c *market.Client, symbol, exchange, currency, name string) ([]store.WireItem, bool, error) {
	form := YahooForm(c, symbol, exchange, currency)
	if form == "" {
		return nil, false, nil
	}
	alias := "finance-US-en-US-ticker-all"
	variables := map[string]any{
		"clientContext": map[string]any{"device": "DESKTOP", "region": "US", "site": "finance", "lang": "en-US"},
		"gqlContext":    []any{map[string]any{"listAlias": "list=" + alias}},
		"imageResize":   []any{},
		"listInput": map[string]any{"disableDedupe": false, "enableBlockedContent": false, "filterClientContext": false, "getFullList": true,
			"enableQueryTimeLicenseCheck": true, "queryVariables": map[string]any{"tickerSymbol": []any{form}}, "slug": "list=" + alias},
		"mlRecsInput": map[string]any{"count": 200, "instance": "FINANCE"}, "first": 100, "mlRecsFirst": 50,
	}
	pace("nexus-gateway-prod.media.yahoo.com", 1.0)
	data, err := c.PostJSON(YahooGateway, map[string]any{"query": YahooNewsQuery, "operationName": "FinancePolarisTickerNews", "variables": variables}, YahooHeaders)
	if err != nil {
		return nil, true, err
	}
	return ParseYahooNews(data, form, symbol, name), true, nil
}

func SAForm(symbol, exchange, currency string) string {
	form := market.TMXForm(exchange, currency)
	sym := strings.ToUpper(market.TMXSymbol(symbol))
	if form == nil {
		return ""
	}
	if *form == ":US" {
		return sym
	}
	if *form == "" {
		return sym + ":CA"
	}
	return ""
}

var itemRE = regexp.MustCompile(`(?s)<item>(.*?)</item>`)
var saSymbolRE = regexp.MustCompile(`(?s)<sa:symbol>(.*?)</sa:symbol>`)

func ParseSANews(xml, form string) []store.WireItem {
	rows := []store.WireItem{}
	for _, m := range itemRE.FindAllStringSubmatch(xml, -1) {
		item := m[1]
		named := false
		for _, s := range saSymbolRE.FindAllStringSubmatch(item, -1) {
			if strings.ToUpper(CleanText(s[1])) == strings.ToUpper(form) {
				named = true
			}
		}
		if !named {
			continue
		}
		guid, title, when := tag(item, "guid"), tag(item, "title"), iso(tag(item, "pubDate"))
		if guid == "" || title == "" || when == "" {
			continue
		}
		u := tag(item, "link")
		if u == "" {
			u = guid
		}
		rows = append(rows, store.WireItem{ID: "sa:" + shortHash(guid), Headline: title, Source: "Seeking Alpha", URL: u, PublishedAt: when, Summary: SummaryText(tag(item, "description"), title), Kind: "story", Via: "sa"})
	}
	return rows
}

func shortHash(s string) string {
	sum := sha1.Sum([]byte(s))
	return hex.EncodeToString(sum[:])[:16]
}

func fetchSA(c *market.Client, symbol, exchange, currency string) ([]store.WireItem, bool, error) {
	form := SAForm(symbol, exchange, currency)
	if form == "" {
		return nil, false, nil
	}
	pace("seekingalpha.com", 1.0)
	xml, err := c.GetText(fmt.Sprintf(SANewsURL, pyQuote(form, ":")), FeedHeaders)
	if err != nil {
		if market.StatusOf(err) == 404 || strings.Contains(err.Error(), "404") {
			return nil, false, nil
		}
		return nil, true, err
	}
	return ParseSANews(xml, form), true, nil
}

func pyQuote(s, safe string) string {
	var b strings.Builder
	for i := 0; i < len(s); i++ {
		ch := s[i]
		if (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') || (ch >= '0' && ch <= '9') || ch == '_' || ch == '.' || ch == '-' || ch == '~' || ch == '/' || strings.IndexByte(safe, ch) >= 0 {
			b.WriteByte(ch)
		} else {
			fmt.Fprintf(&b, "%%%02X", ch)
		}
	}
	return b.String()
}

var corporate = set("inc", "incorporated", "corp", "corporation", "ltd", "limited", "plc", "co", "company", "holdings", "holding", "group",
	"nv", "sa", "ag", "se", "lp", "llc", "the", "class", "units", "unit", "shares", "common", "ordinary", "adr", "trust")
var labelRE = regexp.MustCompile(`(?i)^(class|series)\s+\w+(\s+(units?|shares?))?$|^(units?|shares?|etf|fund|common( shares)?)$`)
var fundishRE = regexp.MustCompile(`(?i)\b(etf|fund|portfolio|trust)\b`)
var quotePageRE = regexp.MustCompile(`(?i)price and chart|\b(stock|share) price\s*[,&]|share price - |holdings list|^technical analysis of|etf profile:|stock forecast and price target|` +
	`^etfs investing in|forecast\s*[–—-]\s*price target|price prediction|tokenomics|price today: live|\brstock\b|` +
	`\s[–—]\s(?:TSX|TSXV|CSE|NEO|NASDAQ|NYSE|AMEX|OTC)\s?:\s?[A-Z0-9.]+\s*$|^\$[^$]+\$$`)
var caVenues = []string{"TSX", "TSXV", "TSX-V", "CVE", "CSE", "CNSX", "CN", "NEO", "CBOE CANADA"}
var usVenues = []string{"NASDAQ", "NYSE", "NYSEARCA", "NYSE ARCA", "NYSEAMERICAN", "NYSE AMERICAN", "AMEX", "BATS", "CBOE"}
var caSuffixes = []string{".TO", ".V", ".CN", ".C", ".NE", ":CA", ":CNX", ":AQL"}
var joiners = set("of", "and", "de", "du", "des", "la", "le", "et", "for", "on", "at", "y")
var generic = set("canadian", "canada", "american", "america", "national", "international", "global", "general", "united", "universal",
	"northern", "southern", "eastern", "western", "northwest", "pacific", "atlantic", "arctic", "central", "british",
	"european", "chinese", "mexican", "brazil", "quebec", "ontario", "alberta", "manitoba", "california", "nevada", "arizona",
	"alaska", "texas", "frontier", "pioneer", "liberty", "patriot", "heritage", "capital", "energy", "energies", "silver",
	"golden", "digital", "quantum", "advanced", "applied", "intuitive", "precision", "premium", "select", "strategic",
	"strategy", "summit", "bright", "lithium", "uranium", "copper", "nickel", "cobalt", "graphite", "metals", "mining",
	"resources", "minerals", "petroleum", "natural", "health", "healthcare", "medical", "pharma", "therapeutics",
	"sciences", "science", "technology", "technologies", "software", "systems", "network", "networks", "solutions",
	"services", "industries", "industrial", "financial", "finance", "investment", "investments", "partners", "income",
	"dividend", "growth", "equity", "innovation", "innovative", "materials", "hydrogen", "battery", "electric", "motors",
	"aerospace", "defence", "defense", "security", "securities", "standard", "interactive", "entertainment", "communications",
	"telecom", "wireless", "insurance", "realty", "properties", "estate", "infrastructure", "renewable", "renewables",
	"environmental", "agricultural", "foods", "brands", "consumer", "retail", "bancorp", "banking", "credit", "mortgage",
	"royalty", "royalties", "exploration", "robotics", "biotech", "semiconductor", "semiconductors", "solar")

func set(words ...string) map[string]bool {
	m := make(map[string]bool, len(words))
	for _, w := range words {
		m[w] = true
	}
	return m
}

var accents = map[rune]string{
	'À': "A", 'Á': "A", 'Â': "A", 'Ã': "A", 'Ä': "A", 'Å': "A", 'Ç': "C", 'È': "E", 'É': "E", 'Ê': "E", 'Ë': "E", 'Ì': "I", 'Í': "I", 'Î': "I", 'Ï': "I",
	'Ñ': "N", 'Ò': "O", 'Ó': "O", 'Ô': "O", 'Õ': "O", 'Ö': "O", 'Ø': "O", 'Ù': "U", 'Ú': "U", 'Û': "U", 'Ü': "U", 'Ý': "Y",
	'à': "a", 'á': "a", 'â': "a", 'ã': "a", 'ä': "a", 'å': "a", 'ç': "c", 'è': "e", 'é': "e", 'ê': "e", 'ë': "e", 'ì': "i", 'í': "i", 'î': "i", 'ï': "i",
	'ñ': "n", 'ò': "o", 'ó': "o", 'ô': "o", 'õ': "o", 'ö': "o", 'ø': "o", 'ù': "u", 'ú': "u", 'û': "u", 'ü': "u", 'ý': "y", 'ÿ': "y",
	'Ā': "A", 'ā': "a", 'Ă': "A", 'ă': "a", 'Ą': "A", 'ą': "a", 'Ć': "C", 'ć': "c", 'Č': "C", 'č': "c", 'Ď': "D", 'ď': "d", 'Ē': "E", 'ē': "e", 'Ė': "E", 'ė': "e", 'Ę': "E", 'ę': "e", 'Ě': "E", 'ě': "e",
	'Ğ': "G", 'ğ': "g", 'Ī': "I", 'ī': "i", 'İ': "I", 'ı': "i", 'Ł': "L", 'ł': "l", 'Ń': "N", 'ń': "n", 'Ň': "N", 'ň': "n", 'Ō': "O", 'ō': "o", 'Ő': "O", 'ő': "o", 'Œ': "OE", 'œ': "oe",
	'Ř': "R", 'ř': "r", 'Ś': "S", 'ś': "s", 'Ş': "S", 'ş': "s", 'Š': "S", 'š': "s", 'Ţ': "T", 'ţ': "t", 'Ť': "T", 'ť': "t", 'Ū': "U", 'ū': "u", 'Ů': "U", 'ů': "u", 'Ű': "U", 'ű': "u",
	'Ź': "Z", 'ź': "z", 'Ż': "Z", 'ż': "z", 'Ž': "Z", 'ž': "z", 'ẞ': "SS", 'ß': "ss", 'Æ': "AE", 'æ': "ae",
}

func fold(text string) string {
	var b strings.Builder
	for _, r := range text {
		if s, ok := accents[r]; ok {
			b.WriteString(s)
		} else {
			b.WriteRune(r)
		}
	}
	return b.String()
}

var wordRE = regexp.MustCompile(`[a-z0-9]+`)
var tokenRE = regexp.MustCompile(`[A-Za-z0-9]+`)

func words(text string) []string {
	return wordRE.FindAllString(strings.ToLower(fold(text)), -1)
}

var parenRE = regexp.MustCompile(`\([^)]*\)`)
var dashSplitRE = regexp.MustCompile(`\s+[-–—]\s+`)
var classTailRE = regexp.MustCompile(`(?i)\s+(class|series)\s+[a-z]\b.*$`)
var nonLetterRE = regexp.MustCompile(`[^a-z]`)

func SearchName(name string) string {
	text := parenRE.ReplaceAllString(name, " ")
	var parts []string
	for _, p := range dashSplitRE.Split(text, -1) {
		p = strings.Trim(p, " .,-")
		if p != "" && !labelRE.MatchString(p) {
			parts = append(parts, p)
		}
	}
	if len(parts) == 0 {
		return ""
	}
	fundLater := false
	for _, p := range parts[1:] {
		if fundishRE.MatchString(p) {
			fundLater = true
		}
	}
	if len(parts) > 1 && !fundishRE.MatchString(parts[0]) && fundLater {
		fund := ""
		for _, p := range parts[1:] {
			if fundishRE.MatchString(p) {
				fund = p
				break
			}
		}
		var brand []string
		for _, w := range strings.Fields(parts[0]) {
			lw := strings.Trim(strings.ToLower(w), ".,")
			if !corporate[lw] && lw != "partners" {
				brand = append(brand, w)
			}
		}
		head := ""
		if len(brand) > 0 {
			head = brand[0]
		}
		if head == "" || strings.HasPrefix(strings.ToLower(fund), strings.ToLower(head)) {
			text = fund
		} else {
			text = head + " " + fund
		}
	} else {
		text = parts[0]
	}
	text = classTailRE.ReplaceAllString(text, "")
	ws := strings.Fields(text)
	for len(ws) > 0 && corporate[nonLetterRE.ReplaceAllString(strings.ToLower(ws[len(ws)-1]), "")] {
		ws = ws[:len(ws)-1]
	}
	return CleanText(strings.Trim(strings.Join(ws, " "), " ,"))
}

func brand(name string) []string {
	out := []string{}
	for _, w := range words(SearchName(name)) {
		if !corporate[w] {
			out = append(out, w)
		}
	}
	return out
}

func hasUpper(s string) bool {
	for _, r := range s {
		if unicode.IsUpper(r) {
			return true
		}
	}
	return false
}

func isAlpha(s string) bool {
	if s == "" {
		return false
	}
	for _, r := range s {
		if !unicode.IsLetter(r) {
			return false
		}
	}
	return true
}

func isDigits(s string) bool {
	if s == "" {
		return false
	}
	for _, r := range s {
		if r < '0' || r > '9' {
			return false
		}
	}
	return true
}

func NamesListing(headline, symbol, name string, us bool) bool {
	head := fold(headline)
	letters, upper := 0, 0
	for _, r := range head {
		if unicode.IsLetter(r) && r < 128 {
			letters++
			if unicode.IsUpper(r) {
				upper++
			}
		}
	}
	mostlyCaps := letters > 0 && float64(upper) > 0.7*float64(letters)
	sym := strings.ToUpper(market.TMXSymbol(symbol))
	if sym != "" {
		e := regexp.QuoteMeta(sym)
		venueList := caVenues
		if us {
			venueList = usVenues
		}
		venues := make([]string, len(venueList))
		for i, v := range venueList {
			venues[i] = strings.ReplaceAll(regexp.QuoteMeta(v), `\ `, `\s?`)
			venues[i] = strings.ReplaceAll(venues[i], ` `, `\s?`)
		}
		forms := []string{`(?:` + strings.Join(venues, "|") + `)\s?:\s?` + e, `\(` + e + `\)`, `\$` + e}
		if !us {
			suffixes := make([]string, len(caSuffixes))
			for i, x := range caSuffixes {
				suffixes[i] = regexp.QuoteMeta(x)
			}
			forms = append(forms, e+`(?:`+strings.Join(suffixes, "|")+`)`)
		}
		formRE := py.RE(`(?i)(?:^|[^A-Za-z0-9])(?:` + strings.Join(forms, "|") + `)(?:[^A-Za-z0-9]|$)`)
		if formRE.MatchString(head) {
			return true
		}
		if len(sym) >= 3 && !mostlyCaps {
			bareRE := py.RE(`(?:^|[^A-Za-z0-9.$])` + e + `(?:[^A-Za-z0-9]|$)`)
			if bareRE.MatchString(head) {
				return true
			}
		}
	}
	br := brand(name)
	tokens := tokenRE.FindAllString(head, -1)
	ws := make([]string, len(tokens))
	for i, t := range tokens {
		ws[i] = strings.ToLower(t)
	}
	named := func(t string) bool { return mostlyCaps || hasUpper(t) }
	meaning := []int{}
	for i, w := range br {
		if !joiners[w] {
			meaning = append(meaning, i)
		}
	}
	if len(meaning) >= 2 {
		prefix := br[:meaning[1]+1]
		n := len(prefix)
		for i := 0; i+n <= len(ws); i++ {
			chunk := ws[i : i+n]
			same := true
			for k := 0; k < n-1; k++ {
				if chunk[k] != prefix[k] {
					same = false
				}
			}
			last := chunk[n-1]
			if !same || !(last == prefix[n-1] || (len(last) >= 4 && strings.HasPrefix(prefix[n-1], last))) {
				continue
			}
			ok := true
			for _, t := range tokens[i : i+n] {
				if !joiners[strings.ToLower(t)] && !named(t) {
					ok = false
				}
			}
			if !ok {
				continue
			}
			j, k := i+n, n
			for k < len(br) && joiners[br[k]] && j < len(ws) && ws[j] == br[k] {
				j, k = j+1, k+1
			}
			if k > n && (k >= len(br) || j >= len(ws) || ws[j] != br[k]) {
				continue
			}
			return true
		}
	}
	if len(meaning) > 0 && meaning[0] == 0 {
		first := br[0]
		if len(first) >= 6 && !isDigits(first) && !generic[first] {
			long := []string{}
			for _, t := range tokens {
				if len(t) > 3 && isAlpha(t) {
					long = append(long, t)
				}
			}
			caps := 0
			for _, t := range long {
				if unicode.IsUpper([]rune(t)[0]) {
					caps++
				}
			}
			titleCase := len(long) >= 3 && float64(caps) > 0.6*float64(len(long))
			for i, t := range tokens {
				if strings.ToLower(t) != first || !unicode.IsUpper([]rune(t)[0]) {
					continue
				}
				if i == 0 || (hasUpper(t[1:]) && !mostlyCaps) || !(titleCase || mostlyCaps) {
					return true
				}
			}
		}
	}
	return false
}

func ParseGoogleNews(xml, symbol, name string, us bool) []store.WireItem {
	rows := []store.WireItem{}
	for _, m := range itemRE.FindAllStringSubmatch(xml, -1) {
		item := m[1]
		title, source, link := tag(item, "title"), tag(item, "source"), tag(item, "link")
		when := iso(tag(item, "pubDate"))
		if title == "" || link == "" || when == "" {
			continue
		}
		if source != "" && strings.HasSuffix(title, " - "+source) {
			title = strings.TrimRight(title[:len(title)-len(" - "+source)], " ")
		}
		if quotePageRE.MatchString(title) || !NamesListing(title, symbol, name, us) {
			continue
		}
		src := source
		if src == "" {
			src = "Google News"
		}
		rows = append(rows, store.WireItem{ID: "gnews:" + shortHash(link), Headline: title, Source: src, URL: link, PublishedAt: when, Kind: KindOf(source), Via: "gnews"})
	}
	return rows
}

func GoogleQueries(symbol, exchange, currency, name string) []string {
	sym := strings.ToUpper(market.TMXSymbol(symbol))
	out := []string{}
	if clean := SearchName(name); clean != "" && strings.ToUpper(clean) != sym {
		out = append(out, `"`+clean+`"`)
	}
	form := market.TMXForm(exchange, currency)
	ex := strings.ToUpper(strings.TrimSpace(exchange))
	venue := ""
	if form != nil {
		switch *form {
		case ":CNX":
			venue = "CSE"
		case ":AQL":
			venue = "NEO"
		case "":
			venue = "TSX"
			if ex == "TSX-V" || ex == "TSXV" {
				venue = "TSXV"
			}
		case ":US":
			if strings.HasPrefix(ex, "NYSE") {
				venue = "NYSE"
			} else if ex == "NASDAQ" || ex == "" {
				venue = "NASDAQ"
			}
		}
	}
	if sym != "" && venue != "" {
		out = append(out, `"`+venue+":"+sym+`"`)
	}
	return out
}

func fetchGoogle(c *market.Client, symbol, exchange, currency, name string) ([]store.WireItem, bool, error) {
	form := market.TMXForm(exchange, currency)
	us := form != nil && *form == ":US"
	queries := GoogleQueries(symbol, exchange, currency, name)
	if len(queries) == 0 {
		return nil, false, nil
	}
	rows, seen := []store.WireItem{}, map[string]bool{}
	for _, q := range queries {
		pace("news.google.com", 1.5)
		xml, err := c.GetText(fmt.Sprintf(GNewsURL, pyQuote(q, "")), FeedHeaders)
		if err != nil {
			return nil, true, err
		}
		for _, r := range ParseGoogleNews(xml, symbol, name, us) {
			if !seen[r.ID] {
				seen[r.ID] = true
				rows = append(rows, r)
			}
		}
	}
	return rows, true, nil
}

func SourcesFor(c *market.Client, symbol, exchange, currency, name string) []string {
	if symbol == Market[0] || market.TMXForm(exchange, currency) == nil {
		return []string{}
	}
	have := map[string]bool{"yahoo": YahooForm(c, symbol, exchange, currency) != "", "sa": SAForm(symbol, exchange, currency) != "", "gnews": len(GoogleQueries(symbol, exchange, currency, name)) > 0}
	out := []string{}
	for _, k := range ExtraSources {
		if have[k] {
			out = append(out, k)
		}
	}
	return out
}

var readExtra = func(c *market.Client, key, symbol, exchange, currency, name string) ([]store.WireItem, bool, error) {
	switch key {
	case "yahoo":
		return fetchYahoo(c, symbol, exchange, currency, name)
	case "sa":
		return fetchSA(c, symbol, exchange, currency)
	}
	return fetchGoogle(c, symbol, exchange, currency, name)
}

func SameStory(whenA, whenB string) bool {
	a, _, okA := py.ParseISO(strings.Replace(whenA, "Z", "+00:00", 1))
	b, _, okB := py.ParseISO(strings.Replace(whenB, "Z", "+00:00", 1))
	if !okA || !okB {
		return false
	}
	d := a.Sub(b)
	if d < 0 {
		d = -d
	}
	return d <= SameStoryHours*time.Hour
}

func NewsText(headline string) string { return strings.Join(words(headline), " ") }

func Origin(id string) string {
	src, _, _ := strings.Cut(id, ":")
	return src
}

func stampKey(source, symbol, exchange string) string {
	return "news_source_fetched:" + source + ":" + store.NewsKey(symbol, exchange)
}

func due(st *store.Store, source, symbol, exchange string, now time.Time) bool {
	last := st.GetMeta(stampKey(source, symbol, exchange))
	age, ok := market.AgeOf(last, now)
	minutes, has := SourceMinutes[source]
	if !has {
		minutes = FreshMinutes
	}
	return !ok || age > time.Duration(minutes)*time.Minute
}

type wireAnswer struct {
	rows    []store.WireItem
	missing map[string]bool
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

var fetchWire = fetchWireImpl

func fetchWireImpl(c *market.Client, symbol, exchange, currency string, now time.Time) (string, *wireAnswer) {
	src := SourceFor(symbol, exchange, currency)
	sym := market.TMXSymbol(symbol)
	if sym == "" {
		return src, &wireAnswer{rows: []store.WireItem{}, missing: map[string]bool{}}
	}
	if src == "" {
		src = "nasdaq"
	}
	if symbol == Market[0] {
		pace("api.nasdaq.com", 0.6)
		data, err := getJSON(c, fmt.Sprintf(NasdaqLatestURL, PerMarket), NasdaqHeaders)
		if err != nil {
			fmt.Fprintf(os.Stderr, "bagholder news: %s from %s failed: %s\n", sym, src, err)
			return src, nil
		}
		return src, &wireAnswer{rows: ParseNasdaqNews(data, now, "", ""), missing: map[string]bool{}}
	}
	if src == "tmx" {
		code := market.TMXQuoteSymbol(symbol, exchange, currency)
		if code == "" {
			return src, &wireAnswer{rows: []store.WireItem{}, missing: map[string]bool{}}
		}
		ask := func(form string) (*wireAnswer, error) {
			out := &wireAnswer{rows: []store.WireItem{}, missing: map[string]bool{}}
			for _, media := range []bool{false, true} {
				pace("app-money.tmx.com", 0.6)
				data, err := c.PostJSON(market.TMXURL, map[string]any{"operationName": "getNewsForSymbol", "variables": map[string]any{"symbol": form, "page": 1, "limit": PerSymbol, "locale": "en", "companyInNews": media}, "query": TMXNewsQuery}, TMXHeaders)
				if err != nil {
					if !media {
						return nil, err
					}
					fmt.Fprintf(os.Stderr, "bagholder news: %s stories from tmx failed: %s\n", form, err)
					out.missing["tmx-media"] = true
					continue
				}
				got := ParseTMXNews(data, form, media)
				if len(got) == 0 {
					if media {
						out.missing["tmx-media"] = true
					} else {
						out.missing["tmx"] = true
					}
				}
				out.rows = append(out.rows, got...)
			}
			return out, nil
		}
		first := c.TMXRemembered(code)
		ans, err := ask(first)
		if err != nil {
			fmt.Fprintf(os.Stderr, "bagholder news: %s from %s failed: %s\n", sym, src, err)
			return src, nil
		}
		if len(ans.rows) == 0 {
			if alt := c.TMXResolve(code); alt != "" && alt != first {
				ans, err = ask(alt)
				if err != nil {
					fmt.Fprintf(os.Stderr, "bagholder news: %s from %s failed: %s\n", sym, src, err)
					return src, nil
				}
			}
		}
		return src, ans
	}
	pace("api.nasdaq.com", 0.6)
	data, err := getJSON(c, fmt.Sprintf(NasdaqNewsURL, sym, PerSymbol), NasdaqHeaders)
	if err != nil {
		fmt.Fprintf(os.Stderr, "bagholder news: %s from %s failed: %s\n", sym, src, err)
		return src, nil
	}
	out := &wireAnswer{rows: []store.WireItem{}, missing: map[string]bool{}}
	for _, r := range ParseNasdaqNews(data, now, sym, "") {
		r.Via = "nasdaq"
		out.rows = append(out.rows, r)
	}
	if len(out.rows) == 0 {
		out.missing["nasdaq"] = true
	}
	pace("api.nasdaq.com", 0.6)
	press, err := getJSON(c, fmt.Sprintf(NasdaqPressURL, sym, PerSymbol), NasdaqHeaders)
	if err != nil {
		out.missing["nasdaq-press"] = true
		fmt.Fprintf(os.Stderr, "bagholder news: %s releases from nasdaq failed: %s\n", sym, err)
		return src, out
	}
	seen := map[string]bool{}
	for _, r := range out.rows {
		seen[r.ID] = true
	}
	added := 0
	for _, r := range ParseNasdaqNews(press, now, sym, "release") {
		if seen[r.ID] {
			continue
		}
		r.Via = "nasdaq-press"
		out.rows = append(out.rows, r)
		added++
	}
	if added == 0 {
		out.missing["nasdaq-press"] = true
	}
	return src, out
}

func FetchSymbol(c *market.Client, symbol, exchange, currency string, now time.Time) (string, []store.WireItem, bool) {
	src, ans := fetchWire(c, symbol, exchange, currency, now)
	if ans == nil {
		return src, nil, false
	}
	return src, ans.rows, true
}

func FetchListing(c *market.Client, symbol, exchange, currency string, now time.Time, name string, force bool) (string, []store.WireItem, map[string]bool, bool) {
	if symbol == Market[0] {
		src, rows, ok := FetchSymbol(c, symbol, exchange, currency, now)
		answered := map[string]bool{}
		if ok {
			answered[src] = true
		}
		return src, rows, answered, ok
	}
	extras := []string{}
	for _, k := range SourcesFor(c, symbol, exchange, currency, name) {
		if force || due(c.Store, k, symbol, exchange, now) {
			extras = append(extras, k)
		}
	}
	type extraResult struct {
		rows  []store.WireItem
		asked bool
		err   error
	}
	results := map[string]extraResult{}
	var mu sync.Mutex
	var wg sync.WaitGroup
	var src string
	var primary *wireAnswer
	wg.Add(1)
	go func() {
		defer wg.Done()
		src, primary = fetchWire(c, symbol, exchange, currency, now)
	}()
	for _, k := range extras {
		wg.Add(1)
		go func(k string) {
			defer wg.Done()
			rows, asked, err := readExtra(c, k, symbol, exchange, currency, name)
			mu.Lock()
			results[k] = extraResult{rows, asked, err}
			mu.Unlock()
		}(k)
	}
	wg.Wait()
	got := map[string][]store.WireItem{}
	for k, r := range results {
		if r.err != nil {
			fmt.Fprintf(os.Stderr, "bagholder news: %s from %s failed: %s\n", symbol, k, r.err)
			continue
		}
		if r.asked {
			got[k] = r.rows
		}
	}
	answered := map[string]bool{}
	for k := range got {
		answered[k] = true
	}
	if primary != nil {
		answered[src] = true
	}
	if len(answered) == 0 {
		return src, nil, answered, false
	}
	for _, k := range extras {
		answered[k] = true
	}
	stored, storedFeed := map[string][]store.WireItem{}, map[string][]store.WireItem{}
	for _, r := range c.Store.NewsFor(symbol, exchange) {
		via := r.Source
		if via == "" {
			via = Origin(r.ID)
		}
		row := store.WireItem{ID: r.ID, Headline: r.Headline, Source: r.Wire, URL: r.URL, PublishedAt: r.PublishedAt, Kind: r.Kind, Via: via}
		stored[Origin(r.ID)] = append(stored[Origin(r.ID)], row)
		storedFeed[via] = append(storedFeed[via], row)
	}
	merged, ids, texts := []store.WireItem{}, map[string]bool{}, map[string][]string{}
	add := func(items []store.WireItem) {
		for _, r := range items {
			text := NewsText(r.Headline)
			if ids[r.ID] {
				continue
			}
			dup := false
			if text != "" {
				for _, w := range texts[text] {
					if SameStory(r.PublishedAt, w) {
						dup = true
					}
				}
			}
			if dup {
				continue
			}
			ids[r.ID] = true
			if text != "" {
				texts[text] = append(texts[text], r.PublishedAt)
			}
			merged = append(merged, r)
		}
	}
	if primary != nil && len(primary.rows) > 0 {
		add(primary.rows)
	} else {
		add(append(append([]store.WireItem{}, stored["tmx"]...), stored["nasdaq"]...))
	}
	if primary != nil {
		for feed := range primary.missing {
			add(storedFeed[feed])
		}
	}
	for _, k := range ExtraSources {
		if rows, ok := got[k]; ok && len(rows) > 0 {
			add(rows)
		} else {
			add(stored[k])
		}
	}
	for i := range merged {
		if merged[i].Via == "" {
			merged[i].Via = Origin(merged[i].ID)
		}
	}
	sort.SliceStable(merged, func(i, j int) bool { return merged[i].PublishedAt > merged[j].PublishedAt })
	if len(merged) > PerListing {
		merged = merged[:PerListing]
	}
	return src, merged, answered, true
}

type OnNew func(symbol, exchange string, rows []store.WireItem, newIDs map[string]bool)

func ReadListing(c *market.Client, symbol, exchange, currency string, now time.Time, name string, force bool, onNew OnNew) (string, []store.WireItem, bool) {
	src, rows, answered, ok := FetchListing(c, symbol, exchange, currency, now, name, force)
	if !ok {
		return src, nil, false
	}
	before, beforeText, firstRead := map[string]bool{}, map[string][]string{}, map[string]bool{}
	if onNew != nil {
		for _, r := range c.Store.NewsFor(symbol, exchange) {
			before[r.ID] = true
			if t := NewsText(r.Headline); t != "" {
				beforeText[t] = append(beforeText[t], r.PublishedAt)
			}
		}
		for _, k := range ExtraSources {
			if answered[k] && c.Store.GetMeta(stampKey(k, symbol, exchange)) == "" {
				firstRead[k] = true
			}
		}
	}
	c.Store.ReplaceNews(symbol, exchange, src, rows, stamp(now))
	for _, k := range ExtraSources {
		if answered[k] {
			c.Store.SetMeta(stampKey(k, symbol, exchange), stamp(now))
		}
	}
	if onNew != nil {
		fresh := map[string]bool{}
		for _, r := range rows {
			if r.ID == "" || before[r.ID] || firstRead[Origin(r.ID)] {
				continue
			}
			told := false
			for _, w := range beforeText[NewsText(r.Headline)] {
				if SameStory(r.PublishedAt, w) {
					told = true
				}
			}
			if !told {
				fresh[r.ID] = true
			}
		}
		func() {
			defer func() {
				if e := recover(); e != nil {
					fmt.Fprintf(os.Stderr, "bagholder news: %s items not told: %v\n", symbol, e)
				}
			}()
			onNew(symbol, exchange, rows, fresh)
		}()
	}
	return src, rows, true
}

func Stale(c *market.Client, listings []Listing, now time.Time, minutes float64) []Listing {
	fetched := c.Store.NewsFetchedAt()
	out := []Listing{}
	for _, l := range listings {
		last := fetched[store.NewsKey(l.Symbol, l.Exchange)]
		age, ok := market.AgeOf(last, now)
		stale := !ok || age > time.Duration(minutes*float64(time.Minute))
		if !stale {
			for _, k := range SourcesFor(c, l.Symbol, l.Exchange, l.Currency, l.Name) {
				if due(c.Store, k, l.Symbol, l.Exchange, now) {
					stale = true
				}
			}
		}
		if stale {
			out = append(out, l)
		}
	}
	return out
}

func Refresh(c *market.Client, listings []Listing, now time.Time, onNew OnNew, onStart func([]Listing), onDone func(Listing, bool)) int {
	due := Stale(c, listings, now, FreshMinutes)
	if onStart != nil {
		onStart(due)
	}
	if len(due) == 0 {
		return 0
	}
	workers := ListingsAtOnce
	if len(due) < workers {
		workers = len(due)
	}
	jobs := make(chan Listing)
	var wg sync.WaitGroup
	var mu sync.Mutex
	done := 0
	for i := 0; i < workers; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for l := range jobs {
				_, _, ok := ReadListing(c, l.Symbol, l.Exchange, l.Currency, now, l.Name, false, onNew)
				if onDone != nil {
					func() {
						defer func() { _ = recover() }()
						onDone(l, ok)
					}()
				}
				if ok {
					mu.Lock()
					done++
					mu.Unlock()
				}
			}
		}()
	}
	for _, l := range due {
		jobs <- l
	}
	close(jobs)
	wg.Wait()
	if done > 0 {
		c.Store.TrimNews(Keep)
	}
	return done
}
