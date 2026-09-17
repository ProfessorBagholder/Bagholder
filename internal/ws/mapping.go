package ws

import (
	"fmt"
	"math"
	"regexp"
	"sort"
	"strings"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

var Months = []string{"JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"}

var SkipTypeMarkers = []string{"SHARE_LENDING", "SHARELENDING", "STOCK_LENDING", "STOCKLENDING"}

var keepStatus = map[string]bool{"POSTED": true, "COMPLETED": true, "SETTLED": true, "COMPLETE": true, "FILLED": true, "EXECUTED": true, "PROCESSED": true, "CONFIRMED": true, "BOOKED": true, "SUCCEEDED": true, "SUCCESS": true}

var corpBlobs = []string{"STKDIS", "STOCKDISTRIBUTION", "STOCKDIV", "SPINOFF", "SPIN", "DIVIDENDINKIND", "INKIND", "CORPORATEACTION", "CODECHANGE", "SYMBOLCHANGE", "TICKERCHANGE", "LISTINGSTATUS", "SECURITYSWAP", "MANDATORYEXCHANGE", "NAMECHANGE"}

var codeChangeBlobs = []string{"CODECHANGE", "SYMBOLCHANGE", "TICKERCHANGE", "LISTINGSTATUS", "SECURITYSWAP", "MANDATORYEXCHANGE", "NAMECHANGE"}

var compactRE = regexp.MustCompile(`[` + py.SpaceClass + `_\-]+`)

type Item = map[string]any

func num(v any) float64 { return py.Num(v, 0) }

func upper(v any) string { return strings.ToUpper(py.Strip(py.S(v))) }

func Compact(v any) string { return compactRE.ReplaceAllString(upper(v), "") }

func DateOnly(occurred string) string {
	s := py.Strip(occurred)
	if s == "" {
		return ""
	}
	if i := strings.Index(s, "T"); i >= 0 {
		s = s[:i]
	}
	if len(s) > 10 {
		s = s[:10]
	}
	return s
}

func AssetSymbol(item Item) string {
	raw := py.Strip(py.S(item["assetSymbol"]))
	if strings.HasPrefix(strings.ToUpper(raw), "EXCHANGE:") {
		_, raw, _ = strings.Cut(raw, ":")
	}
	return py.Strip(strings.ToUpper(raw))
}

func CounterSymbol(item Item) string {
	raw := py.Strip(py.S(item["counterAssetSymbol"]))
	if strings.HasPrefix(strings.ToUpper(raw), "EXCHANGE:") {
		_, raw, _ = strings.Cut(raw, ":")
	}
	return py.Strip(strings.ToUpper(raw))
}

func typeBlob(item Item) (string, string, string) {
	typ := strings.ReplaceAll(upper(item["type"]), "-", "_")
	sub := strings.ReplaceAll(upper(item["subType"]), "-", "_")
	var parts []string
	for _, x := range []any{typ, sub, item["aftTransactionType"], item["aftTransactionCategory"]} {
		if py.S(x) != "" {
			parts = append(parts, Compact(x))
		}
	}
	return typ, sub, strings.Join(parts, "_")
}

func containsAny(s string, needles []string) bool {
	for _, n := range needles {
		if strings.Contains(s, n) {
			return true
		}
	}
	return false
}

func IsCorpShareMove(item Item) bool {
	typ, _, blob := typeBlob(item)
	if containsAny(blob, corpBlobs) {
		return true
	}
	qty := math.Abs(num(item["assetQuantity"]))
	cash := math.Abs(num(item["amount"]))
	if qty != 0 && AssetSymbol(item) != "" && cash == 0 && (strings.Contains(Compact(typ), "DIVIDEND") || strings.Contains(blob, "DISTRIBUT")) {
		return true
	}
	return false
}

func IsCodeChange(item Item) bool {
	_, _, blob := typeBlob(item)
	return containsAny(blob, codeChangeBlobs)
}

func SkipActivity(item Item) bool {
	if len(item) == 0 {
		return true
	}
	if py.Strip(py.S(item["occurredAt"])) == "" {
		return true
	}
	status := Compact(item["status"])
	typ, sub, blob := typeBlob(item)
	if IsCorpShareMove(item) {
		if containsAny(status, []string{"REJECT", "CANCEL", "FAIL", "VOID"}) {
			return true
		}
	} else if (typ == "DIVIDEND" || typ == "INTEREST_CHARGE") && status == "" {
	} else if status == "" || !keepStatus[status] {
		return true
	}
	if typ == "LOAN" || typ == "RECALL" || sub == "LOAN" || sub == "RECALL" {
		return true
	}
	if strings.HasSuffix(typ, "_LOAN") || strings.HasSuffix(sub, "_LOAN") {
		return true
	}
	if strings.HasSuffix(typ, "_RECALL") || strings.HasSuffix(sub, "_RECALL") {
		return true
	}
	if containsAny(blob, SkipTypeMarkers) {
		return true
	}
	if strings.Contains(blob, "SHARE_LENDING") || strings.Contains(blob, "SHARELENDING") {
		return true
	}
	return false
}

func OptionSymbol(item Item) string {
	under := AssetSymbol(item)
	contract := py.S(item["contractType"])
	strike := item["strikePrice"]
	expiry := py.S(item["expiryDate"])
	if contract == "" || strike == nil || expiry == "" || under == "" {
		return under
	}
	ds := py.Strip(expiry)
	if i := strings.Index(ds, "T"); i >= 0 {
		ds = ds[:i]
	}
	ds = strings.ReplaceAll(ds, "/", "-")
	if len(ds) > 10 {
		ds = ds[:10]
	}
	parts := strings.Split(ds, "-")
	if len(parts) != 3 {
		return under
	}
	var year, month, day int
	if _, err := fmt.Sscanf(parts[0], "%d", &year); err != nil {
		return under
	}
	if _, err := fmt.Sscanf(parts[1], "%d", &month); err != nil {
		return under
	}
	if _, err := fmt.Sscanf(parts[2], "%d", &day); err != nil {
		return under
	}
	if month < 1 || month > 12 {
		return under
	}
	strikeF, ok := py.NumOK(strike)
	if !ok {
		return under
	}
	cp := upper(contract)
	if cp == "C" || cp == "CALL" {
		cp = "CALL"
	} else if cp == "P" || cp == "PUT" {
		cp = "PUT"
	}
	return fmt.Sprintf("%s %02d%s%02d %s %s", under, day, Months[month-1], year%100, py.Fixed(strikeF, 2), cp)
}

func SignedCash(item Item) float64 {
	amount := math.Abs(num(item["amount"]))
	typ := strings.ReplaceAll(upper(item["type"]), "-", "_")
	sub := strings.ReplaceAll(upper(item["subType"]), "-", "_")
	if typ == "DIY_BUY" || typ == "OPTIONS_BUY" || typ == "WITHDRAWAL" || (typ == "INTERNAL_TRANSFER" && strings.Contains(sub, "SOURCE")) {
		return -amount
	}
	if typ == "DIY_SELL" || typ == "OPTIONS_SELL" || typ == "DEPOSIT" || typ == "CONTRIBUTION" || typ == "DIVIDEND" || typ == "INTEREST" || (typ == "INTERNAL_TRANSFER" && strings.Contains(sub, "DESTINATION")) {
		return amount
	}
	sign := strings.ToLower(py.Strip(py.S(item["amountSign"])))
	switch sign {
	case "negative", "debit", "-", "neg":
		return -amount
	case "positive", "credit", "+", "pos":
		return amount
	}
	if item["amount"] == nil || py.S(item["amount"]) == "" {
		return 0
	}
	return num(item["amount"])
}

func accountNick(acc Item) string {
	for _, k := range []string{"nickname", "unifiedAccountType", "type"} {
		if v := py.S(acc[k]); v != "" {
			return v
		}
	}
	return ""
}

func AccountType(accountID string, accounts []Item) string {
	for _, a := range accounts {
		if py.S(a["id"]) == accountID {
			return accountNick(a)
		}
	}
	return ""
}

func NavAccountGroups(accounts []Item) (map[string][]string, []string) {
	groups := map[string][]string{}
	var order []string
	for _, acc := range accounts {
		aid := py.Strip(py.S(acc["id"]))
		if aid == "" {
			continue
		}
		nick := py.Strip(accountNick(acc))
		if nick == "" {
			continue
		}
		if _, ok := groups[nick]; !ok {
			order = append(order, nick)
		}
		if !py.Contains(groups[nick], aid) {
			groups[nick] = append(groups[nick], aid)
		}
	}
	return groups, order
}

func FifoPoolIDs(accounts []Item) map[string]string {
	parent := map[string]string{}
	var order []string
	find := func(x string) string {
		if _, ok := parent[x]; !ok {
			parent[x] = x
			order = append(order, x)
		}
		for parent[x] != x {
			parent[x] = parent[parent[x]]
			x = parent[x]
		}
		return x
	}
	union := func(a, b string) {
		if a == "" || b == "" {
			return
		}
		ra, rb := find(a), find(b)
		if ra != rb {
			if ra > rb {
				parent[ra] = rb
			} else {
				parent[rb] = ra
			}
		}
	}
	byNick := map[string][]string{}
	var nickOrder []string
	for _, a := range accounts {
		aid := py.S(a["id"])
		if aid == "" {
			continue
		}
		find(aid)
		if linked, ok := a["linkedAccount"].(map[string]any); ok {
			if lid := py.S(linked["id"]); lid != "" {
				union(aid, lid)
			}
		}
		if nick := py.Strip(py.S(a["nickname"])); nick != "" {
			if _, ok := byNick[nick]; !ok {
				nickOrder = append(nickOrder, nick)
			}
			byNick[nick] = append(byNick[nick], aid)
		}
	}
	for _, nick := range nickOrder {
		ids := byNick[nick]
		for _, other := range ids[1:] {
			union(ids[0], other)
		}
	}
	out := map[string]string{}
	for _, aid := range order {
		out[aid] = find(aid)
	}
	return out
}

func isOption(item Item) bool { return py.S(item["contractType"]) != "" }

func isToClose(sub string) bool {
	c := Compact(sub)
	return strings.Contains(c, "TOCLOSE") || c == "BTC" || c == "STC" || c == "BUYTOCLOSE" || c == "SELLTOCLOSE"
}

func g(v float64) string { return py.G(v) }

func humanDesc(typ, sub, symbol string, qty, px float64) string {
	t := strings.ReplaceAll(upper(typ), "-", "_")
	s := strings.ReplaceAll(upper(sub), "-", "_")
	cs := Compact(sub)
	if t == "DIY_BUY" || (t == "TRADE" && (cs == "BUY" || cs == "BUYTOOPEN" || cs == "BUYTOCLOSE")) {
		verb := "Buy"
		if strings.Contains(cs, "CLOSE") {
			verb = "Buy to close"
		} else if strings.Contains(cs, "OPEN") {
			verb = "Buy to open"
		}
		if qty != 0 && px != 0 {
			return verb + " " + g(qty) + " " + symbol + " @ " + g(px)
		}
		return py.Strip(verb + " " + symbol)
	}
	if t == "DIY_SELL" || (t == "TRADE" && strings.Contains(cs, "SELL")) {
		verb := "Sell"
		if strings.Contains(cs, "CLOSE") {
			verb = "Sell to close"
		} else if strings.Contains(cs, "OPEN") {
			verb = "Sell to open"
		}
		if qty != 0 && px != 0 {
			return verb + " " + g(math.Abs(qty)) + " " + symbol + " @ " + g(px)
		}
		return py.Strip(verb + " " + symbol)
	}
	switch {
	case t == "DEPOSIT" || t == "CONTRIBUTION":
		return "Deposit"
	case t == "WITHDRAWAL":
		return "Withdrawal"
	case t == "INTERNAL_TRANSFER":
		if strings.Contains(s, "SOURCE") {
			return "Transfer out"
		}
		return "Transfer in"
	case t == "DIVIDEND":
		if symbol != "" {
			return "Dividend: " + symbol
		}
		return "Dividend"
	case t == "INTEREST":
		if strings.Contains(s, "FPL") {
			return "Stock Lending Earnings"
		}
		return "Interest"
	case t == "FUNDS_CONVERSION":
		return "Funds conversion"
	case t == "FEE" || t == "REFUND":
		if t == "REFUND" {
			return "Fee refund"
		}
		return "Fee"
	case t == "STOCK_DISTRIBUTION" || t == "STKDIS" || t == "SPIN" || t == "SPINOFF":
		if symbol != "" {
			return "Stock distribution: " + symbol
		}
		return "Stock distribution"
	case t == "EXPIR" || t == "EXPIRY" || t == "EXPIRE" || t == "ASSIGN" || t == "ASSIGNMENT" || t == "EXERCISE" || strings.Contains(t, "EXPIR") || strings.Contains(t, "ASSIGN") || strings.Contains(t, "EXERCISE"):
		label := "Expir"
		if strings.Contains(t, "ASSIGN") {
			label = "Assign"
		} else if strings.Contains(t, "EXERCISE") {
			label = "Exercise"
		}
		return py.Strip(label + " " + symbol)
	}
	if symbol != "" {
		return t + ": " + symbol
	}
	if t == "" {
		return "Activity"
	}
	return py.Title(strings.ReplaceAll(t, "_", " "))
}

func MapActivityRows(item Item, accounts []Item) []store.Activity {
	if len(item) == 0 {
		return nil
	}
	src := AssetSymbol(item)
	dst := CounterSymbol(item)
	qty := math.Abs(num(item["assetQuantity"]))
	if src != "" && dst != "" && src != dst && qty != 0 && IsCorpShareMove(item) {
		cid := py.Strip(py.S(item["canonicalId"]))
		if cid == "" {
			cid = "swap"
		}
		outgoing := cloneItem(item)
		outgoing["assetSymbol"] = src
		outgoing["counterAssetSymbol"] = ""
		outgoing["type"] = "STKDIS"
		outgoing["subType"] = "STKDIS"
		outgoing["assetQuantity"] = -qty
		outgoing["amount"] = 0.0
		outgoing["amountSign"] = "negative"
		outgoing["canonicalId"] = cid + ":out"
		incoming := cloneItem(item)
		incoming["assetSymbol"] = dst
		incoming["counterAssetSymbol"] = ""
		incoming["type"] = "STKDIS"
		incoming["subType"] = "STKDIS"
		incoming["assetQuantity"] = qty
		incoming["amount"] = 0.0
		incoming["amountSign"] = "positive"
		incoming["canonicalId"] = cid + ":in"
		var rows []store.Activity
		for _, it := range []Item{outgoing, incoming} {
			if r := MapActivity(it, accounts); r != nil {
				rows = append(rows, *r)
			}
		}
		return rows
	}
	if r := MapActivity(item, accounts); r != nil {
		return []store.Activity{*r}
	}
	return nil
}

func cloneItem(item Item) Item {
	out := Item{}
	for k, v := range item {
		out[k] = v
	}
	return out
}

func MapActivity(item Item, accounts []Item) *store.Activity {
	if SkipActivity(item) {
		return nil
	}
	occurred := py.Strip(py.S(item["occurredAt"]))
	transactionDate := DateOnly(occurred)
	if transactionDate == "" {
		return nil
	}
	accountID := py.S(item["accountId"])
	typ := strings.ReplaceAll(upper(item["type"]), "-", "_")
	sub := strings.ReplaceAll(upper(item["subType"]), "-", "_")
	qtyRaw := num(item["assetQuantity"])
	qtyAbs := math.Abs(qtyRaw)
	cash := SignedCash(item)
	amountAbs := math.Abs(num(item["amount"]))
	fees := math.Abs(num(item["fees"]))
	isOpt := isOption(item)
	symbol := AssetSymbol(item)
	if isOpt {
		symbol = OptionSymbol(item)
	}
	cur := upper(item["currency"])
	if cur != "CAD" && cur != "USD" {
		cur = "CAD"
		if isOpt {
			cur = "USD"
		}
	}
	unitPrice := 0.0
	if qtyAbs != 0 {
		if isOpt {
			unitPrice = amountAbs / (qtyAbs * 100.0)
		} else {
			unitPrice = amountAbs / qtyAbs
		}
	}
	activityType := "Other"
	activitySub := sub
	if activitySub == "" {
		activitySub = typ
	}
	category := "other"
	quantity := qtyAbs
	ctyp := Compact(typ)
	switch {
	case typ == "DIY_BUY":
		category = "trade"
		activityType = "Trade"
		if isOpt {
			if isToClose(sub) {
				activitySub = "BUYTOCLOSE"
			} else {
				activitySub = "BUYTOOPEN"
			}
		} else {
			activitySub = "BUY"
		}
		quantity = qtyAbs
	case typ == "DIY_SELL":
		category = "trade"
		activityType = "Trade"
		if isOpt {
			if isToClose(sub) {
				activitySub = "SELLTOCLOSE"
			} else {
				activitySub = "SELLTOOPEN"
			}
		} else {
			activitySub = "SELL"
		}
		quantity = -qtyAbs
	case typ == "OPTIONS_BUY":
		category = "trade"
		activityType = "OPTIONS_BUY"
		if isToClose(sub) {
			activitySub = "BUYTOCLOSE"
		} else {
			activitySub = "BUYTOOPEN"
		}
		quantity = qtyAbs
	case typ == "OPTIONS_SELL":
		category = "trade"
		activityType = "OPTIONS_SELL"
		if isToClose(sub) {
			activitySub = "SELLTOCLOSE"
		} else {
			activitySub = "SELLTOOPEN"
		}
		quantity = -qtyAbs
	case strings.Contains(typ, "MULTILEG"):
		category = "trade"
		if cash < 0 {
			activityType, activitySub = "OPTIONS_BUY", "BUYTOCLOSE"
			quantity = qtyAbs
		} else {
			activityType, activitySub = "OPTIONS_SELL", "SELLTOOPEN"
			quantity = 0
			if qtyAbs != 0 {
				quantity = -qtyAbs
			}
		}
	case typ == "EXPIR" || typ == "EXPIRY" || typ == "EXPIRE" || typ == "ASSIGN" || typ == "ASSIGNMENT" || typ == "EXERCISE" || strings.Contains(typ, "EXPIR") || strings.Contains(typ, "ASSIGN") || strings.Contains(typ, "EXERCISE"):
		category = "option_event"
		keep := "EXPIR"
		if strings.Contains(typ, "ASSIGN") {
			keep = "ASSIGN"
		} else if strings.Contains(typ, "EXERCISE") {
			keep = "EXERCISE"
		}
		activityType = keep
		shortExpir := strings.Contains(typ, "SHORT_EXPIR") || (strings.Contains(typ, "SHORT") && strings.Contains(typ, "EXPIR"))
		switch {
		case strings.Contains(typ, "ASSIGN"):
			activitySub = "BUYTOCLOSE"
		case shortExpir:
			activitySub = "BUY"
		case strings.Contains(typ, "EXPIR"):
			activitySub = "SELL"
		case strings.Contains(Compact(sub), "COVER") || isToClose(sub):
			activitySub = "BUY"
		default:
			activitySub = "SELL"
		}
		if activitySub == "SELL" {
			quantity = -qtyAbs
		} else {
			quantity = qtyAbs
		}
		if strings.Contains(typ, "ASSIGN") || math.Abs(cash) < 1e-12 {
			unitPrice = 0
		}
		if !isOpt && symbol == "" {
			symbol = AssetSymbol(item)
		}
	case typ == "DEPOSIT" || typ == "CONTRIBUTION":
		activityType, activitySub, category = "Deposit", "deposit", "deposit"
	case typ == "WITHDRAWAL":
		activityType, activitySub, category = "Withdrawal", "withdrawal", "withdrawal"
	case typ == "INTERNAL_TRANSFER" || ctyp == "TRFIN" || ctyp == "TRFOUT" || ctyp == "TRANSFERIN" || ctyp == "TRANSFEROUT" || ctyp == "INTERNALTRANSFER":
		activityType, activitySub, category = "Transfer", "transfer", "transfer"
	case typ == "DIVIDEND" && !IsCorpShareMove(item):
		activityType, activitySub, category = "Dividend", "dividend", "dividend"
	case typ == "INTEREST" || strings.Contains(sub, "FPL_INTEREST") || ctyp == "FPLINTEREST":
		activityType, activitySub, category = "Interest", "interest", "interest"
	case typ == "FUNDS_CONVERSION":
		activityType, activitySub, category = "FxExchange", "fx", "fx"
	case typ == "FEE" || typ == "REFUND":
		activityType = "Fee"
		if typ == "REFUND" {
			activityType = "Refund"
		}
		activitySub, category = "fee", "fee"
	case IsCorpShareMove(item) || typ == "STOCK_DISTRIBUTION" || typ == "STKDIS" || typ == "SPIN" || typ == "SPINOFF" || typ == "STK_DIS" || strings.Contains(ctyp, "STKDIS") || strings.Contains(ctyp, "STOCKDISTRIBUTION") || strings.Contains(Compact(sub), "STOCKDISTRIBUTION"):
		activityType, category = "STKDIS", "trade"
		unitPrice = 0
		sign := strings.ToLower(py.Strip(py.S(item["amountSign"])))
		outgoing := qtyRaw < 0 || sign == "negative" || sign == "debit" || sign == "-" || sign == "neg"
		if !outgoing && IsCodeChange(item) && CounterSymbol(item) == "" && !strings.Contains(Compact(item["type"]), "STKDIS") {
			outgoing = true
		}
		if outgoing {
			activitySub = "SELL"
			quantity = -qtyAbs
		} else {
			activitySub = "BUY"
			quantity = qtyAbs
		}
	default:
		activityType = py.S(item["type"])
		if activityType == "" {
			activityType = "Other"
		}
		activitySub = py.S(item["subType"])
		if activitySub == "" {
			activitySub = "other"
		}
		category = "other"
	}
	if activitySub == "SELL" || activitySub == "SELLTOOPEN" || activitySub == "SELLTOCLOSE" {
		if qtyAbs != 0 {
			quantity = -qtyAbs
		}
	}
	sign := strings.ToLower(py.Strip(py.S(item["amountSign"])))
	direction := ""
	if sign == "negative" || sign == "debit" || sign == "-" || sign == "neg" || cash < 0 {
		direction = "DEBIT"
	} else if sign == "positive" || sign == "credit" || sign == "+" || sign == "pos" || cash > 0 {
		direction = "CREDIT"
	}
	cid := py.Strip(py.S(item["canonicalId"]))
	if store.LooksLikeHomemadeID(cid) {
		cid = ""
	}
	desc := humanDesc(typ, sub, symbol, quantity, unitPrice)
	name := py.S(item["aftOriginatorName"])
	if name == "" {
		name = py.S(item["institutionName"])
	}
	if name == "" {
		name = symbol
	}
	fifoID := accountID
	accountType := ""
	if len(accounts) > 0 {
		if v, ok := FifoPoolIDs(accounts)[accountID]; ok {
			fifoID = v
		}
		accountType = AccountType(accountID, accounts)
	}
	return &store.Activity{
		CanonicalID:     cid,
		OccurredAt:      occurred,
		TransactionDate: transactionDate,
		SettlementDate:  transactionDate,
		AccountID:       accountID,
		BookID:          accountID,
		FifoID:          fifoID,
		AccountType:     accountType,
		ActivityType:    activityType,
		ActivitySubType: activitySub,
		Description:     desc,
		Direction:       direction,
		Symbol:          symbol,
		Name:            name,
		Currency:        cur,
		Quantity:        quantity,
		UnitPrice:       unitPrice,
		Commission:      fees,
		NetCashAmount:   cash,
		Category:        category,
		Source:          "wealthsimple",
		RawType:         py.S(item["type"]),
		AftType:         py.S(item["aftTransactionType"]),
		CounterSymbol:   CounterSymbol(item),
		SecurityID:      py.Strip(py.S(item["securityId"])),
	}
}

func MarginBoostTarget(acc Item) string {
	features, _ := acc["accountFeatures"].([]any)
	for _, raw := range features {
		f, ok := raw.(map[string]any)
		if !ok || strings.ToUpper(py.S(f["name"])) != "MARGIN_BOOST" {
			continue
		}
		if enabled, _ := f["enabled"].(bool); !enabled {
			continue
		}
		if functional, ok := f["functional"].(bool); ok && !functional {
			continue
		}
		md, _ := f["metadata"].(map[string]any)
		return py.S(md["targetMarginAccountId"])
	}
	return ""
}

func SlimAccount(acc Item) store.Account {
	var nlv *float64
	if fin, ok := acc["financials"].(map[string]any); ok {
		if cc, ok := fin["currentCombined"].(map[string]any); ok {
			if money, ok := cc["netLiquidationValue"].(map[string]any); ok {
				if v, ok := py.NumOK(money["amount"]); ok {
					nlv = &v
				}
			}
		}
	}
	return store.Account{ID: py.S(acc["id"]), Nickname: py.S(acc["nickname"]), UnifiedAccountType: py.S(acc["unifiedAccountType"]), Currency: py.S(acc["currency"]), Status: py.S(acc["status"]), Type: py.S(acc["type"]), NetLiquidationValue: nlv}
}

func SlimAccounts(accounts []Item) []store.Account {
	custodian := map[string]string{}
	for _, a := range accounts {
		cas, _ := a["custodianAccounts"].([]any)
		for _, raw := range cas {
			if c, ok := raw.(map[string]any); ok && py.S(c["id"]) != "" {
				custodian[py.S(c["id"])] = py.S(a["id"])
			}
		}
	}
	out := []store.Account{}
	for _, a := range accounts {
		row := SlimAccount(a)
		if target := MarginBoostTarget(a); target != "" {
			row.MarginAccountID = custodian[target]
		}
		out = append(out, row)
	}
	return out
}

func MarginAccountIDs(accounts []Item) []string {
	var out []string
	for _, a := range accounts {
		typ := strings.ToUpper(py.S(a["unifiedAccountType"]))
		if typ == "" {
			typ = strings.ToUpper(py.S(a["unified_account_type"]))
		}
		status := strings.ToLower(py.S(a["status"]))
		if py.S(a["id"]) != "" && strings.Contains(typ, "MARGIN") && status != "closed" {
			out = append(out, py.S(a["id"]))
		}
	}
	return out
}

func ParseMargin(data map[string]any) *store.Margin {
	acc, _ := data["account"].(map[string]any)
	fin, _ := acc["financials"].(map[string]any)
	cur, _ := fin["current"].(map[string]any)
	m3, _ := cur["marginV3"].(map[string]any)
	trading, _ := m3["trading"].(map[string]any)
	bp, ok := trading["buyingPower"].(map[string]any)
	if !ok {
		return nil
	}
	if py.S(bp["__typename"]) == "BuyingPowerMetricAvailable" {
		total, _ := bp["total"].(map[string]any)
		amount, ok := py.NumOK(total["amount"])
		if !ok {
			return nil
		}
		ccy := py.S(total["currency"])
		if ccy == "" {
			ccy = "CAD"
		}
		return &store.Margin{BuyingPower: &amount, Currency: ccy}
	}
	reason, _ := bp["reason"].(map[string]any)
	why := py.S(reason["__typename"])
	if why == "" {
		why = py.S(bp["__typename"])
	}
	if why == "" {
		why = "unavailable"
	}
	if secs, ok := reason["securities"].([]any); ok && len(secs) > 0 {
		why += fmt.Sprintf(" (%d securities)", len(secs))
	}
	return &store.Margin{Currency: "CAD", Unavailable: why}
}

func SecurityRecord(sec map[string]any, sid string) *store.Security {
	if len(sec) == 0 {
		return nil
	}
	stock, _ := sec["stock"].(map[string]any)
	option, _ := sec["optionDetails"].(map[string]any)
	under, _ := option["underlyingSecurity"].(map[string]any)
	id := py.Strip(py.S(sec["id"]))
	if id == "" {
		id = sid
	}
	return &store.Security{ID: id, Symbol: py.Strip(py.S(stock["symbol"])), Name: py.Strip(py.S(stock["name"])), PrimaryExchange: py.Strip(py.S(stock["primaryExchange"])), PrimaryMic: py.Strip(py.S(stock["primaryMic"])), Currency: py.Strip(py.S(sec["currency"])), UnderlyingID: py.Strip(py.S(under["id"]))}
}

func moneyAmount(node map[string]any, keys ...string) (*float64, string) {
	for _, key := range keys {
		money, ok := node[key].(map[string]any)
		if !ok || money["amount"] == nil {
			continue
		}
		v, ok := py.NumOK(money["amount"])
		if !ok {
			continue
		}
		ccy := py.S(money["currency"])
		if ccy == "" {
			ccy = "CAD"
		}
		return &v, ccy
	}
	return nil, ""
}

func NavPointsFromPayload(data map[string]any) ([]store.NavPoint, map[string]any) {
	var points []store.NavPoint
	ident, _ := data["identity"].(map[string]any)
	acc, _ := data["account"].(map[string]any)
	var fin map[string]any
	if f, ok := ident["financials"].(map[string]any); ok && f != nil {
		fin = f
	} else {
		fin, _ = acc["financials"].(map[string]any)
	}
	hist, _ := fin["historicalDaily"].(map[string]any)
	edges, _ := hist["edges"].([]any)
	for _, raw := range edges {
		edge, _ := raw.(map[string]any)
		node, _ := edge["node"].(map[string]any)
		amt, cur := moneyAmount(node, "netLiquidationValue", "netLiquidationValueV2")
		d := py.S(node["date"])
		if len(d) > 10 {
			d = d[:10]
		}
		if d == "" || amt == nil {
			continue
		}
		if cur == "" {
			cur = "CAD"
		}
		rec := store.NavPoint{Date: d, Equity: *amt, Currency: cur}
		if nd, _ := moneyAmount(node, "netDeposits", "netDepositsV2"); nd != nil {
			rec.NetDeposits = nd
		}
		points = append(points, rec)
	}
	page, _ := hist["pageInfo"].(map[string]any)
	if page == nil {
		page = map[string]any{}
	}
	return points, page
}

func MergeNavPoints(seriesList [][]store.NavPoint) []store.NavPoint {
	byDate := map[string]*store.NavPoint{}
	for _, series := range seriesList {
		for _, rec := range series {
			d := rec.Date
			if len(d) > 10 {
				d = d[:10]
			}
			if d == "" {
				continue
			}
			cur, ok := byDate[d]
			if !ok {
				ccy := rec.Currency
				if ccy == "" {
					ccy = "CAD"
				}
				cur = &store.NavPoint{Date: d, Currency: ccy}
				byDate[d] = cur
			}
			cur.Equity += rec.Equity
			if rec.Currency != "" {
				cur.Currency = rec.Currency
			}
			if rec.NetDeposits != nil {
				cur.NetDeposits = py.Ptr(py.Deref(cur.NetDeposits, 0) + *rec.NetDeposits)
			}
		}
	}
	keys := make([]string, 0, len(byDate))
	for k := range byDate {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	out := make([]store.NavPoint, 0, len(keys))
	for _, k := range keys {
		out = append(out, *byDate[k])
	}
	return out
}

type Quote struct {
	SecurityID    string   `json:"securityId"`
	Symbol        string   `json:"symbol"`
	Name          string   `json:"name"`
	Exchange      string   `json:"exchange"`
	Currency      string   `json:"currency"`
	SecurityType  string   `json:"securityType"`
	Buyable       bool     `json:"buyable"`
	Sellable      bool     `json:"sellable"`
	TradeEligible bool     `json:"tradeEligible"`
	Status        string   `json:"status"`
	Last          *float64 `json:"last"`
	Bid           *float64 `json:"bid"`
	Ask           *float64 `json:"ask"`
	BidSize       *float64 `json:"bidSize"`
	AskSize       *float64 `json:"askSize"`
	Mid           *float64 `json:"mid"`
	Change        *float64 `json:"change"`
	ChangePct     *float64 `json:"changePct"`
	MarketStatus  string   `json:"marketStatus"`
	QuotedAsOf    string   `json:"quotedAsOf"`
	Multiplier    *float64 `json:"multiplier"`
}

func numPtr(v any) *float64 {
	f, ok := py.NumOK(v)
	if !ok {
		return nil
	}
	return &f
}

func ParseQuote(node map[string]any) *Quote {
	if len(node) == 0 || py.S(node["id"]) == "" {
		return nil
	}
	q, _ := node["quoteV2"].(map[string]any)
	stock, _ := node["stock"].(map[string]any)
	opt, _ := node["optionDetails"].(map[string]any)
	last := numPtr(q["price"])
	if last == nil {
		last = numPtr(q["last"])
	}
	base := numPtr(q["previousBaseline"])
	if base == nil {
		base = numPtr(q["referenceClose"])
	}
	bid, ask := numPtr(q["bid"]), numPtr(q["ask"])
	var change, changePct *float64
	if last != nil && base != nil {
		change = py.Ptr(*last - *base)
		if *base != 0 {
			changePct = py.Ptr(*change / *base)
		}
	}
	mid := numPtr(q["mid"])
	if q["mid"] == nil && bid != nil && ask != nil {
		mid = py.Ptr((*bid + *ask) / 2)
	}
	ccy := py.S(q["currency"])
	if ccy == "" {
		ccy = py.S(node["currency"])
	}
	buyable, _ := node["buyable"].(bool)
	sellable, _ := node["sellable"].(bool)
	eligible, _ := node["wsTradeEligible"].(bool)
	var mult *float64
	if opt != nil {
		mult = numPtr(opt["multiplier"])
	}
	return &Quote{SecurityID: py.S(node["id"]), Symbol: py.S(stock["symbol"]), Name: py.S(stock["name"]), Exchange: py.S(stock["primaryExchange"]), Currency: strings.ToUpper(ccy), SecurityType: py.S(node["securityType"]),
		Buyable: buyable, Sellable: sellable, TradeEligible: eligible, Status: py.S(node["status"]), Last: last, Bid: bid, Ask: ask, BidSize: numPtr(q["bidSize"]), AskSize: numPtr(q["askSize"]), Mid: mid,
		Change: change, ChangePct: changePct, MarketStatus: py.S(q["marketStatus"]), QuotedAsOf: py.S(q["quotedAsOf"]), Multiplier: mult}
}

var OrderExecTypes = []string{"MARKET", "LIMIT", "STOP", "STOP_LIMIT"}

type MarketData struct {
	OrderTypes []string `json:"orderTypes"`
	MarginRate *float64 `json:"marginRate"`
}

func ParseMarketData(data map[string]any) MarketData {
	sec, _ := data["security"].(map[string]any)
	subs, _ := sec["allowedOrderSubtypes"].([]any)
	set := map[string]bool{}
	for _, s := range subs {
		if py.S(s) != "" {
			set[strings.ToUpper(py.S(s))] = true
		}
	}
	rates, _ := sec["marginRates"].(map[string]any)
	rate := numPtr(rates["clientMarginRate"])
	if rate != nil && *rate > 1 {
		rate = py.Ptr(*rate / 100.0)
	}
	types := []string{}
	for _, t := range OrderExecTypes {
		if set[t] {
			types = append(types, t)
		}
	}
	return MarketData{OrderTypes: types, MarginRate: rate}
}

type BuyingPower struct {
	BuyingPower *float64
	Cash        *float64
	Currency    string
}

func ParseBuyingPower(data map[string]any) BuyingPower {
	acc, _ := data["account"].(map[string]any)
	fin, _ := acc["financials"].(map[string]any)
	cur, _ := fin["current"].(map[string]any)
	view, _ := cur["tradingBalanceViewV2"].(map[string]any)
	bp, _ := view["buyingPower"].(map[string]any)
	cash, _ := view["cash"].(map[string]any)
	ccy := py.S(bp["currency"])
	if ccy == "" {
		ccy = py.S(cash["currency"])
	}
	return BuyingPower{BuyingPower: numPtr(bp["quantity"]), Cash: numPtr(cash["quantity"]), Currency: ccy}
}

var LookupTypes = map[string]bool{"EQUITY": true, "EXCHANGE_TRADED_FUND": true}
var CanadianSuffixes = []string{".TO", ".V", ".CN", ".NE"}

func BareSymbol(sym string) string {
	s := strings.ToUpper(sym)
	for _, suf := range CanadianSuffixes {
		if strings.HasSuffix(s, suf) {
			return s[:len(s)-len(suf)]
		}
	}
	return s
}

func ParseListingSearch(data map[string]any, symbol, exchange string) *store.Security {
	wantSym, wantEx := BareSymbol(symbol), strings.ToUpper(py.Strip(exchange))
	ss, _ := data["securitySearch"].(map[string]any)
	results, _ := ss["results"].([]any)
	for _, raw := range results {
		r, ok := raw.(map[string]any)
		if !ok || py.S(r["id"]) == "" {
			continue
		}
		stock, _ := r["stock"].(map[string]any)
		if BareSymbol(py.S(stock["symbol"])) != wantSym || strings.ToUpper(py.S(stock["primaryExchange"])) != wantEx {
			continue
		}
		if !LookupTypes[strings.ToUpper(py.S(r["securityType"]))] {
			continue
		}
		return &store.Security{ID: py.S(r["id"]), Symbol: strings.ToUpper(py.S(stock["symbol"])), Name: py.S(stock["name"]), PrimaryExchange: py.S(stock["primaryExchange"]), PrimaryMic: py.S(stock["primaryMic"]), Currency: strings.ToUpper(py.S(r["currency"]))}
	}
	return nil
}

var WSPending = []string{"NEW", "PENDING_SUBMISSION", "PENDING_REVIEW", "PENDING_FUND_TRANSFER", "SUBMITTED", "PLACED", "PARTIALLY_FILLED", "CONTINGENT"}
var WSCancelling = []string{"CANCEL_PENDING"}
var WSStatusMap = map[string]string{"FILLED": "filled", "POSTED": "filled", "CANCELLED": "cancelled", "DELETED": "cancelled", "EXPIRED": "expired", "REJECTED": "rejected"}

func AppStatus(wsStatus string) string {
	s := strings.ToUpper(wsStatus)
	if py.Contains(WSPending, s) {
		return "pending"
	}
	if py.Contains(WSCancelling, s) {
		return "cancelling"
	}
	if v, ok := WSStatusMap[s]; ok {
		return v
	}
	if s != "" {
		return "pending"
	}
	return ""
}

type OrderUpdate struct {
	WsStatus      string
	Status        string
	FilledQty     *float64
	AvgFill       *float64
	SubmittedAt   string
	ExpiresAt     string
	FirstFilledAt string
	LastFilledAt  string
	Error         string
	Quantity      *float64
	LimitPrice    *float64
	StopPrice     *float64
	Tif           string
	Currency      string
	AccountID     string
	SecurityID    string
	Type          string
	Symbol        string
}

func ParseExtendedOrder(data map[string]any) *OrderUpdate {
	o, ok := data["soOrdersExtendedOrder"].(map[string]any)
	if !ok || py.S(o["status"]) == "" {
		return nil
	}
	errText := py.S(o["rejectionCause"])
	if errText == "" {
		errText = py.S(o["rejectionCode"])
	}
	accountID := py.S(o["canonicalAccountId"])
	if accountID == "" {
		accountID = py.S(o["accountId"])
	}
	return &OrderUpdate{WsStatus: strings.ToUpper(py.S(o["status"])), Status: AppStatus(py.S(o["status"])), FilledQty: numPtr(o["filledQuantity"]), AvgFill: numPtr(o["averageFilledPrice"]),
		SubmittedAt: py.S(o["submittedAtUtc"]), ExpiresAt: py.S(o["expiredAtUtc"]), FirstFilledAt: py.S(o["firstFilledAtUtc"]), LastFilledAt: py.S(o["lastFilledAtUtc"]), Error: errText,
		Quantity: numPtr(o["submittedQuantity"]), LimitPrice: numPtr(o["limitPrice"]), StopPrice: numPtr(o["stopPrice"]), Tif: strings.ToUpper(py.S(o["timeInForce"])), Currency: strings.ToUpper(py.S(o["securityCurrency"])),
		AccountID: accountID, SecurityID: py.S(o["securityId"]), Type: strings.ToUpper(py.S(o["orderType"]))}
}
