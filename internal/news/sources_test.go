package news

import (
	"fmt"
	"net/http"
	"reflect"
	"sort"
	"strings"
	"testing"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

func ids(rows []store.WireItem) []string {
	out := []string{}
	for _, r := range rows {
		out = append(out, r.ID)
	}
	return out
}

func row(id, headline, when, source, kind string) store.WireItem {
	return store.WireItem{ID: id, Headline: headline, Source: source, URL: "u-" + id, PublishedAt: when, Kind: kind}
}

func stubExtra(t *testing.T, fn func(key, symbol, exchange, currency, name string) ([]store.WireItem, bool, error)) {
	t.Helper()
	saved := readExtra
	readExtra = func(c *market.Client, key, symbol, exchange, currency, name string) ([]store.WireItem, bool, error) {
		return fn(key, symbol, exchange, currency, name)
	}
	t.Cleanup(func() { readExtra = saved })
}

func stubWire(t *testing.T, src string, rows []store.WireItem, ok bool) {
	t.Helper()
	saved := fetchWire
	fetchWire = func(c *market.Client, symbol, exchange, currency string, now time.Time) (string, *wireAnswer) {
		if !ok {
			return src, nil
		}
		return src, &wireAnswer{rows: append([]store.WireItem{}, rows...), missing: map[string]bool{}}
	}
	t.Cleanup(func() { fetchWire = saved })
}

func quiet(t *testing.T) *market.Client {
	t.Helper()
	return newClient(t, time.Date(2026, 9, 16, 12, 0, 0, 0, time.UTC), func(r *http.Request) (*http.Response, error) { return reply(500, "down"), nil })
}

func TestTMXNamesAListingByItsOwnTopicCodes(t *testing.T) {
	topic := "[ABHI:AQL,ABHI:CA,ART00001,CCHI:AQL,CCHI:CA,DIVIDEND]"
	cases := []struct {
		topic, symbol string
		want          bool
	}{{topic, "CCHI", true}, {"[HG:CNX,MINING01]", "HG:CNX", true}, {"[ASTS,SPACE001]", "ASTS:US", true}, {topic, "CCH", false}, {"[T,VZ,TMUS]", "T", false}, {"[T:CA,BCE:CA]", "T", true}, {"[HG,INSURE01]", "HG:CNX", false}, {"[ASTS:CA]", "ASTS:US", false}, {"", "PNG", false}}
	for _, c := range cases {
		if got := TMXNames(c.topic, c.symbol); got != c.want {
			t.Errorf("TMXNames(%q, %q) = %v, want %v", c.topic, c.symbol, got, c.want)
		}
	}
}

func TestAPublishersStoryIsKeptOnlyWhereTMXTagsTheListing(t *testing.T) {
	data := jsonMap(t, `{"data": {"news": [
		{"newsid": "1", "headline": "Kraken Robotics: Undersea Batteries Drive Solid Revenue Growth", "source": "SeekingAlpha via QuoteMedia", "datetime": "2026-09-05T10:00:00-04:00", "topic": "[PNG:CA,TECH0001]"},
		{"newsid": "2", "headline": "Most shorted stocks on Wall Street", "source": "SeekingAlpha via QuoteMedia", "datetime": "2026-09-05T11:00:00-04:00", "topic": "[ASTS,NBIS]"}]}}`)
	rows := ParseTMXNews(data, "PNG", true)
	if len(rows) != 1 || rows[0].ID != "tmx:1" || rows[0].Kind != "story" || rows[0].Source != "SeekingAlpha" || rows[0].Via != "tmx-media" {
		t.Errorf("a story TMX tags with another listing is not this one's: %+v", rows)
	}
	wire := ParseTMXNews(jsonMap(t, `{"data": {"news": [{"newsid": "3", "headline": "Kraken closes financing", "source": "GlobeNewswire via QuoteMedia", "datetime": "2026-09-05T08:00:00-04:00"}]}}`), "PNG", false)
	if wire[0].Kind != "release" {
		t.Errorf("the press releases tab reads as it always did: %+v", wire)
	}
}

func TestBothOfTMXsTabsAreReadAndAFailingStoriesTabKeepsTheReleases(t *testing.T) {
	release := `{"newsid": "10", "headline": "Kraken closes financing", "source": "GlobeNewswire via QuoteMedia", "datetime": "2026-09-05T08:00:00-04:00", "topic": "[PNG:CA]"}`
	story := `{"newsid": "11", "headline": "3 Top Canadian Defence Stocks", "source": "Motley Fool Canada via QuoteMedia", "datetime": "2026-09-02T09:00:00-04:00", "topic": "[PNG:CA,DEFENCE1]"}`
	now := time.Date(2026, 9, 16, 12, 0, 0, 0, time.UTC)
	tabs := []bool{}
	failStories := false
	c := newClient(t, now, func(r *http.Request) (*http.Response, error) {
		q := readGraphQL(r)
		if q.OperationName != "getNewsForSymbol" {
			return reply(200, tmxQuote("PNG", "TSX Venture Exchange")), nil
		}
		media, _ := q.Variables["companyInNews"].(bool)
		tabs = append(tabs, media)
		if media && failStories {
			return reply(500, "down"), nil
		}
		if media {
			return reply(200, tmxNews(story)), nil
		}
		return reply(200, tmxNews(release)), nil
	})
	src, rows, _ := FetchSymbol(c, "PNG", "TSX-V", "CAD", now)
	got := [][2]string{}
	for _, r := range rows {
		got = append(got, [2]string{r.ID, r.Kind})
	}
	sort.Slice(got, func(i, j int) bool { return got[i][0] < got[j][0] })
	if src != "tmx" || !reflect.DeepEqual(tabs, []bool{false, true}) || !reflect.DeepEqual(got, [][2]string{{"tmx:10", "release"}, {"tmx:11", "story"}}) {
		t.Errorf("src %q tabs %v rows %v", src, tabs, got)
	}
	failStories = true
	_, rows, _ = FetchSymbol(c, "PNG", "TSX-V", "CAD", now)
	if !reflect.DeepEqual(ids(rows), []string{"tmx:10"}) {
		t.Errorf("the releases still arrive when the stories tab fails: %v", ids(rows))
	}
}

func yahooAsset(uuid, title string, tickers []string, provider, when string) string {
	tags := []string{}
	for _, t := range tickers {
		tags = append(tags, fmt.Sprintf(`{"symbol": %q}`, t))
	}
	return fmt.Sprintf(`{"node": {"asset": {"id": %q, "title": %q, "contentAttributes": {"pubDate": %q, "provider": {"displayName": %q}, "canonicalUrl": "https://finance.yahoo.com/news/%s"}, "finance": {"stockTickers": [%s]}}}}`, uuid, title, when, provider, uuid, strings.Join(tags, ","))
}

func yahooList(assets ...string) string {
	return `{"data": {"lightyearList": {"main": {"edges": [` + strings.Join(assets, ",") + `]}}}}`
}

func TestYahooKeepsWhatItsTickerTagsName(t *testing.T) {
	data := jsonMap(t, yahooList(
		yahooAsset("a1", "Kraken Robotics Announces Q2 Results", []string{"PNG.V", "KRKNF"}, "Newsfile", "2026-09-14T13:13:00Z"),
		yahooAsset("a2", "3 Defence Stocks To Watch", []string{"LMT", "RTX"}, "Motley Fool", "2026-09-14T13:13:00Z"),
		yahooAsset("a3", "Kraken Wins Navy Contract", []string{"PNG.V"}, "The Globe and Mail", "2026-09-15T10:00:00.000Z"),
		yahooAsset("a4", "no date", []string{"PNG.V"}, "Newsfile", "")))
	rows := ParseYahooNews(data, "PNG.V", "", "")
	got := [][4]string{}
	for _, r := range rows {
		got = append(got, [4]string{r.ID, r.Kind, r.Source, r.PublishedAt})
	}
	want := [][4]string{{"yahoo:a1", "release", "Newsfile", "2026-09-14T13:13:00Z"}, {"yahoo:a3", "story", "The Globe and Mail", "2026-09-15T10:00:00Z"}}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("an item Yahoo tags with other tickers is theirs; a wire's item is a release: %v, want %v", got, want)
	}
	c := quiet(t)
	c.Store.SetMeta("tmx_form:QIMC", "@:CNX")
	forms := []string{}
	for _, x := range [][3]string{{"PNG", "TSX-V", "CAD"}, {"HG", "CSE", "CAD"}, {"HBIX", "Cboe Canada", "CAD"}, {"ASTS", "NASDAQ", "USD"}, {"LUNR", "NASDAQ", ""}, {"QIMC", "", "CAD"}, {"VEQT", "", "CAD"}, {"F", "", ""}} {
		forms = append(forms, YahooForm(c, x[0], x[1], x[2]))
	}
	if want := []string{"PNG.V", "HG.CN", "HBIX.NE", "ASTS", "LUNR", "QIMC.CN", "VEQT.TO", ""}; !reflect.DeepEqual(forms, want) {
		t.Errorf("the venue decides before the currency: %v, want %v", forms, want)
	}
}

