package model

import (
	"math"
	"slices"
	"sort"
	"strings"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
	"github.com/ProfessorBagholder/Bagholder/internal/symbols"
)

func stableTradeID(t *Slice) string {
	return strings.Join([]string{t.AccountID, t.Symbol, t.Currency, t.EntryDate, t.ExitDate, fmt8(t.Quantity), fmt8(t.EntryPrice), fmt8(t.ExitPrice), t.Side}, "|")
}

func sliceMemberKey(t *Slice) string {
	if t.BuyActivityID != "" && t.SellActivityID != "" {
		return strings.Join([]string{t.BuyActivityID, t.SellActivityID, fmt8(t.Quantity)}, "|")
	}
	return t.ID
}

func GroupIDForKeys(keys []string) string {
	sorted := append([]string{}, keys...)
	sort.Strings(sorted)
	s := strings.Join(sorted, "\n")
	var h uint32 = 2166136261
	for _, ch := range s {
		h ^= uint32(ch)
		h *= 16777619
	}
	return "g_" + strings.ToLower(py.Hex(uint64(h))) + "_" + py.Itoa(len(keys))
}

func fillRank(f *fill) int {
	a := f.a
	t := compact(a.ActivityType)
	s := compact(a.ActivitySubType)
	blob := t + s
	if (strings.Contains(blob, "TOOPEN") || t == "STO" || s == "STO") && f.side == "SELL" {
		return 0
	}
	if isCloseOnly(a) && f.side == "BUY" {
		return 1
	}
	if f.side == "BUY" {
		return 2
	}
	if strings.Contains(blob, "TOCLOSE") || t == "STC" || s == "STC" {
		return 3
	}
	return 4
}

func fillLess(x, y *fill) bool {
	if x.a.TransactionDate != y.a.TransactionDate {
		return x.a.TransactionDate < y.a.TransactionDate
	}
	if x.rank != y.rank {
		return x.rank < y.rank
	}
	if x.a.OccurredAt != y.a.OccurredAt {
		return x.a.OccurredAt < y.a.OccurredAt
	}
	return x.a.ID < y.a.ID
}

func unionFlags(a, b []string) []string {
	seen := map[string]bool{}
	var out []string
	for _, f := range a {
		if !seen[f] {
			seen[f] = true
			out = append(out, f)
		}
	}
	for _, f := range b {
		if !seen[f] {
			seen[f] = true
			out = append(out, f)
		}
	}
	sort.Strings(out)
	if out == nil {
		out = []string{}
	}
	return out
}

func makeSlice(lot *Lot, f *fill, a *Act, matched float64, symbol string) *Slice {
	fillQty := f.qty
	exitCommission := 0.0
	if fillQty > 0 {
		exitCommission = a.Commission * (matched / fillQty)
	}
	entryCommission := 0.0
	if lot.Qty > 0 {
		entryCommission = lot.Commission * (matched / lot.Qty)
	}
	commission := entryCommission + exitCommission
	sym := symbol
	if sym == "" {
		sym = lot.Symbol
	}
	mult := multiplier(sym)
	exitPx := a.UnitPrice
	var rawPnl float64
	if lot.Direction == "LONG" {
		rawPnl = (exitPx - lot.Price) * matched * mult
	} else {
		rawPnl = (lot.Price - exitPx) * matched * mult
	}
	name := lot.Name
	if symbol != "" && a.Name != "" {
		name = a.Name
	}
	secID := lot.SecurityID
	if secID == "" {
		secID = a.SecurityID
	}
	t := &Slice{
		RT:              lot.RT,
		AccountID:       lot.AccountID,
		AccountType:     lot.AccountType,
		Account:         lot.AccountType,
		Symbol:          sym,
		Name:            name,
		Currency:        lot.Currency,
		Kind:            lot.Kind,
		Side:            f.side,
		Quantity:        matched,
		EntryPrice:      lot.Price,
		ExitPrice:       exitPx,
		EntryDate:       lot.Date,
		ExitDate:        a.TransactionDate,
		EntryWhen:       lot.When,
		ExitWhen:        a.OccurredAt,
		HoldDays:        daysBetween(lot.Date, a.TransactionDate),
		Commission:      commission,
		EntryCommission: entryCommission,
		ExitCommission:  exitCommission,
		Pnl:             rawPnl - commission,
		PnlCad:          rawPnl - commission,
		OpenDirection:   lot.Direction,
		BuyActivityID:   lot.ActivityID,
		SellActivityID:  a.ID,
		SecurityID:      secID,
		Flags:           unionFlags(lot.Flags, a.Flags),
	}
	t.ID = stableTradeID(t)
	return t
}

