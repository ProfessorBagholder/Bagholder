package disclosures

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"regexp"
	"slices"
	"sort"
	"strconv"
	"strings"
	"sync"

	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

const (
	EdgarSource      = "SEC"
	TickersURL       = "https://www.sec.gov/files/company_tickers.json"
	SubmissionsURL   = "https://data.sec.gov/submissions/CIK%s.json"
	ArchiveURL       = "https://www.sec.gov/Archives/edgar/data/%d/%s/%s"
	EdgarTimeout     = 30
	EdgarPaceSeconds = 0.3
	EdgarFetchLimit  = 200
)

var EdgarUS = map[string]bool{"NASDAQ": true, "NYSE": true, "NYSEARCA": true, "NYSEAMERICAN": true, "AMEX": true, "ARCA": true, "BATS": true, "US": true, "OTC": true, "OTCMKTS": true, "CBOE": true}

func EdgarUA() string {
	if v := os.Getenv("BAGHOLDER_SEC_UA"); v != "" {
		return v
	}
	return "Bagholder/1.0 (filings admin@bagholder.app)"
}

type cikTitle struct {
	cik   int
	title string
}

type Edgar struct {
	Market  *market.Client
	pacer   *market.Pacer
	mu      sync.Mutex
	tickers map[string]cikTitle
}

func NewEdgar(m *market.Client) *Edgar {
	return &Edgar{Market: m, pacer: market.NewPacer()}
}

func (e *Edgar) Source() string { return EdgarSource }

func (e *Edgar) Available() bool { return true }

func (e *Edgar) get(url string) ([]byte, string, error) {
	e.pacer.Pace("sec", EdgarPaceSeconds)
	raw, ct, err := e.Market.FetchRawWithType(url, map[string]string{"User-Agent": EdgarUA(), "Accept-Encoding": "gzip, deflate", "Accept": "application/json"})
	if err != nil {
		return nil, "", Unavailable("EDGAR request failed: %s", err)
	}
	return raw, ct, nil
}

func (e *Edgar) getJSON(url string, v any) error {
	raw, _, err := e.get(url)
	if err != nil {
		return err
	}
	if err := json.Unmarshal(raw, v); err != nil {
		if t := bytes.TrimSpace(raw); len(t) > 0 && t[0] == '[' && json.Valid(t) {
			return nil
		}
		return Unavailable("EDGAR returned unreadable data: %s", err)
	}
	return nil
}

type tickerRow struct {
	CIK    py.JSONNum  `json:"cik_str"`
	Ticker py.JSONText `json:"ticker"`
	Title  py.JSONText `json:"title"`
}

func (e *Edgar) tickerMap() (map[string]cikTitle, error) {
	e.mu.Lock()
	defer e.mu.Unlock()
	if e.tickers != nil {
		return e.tickers, nil
	}
	raw, _, err := e.get(TickersURL)
	if err != nil {
		return nil, err
	}
	var rows []tickerRow
	var byKey map[string]py.JSONLoose[tickerRow]
	if err := json.Unmarshal(raw, &byKey); err != nil {
		var list []py.JSONLoose[tickerRow]
		if err2 := json.Unmarshal(raw, &list); err2 != nil {
			return nil, Unavailable("EDGAR returned unreadable data: %s", err)
		}
		for _, r := range list {
			rows = append(rows, r.V)
		}
	} else {
		keys := make([]string, 0, len(byKey))
		for k := range byKey {
			keys = append(keys, k)
		}
		sort.Slice(keys, func(i, j int) bool {
			a, ea := strconv.Atoi(keys[i])
			b, eb := strconv.Atoi(keys[j])
			if ea == nil && eb == nil {
				return a < b
			}
			return keys[i] < keys[j]
		})
		for _, k := range keys {
			rows = append(rows, byKey[k].V)
		}
	}
	out := map[string]cikTitle{}
	for _, row := range rows {
		t := strings.ToUpper(string(row.Ticker))
		if t != "" {
			out[t] = cikTitle{int(row.CIK.F), string(row.Title)}
		}
	}
	e.tickers = out
	return out, nil
}