func TestYahooKeepsACanadianCompanysItemsTaggedWithItsUSTwin(t *testing.T) {
	asset := func(uuid, title string, tickers []string) string {
		return yahooAsset(uuid, title, tickers, "PR Newswire", "2026-09-14T13:13:00Z")
	}
	data := jsonMap(t, yahooList(
		asset("a1", "CHARBONE Announces Closing of $1.5M Drawdown", []string{"CH.V", "CHHYF"}),
		asset("a2", "Charbone Announces Its First Hydrogen Supply Hub", []string{"CHHYF"}),
		asset("a3", "ESGFIRE Initiates Coverage on Charbone Corporation", []string{"CHHYF", "PLUG", "FCEL"}),
		asset("a4", "Presenting on Emerging Growth Conference 90 Day 1", []string{"ASPI", "IBX.AX", "STLNF"}),
		asset("a5", "CHARBONE to Present at the Hydrogen East Conference", []string{}),
		asset("a6", "Hydrogen prices climb", []string{})))
	if got := ids(ParseYahooNews(data, "CH.V", "CH", "Charbone Hydrogen Corp")); !reflect.DeepEqual(got, []string{"yahoo:a1", "yahoo:a2", "yahoo:a3", "yahoo:a5"}) {
		t.Errorf("the twin Yahoo tags beside the listing names it; an untagged item counts where its headline names the listing: %v", got)
	}
	partner := jsonMap(t, yahooList(
		asset("p1", "Kraken and Saab sign sonar partnership", []string{"PNG.V", "KRKNF", "SAABF"}),
		asset("p2", "Kraken Robotics orders", []string{"PNG.V", "KRKNF"}),
		asset("p3", "Saab raises its outlook", []string{"SAABF"})))
	if got := ids(ParseYahooNews(partner, "PNG.V", "PNG", "Kraken Robotics Inc.")); !reflect.DeepEqual(got, []string{"yahoo:p1", "yahoo:p2"}) {
		t.Errorf("a partner's symbol on an item naming several is not the listing's twin: %v", got)
	}
	us := jsonMap(t, yahooList(asset("b1", "Palantir wins Army deal", []string{"PLTR"}), asset("b2", "Kraken Robotics orders", []string{"KRKNF"})))
	if got := ids(ParseYahooNews(us, "PLTR", "PLTR", "Palantir Technologies Inc")); !reflect.DeepEqual(got, []string{"yahoo:b1"}) {
		t.Errorf("a US listing has no twin: %v", got)
	}
}

