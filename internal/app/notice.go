package app

import (
	"fmt"
	"html"
	"net/url"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

// What a form is, for the forms whose code is all a row carries until a document has been read. A
// code names the form and not what happened, and "4" alone tells a holder nothing at all.
var formNames = map[string]string{
	"3": "Insider's first report (Form 3)", "4": "Insider transaction (Form 4)", "5": "Insider's annual report (Form 5)",
	"8-K": "Material event (8-K)", "6-K": "Foreign issuer report (6-K)", "10-K": "Annual report (10-K)",
	"10-Q": "Quarterly report (10-Q)", "144": "Notice of proposed sale (144)", "S-1": "Registration (S-1)",
	"SC 13D": "Beneficial ownership (13D)", "SC 13G": "Beneficial ownership (13G)", "DEF 14A": "Proxy statement (DEF 14A)",
	"424B5": "Prospectus supplement (424B5)", "FWP": "Free writing prospectus (FWP)",
}

func formName(code string) string {
	c := strings.ToUpper(py.Strip(code))
	if n, ok := formNames[c]; ok {
		return n
	}
	return py.Strip(code)
}

// A notice carries when the thing happened, as its source dates it. A release found today can have
// been published weeks ago — the app reads a listing's back catalogue the first time it sees it —
// and a notice that shows only when it was told reads as news that is not new.
type noticeRow struct {
	at, url, id, source string
}

func wireNoticeRows(rows []store.WireItem) []noticeRow {
	out := make([]noticeRow, 0, len(rows))
	for _, r := range rows {
		out = append(out, noticeRow{at: r.PublishedAt, url: r.URL, id: r.ID, source: r.Source})
	}
	return out
}

func filingNoticeRows(rows []store.Filing) []noticeRow {
	out := make([]noticeRow, 0, len(rows))
	for _, r := range rows {
		out = append(out, noticeRow{at: r.Date, url: r.URL, id: r.ID, source: r.Source})
	}
	return out
}

func newestNotice(rows []noticeRow) (noticeRow, bool) {
	if len(rows) == 0 {
		return noticeRow{}, false
	}
	sorted := append([]noticeRow{}, rows...)
	sort.SliceStable(sorted, func(i, j int) bool { return sorted[i].at > sorted[j].at })
	return sorted[0], true
}

var docSources = map[string]bool{"SEDAR+": true, "SEC": true, "SEC EDGAR": true}

// Where a notification's rows can be read, and when the newest of them happened. A filed document is
// opened through the app, which is what the Disclosures table does, so it opens the same way from
// here; anything else carries the source's own link.
func noticeExtra(sym, exchange string, rows []noticeRow) map[string]any {
	extra := map[string]any{"symbol": sym}
	if exchange != "" {
		extra["exchange"] = exchange
	}
	newest, ok := newestNotice(rows)
	if !ok {
		return extra
	}
	if at := py.Strip(newest.at); at != "" {
		extra["at"] = at
	}
	url := py.Strip(newest.url)
	if py.Strip(newest.id) != "" && docSources[py.Strip(newest.source)] {
		extra["url"] = url
		extra["doc"] = py.Strip(newest.id)
		extra["source"] = py.Strip(newest.source)
		return extra
	}
	if url != "" {
		extra["url"] = url
	}
	return extra
}

var distributionReleaseRE = regexp.MustCompile(`(?i)\b(distribution|distributions|dividend|dividends)\b`)

// A per-share amount as a release states it: `$0.1489`, trailing zeros gone below four places.
func moneyPerShare(amount float64, currency string) string {
	text := strings.TrimRight(strconv.FormatFloat(amount, 'f', 4, 64), "0")
	whole, cents, _ := strings.Cut(text, ".")
	keep := len(cents)
	if keep < 2 {
		keep = 2
	}
	cents = (cents + "00")[:keep]
	sign := "$"
	if strings.ToUpper(py.Strip(currency)) == "USD" {
		sign = "US$"
	}
	return sign + whole + "." + cents
}

// `2026-08-31` as `Aug 31`, and a year that is not this one carries it.
func stampDay(iso string) string {
	day := cut10(py.Strip(iso))
	when, err := time.Parse("2006-01-02", day)
	if err != nil {
		return ""
	}
	text := when.Format("Jan") + " " + strconv.Itoa(when.Day())
	if when.Year() != time.Now().UTC().Year() {
		text += " " + strconv.Itoa(when.Year())
	}
	return text
}

// What a distribution release means for this listing, from the issuer's own declared record: the
// amount just announced, when it goes ex and when it is paid, and the one it replaces. A release
// headline says only that distributions were announced; the figure is what the holder wants, and
// reading it from the record rather than the release's prose keeps it the same figure the Cashflow
// tab pays from.
func (a *App) distributionDetail(sym string) string {
	key := strings.ToUpper(py.Strip(sym))
	rows := append([]store.Distribution{}, a.st.Distributions()[key]...)
	if len(rows) == 0 {
		return ""
	}
	sort.SliceStable(rows, func(i, j int) bool { return rows[i].ExDate > rows[j].ExDate })
	latest := rows[0]
	out := moneyPerShare(latest.Amount, latest.Currency) + " a share"
	if freq := strings.ToLower(py.Strip(a.st.Quotes()[key].DividendFrequency)); freq != "" {
		out += ", " + freq
	}
	if when := stampDay(latest.ExDate); when != "" {
		out += " · ex " + when
	}
	if paid := stampDay(latest.PayDate); paid != "" {
		out += ", paid " + paid
	}
	for _, r := range rows[1:] {
		if r.Amount != latest.Amount {
			out += " · was " + moneyPerShare(r.Amount, r.Currency)
		}
		break
	}
	return out
}

// What a tab shows when a regulator would not serve a document. SEDAR+ mints a document's address
// inside a live session and puts a bot gate in front of it, so a refusal is ordinary and a retry
// often works; the page says that in words, names the document, and retries on a click.
func (a *App) documentErrorPage(symbol, docID, why string) []byte {
	sym := strings.ToUpper(py.Strip(symbol))
	name, source, when := "This document", "the regulator", ""
	if row := a.st.Filing(sym, docID); row != nil {
		for _, c := range []string{row.Subject, row.Title, row.Type} {
			if py.Strip(c) != "" {
				name = py.Strip(c)
				break
			}
		}
		if py.Strip(row.Source) != "" {
			source = py.Strip(row.Source)
		}
		when = py.Strip(row.DateText)
		if when == "" {
			when = cut10(row.Date)
		}
	}
	dated := ""
	if when != "" {
		dated = ", " + html.EscapeString(when)
	}
	again := "/api/filings/doc?symbol=" + url.QueryEscape(sym) + "&id=" + url.QueryEscape(docID)
	return []byte(fmt.Sprintf("<!doctype html><meta charset=utf-8><title>%s</title>"+
		"<style>:root{color-scheme:dark light}body{margin:0;min-height:100vh;display:grid;place-items:center;"+
		"background:#0e1118;color:#e8ecf3;font:400 14px/1.6 -apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif}"+
		"main{max-width:34rem;padding:2rem}h1{font:600 16px/1.4 inherit;margin:0 0 .75rem}p{margin:0 0 .75rem;color:#aab3c2}"+
		"b{color:#e8ecf3;font-weight:500}a{color:#7aa2f7}</style>"+
		"<main><h1>%s would not serve this document just now</h1>"+
		"<p><b>%s</b>%s</p><p>%s</p><p><a href=\"%s\">Try again</a></p></main>",
		html.EscapeString(name), html.EscapeString(source), html.EscapeString(name), dated, html.EscapeString(why), html.EscapeString(again)))
}

func cut10(s string) string {
	if len(s) > 10 {
		return s[:10]
	}
	return s
}