func dust(remaining float64, f *fill, a *Act) bool {
	qty := f.qty
	px := math.Abs(a.UnitPrice)
	if remaining <= 1e-6*math.Max(1.0, qty) {
		return true
	}
	kind := a.Kind
	if kind == "" {
		kind = kindOf(a)
	}
	if kind == "Crypto" && remaining <= 0.01*qty {
		return true
	}
	return px > 0 && remaining*px*multiplier(a.Symbol) < 0.01
}

type book struct{ lots []*Lot }

func (b *book) pop() { b.lots = b.lots[1:] }

type rolledPool struct {
	long, short []*Lot
	rt          string
}

func (p *rolledPool) at(direction string) *[]*Lot {
	if direction == "SHORT" {
		return &p.short
	}
	return &p.long
}

type matcher struct {
	books         map[string]*book
	bookOrder     []string
	rtOpen        map[string]string
	closed        []*Slice
	unmatched     []Unmatched
	rolled        map[rollKey]*rolledPool
	rolledOrder   []rollKey
	rolledKeys    map[rollKey]bool
	replaced      *replacementIndex
	pendingSplits map[string][]daySplit
}

type daySplit struct {
	day    string
	factor float64
}

func (m *matcher) rolledOf(a *Act) *rolledPool {
	k := rollKeyOf(a)
	p, ok := m.rolled[k]
	if !ok {
		p = &rolledPool{}
		m.rolled[k] = p
		m.rolledOrder = append(m.rolledOrder, k)
	}
	return p
}

func (m *matcher) bookOf(key string) *book {
	b, ok := m.books[key]
	if !ok {
		b = &book{}
		m.books[key] = b
		m.bookOrder = append(m.bookOrder, key)
	}
	return b
}

func (m *matcher) closeRolled(f *fill, a *Act, remaining float64, closingDir string) float64 {
	pool := m.rolledOf(a)
	lots := pool.at(closingDir)
	key := bookKey(a)
	rt := f.rtBefore
	if rt == "" {
		rt = pool.rt
	}
	if rt == "" && len(*lots) > 0 {
		rt = (*lots)[0].RT
		if rt == "" {
			rt = "rt:" + (*lots)[0].ActivityID
		}
	}
	if rt != "" {
		pool.rt = rt
	}
	for remaining > EPS && len(*lots) > 0 {
		lot := (*lots)[0]
		lot.Symbol = a.Symbol
		if rt != "" {
			lot.RT = rt
		} else if lot.RT == "" {
			lot.RT = "rt:" + lot.ActivityID
		}
		matched := math.Min(lot.Qty, remaining)
		m.closed = append(m.closed, makeSlice(lot, f, a, matched, ""))
		lot.Qty -= matched
		remaining -= matched
		if lot.Qty <= EPS {
			*lots = (*lots)[1:]
		}
	}
	if remaining > EPS && m.rolledKeys[rollKeyOf(a)] {
		type other struct{ expiry, key string }
		var others []other
		for _, k2 := range m.bookOrder {
			b2 := m.books[k2]
			if k2 == key || len(b2.lots) == 0 {
				continue
			}
			bits := strings.Split(k2, "::")
			if bits[0] != fifoAccount(a) || bits[2] != a.Currency {
				continue
			}
			if !isOption(bits[1]) || underlying(bits[1]) != underlying(a.Symbol) || optionRight(bits[1]) != optionRight(a.Symbol) {
				continue
			}
			others = append(others, other{optionExpiry(bits[1]), k2})
		}
		sort.Slice(others, func(i, j int) bool {
			if others[i].expiry != others[j].expiry {
				return others[i].expiry < others[j].expiry
			}
			return others[i].key < others[j].key
		})
		for _, o := range others {
			b2 := m.books[o.key]
			for remaining > EPS && len(b2.lots) > 0 && b2.lots[0].Direction == closingDir {
				lot := b2.lots[0]
				matched := math.Min(lot.Qty, remaining)
				s := makeSlice(lot, f, a, matched, a.Symbol)
				s.Flags = unionFlags(s.Flags, []string{"rolled-in"})
				if rt != "" {
					s.RT = rt
				}
				m.closed = append(m.closed, s)
				lot.Qty -= matched
				remaining -= matched
				if lot.Qty <= EPS {
					b2.pop()
				}
			}
			if len(b2.lots) == 0 {
				m.rtOpen[o.key] = ""
			}
		}
	}
	if len(pool.long) == 0 && len(pool.short) == 0 && (m.books[key] == nil || len(m.books[key].lots) == 0) {
		pool.rt = ""
	}
	return remaining
}

