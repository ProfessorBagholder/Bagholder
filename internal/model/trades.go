package model

import (
	"math"
	"sort"
	"strings"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
	"github.com/ProfessorBagholder/Bagholder/internal/symbols"
)

func slimSlice(s *Slice) SlimSlice {
	flags := s.Flags
	if flags == nil {
		flags = []string{}
	}
	return SlimSlice{
		Key:            sliceMemberKey(s),
		Qty:            s.Quantity,
		Entry:          s.EntryPrice,
		Exit:           s.ExitPrice,
		EntryDate:      s.EntryDate,
		ExitDate:       s.ExitDate,
		Pnl:            s.Pnl,
		PnlCad:         s.PnlCad,
		Fees:           s.Commission,
		BuyActivityID:  s.BuyActivityID,
		SellActivityID: s.SellActivityID,
		Flags:          flags,
	}
}

func fillRow(a *Act) Fill {
	when := a.OccurredAt
	if when == "" {
		when = a.TransactionDate
	}
	day, clock := whenParts(when)
	side := store.TradeSide(a)
	qty := math.Abs(a.Quantity)
	if side == "SELL" {
		qty = -qty
	}
	if day == "" {
		day = a.TransactionDate
	}
	flags := a.Flags
	if flags == nil {
		flags = []string{}
	}
	return Fill{
		ID:       a.ID,
		When:     when,
		Date:     day,
		Time:     clock,
		Side:     side,
		Sub:      a.ActivitySubType,
		Qty:      qty,
		Price:    a.UnitPrice,
		Amount:   a.NetCashAmount,
		Fees:     a.Commission,
		Currency: a.Currency,
		Flags:    flags,
	}
}

func sliceLess(a, b *Slice) bool {
	if a.ExitDate != b.ExitDate {
		return a.ExitDate < b.ExitDate
	}
	if a.EntryDate != b.EntryDate {
		return a.EntryDate < b.EntryDate
	}
	return sliceMemberKey(a) < sliceMemberKey(b)
}

func journalOf(journal map[string]store.JournalEntry, id string) (string, string, []string) {
	e, ok := journal[id]
	if !ok {
		return "", "", []string{}
	}
	tags := append([]string{}, e.Tags...)
	if tags == nil {
		tags = []string{}
	}
	return e.Grade, e.Thesis, tags
}

