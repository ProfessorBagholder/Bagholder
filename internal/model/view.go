package model

import (
	"encoding/json"
	"math"
	"regexp"
	"sort"
	"strings"

	"github.com/ProfessorBagholder/Bagholder/internal/exposure"
	"github.com/ProfessorBagholder/Bagholder/internal/instruments"
	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/news"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

type RangeFilter struct {
	Op string   `json:"op"`
	V  *float64 `json:"v"`
}

type ListFilters struct {
	Account  []string `json:"account"`
	Symbol   []string `json:"symbol"`
	Grade    []string `json:"grade"`
	Tag      []string `json:"tag"`
	Kind     []string `json:"kind"`
	Exchange []string `json:"exchange"`
	Side     []string `json:"side"`
	Result   []string `json:"result"`
}

type RangeFilters struct {
	Price RangeFilter `json:"price"`
	Hold  RangeFilter `json:"hold"`
	Pnl   RangeFilter `json:"pnl"`
	Qty   RangeFilter `json:"qty"`
}

type Filters struct {
	Lists     ListFilters  `json:"lists"`
	Ranges    RangeFilters `json:"ranges"`
	Preset    string       `json:"preset"`
	Years     []string     `json:"years"`
	From      string       `json:"from"`
	To        string       `json:"to"`
	Search    string       `json:"search"`
	Benchmark string       `json:"benchmark"`
}

func EmptyFilters() Filters {
	return Filters{
		Lists:     ListFilters{[]string{}, []string{}, []string{}, []string{}, []string{}, []string{}, []string{}, []string{}},
		Ranges:    RangeFilters{RangeFilter{">", nil}, RangeFilter{">", nil}, RangeFilter{">", nil}, RangeFilter{">", nil}},
		Preset:    "all",
		Years:     []string{},
		Benchmark: "SP500",
	}
}

var BenchmarkLabels = map[string]string{"SP500": "S&P 500", "TSX": "S&P/TSX", "TSX60": "TSX 60"}

var PresetDays = map[string]int{"1d": 1, "1w": 7, "1m": 30, "3m": 90, "6m": 180, "1y": 365, "5y": 1826}

var yearRE = regexp.MustCompile(`^\d{4}$`)
var dateRE = regexp.MustCompile(`^\d{4}-\d{2}-\d{2}$`)

func cleanList(raw map[string]any, key string) []string {
	out := []string{}
	vals, ok := raw[key].([]any)
	if !ok {
		return out
	}
	for _, v := range vals {
		if s := py.S(v); s != "" {
			out = append(out, s)
		}
	}
	return out
}

func cleanRange(raw map[string]any, key string) RangeFilter {
	f := RangeFilter{Op: ">"}
	r, ok := raw[key].(map[string]any)
	if !ok {
		return f
	}
	if op := py.S(r["op"]); op == ">" || op == "<" {
		f.Op = op
	}
	v := r["v"]
	if v == nil {
		return f
	}
	if s, ok := v.(string); ok && s == "" {
		return f
	}
	if n, ok := py.NumOK(v); ok {
		f.V = py.Ptr(n)
	}
	return f
}

func CleanFilters(raw any) Filters {
	f := EmptyFilters()
	m, ok := raw.(map[string]any)
	if !ok {
		return f
	}
	lists, _ := m["lists"].(map[string]any)
	if lists == nil {
		lists = map[string]any{}
	}
	f.Lists = ListFilters{cleanList(lists, "account"), cleanList(lists, "symbol"), cleanList(lists, "grade"), cleanList(lists, "tag"), cleanList(lists, "kind"), cleanList(lists, "exchange"), cleanList(lists, "side"), cleanList(lists, "result")}
	ranges, _ := m["ranges"].(map[string]any)
	if ranges == nil {
		ranges = map[string]any{}
	}
	f.Ranges = RangeFilters{cleanRange(ranges, "price"), cleanRange(ranges, "hold"), cleanRange(ranges, "pnl"), cleanRange(ranges, "qty")}
	preset := strings.ToLower(py.S(m["preset"]))
	if _, ok := PresetDays[preset]; ok || preset == "ytd" || preset == "all" {
		f.Preset = preset
	}
	if years, ok := m["years"].([]any); ok {
		seen := map[string]bool{}
		for _, y := range years {
			s := cut(py.S(y), 4)
			if yearRE.MatchString(s) && !seen[s] {
				seen[s] = true
				f.Years = append(f.Years, s)
			}
		}
		sort.Strings(f.Years)
	}
	if v := cut(py.S(m["from"]), 10); dateRE.MatchString(v) {
		f.From = v
	}
	if v := cut(py.S(m["to"]), 10); dateRE.MatchString(v) {
		f.To = v
	}
	f.Search = py.Strip(py.S(m["search"]))
	if b := strings.ToUpper(py.Strip(py.S(m["benchmark"]))); BenchmarkLabels[b] != "" {
		f.Benchmark = b
	}
	return f
}

type bounds struct{ from, to string }

func dateBounds(f *Filters, today string) *bounds {
	if f.From != "" || f.To != "" {
		b := &bounds{f.From, f.To}
		if b.from == "" {
			b.from = "0000-01-01"
		}
		if b.to == "" {
			b.to = "9999-12-31"
		}
		return b
	}
	if len(f.Years) > 0 {
		return nil
	}
	if f.Preset == "ytd" {
		return &bounds{cut(today, 4) + "-01-01", today}
	}
	if days, ok := PresetDays[f.Preset]; ok && days > 0 {
		return &bounds{shiftDate(today, -days), today}
	}
	return nil
}

func inDateScope(f *Filters, today, day string) bool {
	if b := dateBounds(f, today); b != nil {
		return b.from <= day && day <= b.to
	}
	if len(f.Years) > 0 {
		return py.Contains(f.Years, cut(day, 4))
	}
	return true
}

func rangeOK(r RangeFilter, val float64) bool {
	if r.V == nil {
		return true
	}
	if r.Op == ">" && !(val > *r.V) {
		return false
	}
	if r.Op == "<" && !(val < *r.V) {
		return false
	}
	return true
}

func tradeMatches(t *Trade, f *Filters, today string) bool {
	s := strings.ToUpper(f.Search)
	if s != "" && !strings.Contains(strings.ToUpper(t.Symbol), s) && !strings.Contains(strings.ToUpper(t.Underlying), s) && !strings.Contains(strings.ToUpper(t.Name), s) {
		return false
	}
	L := &f.Lists
	if len(L.Account) > 0 && !py.Contains(L.Account, t.Account) {
		return false
	}
	if len(L.Symbol) > 0 && !py.Contains(L.Symbol, t.Symbol) && !py.Contains(L.Symbol, t.Underlying) {
		return false
	}
	if len(L.Grade) > 0 {
		g := t.Grade
		if g == "" {
			g = "Ungraded"
		}
		if !py.Contains(L.Grade, g) {
			return false
		}
	}
	if len(L.Tag) > 0 {
		tags := t.Tags
		if len(tags) == 0 {
			tags = []string{"untagged"}
		}
		any := false
		for _, x := range tags {
			if py.Contains(L.Tag, x) {
				any = true
				break
			}
		}
		if !any {
			return false
		}
	}
	if len(L.Kind) > 0 && !py.Contains(L.Kind, t.Kind) {
		return false
	}
	if len(L.Exchange) > 0 && !py.Contains(L.Exchange, t.Exchange) {
		return false
	}
	if len(L.Side) > 0 && !py.Contains(L.Side, t.Side) {
		return false
	}
	if len(L.Result) > 0 {
		res := "Breakeven"
		if t.PnlCad > 0 {
			res = "Winners"
		} else if t.PnlCad < 0 {
			res = "Losers"
		}
		if !py.Contains(L.Result, res) {
			return false
		}
	}
	R := &f.Ranges
	if !rangeOK(R.Price, t.Entry) || !rangeOK(R.Hold, float64(t.HoldDays)) || !rangeOK(R.Pnl, t.PnlCad) || !rangeOK(R.Qty, t.Qty) {
		return false
	}
	return inDateScope(f, today, t.ExitDate)
}

const Unclassified = "Not classified"

type ExposureSlice struct {
	Name  string  `json:"name"`
	Value float64 `json:"value"`
	Share float64 `json:"share"`
}

type cadFn func(amount float64, currency string) float64

type namedTotals struct {
	totals map[string]float64
	order  []string
}

func (n *namedTotals) add(name string, v float64) {
	if _, ok := n.totals[name]; !ok {
		n.order = append(n.order, name)
	}
	n.totals[name] += v
}

func optionExposureKeys(p *Position) (string, string) {
	under := strings.ToUpper(p.Underlying)
	us, ca := exposure.ShareKey+under+"::US", exposure.ShareKey+under+":"
	if strings.ToUpper(p.Currency) == "USD" {
		return us, ca
	}
	return ca, us
}

func exposureOf(exposures map[string]store.Exposure, key string) *store.Exposure {
	if r, ok := exposures[key]; ok {
		return &r
	}
	return nil
}

func sortedMapKeys(m map[string]float64) []string {
	return store.SortedKeys(m)
}

func exposureSlices(positions []*Position, exposures map[string]store.Exposure, cad cadFn) ([]ExposureSlice, []ExposureSlice) {
	secTot := &namedTotals{totals: map[string]float64{}}
	ctyTot := &namedTotals{totals: map[string]float64{}}
	secUnc, ctyUnc := 0.0, 0.0
	total := 0.0
	for _, p := range positions {
		v := cad(p.MV, p.Currency)
		if v <= 0 {
			continue
		}
		total += v
		var rec *store.Exposure
		if p.SecurityID != "" {
			rec = exposureOf(exposures, p.SecurityID)
		}
		if p.Kind == "Options" {
			first, second := optionExposureKeys(p)
			rec = exposureOf(exposures, first)
			if rec == nil {
				rec = exposureOf(exposures, second)
			}
		}
		var sMap, cMap map[string]float64
		if rec != nil {
			sMap, cMap = rec.Sectors, rec.Countries
		}
		if p.Kind == "Crypto" {
			sMap, cMap = map[string]float64{"Digital assets": 1.0}, map[string]float64{}
		}
		sSum, cSum := 0.0, 0.0
		for _, w := range sMap {
			sSum += w
		}
		for _, w := range cMap {
			cSum += w
		}
		for _, n := range sortedMapKeys(sMap) {
			w := sMap[n]
			name := exposure.NormSector(n)
			if name == "" {
				name = n
			}
			secTot.add(name, v*w)
		}
		for _, n := range sortedMapKeys(cMap) {
			ctyTot.add(n, v*cMap[n])
		}
		secUnc += v * math.Max(0, 1-math.Min(1, sSum))
		ctyUnc += v * math.Max(0, 1-math.Min(1, cSum))
	}
	rows := func(tot *namedTotals, unc float64) []ExposureSlice {
		out := []ExposureSlice{}
		for _, n := range tot.order {
			if v := tot.totals[n]; v > 0 {
				out = append(out, ExposureSlice{Name: n, Value: v})
			}
		}
		sort.SliceStable(out, func(i, j int) bool { return out[i].Value > out[j].Value })
		if unc > 0.005 {
			out = append(out, ExposureSlice{Name: Unclassified, Value: unc})
		}
		for i := range out {
			if total != 0 {
				out[i].Share = out[i].Value / total
			}
		}
		return out
	}
	return rows(secTot, secUnc), rows(ctyTot, ctyUnc)
}

func WatchQuoteKey(symbol, exchange string) string {
	return strings.ToUpper(py.Strip(symbol)) + "@" + strings.ToUpper(py.Strip(exchange))
}

var DefaultTiles = []store.Tile{{"SPX", "INDEX"}, {"NDX", "INDEX"}, {"DJI", "INDEX"}, {"VIX", "INDEX"}, {"GC", "COMEX"}, {"BTCUSD", "FX"}}

const TilesMax = 12

func tileList(base *Base) []*instruments.Instrument {
	rows := DefaultTiles
	if base.TilesSaved {
		rows = base.Tiles
	}
	var out []*instruments.Instrument
	seen := map[string]bool{}
	for _, r := range rows {
		inst := instruments.Find(r.Symbol, r.Exchange)
		if inst != nil && !seen[inst.Symbol] {
			seen[inst.Symbol] = true
			out = append(out, inst)
		}
	}
	if len(out) > TilesMax {
		out = out[:TilesMax]
	}
	return out
}

type QuoteSymbol struct {
	Symbol   string `json:"symbol"`
	Exchange string `json:"exchange"`
	Currency string `json:"currency"`
	Kind     string `json:"kind"`
	QuoteKey string `json:"quoteKey"`
	Yahoo    string `json:"yahoo,omitempty"`
}

func TileSymbols(base *Base) []QuoteSymbol {
	out := []QuoteSymbol{}
	for _, i := range tileList(base) {
		out = append(out, QuoteSymbol{Symbol: i.Symbol, Exchange: i.Exchange, Currency: i.Currency, Kind: "Instrument", QuoteKey: WatchQuoteKey(i.Symbol, i.Exchange), Yahoo: i.Yahoo})
	}
	return out
}

func tileDecimals(inst *instruments.Instrument) int {
	if inst.Symbol == "BTCUSD" {
		return 0
	}
	switch inst.Kind {
	case "Rate":
		return 3
	case "Currency":
		return 4
	}
	return 2
}

type TileRow struct {
	Symbol        string   `json:"symbol"`
	Exchange      string   `json:"exchange"`
	Label         string   `json:"label"`
	Name          string   `json:"name"`
	Kind          string   `json:"kind"`
	Last          *float64 `json:"last"`
	Change        *float64 `json:"change"`
	PercentChange *float64 `json:"percentChange"`
	Decimals      int      `json:"decimals"`
	Rate          *float64 `json:"-"`
	RateChange    *float64 `json:"-"`
}

func (t TileRow) MarshalJSON() ([]byte, error) {
	type plain TileRow
	if t.Rate == nil {
		return json.Marshal(plain(t))
	}
	type withRate struct {
		plain
		Rate       *float64 `json:"rate"`
		RateChange *float64 `json:"rateChange"`
	}
	return json.Marshal(withRate{plain(t), t.Rate, t.RateChange})
}

func tileRows(base *Base) []TileRow {
	out := []TileRow{}
	for _, inst := range tileList(base) {
		q, ok := base.Quotes[WatchQuoteKey(inst.Symbol, inst.Exchange)]
		var price, move, pct *float64
		if ok {
			price, move, pct = copyPtr(q.Price), copyPtr(q.PriceChange), copyPtr(q.PercentChange)
		}
		row := TileRow{Symbol: inst.Symbol, Exchange: inst.Exchange, Label: instruments.Label(inst.Symbol), Name: inst.Name, Kind: inst.Kind, Last: price, Change: move, PercentChange: pct, Decimals: tileDecimals(inst)}
		if rate := instruments.ImpliedRate(inst.Symbol, price); rate != nil {
			row.Rate = rate
			if move != nil {
				row.RateChange = py.Ptr(py.Round(-*move, 4))
			}
		}
		out = append(out, row)
	}
	return out
}

func WatchSymbols(base *Base) []QuoteSymbol {
	out := []QuoteSymbol{}
	for _, w := range base.Watchlist {
		inst := instruments.Find(w.Symbol, w.Exchange)
		crypto := strings.ToUpper(w.Exchange) == "CRYPTO"
		rec := QuoteSymbol{Symbol: w.Symbol, Exchange: w.Exchange, Currency: w.Currency, Kind: "Shares", QuoteKey: WatchQuoteKey(w.Symbol, w.Exchange)}
		if crypto {
			rec.Currency = "USD"
			rec.Kind = "Crypto"
		}
		if inst != nil {
			rec.Kind = "Instrument"
			rec.Yahoo = inst.Yahoo
		}
		out = append(out, rec)
	}
	return out
}

func QuoteSymbols(base *Base) []QuoteSymbol {
	out := WatchSymbols(base)
	keys := map[string]bool{}
	for _, r := range out {
		keys[r.QuoteKey] = true
	}
	for _, rec := range TileSymbols(base) {
		if !keys[rec.QuoteKey] {
			keys[rec.QuoteKey] = true
			out = append(out, rec)
		}
	}
	return out
}

func WatchExposureKey(symbol, exchange, currency string) string {
	return exposure.ShareKey + market.TMXSymbol(symbol) + ":" + market.TMXFormOr(exchange, currency)
}

func dominantSector(rec *store.Exposure) string {
	best, w := "", 0.0
	if rec != nil {
		for _, name := range sortedMapKeys(rec.Sectors) {
			weight := rec.Sectors[name]
			n := exposure.NormSector(name)
			if n == "" {
				n = name
			}
			if weight > w {
				best, w = n, weight
			}
		}
	}
	if best == "" {
		return Unclassified
	}
	return best
}

type WatchRow struct {
	Symbol        string   `json:"symbol"`
	Exchange      string   `json:"exchange"`
	Name          string   `json:"name"`
	Currency      string   `json:"currency"`
	Last          *float64 `json:"last"`
	PriceChange   *float64 `json:"priceChange"`
	PercentChange *float64 `json:"percentChange"`
	Sector        string   `json:"sector"`
	Kind          string   `json:"kind"`
	PositionID    *string  `json:"positionId"`
}

type symEx struct{ symbol, exchange string }

func watchRows(base *Base, positions []*Position) []WatchRow {
	held := map[symEx]*Position{}
	for _, p := range positions {
		k := symEx{p.Symbol, strings.ToUpper(p.Exchange)}
		if _, ok := held[k]; !ok {
			held[k] = p
		}
	}
	out := []WatchRow{}
	for _, w := range base.Watchlist {
		var q *store.Quote
		if qq, ok := base.Quotes[WatchQuoteKey(w.Symbol, w.Exchange)]; ok {
			q = &qq
		}
		pos := held[symEx{w.Symbol, strings.ToUpper(w.Exchange)}]
		inst := instruments.Find(w.Symbol, w.Exchange)
		crypto := strings.ToUpper(w.Exchange) == "CRYPTO"
		var rec *store.Exposure
		if inst == nil && !crypto {
			rec = exposureOf(base.Exposures, WatchExposureKey(w.Symbol, w.Exchange, w.Currency))
		}
		row := WatchRow{Symbol: w.Symbol, Exchange: w.Exchange, Name: w.Name, Currency: w.Currency, Kind: "Shares", Sector: Unclassified}
		if q != nil {
			row.Last, row.PriceChange, row.PercentChange = copyPtr(q.Price), copyPtr(q.PriceChange), copyPtr(q.PercentChange)
		}
		switch {
		case inst != nil:
			row.Exchange = inst.Exchange
			row.Kind = inst.Kind
			if l, ok := instruments.KindLabel[inst.Kind]; ok {
				row.Sector = l
			} else {
				row.Sector = inst.Kind
			}
		case crypto:
			row.Exchange = "Crypto"
			row.Currency = "USD"
			row.Kind = "Crypto"
			row.Sector = "Digital assets"
		default:
			if rec != nil {
				row.Sector = dominantSector(rec)
			}
		}
		if pos != nil {
			id := pos.ID
			row.PositionID = &id
		}
		out = append(out, row)
	}
	return out
}

type HeatItem struct {
	ID            string   `json:"id"`
	Symbol        string   `json:"symbol"`
	Exchange      string   `json:"exchange"`
	Value         float64  `json:"value"`
	PercentChange *float64 `json:"percentChange"`
	Sector        string   `json:"sector"`
}

func heatmapItems(positions []*Position, exposures map[string]store.Exposure, cad cadFn) []*HeatItem {
	out := []*HeatItem{}
	byKey := map[symEx]*HeatItem{}
	for _, p := range positions {
		v := cad(p.MV, p.Currency)
		if !(v > 0) {
			continue
		}
		var sector string
		switch p.Kind {
		case "Crypto":
			sector = "Digital assets"
		case "Options":
			first, second := optionExposureKeys(p)
			rec := exposureOf(exposures, first)
			if rec == nil {
				rec = exposureOf(exposures, second)
			}
			sector = dominantSector(rec)
		default:
			sector = dominantSector(exposureOf(exposures, p.SecurityID))
		}
		key := symEx{p.Symbol, strings.ToUpper(p.Exchange)}
		if cur, ok := byKey[key]; ok {
			cur.Value += v
			continue
		}
		item := &HeatItem{ID: p.ID, Symbol: p.Symbol, Exchange: p.Exchange, Value: v, PercentChange: p.PercentChange, Sector: sector}
		byKey[key] = item
		out = append(out, item)
	}
	return out
}

var nonAlnumRE = regexp.MustCompile(`[^a-z0-9]+`)

func newsTextKey(headline string) string {
	return strings.Join(py.Fields(nonAlnumRE.ReplaceAllString(strings.ToLower(headline), " ")), " ")
}

var frenchWords = map[string]bool{"annonce": true, "annoncent": true, "ses": true, "du": true, "des": true, "une": true, "pour": true, "avec": true, "sur": true, "résultats": true, "clôture": true, "croissance": true, "les": true, "et": true, "au": true, "aux": true, "dans": true, "son": true, "sa": true, "le": true, "la": true}

var accentRE = regexp.MustCompile(`[àâçéèêëîïôûùüÿœ]`)

func looksFrench(headline string) bool {
	t := strings.ToLower(headline)
	if len(accentRE.FindAllStringIndex(t, -1)) >= 2 {
		return true
	}
	n := 0
	for _, w := range wordTokens(t) {
		if frenchWords[w] {
			n++
		}
	}
	return n >= 2
}

func wordTokens(t string) []string {
	var out []string
	start := -1
	for i, r := range t {
		if py.IsWordRune(r) {
			if start < 0 {
				start = i
			}
		} else if start >= 0 {
			out = append(out, t[start:i])
			start = -1
		}
	}
	if start >= 0 {
		out = append(out, t[start:])
	}
	return out
}

func whenMinutes(iso string) *float64 {
	t, _, ok := py.ParseISO(strings.Replace(iso, "Z", "+00:00", 1))
	if !ok {
		return nil
	}
	return py.Ptr(float64(t.UnixNano()) / 1e9 / 60.0)
}

type NewsTag struct {
	Symbol        string   `json:"symbol"`
	Exchange      string   `json:"exchange"`
	Held          bool     `json:"held"`
	Watched       bool     `json:"watched"`
	PercentChange *float64 `json:"percentChange"`
	PositionID    *string  `json:"positionId"`
}

type NewsRow struct {
	ID          string     `json:"id"`
	Headline    string     `json:"headline"`
	Source      string     `json:"source"`
	URL         string     `json:"url"`
	PublishedAt string     `json:"publishedAt"`
	Market      bool       `json:"market"`
	Tags        []*NewsTag `json:"tags"`
	Kind        string     `json:"kind"`
}

func tagKeys(r *NewsRow) map[symEx]bool {
	out := map[symEx]bool{}
	for _, t := range r.Tags {
		out[symEx{t.Symbol, strings.ToUpper(t.Exchange)}] = true
	}
	return out
}

func dropTranslations(rows []*NewsRow) []*NewsRow {
	out := []*NewsRow{}
	for _, r := range rows {
		if looksFrench(r.Headline) {
			tr, kr := whenMinutes(r.PublishedAt), tagKeys(r)
			twin := false
			for _, o := range rows {
				if o == r || looksFrench(o.Headline) || o.Source != r.Source {
					continue
				}
				shared := false
				for k := range tagKeys(o) {
					if kr[k] {
						shared = true
						break
					}
				}
				if !shared || tr == nil {
					continue
				}
				to := whenMinutes(o.PublishedAt)
				if to != nil && math.Abs(*to-*tr) <= 180 {
					twin = true
					break
				}
			}
			if twin {
				continue
			}
		}
		out = append(out, r)
	}
	return out
}

func listingKey(symbol, exchange string) symEx {
	return symEx{market.TMXSymbol(symbol), strings.ToUpper(exchange)}
}

func newsRows(base *Base, positions []*Position, watch []WatchRow) []*NewsRow {
	held := map[symEx]*Position{}
	for _, p := range positions {
		k := listingKey(p.Symbol, p.Exchange)
		if _, ok := held[k]; !ok {
			held[k] = p
		}
	}
	watched := map[symEx]*WatchRow{}
	for i := range watch {
		watched[listingKey(watch[i].Symbol, watch[i].Exchange)] = &watch[i]
	}
	rows := []*NewsRow{}
	byID := map[string]*NewsRow{}
	byText := map[string]*NewsRow{}
	items := append([]store.NewsItem{}, base.News...)
	sort.SliceStable(items, func(i, j int) bool { return items[i].PublishedAt > items[j].PublishedAt })
	for _, n := range items {
		isMarket := n.Symbol == news.Market[0] && strings.ToUpper(n.Exchange) == news.Market[1]
		key := listingKey(n.Symbol, n.Exchange)
		p, w := held[key], watched[key]
		var tag *NewsTag
		if !isMarket {
			tag = &NewsTag{Symbol: key.symbol, Exchange: n.Exchange, Held: p != nil, Watched: w != nil}
			if p != nil {
				tag.PercentChange = p.PercentChange
				id := p.ID
				tag.PositionID = &id
			} else if w != nil {
				tag.PercentChange = w.PercentChange
			}
		}
		text := newsTextKey(n.Headline)
		row := byID[n.ID]
		if row == nil && text != "" {
			row = byText[text]
		}
		if row != nil {
			if isMarket {
				row.Market = true
			} else {
				has := false
				for _, t := range row.Tags {
					if listingKey(t.Symbol, t.Exchange) == key {
						has = true
						break
					}
				}
				if !has {
					row.Tags = append(row.Tags, tag)
				}
			}
			if n.Kind == "release" {
				row.Kind = "release"
			}
			byID[n.ID] = row
			continue
		}
		kind := n.Kind
		if kind == "" {
			kind = "story"
		}
		row = &NewsRow{ID: n.ID, Headline: n.Headline, Source: n.Wire, URL: n.URL, PublishedAt: n.PublishedAt, Market: isMarket, Tags: []*NewsTag{}, Kind: kind}
		if !isMarket {
			row.Tags = []*NewsTag{tag}
		}
		byID[n.ID] = row
		if text != "" {
			byText[text] = row
		}
		rows = append(rows, row)
	}
	sort.SliceStable(rows, func(i, j int) bool { return rows[i].PublishedAt > rows[j].PublishedAt })
	return dropTranslations(rows)
}

type UniverseRow struct {
	ID            *string  `json:"id"`
	Symbol        string   `json:"symbol"`
	Name          string   `json:"name"`
	Value         float64  `json:"value"`
	PercentChange *float64 `json:"percentChange"`
	Sector        string   `json:"sector"`
	Country       string   `json:"country"`
}

type DirectoryRow struct {
	Symbol   string   `json:"symbol"`
	Label    string   `json:"label"`
	Name     string   `json:"name"`
	Exchange string   `json:"exchange"`
	Kind     string   `json:"kind"`
	Aliases  []string `json:"aliases"`
}

type MarketsView struct {
	Holdings    []*HeatItem              `json:"holdings"`
	Watchlist   []WatchRow               `json:"watchlist"`
	News        []*NewsRow               `json:"news"`
	Universes   map[string][]UniverseRow `json:"universes"`
	Tiles       []TileRow                `json:"tiles"`
	Instruments []DirectoryRow           `json:"instruments"`
}

func marketsView(base *Base, positions []*Position) MarketsView {
	fx := base.FX
	today := base.Today
	cad := func(amount float64, currency string) float64 { return toCad(fx, amount, currency, today) }
	watch := watchRows(base, positions)
	universes := map[string][]UniverseRow{}
	for k, rows := range base.Universes {
		out := []UniverseRow{}
		for _, r := range rows {
			sector := r.Sector
			if sector == "" {
				sector = Unclassified
			}
			out = append(out, UniverseRow{Symbol: r.Symbol, Name: r.Name, Value: py.Deref(r.Value, 0), PercentChange: r.PercentChange, Sector: sector, Country: r.Country})
		}
		universes[k] = out
	}
	directory := []DirectoryRow{}
	for _, r := range instruments.Rows() {
		directory = append(directory, DirectoryRow{Symbol: r.Symbol, Label: instruments.Label(r.Symbol), Name: r.Name, Exchange: r.Exchange, Kind: r.Kind, Aliases: r.Aliases})
	}
	return MarketsView{Holdings: heatmapItems(positions, base.Exposures, cad), Watchlist: watch, News: newsRows(base, positions, watch), Universes: universes, Tiles: tileRows(base), Instruments: directory}
}

type AllocRow struct {
	ID      string  `json:"id"`
	Symbol  string  `json:"symbol"`
	Account string  `json:"account"`
	Value   float64 `json:"value"`
	Share   float64 `json:"share"`
}

type PortfolioView struct {
	Allocation                 []AllocRow         `json:"allocation"`
	Sectors                    []ExposureSlice    `json:"sectors"`
	Regions                    []ExposureSlice    `json:"regions"`
	MarketValue                float64            `json:"marketValue"`
	CostBasis                  float64            `json:"costBasis"`
	Unrealized                 float64            `json:"unrealized"`
	UnrealizedPct              *float64           `json:"unrealizedPct"`
	PositionCount              int                `json:"positionCount"`
	AccountCount               int                `json:"accountCount"`
	Nav                        *float64           `json:"nav"`
	NavAccounts                int                `json:"navAccounts"`
	MarginUsed                 float64            `json:"marginUsed"`
	MarginUsedBy               map[string]float64 `json:"marginUsedBy"`
	MarginUsedPct              *float64           `json:"marginUsedPct"`
	AvailableMargin            *float64           `json:"availableMargin"`
	AvailableMarginUnavailable []string           `json:"availableMarginUnavailable"`
	HasMargin                  bool               `json:"hasMargin"`
	Cash                       float64            `json:"cash"`
	CashPct                    *float64           `json:"cashPct"`
	DayChange                  *float64           `json:"dayChange"`
	DayChangePct               *float64           `json:"dayChangePct"`
}

func signedMV(p *Position) float64 {
	if p.Short {
		return -p.MV
	}
	return p.MV
}

func portfolioView(base *Base, f *Filters, positions []*Position) PortfolioView {
	fx := base.FX
	today := base.Today
	cad := func(amount float64, currency string) float64 { return toCad(fx, amount, currency, today) }
	names := f.Lists.Account
	var accounts []AccountRow
	for _, a := range base.Accounts {
		if strings.ToLower(a.Status) != "closed" && (len(names) == 0 || py.Contains(names, a.Name)) {
			accounts = append(accounts, a)
		}
	}
	ids := map[string]bool{}
	nameOf := map[string]string{}
	for _, a := range accounts {
		ids[a.ID] = true
		nameOf[a.ID] = a.Name
	}
	mv, cost, unreal := 0.0, 0.0, 0.0
	for _, p := range positions {
		mv += cad(signedMV(p), p.Currency)
		cost += cad(math.Abs(p.Cost), p.Currency)
		unreal += cad(p.Unreal, p.Currency)
	}
	var navs []float64
	for _, a := range accounts {
		if a.Nav != nil {
			navs = append(navs, cad(*a.Nav, a.Currency))
		}
	}
	cashCcy := base.CashCurrencies
	used := &namedTotals{totals: map[string]float64{}}
	cashBy := &namedTotals{totals: map[string]float64{}}
	for _, b := range base.Balances {
		ccy := cashCcy[b.SecurityID]
		q := py.Deref(b.Quantity, 0)
		if ids[b.AccountID] && ccy != "" && q < 0 {
			used.add(ccy, -q)
		}
	}
	marginUsed := 0.0
	for _, c := range used.order {
		marginUsed += cad(used.totals[c], c)
	}
	for _, b := range base.Balances {
		ccy := cashCcy[b.SecurityID]
		q := py.Deref(b.Quantity, 0)
		if ids[b.AccountID] && ccy != "" && q > 0 {
			cashBy.add(ccy, q)
		}
	}
	cash := 0.0
	for _, c := range cashBy.order {
		cash += cad(cashBy.totals[c], c)
	}
	var quoted []*Position
	for _, p := range positions {
		if p.DayChange != nil {
			quoted = append(quoted, p)
		}
	}
	var dayChange *float64
	prevValue := 0.0
	if len(quoted) > 0 {
		dc := 0.0
		for _, p := range quoted {
			dc += cad(*p.DayChange, p.Currency)
		}
		dayChange = py.Ptr(dc)
		pv := 0.0
		for _, p := range quoted {
			pv += cad(signedMV(p), p.Currency)
		}
		prevValue = pv - dc
	}
	marginIDs := map[string]bool{}
	for _, a := range accounts {
		if strings.Contains(strings.ToUpper(a.Type), "MARGIN") {
			marginIDs[a.ID] = true
		}
	}
	var avail []float64
	unavailable := []string{}
	for _, m := range base.Margin {
		if !marginIDs[m.AccountID] {
			continue
		}
		if m.BuyingPower == nil {
			name, ok := nameOf[m.AccountID]
			if !ok {
				name = m.AccountID
			}
			unavailable = append(unavailable, name)
		} else {
			ccy := m.Currency
			if ccy == "" {
				ccy = "CAD"
			}
			avail = append(avail, cad(*m.BuyingPower, ccy))
		}
	}
	alloc := []AllocRow{}
	for _, p := range positions {
		v := cad(p.MV, p.Currency)
		if v > 0 {
			alloc = append(alloc, AllocRow{ID: p.ID, Symbol: p.Symbol, Account: p.Account, Value: v})
		}
	}
	sort.SliceStable(alloc, func(i, j int) bool { return alloc[i].Value > alloc[j].Value })
	total := 0.0
	for _, x := range alloc {
		total += x.Value
	}
	for i := range alloc {
		if total != 0 {
			alloc[i].Share = alloc[i].Value / total
		}
	}
	sectors, regions := exposureSlices(positions, base.Exposures, cad)
	accountSet := map[string]bool{}
	for _, p := range positions {
		accountSet[p.Account] = true
	}
	marginUsedBy := map[string]float64{}
	for c, v := range used.totals {
		marginUsedBy[c] = py.Round(v, 2)
	}
	sort.Strings(unavailable)
	pv := PortfolioView{
		Allocation:                 alloc,
		Sectors:                    sectors,
		Regions:                    regions,
		MarketValue:                mv,
		CostBasis:                  cost,
		Unrealized:                 unreal,
		PositionCount:              len(positions),
		AccountCount:               len(accountSet),
		NavAccounts:                len(navs),
		MarginUsed:                 marginUsed,
		MarginUsedBy:               marginUsedBy,
		AvailableMarginUnavailable: unavailable,
		HasMargin:                  len(marginIDs) > 0,
		Cash:                       cash,
		DayChange:                  dayChange,
	}
	if cost != 0 {
		pv.UnrealizedPct = py.Ptr(unreal / cost)
	}
	if len(navs) > 0 {
		s := 0.0
		for _, v := range navs {
			s += v
		}
		pv.Nav = py.Ptr(s)
		if s != 0 {
			pv.CashPct = py.Ptr(cash / s)
		}
	}
	if mv != 0 {
		pv.MarginUsedPct = py.Ptr(marginUsed / mv)
	}
	if len(avail) > 0 {
		s := 0.0
		for _, v := range avail {
			s += v
		}
		pv.AvailableMargin = py.Ptr(s)
	}
	if len(quoted) > 0 && prevValue != 0 {
		pv.DayChangePct = py.Ptr(*dayChange / prevValue)
	}
	return pv
}

func positionMatches(p *Position, f *Filters) bool {
	s := strings.ToUpper(f.Search)
	if s != "" && !strings.Contains(strings.ToUpper(p.Symbol), s) && !strings.Contains(strings.ToUpper(p.Name), s) {
		return false
	}
	L := &f.Lists
	if len(L.Account) > 0 && !py.Contains(L.Account, p.Account) {
		return false
	}
	if len(L.Symbol) > 0 && !py.Contains(L.Symbol, p.Symbol) && !py.Contains(L.Symbol, p.Underlying) {
		return false
	}
	if len(L.Kind) > 0 && !py.Contains(L.Kind, p.Kind) {
		return false
	}
	if len(L.Exchange) > 0 && !py.Contains(L.Exchange, p.Exchange) {
		return false
	}
	return true
}

type KPI struct {
	Realized             float64  `json:"realized"`
	Count                int      `json:"count"`
	Wins                 int      `json:"wins"`
	Losses               int      `json:"losses"`
	Breakeven            int      `json:"breakeven"`
	WinRate              *float64 `json:"winRate"`
	GrossWin             float64  `json:"grossWin"`
	GrossLoss            float64  `json:"grossLoss"`
	ProfitFactor         *float64 `json:"profitFactor"`
	ProfitFactorInfinite bool     `json:"profitFactorInfinite"`
	Expectancy           *float64 `json:"expectancy"`
	AvgWin               float64  `json:"avgWin"`
	AvgLoss              float64  `json:"avgLoss"`
	Fees                 float64  `json:"fees"`
	AvgHold              *float64 `json:"avgHold"`
	OpenCount            int      `json:"openCount"`
}

func metrics(trades []*Trade) KPI {
	var gw, gl, total, fees float64
	var wins, losses, be, holds, open int
	for _, t := range trades {
		v := t.PnlCad
		total += v
		if v > 0 {
			wins++
			gw += v
		} else if v < 0 {
			losses++
			gl += v
		} else {
			be++
		}
		fees += t.FeesCad
		holds += t.HoldDays
		if t.Status == "open" {
			open++
		}
	}
	gl = math.Abs(gl)
	n := len(trades)
	k := KPI{Realized: total, Count: n, Wins: wins, Losses: losses, Breakeven: be, GrossWin: gw, GrossLoss: gl, ProfitFactorInfinite: gl == 0 && gw > 0, Fees: fees, OpenCount: open}
	if n > 0 {
		k.WinRate = py.Ptr(float64(wins) / float64(n))
		k.Expectancy = py.Ptr(total / float64(n))
		k.AvgHold = py.Ptr(float64(holds) / float64(n))
	}
	if gl > 0 {
		k.ProfitFactor = py.Ptr(gw / gl)
	} else if !(gw > 0) {
		k.ProfitFactor = py.Ptr(0)
	}
	if wins > 0 {
		k.AvgWin = gw / float64(wins)
	}
	if losses > 0 {
		k.AvgLoss = -gl / float64(losses)
	}
	return k
}

type SymbolRow struct {
	Symbol   string   `json:"symbol"`
	Pnl      float64  `json:"pnl"`
	N        int      `json:"n"`
	Legs     int      `json:"legs"`
	WinRate  float64  `json:"winRate"`
	AvgHold  float64  `json:"avgHold"`
	TradeIDs []string `json:"tradeIds"`
}

func bySymbol(trades []*Trade) []SymbolRow {
	type agg struct {
		pnl                 float64
		n, wins, hold, legs int
		ids                 []string
	}
	by := map[string]*agg{}
	var order []string
	for _, t := range trades {
		k := t.Underlying
		g, ok := by[k]
		if !ok {
			g = &agg{ids: []string{}}
			by[k] = g
			order = append(order, k)
		}
		g.pnl += t.PnlCad
		g.n++
		g.legs += t.LegCount
		g.hold += t.HoldDays
		g.ids = append(g.ids, t.ID)
		if t.PnlCad > 0 {
			g.wins++
		}
	}
	rows := []SymbolRow{}
	for _, k := range order {
		g := by[k]
		row := SymbolRow{Symbol: k, Pnl: g.pnl, N: g.n, Legs: g.legs, TradeIDs: g.ids}
		if g.n > 0 {
			row.WinRate = float64(g.wins) / float64(g.n)
			row.AvgHold = float64(g.hold) / float64(g.n)
		}
		rows = append(rows, row)
	}
	sort.SliceStable(rows, func(i, j int) bool { return rows[i].Pnl > rows[j].Pnl })
	return rows
}

func monthLabel(key string) string {
	m := int(key[5]-'0')*10 + int(key[6]-'0')
	return Months[m-1] + " '" + key[2:4]
}

type MonthRow struct {
	Key      string   `json:"key"`
	Label    string   `json:"label"`
	Value    float64  `json:"value"`
	Count    int      `json:"count"`
	TradeIDs []string `json:"tradeIds"`
}

func monthly(trades []*Trade) []MonthRow {
	by := map[string]*MonthRow{}
	for _, t := range trades {
		k := cut(t.ExitDate, 7)
		if len(k) < 7 {
			continue
		}
		b, ok := by[k]
		if !ok {
			b = &MonthRow{Key: k, Label: monthLabel(k), TradeIDs: []string{}}
			by[k] = b
		}
		b.Value += t.PnlCad
		b.Count++
		b.TradeIDs = append(b.TradeIDs, t.ID)
	}
	out := []MonthRow{}
	for _, k := range sortedKeysOf(by) {
		out = append(out, *by[k])
	}
	return out
}

func sortedKeysOf[V any](m map[string]V) []string {
	return store.SortedKeys(m)
}

type GradeBucket struct {
	Grade    string   `json:"grade"`
	N        int      `json:"n"`
	Pnl      float64  `json:"pnl"`
	TradeIDs []string `json:"tradeIds"`
}

type GradeView struct {
	Buckets  []GradeBucket `json:"buckets"`
	Ungraded int           `json:"ungraded"`
	Graded   int           `json:"graded"`
}

func gradeBuckets(trades []*Trade) GradeView {
	buckets := []GradeBucket{}
	for _, g := range Grades {
		b := GradeBucket{Grade: g, TradeIDs: []string{}}
		for _, t := range trades {
			if t.Grade == g {
				b.N++
				b.Pnl += t.PnlCad
				b.TradeIDs = append(b.TradeIDs, t.ID)
			}
		}
		buckets = append(buckets, b)
	}
	ungraded := 0
	for _, t := range trades {
		if t.Grade == "" {
			ungraded++
		}
	}
	return GradeView{Buckets: buckets, Ungraded: ungraded, Graded: len(trades) - ungraded}
}

type QueueRow struct {
	ID       string  `json:"id"`
	Symbol   string  `json:"symbol"`
	Date     string  `json:"date"`
	Pnl      float64 `json:"pnl"`
	Currency string  `json:"currency"`
	Missing  string  `json:"missing"`
}

func reviewQueue(trades []*Trade) []QueueRow {
	out := []QueueRow{}
	for _, t := range trades {
		noGrade := t.Grade == ""
		noThesis := py.Strip(t.Thesis) == ""
		if !(noGrade || noThesis) {
			continue
		}
		missing := "no thesis"
		if noGrade && noThesis {
			missing = "no grade or thesis"
		} else if noGrade {
			missing = "no grade"
		}
		out = append(out, QueueRow{ID: t.ID, Symbol: t.Symbol, Date: t.ExitDate, Pnl: t.PnlCad, Currency: "CAD", Missing: missing})
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].Date > out[j].Date })
	return out
}

