package model

import (
	"math"
	"regexp"
	"sort"
	"strings"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
	"github.com/ProfessorBagholder/Bagholder/internal/symbols"
)

func compact(s string) string { return symbols.Compact(s) }

func normAccountName(s string) string { return symbols.NormAccountName(s) }

func optionRight(symbol string) string { return symbols.Right(symbol) }

func optionExpiry(symbol string) string { return symbols.Expiry(symbol) }

func isMultileg(a *Act) bool { return strings.Contains(compact(a.RawType), "MULTILEG") }

type rollKey struct{ account, underlying, right string }

func rollKeyOf(a *Act) rollKey {
	return rollKey{fifoAccount(a), underlying(a.Symbol), optionRight(a.Symbol)}
}

func daysBetween(a, b string) int {
	da, oka := py.ParseDate(a)
	db, okb := py.ParseDate(b)
	if !oka || !okb {
		return 0
	}
	d := int(db.Sub(da).Hours() / 24)
	if d < 0 {
		return 0
	}
	return d
}

func shiftDate(iso string, days int) string {
	t, ok := py.ParseDate(iso)
	if !ok {
		if len(iso) > 10 {
			return iso[:10]
		}
		return iso
	}
	return py.DateStr(t.AddDate(0, 0, days))
}

func whenParts(occurred string) (string, string) {
	s := py.Strip(occurred)
	if s == "" {
		return "", ""
	}
	if !strings.Contains(s, "T") {
		return cut(s, 10), ""
	}
	if strings.HasSuffix(s, "Z") {
		s = s[:len(s)-1] + "+00:00"
	}
	t, naive, ok := py.ParseISO(s)
	if !ok {
		return cut(s, 10), ""
	}
	if naive {
		t = time.Date(t.Year(), t.Month(), t.Day(), t.Hour(), t.Minute(), t.Second(), t.Nanosecond(), time.UTC)
	}
	loc := t.In(TimeTZ)
	return loc.Format("2006-01-02"), loc.Format("15:04")
}

func cut(s string, n int) string {
	if len(s) > n {
		return s[:n]
	}
	return s
}

func fmt8(v float64) string { return py.Fixed(v, 8) }

func isCryptoActivity(a *Act) bool {
	return strings.HasPrefix(compact(a.RawType), "CRYPTO") || strings.HasPrefix(compact(a.ActivityType), "CRYPTO")
}

func kindOf(a *Act) string {
	if py.Contains(Kinds, a.Kind) {
		return a.Kind
	}
	if isCryptoActivity(a) {
		return "Crypto"
	}
	if isOption(a.Symbol) {
		return "Options"
	}
	return "Shares"
}

func isIntentionalOpen(a *Act) bool {
	for _, f := range []string{compact(a.ActivityType), compact(a.ActivitySubType)} {
		if strings.Contains(f, "TOOPEN") {
			return true
		}
	}
	for _, f := range []string{compact(a.ActivityType), compact(a.ActivitySubType)} {
		if f == "STO" || f == "BTO" {
			return true
		}
	}
	return false
}

func isCloseOnly(a *Act) bool {
	fields := []string{compact(a.ActivityType), compact(a.ActivitySubType)}
	for _, f := range fields {
		if strings.Contains(f, "TOCLOSE") || f == "BTC" || f == "STC" {
			return true
		}
	}
	for _, f := range fields {
		if strings.Contains(f, "EXPIR") || strings.Contains(f, "ASSIGN") || strings.Contains(f, "EXERCISE") {
			return true
		}
	}
	return false
}

func openingDirection(a *Act, side string) string {
	if side == "BUY" {
		if isCloseOnly(a) {
			return ""
		}
		return "LONG"
	}
	if side == "SELL" {
		if isOption(a.Symbol) {
			if isCloseOnly(a) {
				return ""
			}
			return "SHORT"
		}
		if isIntentionalOpen(a) {
			return "SHORT"
		}
		return ""
	}
	return ""
}