func TestSeekingAlphaKeepsWhatItsSymbolTagsName(t *testing.T) {
	xml := `<rss><channel>
	  <item><title>Kraken Robotics: Undersea Batteries Drive Growth</title><link>https://seekingalpha.com/article/1</link>
	    <guid isPermaLink="false">Article:1</guid><pubDate>Fri, 05 Sep 2026 10:00:00 -0400</pubDate><sa:symbol>PNG:CA</sa:symbol><sa:symbol>KRKNF</sa:symbol></item>
	  <item><title>Most shorted stocks</title><link>https://seekingalpha.com/news/2</link><guid>MarketCurrent:2</guid>
	    <pubDate>Fri, 05 Sep 2026 11:00:00 -0400</pubDate><sa:symbol>ASTS</sa:symbol></item>
	</channel></rss>`
	rows := ParseSANews(xml, "PNG:CA")
	if len(rows) != 1 || rows[0].Headline != "Kraken Robotics: Undersea Batteries Drive Growth" || rows[0].Source != "Seeking Alpha" || rows[0].URL != "https://seekingalpha.com/article/1" || rows[0].PublishedAt != "2026-09-05T14:00:00Z" {
		t.Errorf("rows = %+v", rows)
	}
	forms := []string{}
	for _, x := range [][3]string{{"PNG", "TSX-V", "CAD"}, {"VEQT", "TSX", "CAD"}, {"ASTS", "NASDAQ", "USD"}, {"HG", "CSE", "CAD"}, {"HBIX", "Cboe Canada", "CAD"}} {
		forms = append(forms, SAForm(x[0], x[1], x[2]))
	}
	if want := []string{"PNG:CA", "VEQT:CA", "ASTS", "", ""}; !reflect.DeepEqual(forms, want) {
		t.Errorf("Seeking Alpha has no form for the CSE or Cboe Canada: %v", forms)
	}
}

func TestANameIsSearchedAsThePressWritesIt(t *testing.T) {
	cases := map[string]string{
		"Harvest Reddit Enhanced High Income Shares ETF (the “ETF”)": "Harvest Reddit Enhanced High Income Shares ETF",
		"Harvest Diversified High Income Shares ETF - Class A":       "Harvest Diversified High Income Shares ETF",
		"Ninepoint Partners LP - Cameco Highshares ETF":              "Ninepoint Cameco Highshares ETF",
		"Vanguard All-Equity ETF Portfolio - ETF":                    "Vanguard All-Equity ETF Portfolio",
		"Palantir Technologies Inc (Class A)":                        "Palantir Technologies",
		"Nebius Group N.V. Class A":                                  "Nebius",
		"Micron Technology, Inc.":                                    "Micron Technology",
		"Charbone Hydrogen Corp":                                     "Charbone Hydrogen",
		"":                                                           "",
	}
	for raw, want := range cases {
		if got := SearchName(raw); got != want {
			t.Errorf("SearchName(%q) = %q, want %q", raw, got, want)
		}
	}
	queries := [][]string{GoogleQueries("HG", "CSE", "CAD", "Hydrograph Clean Power Inc."), GoogleQueries("CH", "TSX-V", "CAD", "Charbone Hydrogen Corp"), GoogleQueries("HBIX", "Cboe Canada", "CAD", ""), GoogleQueries("MU", "NASDAQ", "USD", "Micron Technology, Inc.")}
	want := [][]string{{`"Hydrograph Clean Power"`, `"CSE:HG"`}, {`"Charbone Hydrogen"`, `"TSXV:CH"`}, {`"NEO:HBIX"`}, {`"Micron Technology"`, `"NASDAQ:MU"`}}
	if !reflect.DeepEqual(queries, want) {
		t.Errorf("queries = %v, want %v", queries, want)
	}
}