var schedules = []int{52, 26, 24, 12, 6, 4, 2, 1}

func PaymentsPerYear(dates []string) *int {
	seen := map[string]bool{}
	var days []string
	for _, d := range dates {
		s := cut(d, 10)
		if s != "" && !seen[s] {
			seen[s] = true
			days = append(days, s)
		}
	}
	if len(days) < 2 {
		return nil
	}
	sort.Strings(days)
	var gaps []int
	for i := 1; i < len(days); i++ {
		if g := daysBetween(days[i-1], days[i]); g > 0 {
			gaps = append(gaps, g)
		}
	}
	if len(gaps) > 3 {
		gaps = gaps[len(gaps)-3:]
	}
	if len(gaps) == 0 {
		return nil
	}
	sort.Ints(gaps)
	median := gaps[len(gaps)/2]
	perYear := 365.25 / float64(median)
	best := schedules[0]
	bestD := math.Abs(float64(best) - perYear)
	for _, s := range schedules[1:] {
		if d := math.Abs(float64(s) - perYear); d < bestD {
			best, bestD = s, d
		}
	}
	return &best
}

type CashHolding struct {
	ID           string   `json:"id"`
	Symbol       string   `json:"symbol"`
	Account      string   `json:"account"`
	Qty          float64  `json:"qty"`
	Per          *float64 `json:"per"`
	Freq         *int     `json:"freq"`
	FreqVerified bool     `json:"freqVerified"`
	RateSource   string   `json:"rateSource"`
	Cost         float64  `json:"cost"`
	Avg          float64  `json:"avg"`
	Last         float64  `json:"last"`
	PriceSource  string   `json:"priceSource"`
	YTD          float64  `json:"ytd"`
	TTM          float64  `json:"ttm"`
	All          float64  `json:"all"`
	NextExDate   string   `json:"nextExDate"`
	NextPayDate  string   `json:"nextPayDate"`
	ExPast       bool     `json:"exPast"`
	PayPast      bool     `json:"payPast"`
	Yob          *float64 `json:"yob"`
	Annual       *float64 `json:"annual"`
	Yoc          *float64 `json:"yoc"`
	CurrentYield *float64 `json:"currentYield"`
}