func EdgarBare(symbol string) string {
	s := strings.ToUpper(strings.TrimSpace(symbol))
	for _, suf := range []string{".TO", ".V", ".CN", ".NE", ".U"} {
		if strings.HasSuffix(s, suf) {
			s = s[:len(s)-len(suf)]
		}
	}
	return strings.ReplaceAll(s, ".", "-")
}

func (e *Edgar) Covers(symbol, exchange, currency string) bool {
	if EdgarUS[strings.ToUpper(exchange)] || strings.ToUpper(currency) == "USD" {
		return true
	}
	m, err := e.tickerMap()
	if err != nil {
		return false
	}
	_, ok := m[EdgarBare(symbol)]
	return ok
}

var edgarTitles = map[string]string{
	"10-K": "Annual report", "10-Q": "Quarterly report", "8-K": "Current report",
	"20-F": "Annual report (foreign issuer)", "40-F": "Annual report (Canadian issuer)",
	"6-K": "Report of foreign private issuer", "DEF 14A": "Proxy statement", "DEFA14A": "Proxy soliciting material",
	"S-1": "Registration statement", "F-1": "Registration statement", "424B4": "Prospectus",
	"3": "Initial insider ownership", "4": "Insider transaction", "5": "Annual insider statement",
	"144": "Notice of proposed sale", "SC 13D": "Beneficial ownership (activist)",
	"SC 13G": "Beneficial ownership (passive)", "13F-HR": "Institutional holdings",
	"25": "Delisting notice", "425": "Business combination",
}

var fDigitRE = regexp.MustCompile(`^F-\d`)

func hasPrefixAny(s string, prefixes ...string) bool {
	for _, p := range prefixes {
		if strings.HasPrefix(s, p) {
			return true
		}
	}
	return false
}

func EdgarCategory(form string) string {
	f := strings.ToUpper(form)
	switch {
	case hasPrefixAny(f, "10-K", "10-Q", "20-F", "40-F", "6-K", "ARS", "N-CSR"):
		return Financials
	case strings.HasPrefix(f, "8-K") || f == "25" || strings.HasPrefix(f, "25-"):
		return Events
	case strings.Contains(f, "14A") || strings.Contains(f, "14C") || strings.HasPrefix(f, "DEF") || strings.HasPrefix(f, "PRE"):
		return Governance
	case hasPrefixAny(f, "S-", "424", "POS", "DRS", "EFFECT", "425") || fDigitRE.MatchString(f):
		return Offerings
	case f == "3" || f == "4" || f == "5" || f == "3/A" || f == "4/A" || f == "5/A" || f == "144" || strings.Contains(f, "13D") || strings.Contains(f, "13G") || hasPrefixAny(f, "SC 13", "SCHEDULE 13", "13F"):
		return Insider
	}
	return Other
}

func (e *Edgar) Categorize(row Row) string { return EdgarCategory(row.Type) }

func edgarTitle(form, description string) string {
	d := Clean(description)
	f := strings.ToUpper(form)
	if d != "" && strings.ToUpper(d) != f && strings.ToUpper(d) != "FORM "+f {
		return d
	}
	return edgarTitles[f]
}

type submissions struct {
	Filings struct {
		Recent struct {
			Form                  []py.JSONText `json:"form"`
			FilingDate            []py.JSONText `json:"filingDate"`
			PrimaryDocument       []py.JSONText `json:"primaryDocument"`
			AccessionNumber       []py.JSONText `json:"accessionNumber"`
			PrimaryDocDescription []py.JSONText `json:"primaryDocDescription"`
		} `json:"recent"`
	} `json:"filings"`
}

func strList(list []py.JSONText) []string {
	out := make([]string, len(list))
	for i, x := range list {
		out[i] = string(x)
	}
	return out
}