func TestGoogleKeepsAHeadlineOnlyWhereItNamesTheListing(t *testing.T) {
	type head struct {
		text string
		want bool
	}
	cases := []struct {
		sym, name string
		us        bool
		heads     []head
	}{
		{"PLTE", "Harvest Palantir Enhanced High Income Shares ETF - Class A", false, []head{
			{"(PLTE) Equity Market Report (PLTE:CA)", true},
			{"Canadian ETF Express | Harvest Palantir Enhanced High Income Shares ETF Was the Top Gainer, Rising 32.29%", true},
			{"Harvest High Income Shares ETFs Announces August 2026 Distributions", true},
			{"The Ultimate Investor Guide to High-Income TSX ETFs Generating Monthly Cash Flow", false},
			{"Canadian ETF Express | GLOBAL X INVESTMENTS CANADA INC. BETAPRO NATURAL GAS LEVERAGED DAILY BULL Was the Top Gainer, Rising 3.65%", false}}},
		{"CH", "Charbone Hydrogen Corp", false, []head{
			{"CHARBONE Announces Change of Corporate Name and Registered Address", true},
			{"Charbone Reports Q2 2026 Financial Results, Confirming 155% Gas Income Growth", true},
			{"Boeing Announces Second Quarter Deliveries", false}}},
		{"HG", "Hydrograph Clean Power Inc.", false, []head{
			{"HydroGraph Announces Change of Auditor", true},
			{"Is HydroGraph Clean Power (CNSX:HG) Fully Valued After Wider Losses And Fresh Funding?", true},
			{"HydroGraph Clean Power (HG.C): A year ago this thing looked insane. Then it got bigger.", true},
			{"Widespread intensification of global river hydrograph flashiness under climate change", false}}},
		{"YES", "Char Technologies Ltd.", false, []head{
			{"CHAR Tech Receives Patent Notice of Allowance for Pyrogas Treatment to Syngas", true},
			{"CHAR Technologies Ltd. (CVE:YES): Are Analysts Optimistic?", true},
			{"UW Works with Wyoming DEQ-AML, UR Energy on Soil Reclamation Project Using Coal Char", false},
			{"Canada’s Energy Trade Is Alive Again — Why NG Energy International Corp (TSXV:GASX) Matters Now", false}}},
		{"QIMC", "Quebec Innovative Materials Corp", false, []head{
			{"Québec Innovative Materials Corp. Engages Echo Seismic and Strum Consulting", true},
			{"QIMC launches 78-km natural hydrogen survey", true},
			{"The hunt is on for natural 'white' hydrogen in Nova Scotia’s underground", false}}},
		{"SXHI", "Ninepoint SpaceX HighShares ETF", false, []head{
			{"SXHI: SpaceX High-Income ETF's 9.17% Screener Yield Puts Private-Space Exposure in the Spotlight", true},
			{"Ninepoint Partners Announces June 2026 Cash Distributions", true},
			{"Retail investors can now buy Canadian and US IPOs at offering price", false}}},
		{"EASY", "Evolve All-in-One UltraYield ETF", false, []head{
			{"EASY WAYS TO RETIRE EARLY", false},
			{"Evolve Sets September 2026 Distributions Across UltraYield ETFs and Income Funds", true},
			{"How Canada's ETF Industry Continues to Evolve", false}}},
		{"QNC", "Quantum Emotion Corp", false, []head{
			{"Quantum eMotion Submits Quantum Entropy Source for NIST Validation", true},
			{"$Xanadu Quantum Technologies (XNDU.US)$", false},
			{"Why investors are watching Quantum stocks", false}}},
		{"HG", "Hydrograph Clean Power Inc.", false, []head{
			{"MDI joins HydroGraph partner network", true},
			{"Sparc reports positive results using HydroGraph's Fractal Graphene in solvent-based coatings", true}}},
		{"CH", "Charbone Hydrogen Corp", false, []head{
			{"The Supply Gap No One Is Filling: How CHARBONE Is Building the UHP Industrial Gas Platform", true},
			{"Why Charbone shares jumped 30%", true}}},
		{"VEQT", "Vanguard All-Equity ETF Portfolio - ETF", false, []head{
			{"Vanguard Investments Canada Announces Final 2025 Annual Capital Gains Distributions for the Vanguard ETFs", true},
			{"15 cheap, but well-rated ETFs", false},
			{"No Time to Invest? Buy Any of These 3 Vanguard ETF Portfolios to Be Set for Life", false}}},
		{"NA", "National Bank of Canada", false, []head{
			{"National Bank of Canada Reports Record Quarter", true},
			{"National Bank Financial raises its target on Cameco", true},
			{"National Bank of Greece posts record profit", false}}},
		{"RY", "Royal Bank of Canada", false, []head{{"Royal Bank of Scotland to cut jobs", false}}},
		{"HHIS", "Harvest Diversified High Income Shares ETF - Class A", false, []head{
			{"Investors Rush to Harvest Tax Losses Before Year End", false},
			{"Harvest ETFs Announces August 2026 Distributions", true}}},
		{"BMO", "Bank of Montreal", false, []head{
			{"Bank of Montreal Reports Third Quarter Results", true},
			{"Bank of Canada holds rates steady", false}}},
		{"CNQ", "Canadian Natural Resources Limited", false, []head{
			{"Canadian Natural Resources to buy oil sands assets", true},
			{"Canadian dollar weakens as oil slides", false},
			{"Canadian natural gas prices slump", false},
			{"Canadian stocks close higher", false}}},
		{"HXS", "Global X S&P 500 Index Corporate Class ETF", false, []head{{"Global stocks slide on rate fears", false}}},
		{"CH", "", true, []head{{"Chile ETF (CH) hits a new high", true}, {"NYSE:CH moves", true}, {"TSXV:CH moves", false}}},
	}
	for _, c := range cases {
		for _, h := range c.heads {
			if got := NamesListing(h.text, c.sym, c.name, c.us); got != h.want {
				t.Errorf("%s: %q = %v, want %v", c.sym, h.text, got, h.want)
			}
		}
	}
}

