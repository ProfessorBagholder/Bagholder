package model

import "strings"

func rateOn(fx map[string]float64, day string) float64 {
	d := cut(day, 10)
	if d == "" {
		return FXFallback
	}
	for i := 0; i < 12; i++ {
		if r, ok := fx[d]; ok && r > 0 {
			return r
		}
		d = shiftDate(d, -1)
	}
	return FXFallback
}

func toCad(fx map[string]float64, amount float64, currency, day string) float64 {
	ccy := currency
	if ccy == "" {
		ccy = "CAD"
	}
	if strings.ToUpper(ccy) != "USD" {
		return amount
	}
	return amount * rateOn(fx, day)
}

func ApplyFX(slices []*Slice, fx map[string]float64) {
	for _, t := range slices {
		ccy := t.Currency
		if ccy == "" {
			ccy = "CAD"
		}
		ccy = strings.ToUpper(ccy)
		if ccy != "USD" {
			t.PnlCad = t.Pnl
			t.FeesCad = t.Commission
			t.HasFeesCad = true
			continue
		}
		qty := t.Quantity
		mult := multiplier(t.Symbol)
		entryC := t.EntryCommission
		exitC := t.ExitCommission
		entryNotional := t.EntryPrice * qty * mult
		exitNotional := t.ExitPrice * qty * mult
		var pnlCad float64
		if t.OpenDirection == "SHORT" {
			pnlCad = toCad(fx, entryNotional-entryC, ccy, t.EntryDate) - toCad(fx, exitNotional+exitC, ccy, t.ExitDate)
		} else {
			pnlCad = toCad(fx, exitNotional-exitC, ccy, t.ExitDate) - toCad(fx, entryNotional+entryC, ccy, t.EntryDate)
		}
		t.PnlCad = pnlCad
		t.FeesCad = toCad(fx, entryC, ccy, t.EntryDate) + toCad(fx, exitC, ccy, t.ExitDate)
		t.HasFeesCad = true
	}
}

func RateOn(fx map[string]float64, day string) float64 { return rateOn(fx, day) }
