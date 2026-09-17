package disclosures

import (
	"errors"
	"fmt"
	"regexp"
	"sort"
	"strings"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

const (
	Financials = "Financials"
	Events     = "Material events"
	Governance = "Governance"
	Offerings  = "Offerings"
	Insider    = "Insider & ownership"
	News       = "News releases"
	Other      = "Other"
)

var Categories = []string{Financials, Events, Governance, Offerings, Insider, News, Other}

type SourceUnavailable struct{ Msg string }

func (e *SourceUnavailable) Error() string { return e.Msg }

func Unavailable(format string, args ...any) error {
	return &SourceUnavailable{Msg: fmt.Sprintf(format, args...)}
}

func IsUnavailable(err error) bool {
	var su *SourceUnavailable
	return errors.As(err, &su)
}

var tagsRE = regexp.MustCompile(`<[^>]+>`)

func Clean(text string) string {
	return py.Strip(py.CollapseSpace(tagsRE.ReplaceAllString(text, " ")))
}

var nameNoiseRE = regexp.MustCompile(`(?i)\b(inc|corp|corporation|ltd|limited|co|company|plc|the|sa|nv|ag|llc|lp|trust|fund|holdings?)\b`)
var nonAlnumRE = regexp.MustCompile(`[^a-z0-9]+`)

func nameTokens(name string) map[string]bool {
	n := nameNoiseRE.ReplaceAllString(strings.ToLower(name), " ")
	out := map[string]bool{}
	for _, t := range nonAlnumRE.Split(n, -1) {
		if len(t) > 1 {
			out[t] = true
		}
	}
	return out
}

func NamesMatch(a, b string) bool {
	ta, tb := nameTokens(a), nameTokens(b)
	if len(ta) == 0 || len(tb) == 0 {
		return false
	}
	overlap := 0
	for t := range ta {
		if tb[t] {
			overlap++
		}
	}
	min := len(ta)
	if len(tb) < min {
		min = len(tb)
	}
	return overlap > 0 && float64(overlap) >= float64(min)*0.5
}

type Item struct {
	ID        string `json:"id"`
	Source    string `json:"source"`
	Category  string `json:"category"`
	Date      string `json:"date"`
	DateText  string `json:"dateText"`
	Type      string `json:"type"`
	Title     string `json:"title"`
	Size      string `json:"size"`
	URL       string `json:"url"`
	Issuer    string `json:"issuer,omitempty"`
	ProfileNo string `json:"profileNo,omitempty"`
}

type Row struct {
	ID        string
	Source    string
	Category  string
	Type      string
	URL       string
	ProfileNo string
	Issuer    string
}

type Enrichment struct {
	Subject string
	Summary string
	Final   bool
}

type Provider interface {
	Source() string
	Available() bool
	Covers(symbol, exchange, currency string) bool
	Fetch(symbol, name, exchange, currency string, limit int, profileNo string) ([]Item, error)
	Document(row Row) ([]byte, string, error)
	HasFiler(symbol, name, exchange, currency string) (bool, bool)
	Enrichment(row Row) *Enrichment
	Categorize(row Row) string
	Content(row Row) ([]byte, string, error)
}

type SourceStatus struct {
	Available bool   `json:"available"`
	Matched   bool   `json:"matched"`
	Filer     bool   `json:"filer"`
	Count     int    `json:"count"`
	Error     string `json:"error"`
}

type Result struct {
	Items   []Item                  `json:"items"`
	Sources map[string]SourceStatus `json:"sources"`
}

type Pipeline struct {
	Providers []Provider
	Sedar     Provider
}

func (p *Pipeline) Available() bool {
	for _, pr := range p.Providers {
		if pr.Available() {
			return true
		}
	}
	return false
}

func (p *Pipeline) ProvidersFor(symbol, exchange, currency string) []Provider {
	var out []Provider
	for _, pr := range p.Providers {
		if pr.Available() && pr.Covers(symbol, exchange, currency) {
			out = append(out, pr)
		}
	}
	return out
}

func sortKey(it Item) (string, string) {
	d := it.Date
	if d == "" {
		d = it.DateText
	}
	return d, it.Source
}

func (p *Pipeline) Fetch(symbol, name, exchange, currency string, limit int, profileNo string) Result {
	items := []Item{}
	sources := map[string]SourceStatus{}
	for _, pr := range p.Providers {
		covered := pr.Available() && pr.Covers(symbol, exchange, currency)
		if !covered {
			sources[pr.Source()] = SourceStatus{Available: pr.Available()}
			continue
		}
		hint := ""
		if pr == p.Sedar {
			hint = profileNo
		}
		got, err := pr.Fetch(symbol, name, exchange, currency, limit, hint)
		if err != nil {
			if IsUnavailable(err) {
				sources[pr.Source()] = SourceStatus{Error: err.Error()}
			} else {
				sources[pr.Source()] = SourceStatus{Available: true, Error: err.Error()}
			}
			continue
		}
		items = append(items, got...)
		filer := len(got) > 0
		if !filer {
			if has, ok := pr.HasFiler(symbol, name, exchange, currency); ok {
				filer = has
			}
		}
		sources[pr.Source()] = SourceStatus{Available: true, Matched: len(got) > 0, Filer: filer, Count: len(got)}
	}
	sort.SliceStable(items, func(i, j int) bool {
		di, si := sortKey(items[i])
		dj, sj := sortKey(items[j])
		if di != dj {
			return di > dj
		}
		return si > sj
	})
	if limit < 1 {
		limit = 1
	}
	if len(items) > limit {
		items = items[:limit]
	}
	return Result{Items: items, Sources: sources}
}

func (p *Pipeline) bySource(src string) Provider {
	for _, pr := range p.Providers {
		if pr.Source() == src {
			return pr
		}
	}
	return nil
}

func (p *Pipeline) Document(row Row) ([]byte, string, error) {
	if pr := p.bySource(row.Source); pr != nil {
		return pr.Document(row)
	}
	return nil, "", Unavailable("no provider for source %q", row.Source)
}

func (p *Pipeline) Enrichment(row Row) *Enrichment {
	if pr := p.bySource(row.Source); pr != nil {
		return pr.Enrichment(row)
	}
	return nil
}

func (p *Pipeline) Categorize(row Row) string {
	if pr := p.bySource(row.Source); pr != nil {
		if c := pr.Categorize(row); c != "" {
			return c
		}
	}
	return row.Category
}

func (p *Pipeline) Content(row Row) ([]byte, string, error) {
	if pr := p.bySource(row.Source); pr != nil {
		return pr.Content(row)
	}
	return nil, "", Unavailable("no provider for source %q", row.Source)
}