func (e *Edgar) Fetch(symbol, name, exchange, currency string, limit int, profileNo string) ([]Item, error) {
	if limit <= 0 {
		limit = EdgarFetchLimit
	}
	ticker := EdgarBare(symbol)
	m, err := e.tickerMap()
	if err != nil {
		return nil, err
	}
	ct, ok := m[ticker]
	if !ok {
		return []Item{}, nil
	}
	usListed := EdgarUS[strings.ToUpper(exchange)] || strings.ToUpper(currency) == "USD"
	if !usListed && name != "" && !NamesMatch(name, ct.title) {
		return []Item{}, nil
	}
	var sub submissions
	if err := e.getJSON(fmt.Sprintf(SubmissionsURL, fmt.Sprintf("%010d", ct.cik)), &sub); err != nil {
		return nil, err
	}
	recent := sub.Filings.Recent
	forms, dates, docs, accns := strList(recent.Form), strList(recent.FilingDate), strList(recent.PrimaryDocument), strList(recent.AccessionNumber)
	descs := strList(recent.PrimaryDocDescription)
	n := len(forms)
	if len(dates) < n {
		n = len(dates)
	}
	if len(accns) < n {
		n = len(accns)
	}
	items := []Item{}
	for i := 0; i < n; i++ {
		acc := accns[i]
		doc := ""
		if i < len(docs) {
			doc = docs[i]
		}
		url := fmt.Sprintf("https://www.sec.gov/cgi-bin/browse-edgar?action=getcompany&CIK=%d", ct.cik)
		if doc != "" {
			url = fmt.Sprintf(ArchiveURL, ct.cik, strings.ReplaceAll(acc, "-", ""), doc)
		}
		desc := ""
		if i < len(descs) {
			desc = descs[i]
		}
		items = append(items, Item{ID: "sec:" + acc, Source: EdgarSource, Category: EdgarCategory(forms[i]), Date: dates[i], DateText: dates[i], Type: forms[i], Title: edgarTitle(forms[i], desc), URL: url})
	}
	if limit < 1 {
		limit = 1
	}
	if len(items) > limit {
		items = items[:limit]
	}
	return items, nil
}

func (e *Edgar) HasFiler(symbol, name, exchange, currency string) (bool, bool) {
	m, err := e.tickerMap()
	if err != nil {
		return false, true
	}
	ct, ok := m[EdgarBare(symbol)]
	if !ok {
		return false, true
	}
	usListed := EdgarUS[strings.ToUpper(exchange)] || strings.ToUpper(currency) == "USD"
	if !usListed && name != "" && !NamesMatch(name, ct.title) {
		return false, true
	}
	return true, true
}

var skipDocRE = regexp.MustCompile(`(?i)(?:-index|-index-headers)\.(?:htm|html)$|^\d{10}-\d\d-\d{6}\.txt$|R\d+\.htm$`)
var xslRE = regexp.MustCompile(`/xsl[^/]*/`)

func xmlVals(xml, tag string) []string {
	re := py.RE(`<` + tag + `>([^<]+)</` + tag + `>`)
	var out []string
	for _, m := range re.FindAllStringSubmatch(xml, -1) {
		out = append(out, strings.TrimSpace(m[1]))
	}
	return out
}

