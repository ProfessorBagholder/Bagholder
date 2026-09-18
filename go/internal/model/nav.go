package model

import (
	"math"
	"sort"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

type YearRow struct {
	Year string   `json:"year"`
	R    float64  `json:"r"`
	Days int      `json:"days"`
	From string   `json:"from"`
	To   string   `json:"to"`
	Flow *float64 `json:"flow"`
	EndV *float64 `json:"endV"`
	SpR  *float64 `json:"spR"`
}

type Annualized struct {
	Rate  *float64 `json:"rate"`
	Years float64  `json:"years"`
	Count int      `json:"count"`
	First string   `json:"first"`
	Last  string   `json:"last"`
}

type Drawdown struct {
	Pct    *float64 `json:"pct"`
	Abs    *float64 `json:"abs"`
	At     string   `json:"at"`
	PeakAt string   `json:"peakAt"`
}

func EquitySeries(points []store.NavPoint) []EquityPoint {
	out := []EquityPoint{}
	for _, p := range points {
		d := cut(p.Date, 10)
		if d == "" || math.IsNaN(p.Equity) {
			continue
		}
		out = append(out, EquityPoint{D: d, V: p.Equity, Dep: copyPtr(p.NetDeposits)})
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].D < out[j].D })
	return out
}

func navOn(series []EquityPoint, day string) *float64 {
	var v *float64
	for i := range series {
		if series[i].D > day {
			break
		}
		v = py.Ptr(series[i].V)
	}
	return v
}

func depositsOn(series []EquityPoint, day string) *float64 {
	var v *float64
	for i := range series {
		p := &series[i]
		if p.D > day {
			break
		}
		if p.Dep != nil {
			v = copyPtr(p.Dep)
		}
	}
	return v
}

type yearSpan struct {
	r        float64
	from, to string
	days     int
}

func maxV(series []EquityPoint) float64 {
	m := math.Inf(-1)
	for _, p := range series {
		if p.V > m {
			m = p.V
		}
	}
	return m
}

func minStr(a, b string) string {
	if b < a {
		return b
	}
	return a
}

func yearReturn(series []EquityPoint, year, today string) *yearSpan {
	cal := year + "-01-01"
	to := minStr(year+"-12-31", today)
	if len(series) == 0 {
		return nil
	}
	floor := maxV(series) * 0.01
	startDay := shiftDate(cal, -1)
	start := navOn(series, startDay)
	after := startDay
	if !(start != nil && *start > floor) {
		var first *EquityPoint
		for i := range series {
			p := &series[i]
			if cal <= p.D && p.D <= to && p.V > floor {
				first = p
				break
			}
		}
		if first == nil {
			return nil
		}
		start = py.Ptr(first.V)
		after = first.D
	}
	var pts []EquityPoint
	for _, p := range series {
		if after < p.D && p.D <= to {
			pts = append(pts, p)
		}
	}
	if len(pts) == 0 {
		return nil
	}
	prevEq := *start
	prevDep := depositsOn(series, after)
	factor := 1.0
	for _, p := range pts {
		eq := p.V
		if !(prevEq > 0) {
			return nil
		}
		cf := 0.0
		if p.Dep != nil && prevDep != nil {
			cf = *p.Dep - *prevDep
		}
		factor *= 1 + (eq-prevEq-cf)/prevEq
		prevEq = eq
		if p.Dep != nil {
			prevDep = copyPtr(p.Dep)
		}
	}
	r := factor - 1
	if math.IsNaN(r) || math.IsInf(r, 0) {
		return nil
	}
	spanFrom := after
	if after == startDay {
		spanFrom = cal
	}
	return &yearSpan{r: r, from: spanFrom, to: to, days: daysBetween(spanFrom, to)}
}

func benchmarkReturn(bench map[string]float64, year, today, start string) *float64 {
	if len(bench) == 0 {
		return nil
	}
	days := store.SortedKeys(bench)
	cal := cut(start, 10)
	if cal == "" {
		cal = year + "-01-01"
	}
	to := minStr(year+"-12-31", today)
	var prev, end *float64
	for _, d := range days {
		if d < cal {
			prev = py.Ptr(bench[d])
		} else if d <= to {
			end = py.Ptr(bench[d])
		}
	}
	if prev == nil {
		for _, d := range days {
			if cal <= d && d <= to {
				prev = py.Ptr(bench[d])
				break
			}
		}
		if prev == nil {
			return nil
		}
	}
	if *prev == 0 || end == nil {
		return nil
	}
	return py.Ptr(*end / *prev - 1)
}