func NormalizeActivity(activity *Act) *Act {
	c := activity.Clone()
	a := &c
	a.Normalized = true
	a.AccountType = normAccountName(a.AccountType)
	rt := compact(a.RawType)
	at := compact(a.ActivityType)
	cash := a.NetCashAmount
	qty := math.Abs(a.Quantity)
	a.Flags = []string{}

	if rt == "CRYPTOBUY" || at == "CRYPTOBUY" {
		a.Category, a.ActivityType, a.ActivitySubType, a.Kind = "trade", "Trade", "BUY", "Crypto"
		a.Quantity = qty
		a.NetCashAmount = -math.Abs(cash)
		return a
	}
	if rt == "CRYPTOSELL" || at == "CRYPTOSELL" {
		a.Category, a.ActivityType, a.ActivitySubType, a.Kind = "trade", "Trade", "SELL", "Crypto"
		a.Quantity = -qty
		a.NetCashAmount = math.Abs(cash)
		return a
	}
	if rt == "CRYPTOTRANSFER" || at == "CRYPTOTRANSFER" {
		sub := compact(a.ActivitySubType)
		a.Category, a.ActivityType, a.Kind = "trade", "Trade", "Crypto"
		a.Flags = append(a.Flags, "transfer")
		if strings.Contains(sub, "OUT") || cash < 0 {
			a.ActivitySubType = "SELL"
			a.Flags = append(a.Flags, "transfer-out")
			a.Quantity = -qty
			a.NetCashAmount = math.Abs(cash)
		} else {
			a.ActivitySubType = "BUY"
			a.Quantity = qty
			a.NetCashAmount = -math.Abs(cash)
		}
		return a
	}
	if rt == "CRYPTOSTAKINGREWARD" || at == "CRYPTOSTAKINGREWARD" {
		a.Category, a.ActivityType, a.ActivitySubType, a.Kind = "trade", "Trade", "BUY", "Crypto"
		a.Flags = append(a.Flags, "reward")
		a.Quantity = qty
		a.UnitPrice = 0
		a.NetCashAmount = 0
		return a
	}
	if strings.HasPrefix(rt, "CRYPTO") {
		a.Category = "other"
		a.Kind = "Crypto"
		return a
	}

	if at == "STKDIS" && rt == "DIVIDEND" && math.Abs(cash) < EPS {
		a.Category = "other"
		a.Flags = append(a.Flags, "pending-distribution")
		return a
	}

	raw := rt + at
	if strings.Contains(raw, "MULTILEG") {
		a.Category = "trade"
		if cash < 0 || compact(a.Direction) == "DEBIT" {
			a.ActivityType = "OPTIONS_BUY"
			a.ActivitySubType = "BUYTOCLOSE"
		} else {
			a.ActivityType = "OPTIONS_SELL"
			a.ActivitySubType = "SELLTOOPEN"
		}
	} else if strings.Contains(raw, "EXPIR") || strings.Contains(raw, "ASSIGN") || strings.Contains(raw, "EXERCISE") {
		a.Category = "option_event"
		if strings.Contains(raw, "ASSIGN") {
			a.ActivityType = "ASSIGN"
			a.ActivitySubType = "BUYTOCLOSE"
			a.UnitPrice = 0
		} else if strings.Contains(raw, "SHORTEXPIR") {
			a.ActivityType = "EXPIR"
			a.ActivitySubType = "BUY"
		} else if strings.Contains(raw, "EXPIR") {
			a.ActivityType = "EXPIR"
			a.ActivitySubType = "SELL"
		} else {
			a.ActivityType = "EXERCISE"
			a.ActivitySubType = "SELL"
		}
		if strings.Contains(raw, "ASSIGN") || math.Abs(cash) < 1e-12 {
			a.UnitPrice = 0
		}
		if qty > 0 {
			if a.ActivitySubType == "SELL" {
				a.Quantity = -qty
			} else {
				a.Quantity = qty
			}
		}
	}
	a.Kind = kindOf(a)
	return a
}

func NormalizeActivities(activities []store.Activity) []*Act {
	out := make([]*Act, 0, len(activities))
	for i := range activities {
		out = append(out, NormalizeActivity(&activities[i]))
	}
	return out
}

type stkdisKey struct{ symbol, date, currency string }

func foldStkdis(activities []*Act) []*Act {
	rest := make([]*Act, 0, len(activities))
	type group struct {
		pos, neg float64
		sample   *Act
	}
	groups := map[stkdisKey]*group{}
	var order []stkdisKey
	for _, a := range activities {
		if compact(a.ActivityType) != "STKDIS" {
			rest = append(rest, a)
			continue
		}
		k := stkdisKey{a.Symbol, a.TransactionDate, a.Currency}
		g, ok := groups[k]
		if !ok {
			g = &group{sample: a}
			groups[k] = g
			order = append(order, k)
		}
		q := a.Quantity
		if a.ActivitySubType == "SELL" || q < 0 {
			g.neg += math.Abs(q)
		} else {
			g.pos += math.Abs(q)
		}
	}
	for _, k := range order {
		g := groups[k]
		net := g.pos - g.neg
		if net > EPS {
			c := g.sample.Clone()
			c.Quantity = net
			c.ActivitySubType = "BUY"
			c.UnitPrice = 0
			c.NetCashAmount = 0
			c.Category = "trade"
			rest = append(rest, &c)
		}
	}
	return rest
}