func splitLabel(factor float64) string {
	if factor < 1 {
		return "split 1:" + py.Itoa(int(math.RoundToEven(1/factor)))
	}
	return "split " + py.Itoa(int(math.RoundToEven(factor))) + ":1"
}

func (m *matcher) applySplits(key, day string) {
	bits := strings.Split(key, "::")
	skey := strings.Join(bits[:2], "::")
	todo := m.pendingSplits[skey]
	if len(todo) == 0 {
		return
	}
	sort.SliceStable(todo, func(i, j int) bool {
		if todo[i].day != todo[j].day {
			return todo[i].day < todo[j].day
		}
		return todo[i].factor < todo[j].factor
	})
	var keep []daySplit
	for _, sp := range todo {
		if sp.day <= day {
			if b := m.books[key]; b != nil {
				for _, lot := range b.lots {
					lot.Qty *= sp.factor
					lot.Price /= sp.factor
					label := splitLabel(sp.factor)
					if !slices.Contains(lot.Flags, label) {
						lot.Flags = append(lot.Flags, label)
					}
				}
			}
		} else {
			keep = append(keep, sp)
		}
	}
	if len(keep) > 0 {
		m.pendingSplits[skey] = keep
	} else {
		delete(m.pendingSplits, skey)
	}
}

func (m *matcher) closeAgainst(b *book, key string, f *fill, a *Act, remaining float64, symbolOverride string) float64 {
	closingDir := "LONG"
	if f.side == "BUY" {
		closingDir = "SHORT"
	}
	for remaining > EPS && len(b.lots) > 0 && b.lots[0].Direction == closingDir {
		lot := b.lots[0]
		matched := math.Min(lot.Qty, remaining)
		m.closed = append(m.closed, makeSlice(lot, f, a, matched, symbolOverride))
		if lot.Qty > 0 {
			lot.Commission *= (lot.Qty - matched) / lot.Qty
		} else {
			lot.Commission *= 0
		}
		lot.Qty -= matched
		remaining -= matched
		if lot.Qty <= EPS {
			b.pop()
		}
	}
	if len(b.lots) == 0 {
		m.rtOpen[key] = ""
	}
	return remaining
}

func newLot(a *Act, qty, price, commission float64, direction, kind, securityID, rt string) *Lot {
	return &Lot{
		Qty:         qty,
		Price:       price,
		Date:        a.TransactionDate,
		When:        a.OccurredAt,
		Commission:  commission,
		Direction:   direction,
		AccountID:   a.AccountID,
		AccountType: fifoAccount(a),
		Symbol:      a.Symbol,
		Name:        a.Name,
		Currency:    a.Currency,
		Kind:        kind,
		ActivityID:  a.ID,
		SecurityID:  securityID,
		RT:          rt,
		Flags:       append([]string{}, a.Flags...),
	}
}