func TestGoogleItemsLoseThePublisherSuffixQuotePagesAndUndatedPages(t *testing.T) {
	item := func(title, source, when, link string) string {
		return fmt.Sprintf(`<item><title>%s</title><link>%s</link><pubDate>%s</pubDate><source url="x">%s</source></item>`, title, link, when, source)
	}
	xml := "<rss><channel>" + strings.Join([]string{
		item("HydroGraph Announces Change of Auditor - Investing News Network", "Investing News Network", "Mon, 31 Aug 2026 12:00:00 GMT", "https://news.google.com/rss/articles/a"),
		item("HG Stock Price and Chart — CSE:HG - tradingview.com", "tradingview.com", "Thu, 01 Jan 1970 00:00:00 GMT", "https://news.google.com/rss/articles/b"),
		item("HydroGraph Clean Power Stock Price, News, Quote &amp; History - Investing News Network", "Investing News Network", "Tue, 13 Jan 2026 00:00:00 GMT", "https://news.google.com/rss/articles/c"),
		item("Hydrograph Clean Power Inc Revenue Breakdown – CSE:HG - tradingview.com", "tradingview.com", "Fri, 31 Jul 2026 00:00:00 GMT", "https://news.google.com/rss/articles/d"),
		item("HG Forecast — Price Target — Prediction for 2027 - TradingView", "TradingView", "Fri, 31 Jul 2026 00:00:00 GMT", "https://news.google.com/rss/articles/e"),
		item("What is HydroGraph Clean Power rStock | How RHG Works - MEXC", "MEXC", "Fri, 31 Jul 2026 00:00:00 GMT", "https://news.google.com/rss/articles/f"),
		item("$HydroGraph Clean Power (HGRAF.US)$ - Moomoo", "Moomoo", "Fri, 31 Jul 2026 00:00:00 GMT", "https://news.google.com/rss/articles/g"),
	}, "") + "</channel></rss>"
	rows := ParseGoogleNews(xml, "HG", "Hydrograph Clean Power Inc.", false)
	if len(rows) != 1 || rows[0].Headline != "HydroGraph Announces Change of Auditor" || rows[0].Source != "Investing News Network" || rows[0].PublishedAt != "2026-08-31T12:00:00Z" || rows[0].Kind != "story" || !strings.HasPrefix(rows[0].ID, "gnews:") {
		t.Errorf("rows = %+v", rows)
	}
}

func TestEverySourceIsMergedOneRowPerStoryTheWiresCopyFirst(t *testing.T) {
	c := quiet(t)
	now := time.Date(2026, 9, 16, 12, 0, 0, 0, time.UTC)
	stubWire(t, "tmx", []store.WireItem{row("tmx:1", "Charbone Closes Loan", "2026-09-08T12:00:00Z", "TheNewsWire", "release")}, true)
	answers := map[string][]store.WireItem{
		"yahoo": {row("yahoo:u1", "CHARBONE closes loan.", "2026-09-08T12:00:00Z", "TheNewsWire", "release"), row("yahoo:u2", "Charbone delivers electrolyzer", "2026-09-09T12:00:00Z", "BNN Bloomberg", "story")},
		"sa":    {row("sa:1", "Charbone: a hydrogen story", "2026-09-10T12:00:00Z", "Seeking Alpha", "story")},
		"gnews": {row("gnews:1", "Charbone delivers electrolyzer", "2026-09-09T12:05:00Z", "The Globe and Mail", "story"), row("gnews:2", "Charbone Reports Q2 2026 Financial Results", "2026-08-27T12:00:00Z", "The Globe and Mail", "story")},
	}
	asked := []string{}
	stubExtra(t, func(key, symbol, exchange, currency, name string) ([]store.WireItem, bool, error) {
		testMu.Lock()
		asked = append(asked, key+"|"+name)
		testMu.Unlock()
		return answers[key], true, nil
	})
	src, rows, ok := ReadListing(c, "CH", "TSX-V", "CAD", now, "Charbone Hydrogen Corp", false, nil)
	sort.Strings(asked)
	if !ok || src != "tmx" || !reflect.DeepEqual(asked, []string{"gnews|Charbone Hydrogen Corp", "sa|Charbone Hydrogen Corp", "yahoo|Charbone Hydrogen Corp"}) {
		t.Errorf("src %q ok %v asked %v", src, ok, asked)
	}
	if got := ids(rows); !reflect.DeepEqual(got, []string{"sa:1", "yahoo:u2", "tmx:1", "gnews:2"}) {
		t.Errorf("newest first; the same headline from a later source is the earlier source's row: %v", got)
	}
	stored := [][3]string{}
	for _, r := range c.Store.NewsFor("CH", "TSX-V") {
		stored = append(stored, [3]string{r.ID, r.Source, r.Wire})
	}
	if want := [][3]string{{"sa:1", "sa", "Seeking Alpha"}, {"yahoo:u2", "yahoo", "BNN Bloomberg"}, {"tmx:1", "tmx", "TheNewsWire"}, {"gnews:2", "gnews", "The Globe and Mail"}}; !reflect.DeepEqual(stored, want) {
		t.Errorf("each row is stored under the source it was read from: %v, want %v", stored, want)
	}
}