type acctSym struct{ account, symbol string }

type splitKey struct{ account, symbol, day string }

func splitMarkers(activities []*Act) map[splitKey]float64 {
	out := map[splitKey]float64{}
	byBook := map[acctSym][]*Act{}
	for _, a := range activities {
		if (a.Category != "trade" && a.Category != "option_event") || a.Symbol == "" {
			continue
		}
		k := acctSym{fifoAccount(a), a.Symbol}
		byBook[k] = append(byBook[k], a)
	}
	for _, a := range activities {
		if compact(a.ActivityType) != "STKDIS" || compact(a.RawType) != "CORPORATEACTION" {
			continue
		}
		if math.Abs(a.Quantity) > EPS {
			continue
		}
		day := a.TransactionDate
		key := acctSym{fifoAccount(a), a.Symbol}
		var priced []*Act
		for _, x := range byBook[key] {
			if x.UnitPrice > 0 && compact(x.ActivityType) != "STKDIS" {
				priced = append(priced, x)
			}
		}
		sort.SliceStable(priced, func(i, j int) bool {
			if priced[i].TransactionDate != priced[j].TransactionDate {
				return priced[i].TransactionDate < priced[j].TransactionDate
			}
			return priced[i].OccurredAt < priced[j].OccurredAt
		})
		var before, after []float64
		for _, x := range priced {
			if x.TransactionDate < day {
				before = append(before, x.UnitPrice)
			} else {
				after = append(after, x.UnitPrice)
			}
		}
		if len(before) > 3 {
			before = before[len(before)-3:]
		}
		if len(after) > 3 {
			after = after[:3]
		}
		if len(before) == 0 || len(after) == 0 {
			continue
		}
		sort.Float64s(before)
		sort.Float64s(after)
		pre := before[len(before)/2]
		post := after[len(after)/2]
		if !(pre > 0) || !(post > 0) {
			continue
		}
		ratio := post / pre
		var n int
		var factor float64
		if ratio >= 1.5 {
			n = py.RoundInt(ratio)
			factor = 1.0 / float64(n)
		} else if ratio <= 1/1.5 {
			n = py.RoundInt(1 / ratio)
			factor = float64(n)
		} else {
			continue
		}
		if n < 2 || math.Abs(ratio-(1/factor))/(1/factor) > 0.35 {
			continue
		}
		out[splitKey{fifoAccount(a), a.Symbol, day}] = factor
	}
	return out
}

func fifoAccount(a *Act) string {
	nick := normAccountName(a.AccountType)
	if nick != "" {
		return nick
	}
	if a.FifoID != "" {
		return a.FifoID
	}
	return a.AccountID
}

func bookKey(a *Act) string {
	return fifoAccount(a) + "::" + a.Symbol + "::" + a.Currency
}

var removalRE = regexp.MustCompile(`CODECHANGE|SYMBOLCHANGE|TICKERCHANGE|LISTINGSTATUS|SECURITYSWAP`)

type bookID struct{ account, symbol, currency string }

type replacementIndex struct {
	removed map[bookID]string
	trades  map[bookID][]string
}

func buildReplacementIndex(activities []*Act) *replacementIndex {
	idx := &replacementIndex{removed: map[bookID]string{}, trades: map[bookID][]string{}}
	for _, a := range activities {
		key := bookID{fifoAccount(a), a.Symbol, a.Currency}
		t := compact(a.ActivityType)
		d := a.TransactionDate
		if t == "STKDIS" {
			sub := compact(a.ActivitySubType)
			if sub == "SELL" || a.Quantity < 0 {
				if cur, ok := idx.removed[key]; d != "" && (!ok || d < cur) {
					idx.removed[key] = d
				}
			}
			continue
		}
		raw := compact(a.RawType) + compact(a.AftType)
		if removalRE.MatchString(raw) {
			if cur, ok := idx.removed[key]; d != "" && (!ok || d < cur) {
				idx.removed[key] = d
			}
		}
		if (a.Category == "trade" || a.Category == "option_event") && store.TradeSide(a) != "" {
			idx.trades[key] = append(idx.trades[key], d)
		}
	}
	return idx
}