func MatchFIFO(activities []*Act) *FIFOResult {
	acts := make([]*Act, 0, len(activities))
	for _, a := range activities {
		if a.Normalized {
			acts = append(acts, a)
		} else {
			acts = append(acts, NormalizeActivity(a))
		}
	}
	normalized := make([]*Act, 0, len(acts))
	for _, a := range acts {
		if !a.HasFlag("pending-distribution") {
			normalized = append(normalized, a)
		}
	}
	folded := foldStkdis(normalized)
	var fills []*fill
	for _, a := range folded {
		if (a.Category != "trade" && a.Category != "option_event") || a.Symbol == "" {
			continue
		}
		side := store.TradeSide(a)
		if side == "" {
			continue
		}
		f := &fill{a: a, side: side, qty: math.Abs(a.Quantity)}
		f.rank = fillRank(f)
		fills = append(fills, f)
	}
	sort.SliceStable(fills, func(i, j int) bool { return fillLess(fills[i], fills[j]) })
	inferZeroQtyOptionFills(fills)
	var usable []*fill
	for _, f := range fills {
		if f.qty > 0 {
			usable = append(usable, f)
		}
	}

	m := &matcher{
		books:         map[string]*book{},
		rtOpen:        map[string]string{},
		closed:        []*Slice{},
		unmatched:     []Unmatched{},
		rolled:        map[rollKey]*rolledPool{},
		rolledKeys:    map[rollKey]bool{},
		replaced:      buildReplacementIndex(normalized),
		pendingSplits: map[string][]daySplit{},
	}
	splits := splitMarkers(normalized)
	var splitKeys []splitKey
	for k := range splits {
		splitKeys = append(splitKeys, k)
	}
	sort.Slice(splitKeys, func(i, j int) bool {
		a, b := splitKeys[i], splitKeys[j]
		if a.account != b.account {
			return a.account < b.account
		}
		if a.symbol != b.symbol {
			return a.symbol < b.symbol
		}
		return a.day < b.day
	})
	for _, k := range splitKeys {
		sk := k.account + "::" + k.symbol
		m.pendingSplits[sk] = append(m.pendingSplits[sk], daySplit{k.day, splits[k]})
	}

	for _, f := range usable {
		a := f.a
		key := bookKey(a)
		b := m.bookOf(key)
		m.applySplits(key, a.TransactionDate)
		if isOption(a.Symbol) && isMultileg(a) && f.rollDirection != "" {
			direction := f.rollDirection
			cash := a.NetCashAmount
			per := 0.0
			if f.qty > 0 {
				per = math.Abs(cash) / (f.qty * 100.0)
			}
			debit := cash < 0
			exitPx, entryPx := 0.0, 0.0
			if (direction == "SHORT") == debit {
				exitPx = per
			} else {
				entryPx = per
			}
			a.UnitPrice = exitPx
			before := len(m.closed)
			f.rtBefore = ""
			if len(b.lots) > 0 {
				f.rtBefore = m.rtOpen[key]
			}
			remaining := m.closeAgainst(b, key, f, a, f.qty, "")
			remaining = m.closeRolled(f, a, remaining, direction)
			moved := f.qty - remaining
			m.rolledKeys[rollKeyOf(a)] = true
			if moved > EPS {
				for _, s := range m.closed[before:] {
					if !slices.Contains(s.Flags, "rolled") {
						s.Flags = append(s.Flags, "rolled")
					}
				}
				chainRT := f.rtBefore
				if chainRT == "" {
					chainRT = m.rolledOf(a).rt
				}
				if chainRT == "" && len(m.closed) > before {
					chainRT = m.closed[before].RT
				}
				m.rolledOf(a).rt = chainRT
				lot := newLot(a, moved, entryPx, 0, direction, "Options", "", chainRT)
				lot.Flags = []string{"rolled-in"}
				pl := m.rolledOf(a).at(direction)
				*pl = append(*pl, lot)
			}
			if remaining > EPS {
				opening := "SHORT"
				if debit {
					opening = "LONG"
				}
				a.UnitPrice = per
				if opening == "LONG" {
					f.side = "BUY"
				} else {
					f.side = "SELL"
				}
				if len(b.lots) == 0 || m.rtOpen[key] == "" {
					m.rtOpen[key] = "rt:" + a.ID
				}
				b.lots = append(b.lots, newLot(a, remaining, per, 0, opening, "Options", a.SecurityID, m.rtOpen[key]))
			}
			continue
		}
		if a.HasFlag("transfer-out") {
			remaining := f.qty
			for remaining > EPS && len(b.lots) > 0 && b.lots[0].Direction == "LONG" {
				lot := b.lots[0]
				matched := math.Min(lot.Qty, remaining)
				if lot.Qty > 0 {
					lot.Commission *= (lot.Qty - matched) / lot.Qty
				} else {
					lot.Commission *= 0
				}
				lot.Qty -= matched
				remaining -= matched
				if lot.Qty <= EPS {
					b.pop()
				}
			}
			if len(b.lots) == 0 {
				m.rtOpen[key] = ""
			}
			continue
		}
		f.rtBefore = ""
		if len(b.lots) > 0 {
			f.rtBefore = m.rtOpen[key]
		}
		remaining := m.closeAgainst(b, key, f, a, f.qty, "")
		if remaining > EPS && isOption(a.Symbol) {
			closingDir := "LONG"
			if f.side == "BUY" {
				closingDir = "SHORT"
			}
			remaining = m.closeRolled(f, a, remaining, closingDir)
		}
		if remaining > EPS && f.side == "SELL" {
			for _, dk := range m.bookOrder {
				dbook := m.books[dk]
				if len(dbook.lots) == 0 || dk == key {
					continue
				}
				bits := strings.Split(dk, "::")
				if bits[0] != fifoAccount(a) || bits[2] != a.Currency {
					continue
				}
				if !m.replaced.tickerWasReplaced(bits[0], bits[1], bits[2], a.TransactionDate) {
					continue
				}
				remaining = m.closeAgainst(dbook, dk, f, a, remaining, a.Symbol)
				if remaining <= EPS {
					break
				}
			}
		}
		if remaining > EPS && f.side == "SELL" && openingDirection(a, f.side) == "" && dust(remaining, f, a) {
			remaining = 0
		}
		if remaining > EPS {
			opening := openingDirection(a, f.side)
			if opening != "" {
				if len(b.lots) == 0 || m.rtOpen[key] == "" {
					m.rtOpen[key] = "rt:" + a.ID
				}
				commission := 0.0
				if f.qty > 0 {
					commission = a.Commission * (remaining / f.qty)
				}
				kind := a.Kind
				if kind == "" {
					kind = kindOf(a)
				}
				b.lots = append(b.lots, newLot(a, remaining, a.UnitPrice, commission, opening, kind, a.SecurityID, m.rtOpen[key]))
			} else {
				m.unmatched = append(m.unmatched, Unmatched{
					Symbol:      a.Symbol,
					Currency:    a.Currency,
					Side:        f.side,
					Quantity:    remaining,
					Price:       a.UnitPrice,
					Date:        a.TransactionDate,
					Description: a.Description,
					AccountID:   a.AccountID,
					Account:     fifoAccount(a),
					ActivityID:  a.ID,
				})
			}
		}
	}

	for _, key := range m.bookOrder {
		m.applySplits(key, "9999-12-31")
	}
	for _, rk := range m.rolledOrder {
		pool := m.rolled[rk]
		for _, direction := range []string{"LONG", "SHORT"} {
			for _, lot := range *pool.at(direction) {
				if lot.Qty <= EPS {
					continue
				}
				pa := &Act{ID: "roll-out:" + lot.ActivityID, TransactionDate: lot.Date, OccurredAt: lot.When, Name: lot.Name, Flags: []string{"rolled-out"}, Normalized: true}
				side := "SELL"
				if direction == "SHORT" {
					side = "BUY"
				}
				pseudo := &fill{a: pa, side: side, qty: lot.Qty}
				if lot.RT == "" {
					lot.RT = "rt:" + lot.ActivityID
				}
				s := makeSlice(lot, pseudo, pa, lot.Qty, "")
				s.SellActivityID = ""
				m.closed = append(m.closed, s)
			}
		}
	}
	openLots := []*Lot{}
	for _, key := range m.bookOrder {
		for _, lot := range m.books[key].lots {
			if lot.Qty <= 1e-6 {
				continue
			}
			if lot.Kind == "Crypto" && lot.Qty*lot.Price < 1.0 {
				continue
			}
			openLots = append(openLots, lot.clone())
		}
	}
	closed := m.closed
	sort.SliceStable(closed, func(i, j int) bool {
		if closed[i].ExitDate != closed[j].ExitDate {
			return closed[i].ExitDate < closed[j].ExitDate
		}
		return closed[i].ID < closed[j].ID
	})
	closed = foldOptionRolls(closed, openLots)
	return &FIFOResult{Closed: closed, Open: openLots, Unmatched: m.unmatched}
}