func TestASourceThatFailsOrIsNotDueKeepsItsStoredStories(t *testing.T) {
	c := quiet(t)
	now := time.Date(2026, 9, 16, 12, 0, 0, 0, time.UTC)
	first := map[string][]store.WireItem{"yahoo": {row("yahoo:u1", "Yahoo story", "2026-09-10T12:00:00Z", "Pub", "story")}, "sa": {row("sa:1", "SA story", "2026-09-11T12:00:00Z", "Pub", "story")}, "gnews": {row("gnews:1", "Google story", "2026-09-12T12:00:00Z", "Pub", "story")}}
	stubWire(t, "tmx", []store.WireItem{row("tmx:1", "Wire item", "2026-09-09T12:00:00Z", "Pub", "story")}, true)
	stubExtra(t, func(key, symbol, exchange, currency, name string) ([]store.WireItem, bool, error) {
		return first[key], true, nil
	})
	ReadListing(c, "CH", "TSX-V", "CAD", now, "", false, nil)
	asked := []string{}
	stubWire(t, "tmx", []store.WireItem{row("tmx:2", "New wire item", "2026-09-16T12:10:00Z", "Pub", "story")}, true)
	stubExtra(t, func(key, symbol, exchange, currency, name string) ([]store.WireItem, bool, error) {
		testMu.Lock()
		asked = append(asked, key)
		testMu.Unlock()
		if key == "yahoo" {
			return nil, true, fmt.Errorf("down")
		}
		return []store.WireItem{}, true, nil
	})
	_, rows, _ := ReadListing(c, "CH", "TSX-V", "CAD", now.Add(16*time.Minute), "", false, nil)
	if !reflect.DeepEqual(asked, []string{"yahoo"}) {
		t.Errorf("Seeking Alpha and Google are read every thirty minutes: asked %v", asked)
	}
	if got := ids(rows); !reflect.DeepEqual(got, []string{"tmx:2", "gnews:1", "sa:1", "yahoo:u1"}) {
		t.Errorf("the wire's item is replaced; the failing and the resting sources keep what they had: %v", got)
	}
	stubWire(t, "tmx", nil, false)
	stubExtra(t, func(key, symbol, exchange, currency, name string) ([]store.WireItem, bool, error) {
		return nil, true, fmt.Errorf("down")
	})
	if _, rows, ok := ReadListing(c, "CH", "TSX-V", "CAD", now.Add(45*time.Minute), "", true, nil); ok || rows != nil {
		t.Errorf("nothing answers at all: the stored list stands untouched, got ok=%v rows=%v", ok, rows)
	}
	if got := ids(wireItems(c.Store.NewsFor("CH", "TSX-V"))); !reflect.DeepEqual(got, []string{"tmx:2", "gnews:1", "sa:1", "yahoo:u1"}) {
		t.Errorf("stored = %v", got)
	}
	stubWire(t, "tmx", []store.WireItem{row("tmx:2", "New wire item", "2026-09-16T12:10:00Z", "Pub", "story")}, true)
	stubExtra(t, func(key, symbol, exchange, currency, name string) ([]store.WireItem, bool, error) {
		if key == "yahoo" {
			return []store.WireItem{}, true, nil
		}
		return first[key], true, nil
	})
	_, rows, _ = ReadListing(c, "CH", "TSX-V", "CAD", now.Add(60*time.Minute), "", true, nil)
	found := false
	for _, r := range rows {
		if r.ID == "yahoo:u1" {
			found = true
		}
	}
	if !found {
		t.Errorf("a list a source had does not vanish on one empty answer: %v", ids(rows))
	}
}

func wireItems(items []store.NewsItem) []store.WireItem {
	out := []store.WireItem{}
	for _, r := range items {
		out = append(out, store.WireItem{ID: r.ID, Headline: r.Headline, Source: r.Wire, URL: r.URL, PublishedAt: r.PublishedAt, Kind: r.Kind, Via: r.Source})
	}
	return out
}