func collapseTrade(gid string, members []*Slice, locked bool, status string, actsByID map[string]*Act, securities *Securities, journal map[string]store.JournalEntry) *Trade {
	slices := append([]*Slice{}, members...)
	sort.SliceStable(slices, func(i, j int) bool { return sliceLess(slices[i], slices[j]) })
	t0 := slices[0]
	var qty, entryNotional, exitNotional, pnl, pnlCad, fees, feesCad float64
	entryDate, exitDate := slices[0].EntryDate, slices[0].ExitDate
	entryWhen, exitWhen := "", ""
	for i, s := range slices {
		qty += s.Quantity
		entryNotional += s.EntryPrice * s.Quantity
		exitNotional += s.ExitPrice * s.Quantity
		pnl += s.Pnl
		pnlCad += s.PnlCad
		fees += s.Commission
		if s.HasFeesCad {
			feesCad += s.FeesCad
		} else {
			feesCad += s.Commission
		}
		if s.EntryDate < entryDate {
			entryDate = s.EntryDate
		}
		if s.ExitDate > exitDate {
			exitDate = s.ExitDate
		}
		ew := s.EntryWhen
		if ew == "" {
			ew = s.EntryDate
		}
		xw := s.ExitWhen
		if xw == "" {
			xw = s.ExitDate
		}
		if i == 0 || ew < entryWhen {
			entryWhen = ew
		}
		if i == 0 || xw > exitWhen {
			exitWhen = xw
		}
	}
	mult := multiplier(t0.Symbol)
	entry := 0.0
	if qty != 0 {
		entry = entryNotional / qty
	}
	exitPx := t0.ExitPrice
	if qty != 0 {
		exitPx = exitNotional / qty
	}
	basis := math.Abs(entry * qty * mult)
	secID := ""
	for _, s := range slices {
		if s.SecurityID != "" {
			secID = s.SecurityID
			break
		}
	}
	var ids []string
	for _, s := range slices {
		for _, k := range []string{s.BuyActivityID, s.SellActivityID} {
			if k != "" && !py.Contains(ids, k) {
				ids = append(ids, k)
			}
		}
	}
	fills := []Fill{}
	for _, i := range ids {
		if a, ok := actsByID[i]; ok {
			fills = append(fills, fillRow(a))
		}
	}
	openedIDs := map[string]bool{}
	closedIDs := map[string]bool{}
	for _, s := range slices {
		openedIDs[s.BuyActivityID] = true
		closedIDs[s.SellActivityID] = true
	}
	for i := range fills {
		f := &fills[i]
		opened, closed := openedIDs[f.ID], closedIDs[f.ID]
		side := "SELL"
		if f.Side == "BUY" {
			side = "BUY"
		}
		if t0.Kind != "Options" {
			f.Sub = side
			if opened && closed {
				f.Sub = side + " (close + open)"
			}
		} else if closed && !opened {
			f.Sub = side + " TO CLOSE"
		} else if opened && !closed {
			f.Sub = side + " TO OPEN"
		} else if opened && closed {
			f.Sub = side + " (close + open)"
		}
	}
	sort.SliceStable(fills, func(i, j int) bool { return fills[i].When > fills[j].When })
	openSide := "SELL"
	if t0.OpenDirection == "LONG" {
		openSide = "BUY"
	}
	opens, closes := 0, 0
	for _, f := range fills {
		if f.Side == openSide {
			opens++
		} else {
			closes++
		}
	}
	flagSet := map[string]bool{}
	flags := []string{}
	for _, s := range slices {
		for _, fl := range s.Flags {
			if !flagSet[fl] {
				flagSet[fl] = true
				flags = append(flags, fl)
			}
		}
	}
	sort.Strings(flags)
	grade, thesis, tags := journalOf(journal, gid)
	name := t0.Name
	if name == "" {
		name = t0.Symbol
	}
	exchange := "Crypto"
	if t0.Kind != "Crypto" {
		exchange = securities.Exchange(secID)
	}
	side := "COVER"
	if t0.OpenDirection == "LONG" {
		side = "SELL"
	}
	var pnlPct *float64
	if basis > 0 {
		pnlPct = py.Ptr(pnl / basis)
	}
	legs := make([]SlimSlice, 0, len(slices))
	for _, s := range slices {
		legs = append(legs, slimSlice(s))
	}
	return &Trade{TradeCore: TradeCore{
		ID:            gid,
		Status:        status,
		Locked:        locked,
		Symbol:        t0.Symbol,
		Underlying:    underlying(t0.Symbol),
		Name:          securities.Name(secID, name),
		Exchange:      exchange,
		Kind:          t0.Kind,
		Currency:      t0.Currency,
		Account:       t0.Account,
		AccountID:     t0.AccountID,
		SecurityID:    secID,
		Side:          side,
		OpenDirection: t0.OpenDirection,
		Qty:           qty,
		Mult:          mult,
		Entry:         entry,
		Exit:          exitPx,
		EntryDate:     entryDate,
		ExitDate:      exitDate,
		EntryWhen:     entryWhen,
		ExitWhen:      exitWhen,
		HoldDays:      daysBetween(entryDate, exitDate),
		Pnl:           pnl,
		PnlCad:        pnlCad,
		Fees:          fees,
		FeesCad:       feesCad,
		PnlPct:        pnlPct,
		LegCount:      len(slices),
		Opened:        Summary{Qty: qty, Avg: entry, Fills: opens},
		Closed:        Summary{Qty: qty, Avg: exitPx, Fills: closes},
		NetCash:       pnl,
		Flags:         flags,
		Grade:         grade,
		Thesis:        thesis,
		Tags:          tags,
	}, Legs: legs, Fills: fills}
}