func (e *Edgar) Enrichment(row Row) *Enrichment {
	typ := strings.ToUpper(row.Type)
	if !strings.HasPrefix(typ, "SCHEDULE 13") {
		return nil
	}
	rawURL := xslRE.ReplaceAllString(row.URL, "/")
	data, _, err := e.Document(Row{URL: rawURL})
	if err != nil {
		return nil
	}
	xml := strings.ToValidUTF8(string(data), "�")
	if !strings.Contains(xml, "reportingPersonName") {
		return nil
	}
	var owners []string
	for _, n := range xmlVals(xml, "reportingPersonName") {
		if n != "" && !slices.Contains(owners, n) {
			owners = append(owners, n)
		}
	}
	if len(owners) == 0 {
		return nil
	}
	first := func(list []string, def string) string {
		if len(list) > 0 {
			return list[0]
		}
		return def
	}
	issuer := first(xmlVals(xml, "issuerName"), "")
	pct := first(xmlVals(xml, "classPercent"), "")
	amended := strings.Contains(first(xmlVals(xml, "submissionType"), typ), "/A")
	who := owners[0]
	if len(owners) > 1 {
		who += " and affiliates"
	}
	single := len(owners) == 1
	subject := "Beneficial ownership — " + owners[0]
	if pct != "" {
		subject = pct + "% stake — " + owners[0]
	}
	verb := "report"
	if amended {
		verb = "amend their"
		if single {
			verb = "amends its"
		}
	} else if single {
		verb = "reports"
	}
	tail := " beneficial ownership"
	if amended {
		tail = " Schedule 13G report of beneficial ownership"
	}
	stake := ""
	if pct != "" {
		stake = " of " + pct + "%"
	}
	ofIssuer := ""
	if issuer != "" {
		ofIssuer = " of " + issuer
	}
	summary := who + " " + verb + tail + stake + ofIssuer + "'s common shares."
	return &Enrichment{Subject: cutRunes(subject, 90), Summary: cutRunes(summary, 240)}
}

func cutRunes(s string, n int) string {
	r := []rune(s)
	if len(r) > n {
		return string(r[:n])
	}
	return s
}

func (e *Edgar) Content(row Row) ([]byte, string, error) {
	url := row.URL
	if !strings.HasPrefix(url, "https://www.sec.gov/") {
		return e.Document(row)
	}
	idx := strings.LastIndex(url, "/")
	base, primary := url[:idx], url[idx+1:]
	var listing struct {
		Directory struct {
			Item []py.JSONLoose[struct {
				Name py.JSONText `json:"name"`
				Size py.JSONNum  `json:"size"`
			}] `json:"item"`
		} `json:"directory"`
	}
	if err := e.getJSON(base+"/index.json", &listing); err != nil {
		return e.Document(row)
	}
	type cand struct {
		name string
		size int
	}
	var cands []cand
	for _, raw := range listing.Directory.Item {
		it := raw.V
		n := string(it.Name)
		low := strings.ToLower(n)
		if !(strings.HasSuffix(low, ".htm") || strings.HasSuffix(low, ".html") || strings.HasSuffix(low, ".txt") || strings.HasSuffix(low, ".xml")) {
			continue
		}
		if strings.Contains(low, "index") || skipDocRE.MatchString(low) {
			continue
		}
		cands = append(cands, cand{n, int(it.Size.F)})
	}
	if len(cands) == 0 {
		return e.Document(row)
	}
	sort.SliceStable(cands, func(i, j int) bool {
		if cands[i].size != cands[j].size {
			return cands[i].size > cands[j].size
		}
		return cands[i].name == primary && cands[j].name != primary
	})
	best := cands[0].name
	if best == primary {
		return e.Document(row)
	}
	data, ct, err := e.Document(Row{URL: base + "/" + best})
	if err != nil {
		return e.Document(row)
	}
	return data, ct, nil
}

func (e *Edgar) Document(row Row) ([]byte, string, error) {
	url := row.URL
	if !strings.HasPrefix(url, "https://www.sec.gov/") {
		return nil, "", Unavailable("not an SEC document url")
	}
	e.pacer.Pace("sec", EdgarPaceSeconds)
	raw, ct, err := e.Market.FetchRawWithType(url, map[string]string{"User-Agent": EdgarUA(), "Accept-Encoding": "gzip, deflate"})
	if err != nil {
		return nil, "", Unavailable("EDGAR document fetch failed: %s", err)
	}
	if ct == "" {
		ct = "application/octet-stream"
	}
	return raw, ct, nil
}