func TestAReleaseIsNewOnceWhicheverSourceCarriesItAndAFirstReadSourceIsHistory(t *testing.T) {
	c := quiet(t)
	now := time.Date(2026, 9, 16, 12, 0, 0, 0, time.UTC)
	told := [][]string{}
	onNew := func(sym, ex string, rows []store.WireItem, fresh map[string]bool) {
		got := []string{}
		for id := range fresh {
			got = append(got, id)
		}
		sort.Strings(got)
		told = append(told, got)
	}
	release := func(id, when string) store.WireItem {
		return row(id, "Charbone Closes Loan", when, "TheNewsWire", "release")
	}
	stubWire(t, "tmx", []store.WireItem{row("tmx:1", "Old wire item", "2026-09-01T12:00:00Z", "Pub", "story")}, true)
	stubExtra(t, func(key, symbol, exchange, currency, name string) ([]store.WireItem, bool, error) {
		if key == "yahoo" {
			return []store.WireItem{row("yahoo:old", "Charbone old release", "2026-09-10T12:00:00Z", "NewMediaWire", "release")}, true, nil
		}
		return nil, false, nil
	})
	ReadListing(c, "CH", "TSX-V", "CAD", now, "", false, onNew)
	if !reflect.DeepEqual(told[len(told)-1], []string{"tmx:1"}) {
		t.Errorf("a source met for the first time brings history, not news: %v", told)
	}
	stubWire(t, "tmx", nil, false)
	stubExtra(t, func(key, symbol, exchange, currency, name string) ([]store.WireItem, bool, error) {
		if key == "yahoo" {
			return []store.WireItem{release("yahoo:new", "2026-09-16T11:00:00Z")}, true, nil
		}
		return nil, false, nil
	})
	ReadListing(c, "CH", "TSX-V", "CAD", now.Add(16*time.Minute), "", false, onNew)
	if !reflect.DeepEqual(told[len(told)-1], []string{"yahoo:new"}) {
		t.Errorf("told = %v", told)
	}
	stubWire(t, "tmx", []store.WireItem{release("tmx:999", "2026-09-16T11:00:00Z")}, true)
	_, rows, _ := ReadListing(c, "CH", "TSX-V", "CAD", now.Add(32*time.Minute), "", false, onNew)
	has := false
	for _, r := range rows {
		if r.ID == "tmx:999" {
			has = true
		}
	}
	if !has || len(told[len(told)-1]) != 0 {
		t.Errorf("the wire's copy is the row and the same headline under the wire's id is the story already told: rows %v told %v", ids(rows), told)
	}
}

func TestAHeadlineRepeatedMonthsLaterIsANewRelease(t *testing.T) {
	c := quiet(t)
	now := time.Date(2026, 9, 16, 12, 0, 0, 0, time.UTC)
	halt := func(id, when string) store.WireItem {
		return row(id, "IIROC Trading Halt - QNC", when, "TMX Newsfile", "release")
	}
	stubExtra(t, func(key, symbol, exchange, currency, name string) ([]store.WireItem, bool, error) {
		return nil, false, nil
	})
	stubWire(t, "tmx", []store.WireItem{halt("tmx:1", "2026-06-10T14:00:00Z")}, true)
	ReadListing(c, "QNC", "TSX-V", "CAD", now, "", false, nil)
	told := [][]string{}
	stubWire(t, "tmx", []store.WireItem{halt("tmx:2", "2026-09-16T13:00:00Z"), halt("tmx:1", "2026-06-10T14:00:00Z")}, true)
	_, rows, _ := ReadListing(c, "QNC", "TSX-V", "CAD", now.Add(16*time.Minute), "", false, func(s, e string, r []store.WireItem, fresh map[string]bool) {
		got := []string{}
		for id := range fresh {
			got = append(got, id)
		}
		sort.Strings(got)
		told = append(told, got)
	})
	if !reflect.DeepEqual(ids(rows), []string{"tmx:2", "tmx:1"}) || !reflect.DeepEqual(told, [][]string{{"tmx:2"}}) {
		t.Errorf("the same words months apart are two halts, the second one new: %v %v", ids(rows), told)
	}
}

func TestAWireFeedThatFailsKeepsItsStoredItems(t *testing.T) {
	now := time.Date(2026, 9, 16, 12, 0, 0, 0, time.UTC)
	release := `{"newsid": "10", "headline": "Kraken closes financing", "source": "GlobeNewswire via QuoteMedia", "datetime": "2026-09-05T08:00:00-04:00"}`
	story := `{"newsid": "11", "headline": "3 Top Canadian Defence Stocks", "source": "Motley Fool Canada via QuoteMedia", "datetime": "2026-09-02T09:00:00-04:00", "topic": "[PNG:CA]"}`
	storiesDown := false
	c := newClient(t, now, func(r *http.Request) (*http.Response, error) {
		q := readGraphQL(r)
		if q.OperationName != "getNewsForSymbol" {
			return reply(200, tmxQuote("PNG", "TSX Venture Exchange")), nil
		}
		if media, _ := q.Variables["companyInNews"].(bool); media {
			if storiesDown {
				return reply(500, "down"), nil
			}
			return reply(200, tmxNews(story)), nil
		}
		return reply(200, tmxNews(release)), nil
	})
	stubExtra(t, func(key, symbol, exchange, currency, name string) ([]store.WireItem, bool, error) {
		return nil, false, nil
	})
	ReadListing(c, "PNG", "TSX-V", "CAD", now, "", false, nil)
	storiesDown = true
	_, rows, _ := ReadListing(c, "PNG", "TSX-V", "CAD", now.Add(16*time.Minute), "", false, nil)
	got := ids(rows)
	sort.Strings(got)
	if !reflect.DeepEqual(got, []string{"tmx:10", "tmx:11"}) {
		t.Errorf("In The Media failing leaves the stories it had: %v", got)
	}
	via := map[string]string{}
	for _, r := range c.Store.NewsFor("PNG", "TSX-V") {
		via[r.ID] = r.Source
	}
	if !reflect.DeepEqual(via, map[string]string{"tmx:10": "tmx", "tmx:11": "tmx-media"}) {
		t.Errorf("via = %v", via)
	}
}