func BuildTrades(closed []*Slice, openLots []*Lot, saved []store.TradeGroup, actsByID map[string]*Act, securities *Securities, journal map[string]store.JournalEntry) []*Trade {
	byKey := map[string]*Slice{}
	for _, s := range closed {
		byKey[sliceMemberKey(s)] = s
	}
	used := map[string]bool{}
	type group struct {
		id      string
		members []*Slice
		locked  bool
	}
	var groups []group
	for _, rec := range saved {
		var members []*Slice
		for _, k := range rec.Members {
			s := byKey[k]
			if s == nil {
				continue
			}
			mk := sliceMemberKey(s)
			if !used[mk] {
				members = append(members, s)
				used[mk] = true
			}
		}
		if len(members) > 0 {
			id := rec.ID
			if id == "" {
				keys := make([]string, 0, len(members))
				for _, m := range members {
					keys = append(keys, sliceMemberKey(m))
				}
				id = GroupIDForKeys(keys)
			}
			groups = append(groups, group{id, members, true})
		}
	}
	byRT := map[string][]*Slice{}
	var order []string
	for _, s := range closed {
		if used[sliceMemberKey(s)] {
			continue
		}
		rt := s.RT
		if rt == "" {
			rt = "rt:" + sliceMemberKey(s)
		}
		if _, ok := byRT[rt]; !ok {
			order = append(order, rt)
		}
		byRT[rt] = append(byRT[rt], s)
	}
	for _, rt := range order {
		groups = append(groups, group{rt, byRT[rt], false})
	}
	trades := make([]*Trade, 0, len(groups))
	for _, g := range groups {
		trades = append(trades, collapseTrade(g.id, g.members, g.locked, "closed", actsByID, securities, journal))
	}
	sort.SliceStable(trades, func(i, j int) bool {
		a, b := trades[i], trades[j]
		if a.ExitDate != b.ExitDate {
			return a.ExitDate > b.ExitDate
		}
		return a.ID > b.ID
	})
	return trades
}

func LastFillPrices(activities []*Act) map[string]LastPrice {
	acts := append([]*Act{}, activities...)
	sort.SliceStable(acts, func(i, j int) bool {
		if acts[i].TransactionDate != acts[j].TransactionDate {
			return acts[i].TransactionDate < acts[j].TransactionDate
		}
		return acts[i].OccurredAt < acts[j].OccurredAt
	})
	out := map[string]LastPrice{}
	for _, a := range acts {
		if a.Category != "trade" && a.Category != "option_event" {
			continue
		}
		if a.UnitPrice > 0 && a.Symbol != "" {
			out[a.Symbol] = LastPrice{Price: a.UnitPrice, Date: a.TransactionDate}
		}
	}
	return out
}

func UnderTicker[T any](mapping map[string]T, symbol string) (T, bool) {
	if v, ok := mapping[symbol]; ok {
		return v, true
	}
	v, ok := mapping[symbols.TMXSymbol(symbol)]
	return v, ok
}

func QuoteFits(quote *store.Quote, kind string) bool {
	if quote == nil || quote.Source == "" {
		return true
	}
	source := quote.Source
	if kind == "Crypto" {
		return source == "coinbase"
	}
	if kind == "Options" {
		return source == "cboe_options"
	}
	return source != "coinbase" && source != "cboe_options"
}

type balKey struct{ account, security string }