type CashMonth struct {
	Key   string  `json:"key"`
	Label string  `json:"label"`
	Value float64 `json:"value"`
	Count int     `json:"count"`
}

type CashflowView struct {
	Tiles          []map[string]any `json:"tiles"`
	Months         []CashMonth      `json:"months"`
	Holdings       []CashHolding    `json:"holdings"`
	Rows           []CashRow        `json:"rows"`
	Other          []CashRow        `json:"other"`
	Total          float64          `json:"total"`
	Count          int              `json:"count"`
	SkippedFilters []string         `json:"skippedFilters"`
	Interest       float64          `json:"interest"`
	Withholding    float64          `json:"withholding"`
}

type payRate struct {
	per, annual float64
	freq        int
	verified    bool
	source      string
}

func cashflowView(base *Base, f *Filters, positionsAll []*Position, marginUsed float64, hasMargin bool) CashflowView {
	today := base.Today
	L := &f.Lists
	accts := L.Account
	search := strings.ToUpper(f.Search)
	inScope := func(r *CashRow) bool {
		if len(accts) > 0 && !py.Contains(accts, r.Account) {
			return false
		}
		if search != "" && !strings.Contains(strings.ToUpper(r.Symbol), search) {
			return false
		}
		if len(L.Symbol) > 0 && !py.Contains(L.Symbol, r.Symbol) {
			return false
		}
		return inDateScope(f, today, r.Date)
	}
	everything := []CashRow{}
	for i := range base.Cashflow {
		if inScope(&base.Cashflow[i]) {
			everything = append(everything, base.Cashflow[i])
		}
	}
	recs := []CashRow{}
	for _, r := range everything {
		if r.Kind == "Dividend" {
			recs = append(recs, r)
		}
	}
	skipped := []string{}
	for _, kv := range []struct {
		k string
		v []string
	}{{"grade", L.Grade}, {"tag", L.Tag}, {"kind", L.Kind}, {"exchange", L.Exchange}, {"side", L.Side}, {"result", L.Result}} {
		if len(kv.v) > 0 {
			skipped = append(skipped, kv.k)
		}
	}
	for _, kv := range []struct {
		k string
		r RangeFilter
	}{{"price", f.Ranges.Price}, {"hold", f.Ranges.Hold}, {"pnl", f.Ranges.Pnl}, {"qty", f.Ranges.Qty}} {
		if kv.r.V != nil {
			skipped = append(skipped, kv.k)
		}
	}

	var keys []string
	type bucketT struct {
		sum float64
		n   int
	}
	bucket := map[string]*bucketT{}
	if len(recs) > 0 {
		first, last := cut(recs[0].Date, 7), cut(recs[0].Date, 7)
		for _, r := range recs {
			m := cut(r.Date, 7)
			if m < first {
				first = m
			}
			if m > last {
				last = m
			}
		}
		endDay := today
		if b := dateBounds(f, today); b != nil {
			endDay = minStr(b.to, today)
		} else if len(f.Years) > 0 {
			endDay = minStr(f.Years[len(f.Years)-1]+"-12-31", today)
		}
		if e := cut(endDay, 7); e > last {
			last = e
		}
		y, m := atoi(first[:4]), atoi(first[5:7])
		for {
			k := pad4(y) + "-" + pad2s(m)
			if k > last {
				break
			}
			keys = append(keys, k)
			bucket[k] = &bucketT{}
			m++
			if m > 12 {
				m = 1
				y++
			}
		}
	}
	for _, r := range recs {
		if b, ok := bucket[cut(r.Date, 7)]; ok {
			b.sum += r.AmountCad
			b.n++
		}
	}
	months := []CashMonth{}
	for _, k := range keys {
		months = append(months, CashMonth{Key: k, Label: monthLabel(k), Value: bucket[k].sum, Count: bucket[k].n})
	}

	payers := map[string]bool{}
	for _, r := range base.Cashflow {
		if r.Kind == "Dividend" {
			payers[r.Symbol] = true
		}
	}
	var held []*Position
	for _, p := range positionsAll {
		if !payers[p.Symbol] || p.Short {
			continue
		}
		if (len(accts) == 0 || py.Contains(accts, p.Account)) && (search == "" || strings.Contains(strings.ToUpper(p.Symbol), search)) {
			held = append(held, p)
		}
	}
	var forYoc []CashRow
	for _, r := range base.Cashflow {
		if r.Kind == "Dividend" && (len(accts) == 0 || py.Contains(accts, r.Account)) && (search == "" || strings.Contains(strings.ToUpper(r.Symbol), search)) {
			forYoc = append(forYoc, r)
		}
	}
	lastRec := today
	if len(recs) > 0 {
		lastRec = recs[0].Date
	}
	cutDt, ok := py.ParseDate(lastRec)
	if !ok {
		cutDt, _ = py.ParseDate(today)
	}
	cm := int(cutDt.Month()) - 11
	cy := cutDt.Year()
	for cm <= 0 {
		cm += 12
		cy--
	}
	cutKey := pad4(cy) + "-" + pad2s(cm)
	thisYear := cut(today, 4)

	sumFor := func(sym string, pred func(r *CashRow) bool) float64 {
		s := 0.0
		for i := range forYoc {
			r := &forYoc[i]
			if r.Symbol == sym && pred(r) {
				s += r.AmountCad
			}
		}
		return s
	}
	public := base.Distributions
	quotes := base.Quotes

	rateFor := func(sym string) *payRate {
		var declared []store.Distribution
		for _, d := range public[sym] {
			if d.ExDate <= today {
				declared = append(declared, d)
			}
		}
		if len(declared) > 0 {
			sort.SliceStable(declared, func(i, j int) bool { return declared[i].ExDate > declared[j].ExDate })
			per := declared[0].Amount
			var dates []string
			for _, d := range public[sym] {
				dates = append(dates, d.ExDate)
			}
			freq := PaymentsPerYear(dates)
			if per != 0 && freq != nil {
				return &payRate{per: per, freq: *freq, annual: per * float64(*freq), verified: true, source: "declared"}
			}
		}
		var rs []CashRow
		for _, r := range forYoc {
			if r.Symbol == sym && r.Per != nil && *r.Per != 0 {
				rs = append(rs, r)
			}
		}
		if len(rs) == 0 {
			return nil
		}
		sort.SliceStable(rs, func(i, j int) bool { return rs[i].Date > rs[j].Date })
		per := *rs[0].Per
		if per == 0 {
			return nil
		}
		var dates []string
		for _, r := range forYoc {
			if r.Symbol == sym {
				dates = append(dates, r.Date)
			}
		}
		freq := PaymentsPerYear(dates)
		verified := freq != nil
		fq := 12
		if verified {
			fq = *freq
		}
		return &payRate{per: per, freq: fq, annual: per * float64(fq), verified: verified, source: "payments"}
	}

	distributionDates := func(sym string) (string, string, bool, bool) {
		recs_ := append([]store.Distribution{}, public[sym]...)
		payOr := func(d store.Distribution) string {
			if p := cut(d.PayDate, 10); p != "" {
				return p
			}
			return d.ExDate
		}
		sort.SliceStable(recs_, func(i, j int) bool {
			a, b := payOr(recs_[i]), payOr(recs_[j])
			if a != b {
				return a < b
			}
			return recs_[i].ExDate < recs_[j].ExDate
		})
		var pick *store.Distribution
		for i := range recs_ {
			if payOr(recs_[i]) >= today {
				pick = &recs_[i]
				break
			}
		}
		if pick == nil && len(recs_) > 0 {
			pick = &recs_[len(recs_)-1]
		}
		var ex, pay string
		if pick != nil {
			ex, pay = pick.ExDate, cut(pick.PayDate, 10)
		} else {
			if q, ok := quotes[sym]; ok {
				ex = cut(q.ExDividendDate, 10)
			}
			var paid []string
			for _, r := range forYoc {
				if r.Symbol == sym {
					paid = append(paid, r.Date)
				}
			}
			sort.Strings(paid)
			if len(paid) > 0 {
				pay = paid[len(paid)-1]
			}
		}
		return ex, pay, ex != "" && ex < today, pay != "" && pay < today
	}

	lastPrice := func(p *Position) (float64, string) {
		var q *store.Quote
		if qq, ok := quotes[p.Symbol]; ok {
			q = &qq
		}
		if !QuoteFits(q, p.Kind) {
			q = nil
		}
		if q != nil && q.Price != nil && *q.Price != 0 && *q.Price > 0 {
			return *q.Price, "close"
		}
		return p.Last, "fill"
	}

	holdings := []CashHolding{}
	for _, p := range held {
		r := rateFor(p.Symbol)
		basis := p.Cost
		avg := p.Avg
		lastPx, priceSource := lastPrice(p)
		ex, pay, exPast, payPast := distributionDates(p.Symbol)
		h := CashHolding{
			ID: p.ID, Symbol: p.Symbol, Account: p.Account, Qty: p.Qty,
			Cost: basis, Avg: avg, Last: lastPx, PriceSource: priceSource,
			YTD:        sumFor(p.Symbol, func(x *CashRow) bool { return cut(x.Date, 4) == thisYear }),
			TTM:        sumFor(p.Symbol, func(x *CashRow) bool { return cut(x.Date, 7) >= cutKey }),
			All:        sumFor(p.Symbol, func(x *CashRow) bool { return true }),
			NextExDate: ex, NextPayDate: pay, ExPast: exPast, PayPast: payPast,
		}
		if r != nil {
			h.Per = py.Ptr(r.per)
			fq := r.freq
			h.Freq = &fq
			h.FreqVerified = r.verified
			h.RateSource = r.source
			h.Yob = py.Ptr(r.per * p.Qty)
			h.Annual = py.Ptr(r.annual * p.Qty)
			if avg != 0 {
				h.Yoc = py.Ptr(r.annual / avg)
			}
			if lastPx != 0 {
				h.CurrentYield = py.Ptr(r.annual / lastPx)
			}
		}
		holdings = append(holdings, h)
	}
	var basisAll, earnedAll, annualAll float64
	for _, h := range holdings {
		if h.Annual != nil {
			basisAll += h.Cost
			earnedAll += h.TTM
			annualAll += *h.Annual
		}
	}
	total := 0.0
	for _, r := range recs {
		total += r.AmountCad
	}
	thisYr := atoi(thisYear)
	tiles := []map[string]any{}
	for _, y := range []int{thisYr - 2, thisYr - 1, thisYr} {
		ys := pad4(y)
		sm, n := 0.0, 0
		for _, r := range recs {
			if cut(r.Date, 4) == ys {
				sm += r.AmountCad
				n++
			}
		}
		paid := 0
		for _, k := range keys {
			if cut(k, 4) == ys && bucket[k].n > 0 {
				paid++
			}
		}
		if paid == 0 {
			paid = 1
		}
		label := ys
		if y == thisYr {
			label = ys + " YTD"
		}
		tiles = append(tiles, map[string]any{"label": label, "total": sm, "perMonth": sm / float64(paid), "count": n})
	}
	monthsInScope := 0
	for _, k := range keys {
		if bucket[k].n > 0 {
			monthsInScope++
		}
	}
	if monthsInScope == 0 {
		monthsInScope = 1
	}
	tiles = append(tiles, map[string]any{"label": "All time", "total": total, "perMonth": total / float64(monthsInScope), "count": len(recs)})
	if hasMargin {
		chargeMonths := map[string]bool{}
		charged := 0.0
		for _, r := range everything {
			if r.Kind == "Interest charge" {
				chargeMonths[cut(r.Date, 7)] = true
				charged += -r.AmountCad
			}
		}
		perMonth := 0.0
		if len(chargeMonths) > 0 {
			perMonth = charged / float64(len(chargeMonths))
		}
		tiles = append(tiles, map[string]any{"label": "Margin used", "marginUsed": marginUsed, "interestPerMonth": perMonth, "interestMonths": len(chargeMonths)})
	} else {
		since := shiftDate(today, -365)
		sm, n := 0.0, 0
		paidMonths := map[string]bool{}
		for _, r := range recs {
			if since < r.Date && r.Date <= today {
				sm += r.AmountCad
				n++
				paidMonths[cut(r.Date, 7)] = true
			}
		}
		paid := len(paidMonths)
		if paid == 0 {
			paid = 1
		}
		tiles = append(tiles, map[string]any{"label": "Last 12 months", "total": sm, "perMonth": sm / float64(paid), "count": n})
	}
	var yield *float64
	if basisAll != 0 {
		yield = py.Ptr(annualAll / basisAll)
	}
	tiles = append(tiles, map[string]any{"label": "Yield on cost", "yield": yield, "projected": annualAll / 12, "earned": earnedAll, "book": basisAll})
	other := []CashRow{}
	interest, withholding := 0.0, 0.0
	for _, r := range everything {
		if r.Kind != "Dividend" {
			other = append(other, r)
			if r.Kind == "Interest" {
				interest += r.AmountCad
			} else if r.Kind == "Withholding tax" {
				withholding += r.AmountCad
			}
		}
	}
	return CashflowView{Tiles: tiles, Months: months, Holdings: holdings, Rows: recs, Other: other, Total: total, Count: len(recs), SkippedFilters: skipped, Interest: interest, Withholding: withholding}
}

