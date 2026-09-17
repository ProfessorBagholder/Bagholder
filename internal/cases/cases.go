package cases

import (
	"bytes"
	"encoding/json"
	"math"
	"os"
	"sort"
	"strconv"
	"strings"
	"unicode/utf8"

	"github.com/ProfessorBagholder/Bagholder/internal/model"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

var (
	TradeKeys      = []string{"id", "symbol", "kind", "currency", "side", "qty", "mult", "entry", "exit", "entryDate", "exitDate", "holdDays", "pnl", "pnlCad", "pnlPct", "status", "fees", "account", "exchange", "grade", "tags"}
	KPIKeys        = []string{"count", "wins", "losses", "breakeven", "winRate", "realized", "expectancy", "profitFactor", "avgHold", "avgWin", "avgLoss", "grossWin", "grossLoss"}
	PositionKeys   = []string{"id", "symbol", "kind", "currency", "account", "exchange", "qty", "avg", "cost", "held", "alloc", "short", "dayChange", "grade"}
	PortfolioKeys  = []string{"marketValue", "costBasis", "unrealized", "unrealizedPct", "positionCount", "accountCount", "nav", "navAccounts", "marginUsed", "marginUsedBy", "marginUsedPct", "availableMargin", "availableMarginUnavailable", "hasMargin", "cash", "cashPct", "dayChange", "dayChangePct"}
	AllocationKeys = []string{"id", "symbol", "account", "value", "share"}
	YearKeys       = []string{"year", "r", "days", "from", "to", "flow", "endV", "spR"}
	MonthKeys      = []string{"key", "label", "value", "count"}
	SymbolKeys     = []string{"symbol", "pnl", "n", "legs", "winRate", "avgHold"}
	QueueKeys      = []string{"id", "symbol", "date", "pnl", "missing"}
	HoldingKeys    = []string{"symbol", "qty", "per", "freq", "freqVerified", "annual", "yoc", "ytd", "ttm", "all", "nextExDate", "nextPayDate", "exPast", "payPast"}
	TileKeys       = []string{"label", "total", "perMonth", "count", "yield", "projected", "earned", "book", "marginUsed", "interestPerMonth", "interestMonths"}
)

type Market struct {
	FX            map[string]float64              `json:"fx"`
	Benchmark     map[string]float64              `json:"benchmark"`
	Benchmarks    map[string]map[string]float64   `json:"benchmarks,omitempty"`
	Distributions map[string][]store.Distribution `json:"distributions"`
	Quotes        map[string]store.Quote          `json:"quotes"`
}

type Doc struct {
	Today    string                        `json:"today"`
	Snapshot store.Snapshot                `json:"snapshot"`
	Market   Market                        `json:"market"`
	Filters  any                           `json:"filters"`
	Journal  map[string]store.JournalEntry `json:"journal"`
	Expect   map[string]any                `json:"expect"`
}

func Load(path string) (*Doc, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var doc Doc
	if err := json.Unmarshal(raw, &doc); err != nil {
		return nil, err
	}
	return &doc, nil
}

func (d *Doc) MarketData() *store.MarketData {
	return &store.MarketData{FX: d.Market.FX, Benchmark: d.Market.Benchmark, Benchmarks: d.Market.Benchmarks, Distributions: d.Market.Distributions, Quotes: d.Market.Quotes}
}

func pick(d map[string]any, keys []string) map[string]any {
	out := map[string]any{}
	for _, k := range keys {
		if v, ok := d[k]; ok {
			out[k] = v
		}
	}
	return out
}

func toMap(v any) map[string]any {
	b, _ := json.Marshal(v)
	var out map[string]any
	_ = json.Unmarshal(b, &out)
	return out
}

func toList(v any) []map[string]any {
	b, _ := json.Marshal(v)
	var out []map[string]any
	_ = json.Unmarshal(b, &out)
	return out
}

func fillSubs(fills []model.Fill) []any {
	fs := append([]model.Fill{}, fills...)
	sort.SliceStable(fs, func(i, j int) bool { return fs[i].When < fs[j].When })
	out := []any{}
	for _, f := range fs {
		out = append(out, f.Sub)
	}
	return out
}

func Expect(snap *store.Snapshot, market *store.MarketData, today string, filters any, journal map[string]store.JournalEntry) map[string]any {
	if journal == nil {
		journal = map[string]store.JournalEntry{}
	}
	base := model.BuildBase(snap, market, journal, today, nil)
	view := model.BuildView(base, filters)
	trades := append([]*model.Trade{}, view.Trades...)
	sort.SliceStable(trades, func(i, j int) bool {
		a, b := trades[i], trades[j]
		if a.EntryDate != b.EntryDate {
			return a.EntryDate < b.EntryDate
		}
		if a.ExitDate != b.ExitDate {
			return a.ExitDate < b.ExitDate
		}
		return a.Symbol < b.Symbol
	})
	positions := append([]*model.Position{}, view.Positions...)
	sort.SliceStable(positions, func(i, j int) bool {
		a, b := positions[i], positions[j]
		if a.Symbol != b.Symbol {
			return a.Symbol < b.Symbol
		}
		return a.Account < b.Account
	})
	cf := view.Cashflow
	tl := []any{}
	for _, t := range trades {
		row := pick(toMap(t.TradeCore), TradeKeys)
		row["fills"] = fillSubs(t.Fills)
		tl = append(tl, row)
	}
	pl := []any{}
	for _, p := range positions {
		row := pick(toMap(p.PositionCore), PositionKeys)
		row["fills"] = fillSubs(p.Fills)
		pl = append(pl, row)
	}
	pf := pick(toMap(view.Portfolio), PortfolioKeys)
	alloc := []any{}
	for _, a := range toList(view.Portfolio.Allocation) {
		alloc = append(alloc, pick(a, AllocationKeys))
	}
	pf["allocation"] = alloc
	series := []any{}
	for _, p := range view.Equity.Series {
		series = append(series, map[string]any{"d": p.D, "v": p.V})
	}
	pickList := func(v any, keys []string) []any {
		out := []any{}
		for _, r := range toList(v) {
			out = append(out, pick(r, keys))
		}
		return out
	}
	holdings := append([]model.CashHolding{}, cf.Holdings...)
	sort.SliceStable(holdings, func(i, j int) bool { return holdings[i].Symbol < holdings[j].Symbol })
	opts := toMap(view.Options)
	return Rounded(map[string]any{
		"kpi":              pick(toMap(view.KPI), KPIKeys),
		"trades":           tl,
		"positions":        pl,
		"positionsSummary": toMap(view.PositionsSummary),
		"portfolio":        pf,
		"equity":           map[string]any{"label": view.Equity.Label, "series": series, "drawdown": toMap(view.Equity.Drawdown), "annualized": toMap(view.Equity.Annualized)},
		"years":            pickList(view.Years, YearKeys),
		"benchmark":        toMap(view.Benchmark),
		"monthly":          pickList(view.Monthly, MonthKeys),
		"bySymbol":         pickList(view.BySymbol, SymbolKeys),
		"grades":           map[string]any{"buckets": pickList(view.Grades.Buckets, []string{"grade", "n", "pnl"}), "ungraded": view.Grades.Ungraded, "graded": view.Grades.Graded},
		"queue":            pickList(view.Queue, QueueKeys),
		"options":          pick(opts, []string{"accounts", "symbols", "tags", "exchanges", "kinds", "years"}),
		"cashflowHoldings": pickList(holdings, HoldingKeys),
		"cashflowTiles":    pickList(cf.Tiles, TileKeys),
		"cashflowMonths":   pickList(cf.Months, MonthKeys),
		"cashflowTotal":    cf.Total, "cashflowCount": cf.Count, "cashflowSkipped": cf.SkippedFilters,
	}).(map[string]any)
}

func Rounded(v any) any {
	switch x := v.(type) {
	case float64:
		return py.Round(x, 6)
	case map[string]any:
		out := make(map[string]any, len(x))
		for k, e := range x {
			out[k] = Rounded(e)
		}
		return out
	case []any:
		out := make([]any, len(x))
		for i, e := range x {
			out[i] = Rounded(e)
		}
		return out
	default:
		b, err := json.Marshal(v)
		if err != nil {
			return v
		}
		var generic any
		if err := json.Unmarshal(b, &generic); err != nil {
			return v
		}
		if _, isScalar := generic.(float64); isScalar {
			return Rounded(generic)
		}
		if _, ok := generic.(map[string]any); ok {
			return Rounded(generic)
		}
		if _, ok := generic.([]any); ok {
			return Rounded(generic)
		}
		return generic
	}
}

func Diff(path string, want, got any, out *[]string) {
	switch w := want.(type) {
	case map[string]any:
		g, ok := got.(map[string]any)
		if !ok {
			*out = append(*out, path+": want object, got "+py.S(got))
			return
		}
		for k, wv := range w {
			gv, ok := g[k]
			if !ok {
				*out = append(*out, path+"."+k+": missing")
				continue
			}
			Diff(path+"."+k, wv, gv, out)
		}
		for k := range g {
			if _, ok := w[k]; !ok {
				*out = append(*out, path+"."+k+": unexpected")
			}
		}
	case []any:
		g, ok := got.([]any)
		if !ok {
			*out = append(*out, path+": want list, got "+py.S(got))
			return
		}
		if len(w) != len(g) {
			*out = append(*out, path+": want "+strconv.Itoa(len(w))+" items, got "+strconv.Itoa(len(g)))
			return
		}
		for i := range w {
			Diff(path+"["+strconv.Itoa(i)+"]", w[i], g[i], out)
		}
	case float64:
		g, ok := got.(float64)
		if !ok || math.Abs(py.Round(w, 6)-py.Round(g, 6)) > 1e-9 {
			*out = append(*out, path+": want "+py.Repr(w)+", got "+py.S(got))
		}
	case nil:
		if got != nil {
			*out = append(*out, path+": want null, got "+py.S(got))
		}
	default:
		wb, _ := json.Marshal(want)
		gb, _ := json.Marshal(got)
		if string(wb) != string(gb) {
			*out = append(*out, path+": want "+string(wb)+", got "+string(gb))
		}
	}
}

func Generic(v any) any {
	b, _ := json.Marshal(v)
	var out any
	_ = json.Unmarshal(b, &out)
	return out
}

type Writer struct {
	old map[string]json.Number
}

func NewWriter(previous []byte) *Writer {
	w := &Writer{old: map[string]json.Number{}}
	if len(previous) == 0 {
		return w
	}
	dec := json.NewDecoder(bytes.NewReader(previous))
	dec.UseNumber()
	var doc any
	if err := dec.Decode(&doc); err != nil {
		return w
	}
	w.walk("", doc)
	return w
}

func (w *Writer) walk(path string, v any) {
	switch x := v.(type) {
	case map[string]any:
		for k, e := range x {
			w.walk(path+"/"+k, e)
		}
	case []any:
		for i, e := range x {
			w.walk(path+"/"+strconv.Itoa(i), e)
		}
	case json.Number:
		w.old[path] = x
	}
}

func (w *Writer) Marshal(v any) []byte {
	var buf bytes.Buffer
	w.write(&buf, "", v, 0)
	buf.WriteByte('\n')
	return buf.Bytes()
}

func Raw(data []byte) (map[string]any, error) {
	dec := json.NewDecoder(bytes.NewReader(data))
	dec.UseNumber()
	var doc map[string]any
	if err := dec.Decode(&doc); err != nil {
		return nil, err
	}
	return doc, nil
}

func (w *Writer) number(path string, f float64) string {
	old, had := w.old[path]
	oldFloat := had && (strings.ContainsAny(string(old), ".eE"))
	if had && !oldFloat {
		if f == math.Trunc(f) && math.Abs(f) < 1e15 {
			return strconv.FormatInt(int64(f), 10)
		}
		return py.Repr(f)
	}
	if oldFloat || f != math.Trunc(f) || math.Abs(f) >= 1e15 {
		return py.Repr(f)
	}
	return strconv.FormatInt(int64(f), 10)
}

func (w *Writer) write(buf *bytes.Buffer, path string, v any, depth int) {
	pad := strings.Repeat("  ", depth+1)
	end := strings.Repeat("  ", depth)
	switch x := v.(type) {
	case nil:
		buf.WriteString("null")
	case bool:
		if x {
			buf.WriteString("true")
		} else {
			buf.WriteString("false")
		}
	case float64:
		buf.WriteString(w.number(path, x))
	case json.Number:
		buf.WriteString(string(x))
	case string:
		writeString(buf, x)
	case []any:
		if len(x) == 0 {
			buf.WriteString("[]")
			return
		}
		buf.WriteString("[\n")
		for i, e := range x {
			buf.WriteString(pad)
			w.write(buf, path+"/"+strconv.Itoa(i), e, depth+1)
			if i < len(x)-1 {
				buf.WriteByte(',')
			}
			buf.WriteByte('\n')
		}
		buf.WriteString(end + "]")
	case map[string]any:
		if len(x) == 0 {
			buf.WriteString("{}")
			return
		}
		keys := make([]string, 0, len(x))
		for k := range x {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		buf.WriteString("{\n")
		for i, k := range keys {
			buf.WriteString(pad)
			writeString(buf, k)
			buf.WriteString(": ")
			w.write(buf, path+"/"+k, x[k], depth+1)
			if i < len(keys)-1 {
				buf.WriteByte(',')
			}
			buf.WriteByte('\n')
		}
		buf.WriteString(end + "}")
	}
}

func writeString(buf *bytes.Buffer, s string) {
	buf.WriteByte('"')
	for _, r := range s {
		switch r {
		case '"':
			buf.WriteString(`\"`)
		case '\\':
			buf.WriteString(`\\`)
		case '\n':
			buf.WriteString(`\n`)
		case '\r':
			buf.WriteString(`\r`)
		case '\t':
			buf.WriteString(`\t`)
		case '\b':
			buf.WriteString(`\b`)
		case '\f':
			buf.WriteString(`\f`)
		default:
			if r < 0x20 || r > 0x7e {
				if r > 0xffff {
					r -= 0x10000
					buf.WriteString(`\u` + hex4(0xd800+(r>>10)) + `\u` + hex4(0xdc00+(r&0x3ff)))
				} else {
					buf.WriteString(`\u` + hex4(r))
				}
			} else {
				buf.WriteRune(r)
			}
		}
	}
	buf.WriteByte('"')
}

func hex4(r rune) string {
	s := strconv.FormatInt(int64(r), 16)
	for len(s) < 4 {
		s = "0" + s
	}
	return s
}

var _ = utf8.RuneLen