func TestASourceWithNothingToAskDoesNotCountAsAnAnswer(t *testing.T) {
	c := quiet(t)
	gets := 0
	c.HTTP.Transport = stubTransport(func(r *http.Request) (*http.Response, error) {
		if r.Method == http.MethodGet {
			gets++
		}
		return reply(500, "down"), nil
	})
	stubWire(t, "tmx", nil, false)
	if _, rows, ok := ReadListing(c, "HG", "CSE", "CAD", time.Date(2026, 9, 16, 12, 0, 0, 0, time.UTC), "Hydrograph Clean Power Inc.", true, nil); ok || rows != nil {
		t.Errorf("nothing answered, so the listing is asked again next pass: ok=%v", ok)
	}
	if got, asked, _ := fetchSA(c, "HG", "CSE", "CAD"); got != nil || asked {
		t.Errorf("Seeking Alpha has no CSE feed: %v %v", got, asked)
	}
}

func TestAListingIsDueWhileAnyOfItsSourcesIs(t *testing.T) {
	c := quiet(t)
	now := time.Date(2026, 9, 16, 12, 0, 0, 0, time.UTC)
	c.Store.ReplaceNews("CH", "TSX-V", "tmx", []store.WireItem{row("tmx:1", "Wire item", "2026-09-16T11:00:00Z", "Pub", "story")}, stamp(now))
	c.Store.ReplaceNews("HG", "CSE", "tmx", []store.WireItem{row("tmx:2", "Wire item", "2026-09-16T11:00:00Z", "Pub", "story")}, stamp(now))
	listings := []Listing{{"CH", "TSX-V", "CAD", "Charbone Hydrogen Corp"}, {"HG", "CSE", "CAD", "Hydrograph Clean Power Inc."}}
	syms := func(ls []Listing) []string {
		out := []string{}
		for _, l := range ls {
			out = append(out, l.Symbol)
		}
		return out
	}
	if got := syms(Stale(c, listings, now.Add(time.Minute), FreshMinutes)); !reflect.DeepEqual(got, []string{"CH", "HG"}) {
		t.Errorf("the wire is fresh, the other sources never read: %v", got)
	}
	for _, s := range SourcesFor(c, "HG", "CSE", "CAD", "Hydrograph Clean Power Inc.") {
		if s == "sa" {
			t.Error("Seeking Alpha has no CSE feed")
		}
	}
	stubWire(t, "tmx", []store.WireItem{}, true)
	stubExtra(t, func(key, symbol, exchange, currency, name string) ([]store.WireItem, bool, error) {
		return []store.WireItem{}, true, nil
	})
	started, landed := []string{}, []string{}
	n := Refresh(c, listings, now.Add(time.Minute), nil, func(due []Listing) {
		testMu.Lock()
		started = append(started, syms(due)...)
		testMu.Unlock()
	}, func(l Listing, ok bool) {
		testMu.Lock()
		landed = append(landed, fmt.Sprintf("%s:%v", l.Symbol, ok))
		testMu.Unlock()
	})
	sort.Strings(started)
	sort.Strings(landed)
	if n != 2 || !reflect.DeepEqual(started, []string{"CH", "HG"}) || !reflect.DeepEqual(landed, []string{"CH:true", "HG:true"}) {
		t.Errorf("n %d started %v landed %v", n, started, landed)
	}
	if got := Stale(c, listings, now.Add(2*time.Minute), FreshMinutes); len(got) != 0 {
		t.Errorf("every source read: nothing due, a source with nothing to ask included: %v", got)
	}
	if got := syms(Stale(c, listings, now.Add(17*time.Minute), FreshMinutes)); !reflect.DeepEqual(got, []string{"CH", "HG"}) {
		t.Errorf("stale = %v", got)
	}
}

func TestAListingWithNoVenueIsLeftToTheWire(t *testing.T) {
	c := quiet(t)
	stubWire(t, "nasdaq", []store.WireItem{}, true)
	called := false
	stubExtra(t, func(key, symbol, exchange, currency, name string) ([]store.WireItem, bool, error) {
		called = true
		return nil, false, nil
	})
	ReadListing(c, "F", "", "", time.Date(2026, 9, 16, 12, 0, 0, 0, time.UTC), "", false, nil)
	if called {
		t.Error("a listing with no venue is left to the wire")
	}
}