func atoi(s string) int {
	n := 0
	for _, c := range s {
		if c < '0' || c > '9' {
			break
		}
		n = n*10 + int(c-'0')
	}
	return n
}

func pad4(n int) string {
	s := py.Itoa(n)
	for len(s) < 4 {
		s = "0" + s
	}
	return s
}

func pad2s(n int) string {
	if n < 10 {
		return "0" + py.Itoa(n)
	}
	return py.Itoa(n)
}

type Listing struct {
	Name     string `json:"name"`
	Exchange string `json:"exchange"`
	Kind     string `json:"kind"`
	Currency string `json:"currency"`
}

type ViewOptions struct {
	Accounts  []string            `json:"accounts"`
	Symbols   []string            `json:"symbols"`
	Listings  map[string]*Listing `json:"listings"`
	Tags      []string            `json:"tags"`
	Exchanges []string            `json:"exchanges"`
	Kinds     []string            `json:"kinds"`
	Grades    []string            `json:"grades"`
	Sides     []string            `json:"sides"`
	Results   []string            `json:"results"`
	Years     []string            `json:"years"`
}

type EquityView struct {
	Label      string        `json:"label"`
	Series     []EquityPoint `json:"series"`
	Drawdown   Drawdown      `json:"drawdown"`
	Annualized Annualized    `json:"annualized"`
}