func BuildPositions(openLots []*Lot, lastPrices map[string]LastPrice, balances []store.Balance, accounts []store.Account, securities *Securities, journal map[string]store.JournalEntry, today string, quotes map[string]store.Quote, actsByID map[string]*Act) []*Position {
	nickIDs := map[string][]string{}
	for _, acc := range accounts {
		nick := normAccountName(firstNonEmpty(acc.Nickname, acc.UnifiedAccountType, acc.Type))
		if !py.Contains(nickIDs[nick], acc.ID) {
			nickIDs[nick] = append(nickIDs[nick], acc.ID)
		}
	}
	bal := map[balKey]float64{}
	for _, b := range balances {
		k := balKey{b.AccountID, b.SecurityID}
		bal[k] = bal[k] + py.Deref(b.Quantity, 0)
	}
	type gkey struct{ symbol, account, currency, direction string }
	groups := map[gkey][]*Lot{}
	var order []gkey
	for _, lot := range openLots {
		k := gkey{lot.Symbol, lot.AccountType, lot.Currency, lot.Direction}
		if _, ok := groups[k]; !ok {
			order = append(order, k)
		}
		groups[k] = append(groups[k], lot)
	}
	rows := []*Position{}
	for _, k := range order {
		lots := append([]*Lot{}, groups[k]...)
		sort.SliceStable(lots, func(i, j int) bool {
			if lots[i].Date != lots[j].Date {
				return lots[i].Date < lots[j].Date
			}
			return lots[i].When < lots[j].When
		})
		symbol, account, currency, direction := k.symbol, k.account, k.currency, k.direction
		mult := multiplier(symbol)
		qty := 0.0
		for _, l := range lots {
			qty += l.Qty
		}
		if qty <= 1e-9 {
			continue
		}
		cost, fees := 0.0, 0.0
		for _, l := range lots {
			cost += l.Qty * l.Price * mult
			fees += l.Commission
		}
		secID := ""
		for _, l := range lots {
			if l.SecurityID != "" {
				secID = l.SecurityID
				break
			}
		}
		last, hasLast := lastPrices[symbol]
		lastPx := 0.0
		lastAt := ""
		if hasLast {
			lastPx = last.Price
			lastAt = last.Date
		} else if qty != 0 {
			lastPx = cost / (qty * mult)
		}
		priceSource := "fill"
		var quote *store.Quote
		if q, ok := UnderTicker(quotes, symbol); ok {
			qc := q
			quote = &qc
			if !QuoteFits(quote, lots[0].Kind) {
				quote = nil
			}
		}
		if quote != nil && quote.Price != nil && *quote.Price != 0 {
			lastPx = *quote.Price
			lastAt = quote.FetchedAt
			priceSource = "quote"
		}
		mv := qty * lastPx * mult
		unreal := mv - cost
		if direction != "LONG" {
			unreal = cost - mv
		}
		held := 0.0
		for _, l := range lots {
			held += l.Qty * float64(daysBetween(l.Date, today))
		}
		var wsQty *float64
		if secID != "" {
			if ids, ok := nickIDs[account]; ok {
				total := 0.0
				found := false
				for _, aid := range ids {
					if v, ok := bal[balKey{aid, secID}]; ok {
						total += v
						found = true
					}
				}
				if found {
					wsQty = py.Ptr(total)
				}
			}
		}
		legacyPID := "pos:" + strings.Join([]string{account, symbol, currency}, "|")
		pid := lots[0].RT
		if pid == "" {
			pid = legacyPID
		}
		jid := pid
		if _, ok := journal[pid]; !ok {
			jid = legacyPID
		}
		grade, thesis, tags := journalOf(journal, jid)
		var priceChange, percentChange, dayChange *float64
		if quote != nil {
			priceChange = copyPtr(quote.PriceChange)
			percentChange = copyPtr(quote.PercentChange)
		}
		if priceChange != nil {
			sign := 1.0
			if direction == "SHORT" {
				sign = -1
			}
			dayChange = py.Ptr(qty * *priceChange * mult * sign)
		}
		fills := []Fill{}
		for _, l := range lots {
			if a, ok := actsByID[l.ActivityID]; ok {
				fills = append(fills, fillRow(a))
			}
		}
		sort.SliceStable(fills, func(i, j int) bool { return fills[i].When > fills[j].When })
		plots := make([]PositionLot, 0, len(lots))
		for _, l := range lots {
			flags := l.Flags
			if flags == nil {
				flags = []string{}
			}
			plots = append(plots, PositionLot{Opened: l.Date, Qty: l.Qty, Price: l.Price, Basis: l.Qty * l.Price * mult, Held: daysBetween(l.Date, today), Flags: flags, ActivityID: l.ActivityID})
		}
		name := lots[0].Name
		if name == "" {
			name = symbol
		}
		exchange := "Crypto"
		if lots[0].Kind != "Crypto" {
			exchange = securities.Exchange(secID)
		}
		var unrealPct *float64
		if cost != 0 {
			unrealPct = py.Ptr(unreal / cost)
		}
		heldDays := 0
		if qty != 0 {
			heldDays = py.RoundInt(held / qty)
		}
		avg := 0.0
		if qty != 0 {
			avg = cost / (qty * mult)
		}
		var rt *string
		if lots[0].RT != "" {
			r := lots[0].RT
			rt = &r
		}
		rows = append(rows, &Position{PositionCore: PositionCore{
			ID:            pid,
			Symbol:        symbol,
			Underlying:    underlying(symbol),
			Name:          securities.Name(secID, name),
			Exchange:      exchange,
			Kind:          lots[0].Kind,
			Account:       account,
			AccountID:     lots[0].AccountID,
			Currency:      currency,
			SecurityID:    secID,
			Short:         direction == "SHORT",
			Qty:           qty,
			Mult:          mult,
			Avg:           avg,
			Cost:          cost,
			Fees:          fees,
			Last:          lastPx,
			LastAt:        lastAt,
			PriceSource:   priceSource,
			PriceChange:   priceChange,
			PercentChange: percentChange,
			DayChange:     dayChange,
			MV:            mv,
			Unreal:        unreal,
			UnrealPct:     unrealPct,
			Held:          heldDays,
			Opened:        lots[0].Date,
			WsQty:         wsQty,
			RT:            rt,
			Lots:          plots,
			Grade:         grade,
			Thesis:        thesis,
			Tags:          tags,
		}, Fills: fills})
	}
	book := 0.0
	for _, r := range rows {
		book += math.Abs(r.Cost)
	}
	for _, r := range rows {
		if book != 0 {
			r.Alloc = math.Abs(r.Cost) / book
		}
	}
	sort.SliceStable(rows, func(i, j int) bool { return rows[i].Alloc > rows[j].Alloc })
	return rows
}