func YearlyReturns(series []EquityPoint, bench map[string]float64, today string) []YearRow {
	out := []YearRow{}
	if len(series) == 0 {
		return out
	}
	seen := map[string]bool{}
	var years []string
	for _, p := range series {
		y := cut(p.D, 4)
		if !seen[y] {
			seen[y] = true
			years = append(years, y)
		}
	}
	sort.Strings(years)
	peak := maxV(series)
	for _, y := range years {
		yearPeak := 0.0
		first := true
		for _, p := range series {
			if cut(p.D, 4) == y {
				if first || p.V > yearPeak {
					yearPeak = p.V
					first = false
				}
			}
		}
		if peak > 0 && yearPeak < peak*0.01 {
			continue
		}
		yr := yearReturn(series, y, today)
		if yr == nil {
			continue
		}
		startDep := depositsOn(series, shiftDate(y+"-01-01", -1))
		endDep := depositsOn(series, yr.to)
		var flow *float64
		if startDep != nil && endDep != nil {
			flow = py.Ptr(*endDep - *startDep)
		}
		endV := navOn(series, yr.to)
		spStart := ""
		if yr.from != y+"-01-01" {
			spStart = yr.from
		}
		out = append(out, YearRow{Year: y, R: yr.r, Days: yr.days, From: yr.from, To: yr.to, Flow: flow, EndV: endV, SpR: benchmarkReturn(bench, y, today, spStart)})
	}
	return out
}

func AnnualizedOf(years []YearRow) Annualized {
	prod := 1.0
	days := 0
	var used []string
	for _, y := range years {
		if y.R <= -1 || y.Days < 30 {
			continue
		}
		prod *= 1 + y.R
		days += y.Days
		used = append(used, y.Year)
	}
	if days == 0 {
		return Annualized{Rate: nil, Years: 0, Count: 0, First: "", Last: ""}
	}
	yrs := float64(days) / 365.25
	var rate float64
	if yrs >= 1.0/12 {
		rate = math.Pow(prod, 1/yrs) - 1
	} else {
		rate = prod - 1
	}
	return Annualized{Rate: py.Ptr(rate), Years: yrs, Count: len(used), First: used[0], Last: used[len(used)-1]}
}

func pairedFlows(series []EquityPoint) []float64 {
	n := len(series)
	flows := make([]float64, n)
	for i := 1; i < n; i++ {
		p, prev := series[i], series[i-1]
		if p.Dep == nil || prev.Dep == nil {
			continue
		}
		cf := *p.Dep - *prev.Dep
		if math.Abs(cf) < EPS {
			continue
		}
		changeToday := p.V - prev.V
		if i+1 < n {
			changeNext := series[i+1].V - p.V
			if math.Abs(changeToday-cf) > math.Abs(changeNext-cf) && math.Abs(changeToday) < math.Abs(cf)*0.5 {
				flows[i+1] += cf
				continue
			}
		}
		flows[i] += cf
	}
	return flows
}

func DrawdownOf(series []EquityPoint) Drawdown {
	if len(series) == 0 {
		return Drawdown{}
	}
	peakV := maxV(series)
	floor := peakV * 0.01
	idx := 1.0
	var prev *EquityPoint
	peakIdx := 0.0
	peakAt := ""
	peakEquity := 0.0
	dd, ddAbs := 0.0, 0.0
	ddAt, ddPeakAt := "", ""
	flows := pairedFlows(series)
	for i := range series {
		p := &series[i]
		if prev != nil && prev.V > floor && prev.V > 0 {
			idx *= 1 + (p.V-prev.V-flows[i])/prev.V
		}
		prev = p
		if p.V < floor {
			continue
		}
		if idx >= peakIdx {
			peakIdx = idx
			peakAt = p.D
			peakEquity = p.V
		}
		if peakIdx <= 0 {
			continue
		}
		drop := idx/peakIdx - 1
		if drop < dd {
			dd = drop
			ddAbs = drop * peakEquity
			ddAt = p.D
			ddPeakAt = peakAt
		}
	}
	return Drawdown{Pct: py.Ptr(dd), Abs: py.Ptr(ddAbs), At: ddAt, PeakAt: ddPeakAt}
}