func (idx *replacementIndex) tickerWasReplaced(account, symbol, currency, byDate string) bool {
	key := bookID{account, symbol, currency}
	removedOn, ok := idx.removed[key]
	if !ok || removedOn == "" || removedOn > byDate {
		return false
	}
	for _, d := range idx.trades[key] {
		if d > removedOn {
			return false
		}
	}
	return true
}

type fill struct {
	a             *Act
	side          string
	qty           float64
	rollDirection string
	rtBefore      string
}

func setFillSide(f *fill, side, sub string) {
	f.side = side
	f.a.ActivitySubType = sub
	q := math.Abs(f.a.Quantity)
	if q > 0 {
		if side == "SELL" {
			f.a.Quantity = -q
		} else {
			f.a.Quantity = q
		}
	}
}

type dirs struct{ long, short float64 }

func (d *dirs) at(direction string) *float64 {
	if direction == "SHORT" {
		return &d.short
	}
	return &d.long
}

func resolveOptionFillSide(f *fill, rem *dirs) {
	a := f.a
	raw := compact(a.RawType) + compact(a.ActivityType)
	expirish := strings.Contains(raw, "EXPIR") || strings.Contains(raw, "ASSIGN") || strings.Contains(raw, "EXERCISE")
	if expirish {
		if strings.Contains(raw, "ASSIGN") || strings.Contains(raw, "SHORTEXPIR") {
			sub := "BUY"
			if strings.Contains(raw, "ASSIGN") {
				sub = "BUYTOCLOSE"
			}
			setFillSide(f, "BUY", sub)
		} else if strings.Contains(raw, "EXPIR") && !strings.Contains(raw, "SHORT") {
			setFillSide(f, "SELL", "SELL")
		} else if f.side == "BUY" && rem.long > EPS && rem.short <= EPS {
			setFillSide(f, "SELL", "SELL")
		} else if f.side == "SELL" && rem.short > EPS && rem.long <= EPS {
			setFillSide(f, "BUY", "BUY")
		}
		return
	}
	if !(strings.Contains(raw, "MULTILEG") || isCloseOnly(a)) {
		return
	}
	if f.side == "BUY" {
		if rem.short > EPS {
			a.ActivitySubType = "BUYTOCLOSE"
		} else {
			a.ActivitySubType = "BUYTOOPEN"
		}
	} else if f.side == "SELL" {
		if rem.long > EPS {
			a.ActivitySubType = "SELLTOCLOSE"
		} else {
			a.ActivitySubType = "SELLTOOPEN"
		}
	}
}

func isCleanOptionQty(cash, qty float64) bool {
	if !(qty > 0) {
		return false
	}
	px := math.Abs(cash) / (qty * 100.0)
	if px < 0 {
		return false
	}
	if math.Abs(px*100-math.RoundToEven(px*100)) < 1e-6 {
		return true
	}
	if math.Abs(px*10000-math.RoundToEven(px*10000)) < 1e-4 {
		return true
	}
	return false
}

func inferStandaloneOptionQty(cash float64) float64 {
	absCash := math.Abs(cash)
	if !(absCash > 0) {
		return 0
	}
	maxQty := py.RoundInt(absCash)
	if maxQty < 1 {
		maxQty = 1
	}
	if maxQty > 10000 {
		maxQty = 10000
	}
	for q := 1; q <= maxQty; q++ {
		if isCleanOptionQty(absCash, float64(q)) {
			return float64(q)
		}
	}
	return 1
}

func pickCleanQty(cash, openSz float64) float64 {
	picked := 0.0
	cap := int(openSz + 1e-9)
	if cap < 1 {
		cap = 1
	}
	for q := 1; q <= cap; q++ {
		if isCleanOptionQty(cash, float64(q)) {
			picked = float64(q)
			break
		}
	}
	qty := picked
	if qty == 0 {
		qty = inferStandaloneOptionQty(cash)
	}
	if qty > openSz {
		qty = openSz
	}
	return qty
}