func dayOf(s string) string { return cut(s, 10) }

func rollBook(t *Slice) string {
	acct := t.Account
	if acct == "" {
		acct = t.AccountType
	}
	return strings.Join([]string{acct, t.Currency, underlying(t.Symbol)}, "::")
}

func foldOptionRolls(closed []*Slice, openLots []*Lot) []*Slice {
	if len(closed) == 0 {
		return closed
	}
	var covers []*Slice
	for _, t := range closed {
		if t.OpenDirection == "SHORT" && isOption(t.Symbol) {
			covers = append(covers, t)
		}
	}
	sort.SliceStable(covers, func(i, j int) bool {
		a, b := covers[i], covers[j]
		if dayOf(a.EntryDate) != dayOf(b.EntryDate) {
			return dayOf(a.EntryDate) < dayOf(b.EntryDate)
		}
		if dayOf(a.ExitDate) != dayOf(b.ExitDate) {
			return dayOf(a.ExitDate) < dayOf(b.ExitDate)
		}
		return a.ID < b.ID
	})
	drop := map[string]bool{}
	for _, cover := range covers {
		if drop[cover.ID] {
			continue
		}
		d := dayOf(cover.ExitDate)
		if d == "" {
			continue
		}
		under := underlying(cover.Symbol)
		if under == "" || under == "—" {
			continue
		}
		ck := rollBook(cover)
		var closedCands []*Slice
		for _, t := range closed {
			if t.ID != cover.ID && !drop[t.ID] && t.OpenDirection == "SHORT" && isOption(t.Symbol) && t.Symbol != cover.Symbol && rollBook(t) == ck && dayOf(t.EntryDate) == d {
				closedCands = append(closedCands, t)
			}
		}
		var openCands []*Lot
		for _, l := range openLots {
			if l.Direction == "SHORT" && isOption(l.Symbol) && l.Symbol != cover.Symbol && strings.Join([]string{l.AccountType, l.Currency, underlying(l.Symbol)}, "::") == ck && dayOf(l.Date) == d {
				openCands = append(openCands, l)
			}
		}
		cq := math.Abs(cover.Quantity)
		if len(closedCands) > 0 {
			sort.SliceStable(closedCands, func(i, j int) bool {
				a, b := closedCands[i], closedCands[j]
				da, db := math.Abs(math.Abs(a.Quantity)-cq), math.Abs(math.Abs(b.Quantity)-cq)
				if da != db {
					return da < db
				}
				return a.Symbol < b.Symbol
			})
			row := closedCands[0]
			qty := math.Abs(row.Quantity)
			if !(qty > 0) {
				continue
			}
			adj := cover.Pnl / (qty * multiplier(row.Symbol))
			row.EntryPrice += adj
			mult := multiplier(row.Symbol)
			var raw float64
			if row.OpenDirection == "SHORT" {
				raw = (row.EntryPrice - row.ExitPrice) * qty * mult
			} else {
				raw = (row.ExitPrice - row.EntryPrice) * qty * mult
			}
			row.Pnl = raw - row.Commission
			row.PnlCad = row.Pnl
			row.ID = stableTradeID(row)
			if !slices.Contains(row.Flags, "rolled") {
				row.Flags = append(row.Flags, "rolled")
			}
		} else if len(openCands) > 0 {
			sort.SliceStable(openCands, func(i, j int) bool {
				a, b := openCands[i], openCands[j]
				da, db := math.Abs(math.Abs(a.Qty)-cq), math.Abs(math.Abs(b.Qty)-cq)
				if da != db {
					return da < db
				}
				return a.Symbol < b.Symbol
			})
			row := openCands[0]
			qty := math.Abs(row.Qty)
			if !(qty > 0) {
				continue
			}
			adj := cover.Pnl / (qty * multiplier(row.Symbol))
			row.Price += adj
			if !slices.Contains(row.Flags, "rolled") {
				row.Flags = append(row.Flags, "rolled")
			}
		} else {
			continue
		}
		drop[cover.ID] = true
	}
	if len(drop) == 0 {
		return closed
	}
	out := closed[:0:0]
	for _, t := range closed {
		if !drop[t.ID] {
			out = append(out, t)
		}
	}
	return out
}