type BenchmarkView struct {
	Key   string `json:"key"`
	Label string `json:"label"`
}

type MarketStamp struct {
	FxLast        string `json:"fxLast"`
	BenchmarkLast string `json:"benchmarkLast"`
}

type PositionsSummary struct {
	Count  int     `json:"count"`
	Book   float64 `json:"book"`
	MV     float64 `json:"mv"`
	Unreal float64 `json:"unreal"`
}

type View struct {
	OK               bool             `json:"ok"`
	Generated        string           `json:"generated"`
	Today            string           `json:"today"`
	SyncedAt         string           `json:"syncedAt"`
	Currency         string           `json:"currency"`
	Market           MarketStamp      `json:"market"`
	Filters          Filters          `json:"filters"`
	Options          ViewOptions      `json:"options"`
	KPI              KPI              `json:"kpi"`
	Equity           EquityView       `json:"equity"`
	Years            []YearRow        `json:"years"`
	Benchmark        BenchmarkView    `json:"benchmark"`
	Monthly          []MonthRow       `json:"monthly"`
	BySymbol         []SymbolRow      `json:"bySymbol"`
	Grades           GradeView        `json:"grades"`
	Queue            []QueueRow       `json:"queue"`
	Trades           []*Trade         `json:"trades"`
	TradeCount       int              `json:"tradeCount"`
	TradeTotal       int              `json:"tradeTotal"`
	Positions        []*Position      `json:"positions"`
	PositionsSummary PositionsSummary `json:"positionsSummary"`
	Portfolio        PortfolioView    `json:"portfolio"`
	Markets          MarketsView      `json:"markets"`
	Cashflow         CashflowView     `json:"cashflow"`
	Unmatched        []Unmatched      `json:"unmatched"`
	Accounts         []AccountRow     `json:"accounts"`
	ActivityCount    int              `json:"activityCount"`
}