func inferZeroQtyOptionFills(fills []*fill) {
	remaining := map[string]*dirs{}
	remOf := func(a *Act) *dirs {
		k := bookKey(a)
		r, ok := remaining[k]
		if !ok {
			r = &dirs{}
			remaining[k] = r
		}
		return r
	}
	zerosByBook := map[string][]int{}
	pools := map[rollKey]*dirs{}
	poolOf := func(a *Act) *dirs {
		k := rollKeyOf(a)
		p, ok := pools[k]
		if !ok {
			p = &dirs{}
			pools[k] = p
		}
		return p
	}
	upcoming := func(k string, i int) int {
		n := 0
		for _, j := range zerosByBook[k] {
			if j > i {
				n++
			}
		}
		return n
	}

	for i, f := range fills {
		a := f.a
		qty := math.Abs(a.Quantity)
		cash := a.NetCashAmount
		if !isOption(a.Symbol) || f.side == "" {
			continue
		}
		if qty == 0 && math.Abs(cash) > 1e-9 {
			k := bookKey(a)
			zerosByBook[k] = append(zerosByBook[k], i)
		}
	}

	for i, f := range fills {
		a := f.a
		if f.side == "" {
			continue
		}
		rem := remOf(a)
		if isOption(a.Symbol) && isMultileg(a) {
			pool := poolOf(a)
			var direction string
			switch {
			case rem.short > EPS:
				direction = "SHORT"
			case rem.long > EPS:
				direction = "LONG"
			case pool.short >= pool.long:
				direction = "SHORT"
			default:
				direction = "LONG"
			}
			openSz := *rem.at(direction) + *pool.at(direction)
			qty := math.Abs(a.Quantity)
			cash := a.NetCashAmount
			if qty == 0 {
				k := bookKey(a)
				if *rem.at(direction) > EPS && upcoming(k, i) == 0 {
					qty = *rem.at(direction)
				} else if openSz > EPS {
					qty = pickCleanQty(cash, openSz)
				} else {
					qty = inferStandaloneOptionQty(cash)
				}
				if qty > 0 {
					a.UnitPrice = math.Abs(cash) / (qty * 100.0)
				} else {
					a.UnitPrice = 0
				}
			}
			if direction == "SHORT" {
				f.side = "BUY"
				a.ActivitySubType = "BUYTOCLOSE"
				a.Quantity = qty
			} else {
				f.side = "SELL"
				a.ActivitySubType = "SELLTOCLOSE"
				a.Quantity = -qty
			}
			f.qty = qty
			f.rollDirection = direction
			closed := math.Min(qty, *rem.at(direction))
			*rem.at(direction) -= closed
			*pool.at(direction) -= math.Min(qty-closed, *pool.at(direction))
			if closed > EPS || qty > EPS {
				*pool.at(direction) += qty
			}
			continue
		}
		if isOption(a.Symbol) {
			resolveOptionFillSide(f, rem)
		}
		qty := math.Abs(a.Quantity)
		cash := a.NetCashAmount
		if isOption(a.Symbol) && qty == 0 {
			closingDir := "LONG"
			if f.side == "BUY" {
				closingDir = "SHORT"
			}
			openSz := *rem.at(closingDir)
			if math.Abs(cash) > 1e-9 {
				k := bookKey(a)
				if openSz > 0 && upcoming(k, i) == 0 {
					qty = openSz
				} else if openSz > 0 {
					qty = pickCleanQty(cash, openSz)
				} else {
					qty = inferStandaloneOptionQty(cash)
				}
				if qty > 0 {
					a.UnitPrice = math.Abs(cash) / (qty * 100.0)
				} else {
					a.UnitPrice = 0
				}
			} else if openSz > 0 && isCloseOnly(a) {
				qty = openSz
				a.UnitPrice = 0
			}
			if qty > 0 {
				if f.side == "SELL" {
					a.Quantity = -qty
				} else {
					a.Quantity = qty
				}
				f.qty = qty
			}
		}
		if isOption(a.Symbol) && (strings.Contains(compact(a.RawType), "ASSIGN") || strings.Contains(compact(a.ActivityType), "ASSIGN")) {
			a.UnitPrice = 0
		}
		if f.qty > 0 {
			closingDir := "LONG"
			if f.side == "BUY" {
				closingDir = "SHORT"
			}
			opening := openingDirection(a, f.side)
			left := f.qty
			closeAmt := math.Min(left, *rem.at(closingDir))
			*rem.at(closingDir) -= closeAmt
			left -= closeAmt
			if left > EPS && isOption(a.Symbol) {
				pool := poolOf(a)
				pooled := math.Min(left, *pool.at(closingDir))
				*pool.at(closingDir) -= pooled
				left -= pooled
			}
			if left > EPS && opening != "" {
				*rem.at(opening) += left
			}
		}
	}
}