func SynthesizeAssignmentShares(activities []*Act, securities *Securities) []*Act {
	var out []*Act
	for _, a := range activities {
		if a.Category != "option_event" || compact(a.ActivityType) != "ASSIGN" {
			continue
		}
		symbol := a.Symbol
		if !isOption(symbol) {
			continue
		}
		contracts := math.Abs(a.Quantity)
		if contracts <= 0 {
			continue
		}
		shares := contracts * 100
		cash := a.NetCashAmount
		strike := 0.0
		if math.Abs(cash) > EPS {
			strike = math.Abs(cash) / shares
		}
		if strike <= 0 {
			strike = strikeOf(symbol)
		}
		if strike <= 0 {
			continue
		}
		up := strings.TrimRight(strings.ToUpper(symbol), py.SpaceChars)
		isCall := strings.HasSuffix(up, "CALL") || strings.HasSuffix(up, " C")
		sell := cash > 0
		if math.Abs(cash) <= EPS {
			sell = isCall
		}
		under := underlying(symbol)
		underID := ""
		if sec := securities.ByID[a.SecurityID]; sec != nil {
			underID = sec.UnderlyingID
		}
		occurred := a.OccurredAt
		if occurred == "" {
			occurred = a.TransactionDate + "T21:30:00+00:00"
		}
		bookID := a.BookID
		if bookID == "" {
			bookID = a.AccountID
		}
		fifoID := a.FifoID
		if fifoID == "" {
			fifoID = a.AccountID
		}
		desc := "Put to you"
		sub, direction := "BUY", "DEBIT"
		qty, net := shares, -shares*strike
		if sell {
			desc = "Called away"
			sub, direction = "SELL", "CREDIT"
			qty, net = -shares, shares*strike
		}
		out = append(out, &Act{
			ID:              "assign-shares:" + a.ID,
			OccurredAt:      occurred,
			TransactionDate: a.TransactionDate,
			SettlementDate:  a.TransactionDate,
			AccountID:       a.AccountID,
			BookID:          bookID,
			FifoID:          fifoID,
			AccountType:     a.AccountType,
			ActivityType:    "Trade",
			ActivitySubType: sub,
			Description:     desc + ": " + py.Repr(shares) + " " + under + " @ " + py.Repr(strike),
			Direction:       direction,
			Symbol:          under,
			Name:            under,
			Currency:        a.Currency,
			Quantity:        qty,
			UnitPrice:       strike,
			Commission:      0,
			NetCashAmount:   net,
			Category:        "trade",
			Source:          "derived",
			RawType:         "OPTIONS_ASSIGN_SHARES",
			SecurityID:      underID,
			Kind:            "Shares",
			Flags:           []string{"assignment"},
			Normalized:      true,
		})
	}
	return out
}