func sortedSet(m map[string]bool) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}

func BuildView(base *Base, filters any) *View {
	f := CleanFilters(filters)
	today := base.Today
	tradesAll := base.Trades
	trades := []*Trade{}
	for _, t := range tradesAll {
		if tradeMatches(t, &f, today) {
			trades = append(trades, t)
		}
	}
	positionsAll := base.Positions
	positions := []*Position{}
	for _, p := range positionsAll {
		if positionMatches(p, &f) {
			positions = append(positions, p)
		}
	}
	accts := f.Lists.Account
	series := base.Equity
	seriesLabel := "All accounts"
	if len(accts) == 1 {
		if s, ok := base.EquityByAccount[accts[0]]; ok {
			series = s
			seriesLabel = accts[0]
		}
	}
	benchKey := f.Benchmark
	years := YearlyReturns(series, base.Benchmarks[benchKey], today)
	ann := AnnualizedOf(years)
	dd := DrawdownOf(series)
	b := dateBounds(&f, today)
	portfolio := portfolioView(base, &f, positions)
	shown := series
	if b != nil {
		shown = []EquityPoint{}
		for _, p := range series {
			if b.from <= p.D && p.D <= b.to {
				shown = append(shown, p)
			}
		}
	} else if len(f.Years) > 0 {
		shown = []EquityPoint{}
		for _, p := range series {
			if py.Contains(f.Years, cut(p.D, 4)) {
				shown = append(shown, p)
			}
		}
	}
	if len(shown) > 0 {
		peak := maxV(shown)
		firstIdx := 0
		for i, p := range shown {
			if p.V > peak*0.01 {
				firstIdx = i
				break
			}
		}
		shown = shown[firstIdx:]
	}
	if shown == nil {
		shown = []EquityPoint{}
	}

	tagSet := map[string]bool{}
	symbolSet := map[string]bool{}
	accountSet := map[string]bool{}
	exchangeSet := map[string]bool{}
	yearSet := map[string]bool{}
	listings := map[string]*Listing{}
	for _, t := range tradesAll {
		for _, tag := range t.Tags {
			tagSet[tag] = true
		}
		symbolSet[t.Symbol] = true
		accountSet[t.Account] = true
		if t.Exchange != "" {
			exchangeSet[t.Exchange] = true
		}
		if t.ExitDate != "" {
			yearSet[cut(t.ExitDate, 4)] = true
		}
		addListing(listings, t.Symbol, t.Name, t.Exchange, t.Kind, t.Currency)
	}
	for _, p := range positionsAll {
		symbolSet[p.Symbol] = true
		accountSet[p.Account] = true
		if p.Exchange != "" {
			exchangeSet[p.Exchange] = true
		}
		addListing(listings, p.Symbol, p.Name, p.Exchange, p.Kind, p.Currency)
	}
	for _, r := range base.Cashflow {
		accountSet[r.Account] = true
	}
	kinds := []string{}
	for _, k := range Kinds {
		found := false
		for _, t := range tradesAll {
			if t.Kind == k {
				found = true
				break
			}
		}
		if !found {
			for _, p := range positionsAll {
				if p.Kind == k {
					found = true
					break
				}
			}
		}
		if found {
			kinds = append(kinds, k)
		}
	}
	yearOptions := sortedSet(yearSet)
	sort.Sort(sort.Reverse(sort.StringSlice(yearOptions)))

	book, mv, unreal := 0.0, 0.0, 0.0
	for _, p := range positions {
		book += math.Abs(p.Cost)
		mv += signedMV(p)
		unreal += p.Unreal
	}
	grades := append(append([]string{}, Grades...), "Ungraded")
	return &View{
		OK:        true,
		Generated: py.NowStamp(),
		Today:     today,
		SyncedAt:  base.SyncedAt,
		Currency:  "CAD",
		Market:    MarketStamp{base.FxLast, base.BenchmarkLast},
		Filters:   f,
		Options: ViewOptions{
			Accounts:  sortedSet(accountSet),
			Symbols:   sortedSet(symbolSet),
			Listings:  listings,
			Tags:      sortedSet(tagSet),
			Exchanges: sortedSet(exchangeSet),
			Kinds:     kinds,
			Grades:    grades,
			Sides:     []string{"SELL", "COVER"},
			Results:   []string{"Winners", "Losers", "Breakeven"},
			Years:     yearOptions,
		},
		KPI:              metrics(trades),
		Equity:           EquityView{Label: seriesLabel, Series: shown, Drawdown: dd, Annualized: ann},
		Years:            years,
		Benchmark:        BenchmarkView{benchKey, BenchmarkLabels[benchKey]},
		Monthly:          monthly(trades),
		BySymbol:         bySymbol(trades),
		Grades:           gradeBuckets(trades),
		Queue:            reviewQueue(trades),
		Trades:           trades,
		TradeCount:       len(trades),
		TradeTotal:       len(tradesAll),
		Positions:        positions,
		PositionsSummary: PositionsSummary{len(positions), book, mv, unreal},
		Portfolio:        portfolio,
		Markets:          marketsView(base, positions),
		Cashflow:         cashflowView(base, &f, positionsAll, portfolio.MarginUsed, portfolio.HasMargin),
		Unmatched:        base.Unmatched,
		Accounts:         base.Accounts,
		ActivityCount:    base.ActivityCount,
	}
}

func addListing(listings map[string]*Listing, symbol, name, exchange, kind, currency string) {
	cur, ok := listings[symbol]
	if !ok {
		cur = &Listing{Kind: kind, Currency: currency}
		listings[symbol] = cur
	}
	if cur.Name == "" && name != "" && name != symbol {
		cur.Name = name
	}
	if cur.Exchange == "" && exchange != "" {
		cur.Exchange = exchange
	}
}