func copyPtr(p *float64) *float64 {
	if p == nil {
		return nil
	}
	v := *p
	return &v
}

func firstNonEmpty(vals ...string) string {
	for _, v := range vals {
		if v != "" {
			return v
		}
	}
	return ""
}

func BuildCashflow(activities []*Act, securities *Securities, fx map[string]float64) []CashRow {
	rows := []CashRow{}
	for _, a := range activities {
		cat := a.Category
		raw := compact(a.RawType)
		at := compact(a.ActivityType)
		cash := a.NetCashAmount
		kind := ""
		switch {
		case cat == "dividend":
			kind = "Dividend"
		case cat == "interest":
			kind = "Interest"
		case raw == "WITHHOLDINGTAX" || at == "WITHHOLDINGTAX":
			kind = "Withholding tax"
		case raw == "INTERESTCHARGE" || at == "INTERESTCHARGE":
			kind = "Interest charge"
		default:
			continue
		}
		if math.Abs(cash) < EPS {
			continue
		}
		when := a.OccurredAt
		if when == "" {
			when = a.TransactionDate
		}
		day, clock := whenParts(when)
		symbol := py.Strip(a.Symbol)
		if symbol == "" && (kind == "Interest" || kind == "Interest charge") {
			symbol = "Cash"
		}
		nameFallback := ""
		if a.Name != symbol {
			nameFallback = a.Name
		}
		date := a.TransactionDate
		if date == "" {
			date = day
		}
		shown := symbol
		if shown == "" {
			shown = "—"
		}
		account := normAccountName(a.AccountType)
		if account == "" {
			account = a.AccountID
		}
		var qty, per *float64
		if a.Quantity != 0 {
			qty = py.Ptr(a.Quantity)
		}
		if a.UnitPrice != 0 {
			per = py.Ptr(a.UnitPrice)
		}
		ccy := a.Currency
		if ccy == "" {
			ccy = "CAD"
		}
		rows = append(rows, CashRow{
			ID:        a.ID,
			Date:      date,
			Time:      clock,
			Symbol:    shown,
			Name:      securities.Name(a.SecurityID, nameFallback),
			Kind:      kind,
			Account:   account,
			AccountID: a.AccountID,
			Qty:       qty,
			Per:       per,
			Amount:    cash,
			Currency:  ccy,
			AmountCad: toCad(fx, cash, a.Currency, a.TransactionDate),
		})
	}
	sort.SliceStable(rows, func(i, j int) bool {
		if rows[i].Date != rows[j].Date {
			return rows[i].Date > rows[j].Date
		}
		return rows[i].ID > rows[j].ID
	})
	return rows
}