func strikeOf(symbol string) float64 {
	return symbols.Strike(symbol)
}

type expiryKey struct{ account, symbol, currency string }

func SynthesizeExpiries(activities []*Act, openLots []*Lot, today string) []*Act {
	var out []*Act
	seen := map[expiryKey]bool{}
	for _, lot := range openLots {
		exp := optionExpiry(lot.Symbol)
		if exp == "" || exp >= today {
			continue
		}
		key := expiryKey{lot.AccountType, lot.Symbol, lot.Currency}
		if seen[key] {
			continue
		}
		seen[key] = true
		qty := 0.0
		for _, l := range openLots {
			if l.AccountType == key.account && l.Symbol == key.symbol && l.Currency == key.currency && l.Direction == lot.Direction {
				qty += l.Qty
			}
		}
		if qty <= EPS {
			continue
		}
		short := lot.Direction == "SHORT"
		sub, q, raw := "SELL", -qty, "OPTIONS_EXPIRY"
		if short {
			sub, q, raw = "BUY", qty, "OPTIONS_SHORT_EXPIRY"
		}
		out = append(out, &Act{
			ID:              "expiry:" + lot.AccountType + "|" + lot.Symbol + "|" + lot.Currency,
			OccurredAt:      exp + "T21:30:00+00:00",
			TransactionDate: exp,
			SettlementDate:  exp,
			AccountID:       lot.AccountID,
			BookID:          lot.AccountID,
			FifoID:          lot.AccountID,
			AccountType:     lot.AccountType,
			ActivityType:    "EXPIR",
			ActivitySubType: sub,
			Description:     "Expired (assumed): " + lot.Symbol,
			Symbol:          lot.Symbol,
			Name:            lot.Name,
			Currency:        lot.Currency,
			Quantity:        q,
			UnitPrice:       0,
			Commission:      0,
			NetCashAmount:   0,
			Category:        "option_event",
			Source:          "derived",
			RawType:         raw,
			SecurityID:      lot.SecurityID,
			Kind:            "Options",
			Flags:           []string{"assumed-expiry"},
			Normalized:      true,
		})
	}
	return out
}
