package csvimport

import (
	"encoding/json"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
	"github.com/ProfessorBagholder/Bagholder/internal/symbols"
)

const (
	WatchMeta      = "watch_folder"
	WatchFilesMeta = "watch_files"
	WatchLastMeta  = "watch_last"
)

var months = map[string]string{"jan": "01", "feb": "02", "mar": "03", "apr": "04", "may": "05", "jun": "06", "jul": "07", "aug": "08", "sep": "09", "oct": "10", "nov": "11", "dec": "12"}

var headerSepRE = regexp.MustCompile(`[` + py.SpaceClass + `\-]+`)

func NormalizeHeader(h string) string {
	s := strings.ReplaceAll(h, "\ufeff", "")
	s = py.Strip(s)
	s = strings.Trim(s, "\"'")
	s = strings.ToLower(py.Strip(s))
	return headerSepRE.ReplaceAllString(s, "_")
}

var numJunkRE = regexp.MustCompile(`[$£€,` + py.SpaceClass + `]|CAD|USD|cad|usd`)

func ParseNumber(raw string) float64 {
	s := py.Strip(raw)
	if s == "" || s == "-" || s == "—" || strings.ToLower(s) == "n/a" {
		return 0
	}
	paren := strings.HasPrefix(s, "(") && strings.HasSuffix(s, ")")
	s = strings.ReplaceAll(strings.ReplaceAll(s, "(", ""), ")", "")
	s = numJunkRE.ReplaceAllString(s, "")
	if s == "" {
		return 0
	}
	n, ok := py.NumOK(s)
	if !ok {
		return 0
	}
	if paren {
		return -math.Abs(n)
	}
	return n
}

var (
	isoDateRE   = regexp.MustCompile(`^(\d{4})-(\d{2})-(\d{2})(?:[T` + py.SpaceClass + `].*)?$`)
	slashYMDRE  = regexp.MustCompile(`^(\d{4})[/.](\d{1,2})[/.](\d{1,2})(?:[` + py.SpaceClass + `].*)?$`)
	dMonYRE     = regexp.MustCompile(`^(\d{1,2})[- ]([A-Za-z]{3})[- ](\d{4})$`)
	monDYRE     = regexp.MustCompile(`^([A-Za-z]{3})[- ](\d{1,2}),?[- ](\d{4})$`)
	dmyRE       = regexp.MustCompile(`^(\d{1,2})[/\-.](\d{1,2})[/\-.](\d{4})(?:[` + py.SpaceClass + `].*)?$`)
	serialRE    = regexp.MustCompile(`^\d{4,6}(\.\d+)?$`)
	footerRE    = regexp.MustCompile(`(?i)^[` + py.SpaceClass + `]*as of[` + py.SpaceClass + `]+\d{4}-\d{2}-\d{2}`)
	compactRE   = regexp.MustCompile(`[` + py.SpaceClass + `_\-]`)
	dashRE      = regexp.MustCompile(`^([A-Za-z][A-Za-z0-9.\-]{0,20})[` + py.SpaceClass + `]+-[` + py.SpaceClass + `]+(.+)$`)
	wsRE        = regexp.MustCompile(`[` + py.SpaceClass + `]`)
	tickerRE    = regexp.MustCompile(`^[A-Za-z][A-Za-z0-9.\-]{0,20}$`)
	sharesRE    = regexp.MustCompile(`(?i)(-?[\d,]+(?:\.\d+)?)[` + py.SpaceClass + `]+shares?\b`)
	contractsRE = regexp.MustCompile(`(?i)(-?[\d,]+(?:\.\d+)?)[` + py.SpaceClass + `]+contracts?\b`)
	priceRE     = regexp.MustCompile(`(?i)\bat[` + py.SpaceClass + `]+\$?([\d,]+(?:\.\d+)?)[` + py.SpaceClass + `]+per[` + py.SpaceClass + `]+share\b`)
	execRE      = regexp.MustCompile(`(?i)\([` + py.SpaceClass + `]*executed at[` + py.SpaceClass + `]+(\d{4}-\d{2}-\d{2})[` + py.SpaceClass + `]*\)`)
	ccyKeyRE    = regexp.MustCompile(`(?i)\b(USD|CAD)\b`)
	bookFileRE1 = regexp.MustCompile(`(?i)([A-Z0-9]{8,}(?:CAD|USD))-\d{4}-\d{2}-\d{2}`)
	bookFileRE2 = regexp.MustCompile(`(?i)([A-Z0-9]{8,}(?:CAD|USD))`)
)

func atoi(s string) int {
	n, _ := strconv.Atoi(s)
	return n
}

func ParseDate(raw string) string {
	s := py.Strip(raw)
	if s == "" {
		return ""
	}
	if m := isoDateRE.FindStringSubmatch(s); m != nil {
		return m[1] + "-" + m[2] + "-" + m[3]
	}
	if m := slashYMDRE.FindStringSubmatch(s); m != nil {
		return fmt.Sprintf("%s-%02d-%02d", m[1], atoi(m[2]), atoi(m[3]))
	}
	if m := dMonYRE.FindStringSubmatch(s); m != nil {
		if mo, ok := months[strings.ToLower(m[2])]; ok {
			return fmt.Sprintf("%s-%s-%02d", m[3], mo, atoi(m[1]))
		}
	}
	if m := monDYRE.FindStringSubmatch(s); m != nil {
		if mo, ok := months[strings.ToLower(m[1])]; ok {
			return fmt.Sprintf("%s-%s-%02d", m[3], mo, atoi(m[2]))
		}
	}
	if m := dmyRE.FindStringSubmatch(s); m != nil {
		a, b, y := atoi(m[1]), atoi(m[2]), m[3]
		if a > 12 && b <= 12 {
			return fmt.Sprintf("%s-%02d-%02d", y, b, a)
		}
		return fmt.Sprintf("%s-%02d-%02d", y, a, b)
	}
	if serialRE.MatchString(s) {
		serial, _ := strconv.ParseFloat(s, 64)
		if 20000 < serial && serial < 80000 {
			return time.Date(1899, 12, 30, 0, 0, 0, 0, time.UTC).AddDate(0, 0, py.RoundInt(serial)).Format("2006-01-02")
		}
	}
	return ""
}

func IsFooterLine(text string) bool { return footerRE.MatchString(text) }

func DetectFormat(headers []string) string {
	norms := map[string]bool{}
	for _, h := range headers {
		norms[NormalizeHeader(h)] = true
	}
	canonHits, legacyHits := 0, 0
	for _, h := range []string{"transaction_date", "activity_type", "activity_sub_type", "net_cash_amount", "unit_price"} {
		if norms[h] {
			canonHits++
		}
	}
	for _, h := range []string{"date", "action", "symbol", "quantity", "price", "amount"} {
		if norms[h] {
			legacyHits++
		}
	}
	if canonHits >= 3 {
		return "canonical"
	}
	if norms["transaction_date"] || norms["activity_type"] {
		return "canonical"
	}
	if norms["date"] && norms["transaction"] && norms["description"] && norms["amount"] {
		return "statement"
	}
	if legacyHits >= 5 && (norms["action"] || norms["date"]) {
		return "legacy"
	}
	if norms["action"] && norms["date"] {
		return "legacy"
	}
	return "unknown"
}

func compactLower(s string) string {
	return compactRE.ReplaceAllString(strings.ToLower(py.Strip(s)), "")
}

func containsAny(s string, needles ...string) bool {
	for _, n := range needles {
		if strings.Contains(s, n) {
			return true
		}
	}
	return false
}

func Categorize(activityType, activitySubType string) string {
	t := compactLower(activityType)
	s := compactLower(activitySubType)
	blob := t + " " + s
	if t == "fxexchange" || t == "fx" || s == "fxexchange" || strings.Contains(blob, "fxexchange") {
		return "fx"
	}
	for _, k := range []string{"expir", "exercise", "assign"} {
		if strings.Contains(t, k) || strings.Contains(s, k) {
			return "option_event"
		}
	}
	if t == "trade" || s == "buy" || s == "sell" {
		return "trade"
	}
	both := func(k string) bool { return strings.Contains(t, k) || strings.Contains(s, k) }
	switch {
	case both("dividend"):
		return "dividend"
	case both("deposit"):
		return "deposit"
	case both("withdraw"):
		return "withdrawal"
	case both("interest"):
		return "interest"
	case both("fee"):
		return "fee"
	case both("transfer"):
		return "transfer"
	}
	return "other"
}

func ExtractInstrument(description string) (string, string) {
	text := py.Strip(description)
	if text == "" {
		return "", ""
	}
	colon := strings.Index(text, ":")
	if colon < 0 {
		if m := dashRE.FindStringSubmatch(text); m != nil {
			return strings.ToUpper(m[1]), py.Strip(m[2])
		}
		return "", ""
	}
	left := py.Strip(text[:colon])
	if m := dashRE.FindStringSubmatch(left); m != nil {
		return strings.ToUpper(m[1]), py.Strip(m[2])
	}
	if wsRE.MatchString(left) {
		sym := strings.ToUpper(py.CollapseSpace(left))
		return sym, sym
	}
	if tickerRE.MatchString(left) {
		return strings.ToUpper(left), strings.ToUpper(left)
	}
	return "", ""
}

type parsedDesc struct {
	Symbol, Name   string
	Quantity       float64
	UnitPrice      float64
	ExecutedAt     string
	FillParsed     bool
	ContractSigned float64
	SharesSigned   float64
}

func ParseStatementDescription(description string) parsedDesc {
	p := parsedDesc{}
	p.Symbol, p.Name = ExtractInstrument(description)
	shares := sharesRE.FindStringSubmatch(description)
	contracts := contractsRE.FindStringSubmatch(description)
	price := priceRE.FindStringSubmatch(description)
	if shares != nil {
		p.SharesSigned = ParseNumber(shares[1])
		p.Quantity = math.Abs(p.SharesSigned)
		p.FillParsed = p.Quantity > 0
	} else if contracts != nil {
		p.ContractSigned = ParseNumber(contracts[1])
		p.Quantity = math.Abs(p.ContractSigned)
		p.FillParsed = p.Quantity > 0
	}
	if price != nil {
		p.UnitPrice = ParseNumber(price[1])
	}
	if m := execRE.FindStringSubmatch(description); m != nil {
		p.ExecutedAt = ParseDate(m[1])
	}
	return p
}

var (
	rocRE      = regexp.MustCompile(`\breturn of capital\b|\broc\b`)
	tradeRE    = regexp.MustCompile(`\b(bought|sold|buy|sell)\b`)
	fxWordRE   = regexp.MustCompile(`\bfx\b`)
	convertRE  = regexp.MustCompile(`conversion|convert`)
	sellRE     = regexp.MustCompile(`\bsell|sold\b`)
	divRE      = regexp.MustCompile(`dividend|distribution`)
	transferRE = regexp.MustCompile(`transfer|trfout|trfin`)
	fxAnyRE    = regexp.MustCompile(`\bfx\b|conversion|convert`)
	feeRE      = regexp.MustCompile(`\bfee\b|commission|fchrg`)
)

func MapStatementType(code, description string) (string, string, string) {
	raw := py.Strip(code)
	c := compactRE.ReplaceAllString(strings.ToUpper(raw), "")
	blob := strings.ToLower(c + " " + description)
	or := func(v, def string) string {
		if v != "" {
			return v
		}
		return def
	}
	switch {
	case strings.Contains(c, "EXPIR") || c == "ASSIGN" || c == "ASSIGNMENT" || c == "EXERCISE":
		return raw, strings.ToUpper(raw), "option_event"
	case c == "LOAN" || c == "RECALL":
		return or(raw, c), c, "other"
	case c == "STKDIS" || c == "STKDIV" || c == "SPIN" || c == "SPINOFF":
		return or(raw, "STKDIS"), "STKDIS", "trade"
	case c == "ROC" || c == "RETURNOFCAPITAL":
		return or(raw, "ROC"), "ROC", "other"
	case c == "DIV" || c == "DIVIDEND" || strings.Contains(c, "DIVIDEND"):
		return "Dividend", or(raw, "DIV"), "dividend"
	case c == "CONT" || c == "CONTRIBUTION" || strings.Contains(c, "CONTRIB"):
		return "Deposit", or(raw, "CONT"), "deposit"
	case c == "WD" || c == "WITHDRAWAL" || strings.Contains(c, "WITHDRAW"):
		return "Withdrawal", or(raw, "WD"), "withdrawal"
	case c == "INTCHARGED" || c == "INTPAID" || c == "INTEREST" || strings.HasPrefix(c, "INT"):
		return "Interest", or(raw, "INTEREST"), "interest"
	case c == "TRFOUT" || c == "TRFIN" || c == "TRANSFER" || strings.HasPrefix(c, "TRF") || strings.Contains(c, "TRANSFER"):
		return "Transfer", or(raw, "TRANSFER"), "transfer"
	case c == "FXCONVERSION" || c == "FX" || c == "CONVERT" || strings.Contains(c, "FX") || strings.Contains(c, "CONVERT"):
		return "FxExchange", or(raw, "FX"), "fx"
	case c == "FEE" || c == "FCHRG" || c == "COMM" || strings.Contains(c, "FEE") || strings.Contains(c, "FCHRG"):
		return "Fee", or(raw, "FEE"), "fee"
	case c == "BUY" || c == "SELL":
		return "Trade", c, "trade"
	case strings.Contains(c, "SELL"):
		return or(raw, "Trade"), "SELL", "trade"
	case strings.Contains(c, "BUY"):
		return or(raw, "Trade"), "BUY", "trade"
	case rocRE.MatchString(blob):
		return or(raw, "ROC"), "ROC", "other"
	case tradeRE.MatchString(blob) && !fxWordRE.MatchString(blob) && !convertRE.MatchString(blob):
		if sellRE.MatchString(blob) {
			return "Trade", "SELL", "trade"
		}
		return "Trade", "BUY", "trade"
	case divRE.MatchString(blob):
		return "Dividend", or(raw, "DIV"), "dividend"
	case strings.Contains(blob, "interest"):
		return "Interest", or(raw, "INTEREST"), "interest"
	case transferRE.MatchString(blob):
		return "Transfer", or(raw, "TRANSFER"), "transfer"
	case fxAnyRE.MatchString(blob):
		return "FxExchange", or(raw, "FX"), "fx"
	case feeRE.MatchString(blob):
		return "Fee", or(raw, "FEE"), "fee"
	case strings.Contains(blob, "deposit"):
		return "Deposit", or(raw, "DEPOSIT"), "deposit"
	case strings.Contains(blob, "withdraw"):
		return "Withdrawal", or(raw, "WITHDRAWAL"), "withdrawal"
	}
	return or(raw, "Unknown"), strings.ToUpper(raw), Categorize(raw, description)
}

func BookIDFromFileName(name string) string {
	parts := strings.Split(name, "/")
	n := parts[len(parts)-1]
	if m := bookFileRE1.FindStringSubmatch(n); m != nil {
		return strings.ToUpper(m[1])
	}
	if m := bookFileRE2.FindStringSubmatch(n); m != nil {
		return strings.ToUpper(m[1])
	}
	return n
}

type row map[string]string

func (r row) pick(keys ...string) string {
	for _, k := range keys {
		if v, ok := r[k]; ok && py.Strip(v) != "" {
			return py.Strip(v)
		}
	}
	return ""
}

func MapStatement(r row, bookID string) (*store.Activity, string) {
	settlement := ParseDate(r.pick("date", "settlement_date", "transaction_date"))
	if settlement == "" {
		return nil, ""
	}
	code := r.pick("transaction", "activity_type", "type", "action")
	description := r.pick("description", "memo", "details")
	parsed := ParseStatementDescription(description)
	activityType, sub, category := MapStatementType(code, description)
	currency := strings.ToUpper(r.pick("currency", "ccy"))
	if currency == "" {
		currency = "CAD"
	}
	if m := ccyKeyRE.FindStringSubmatch(parsed.Symbol); m != nil && currency != "CAD" && currency != "USD" {
		currency = strings.ToUpper(m[1])
	}
	var balance *float64
	if balRaw := r.pick("balance"); balRaw != "" {
		balance = py.Ptr(ParseNumber(balRaw))
	}
	netCash := ParseNumber(r.pick("amount", "net_cash_amount", "net_amount"))
	compactCode := compactRE.ReplaceAllString(strings.ToUpper(code), "")
	stk := compactCode == "STKDIS" || compactCode == "STKDIV" || compactCode == "SPIN" || compactCode == "SPINOFF"
	if stk {
		activityType, category = "STKDIS", "trade"
		sub = "BUY"
		if parsed.SharesSigned < 0 {
			sub = "SELL"
		}
	}
	if parsed.FillParsed && category == "other" && compactCode != "LOAN" && compactCode != "RECALL" {
		sub = "SELL"
		if netCash < 0 {
			sub = "BUY"
		}
		if activityType == "" || activityType == "Unknown" {
			activityType = "Trade"
		}
		category = "trade"
	}
	quantity := parsed.Quantity
	unitPrice := parsed.UnitPrice
	if stk {
		unitPrice = 0
	}
	if parsed.FillParsed && unitPrice == 0 && parsed.Quantity > 0 && !stk {
		denom := parsed.Quantity * symbols.Multiplier(parsed.Symbol)
		if denom > 0 {
			unitPrice = math.Abs(netCash) / denom
		}
	}
	if category == "option_event" && parsed.FillParsed {
		if parsed.ContractSigned < 0 {
			sub = "BUY"
		} else if parsed.ContractSigned > 0 {
			sub = "SELL"
		}
	}
	if sub == "SELL" {
		quantity = -math.Abs(quantity)
	} else if sub == "BUY" {
		quantity = math.Abs(quantity)
	}
	issue := ""
	if category == "trade" && (sub == "BUY" || sub == "SELL") && (!parsed.FillParsed || parsed.Quantity == 0) {
		issue = "Could not parse quantity/price from description for " + sub
	}
	transactionDate := parsed.ExecutedAt
	if transactionDate == "" {
		transactionDate = settlement
	}
	return &store.Activity{
		ID: py.UUID4(), OccurredAt: transactionDate, TransactionDate: transactionDate, SettlementDate: settlement,
		BookID: py.Strip(bookID), ActivityType: activityType, ActivitySubType: sub, Description: description,
		Symbol: parsed.Symbol, Name: parsed.Name, Currency: currency, Quantity: quantity, UnitPrice: unitPrice,
		NetCashAmount: netCash, Category: category, Balance: balance, Source: "statement",
	}, issue
}

func MapCanonical(r row) *store.Activity {
	transactionDate := ParseDate(r.pick("transaction_date", "date", "trade_date", "activity_date"))
	if transactionDate == "" {
		return nil
	}
	activityType := r.pick("activity_type", "type")
	sub := r.pick("activity_sub_type", "activity_subtype", "sub_type", "subtype")
	settlement := ParseDate(r.pick("settlement_date", "settle_date"))
	if settlement == "" {
		settlement = transactionDate
	}
	at := activityType
	if at == "" {
		at = "Unknown"
	}
	ccy := strings.ToUpper(r.pick("currency", "ccy"))
	if ccy == "" {
		ccy = "CAD"
	}
	return &store.Activity{
		ID: py.UUID4(), OccurredAt: transactionDate, TransactionDate: transactionDate, SettlementDate: settlement,
		AccountID: r.pick("account_id", "account"), AccountType: r.pick("account_type"), ActivityType: at, ActivitySubType: sub,
		Description: r.pick("description", "memo", "details"), Direction: strings.ToUpper(r.pick("direction")),
		Symbol: r.pick("symbol", "ticker"), Name: r.pick("name", "security_name", "instrument"), Currency: ccy,
		Quantity: ParseNumber(r.pick("quantity", "qty")), UnitPrice: ParseNumber(r.pick("unit_price", "price", "fill_price")),
		Commission: math.Abs(ParseNumber(r.pick("commission", "fee", "fees"))), NetCashAmount: ParseNumber(r.pick("net_cash_amount", "amount", "net_amount", "net_cash")),
		Category: Categorize(activityType, sub), Source: "canonical",
	}
}

func MapLegacy(r row) *store.Activity {
	transactionDate := ParseDate(r.pick("date", "transaction_date"))
	if transactionDate == "" {
		return nil
	}
	action := strings.ToLower(r.pick("action", "type", "activity"))
	activityType, sub := "Other", ""
	switch {
	case action == "buy" || action == "sell":
		activityType, sub = "Trade", strings.ToUpper(action)
	case strings.Contains(action, "dividend"):
		activityType, sub = "Dividend", "DIVIDEND"
	case strings.Contains(action, "deposit"):
		activityType, sub = "Deposit", "DEPOSIT"
	case strings.Contains(action, "withdraw"):
		activityType, sub = "Withdrawal", "WITHDRAWAL"
	case strings.Contains(action, "interest"):
		activityType, sub = "Interest", "INTEREST"
	case strings.Contains(action, "fee"):
		activityType, sub = "Fee", "FEE"
	case strings.Contains(action, "fx"):
		activityType, sub = "FxExchange", strings.ToUpper(action)
	case action != "":
		activityType, sub = strings.ToUpper(action[:1])+action[1:], strings.ToUpper(action)
	}
	quantity := ParseNumber(r.pick("quantity", "qty"))
	if sub == "SELL" {
		quantity = -math.Abs(quantity)
	} else if sub == "BUY" {
		quantity = math.Abs(quantity)
	}
	accountID := r.pick("account_id", "account")
	if accountID == "" {
		accountID = "legacy"
	}
	ccy := strings.ToUpper(r.pick("currency", "ccy"))
	if ccy == "" {
		ccy = "CAD"
	}
	return &store.Activity{
		ID: py.UUID4(), OccurredAt: transactionDate, TransactionDate: transactionDate, SettlementDate: transactionDate,
		AccountID: accountID, AccountType: r.pick("account_type"), ActivityType: activityType, ActivitySubType: sub,
		Description: r.pick("description", "memo"), Symbol: r.pick("symbol", "ticker"), Name: r.pick("name", "security_name"), Currency: ccy,
		Quantity: quantity, UnitPrice: ParseNumber(r.pick("price", "unit_price")), Commission: math.Abs(ParseNumber(r.pick("commission", "fee", "fees"))),
		NetCashAmount: ParseNumber(r.pick("amount", "net_cash_amount", "net_amount")), Category: Categorize(activityType, sub), Source: "legacy",
	}
}

func readCSV(text string) [][]string {
	var table [][]string
	var rec []string
	var field strings.Builder
	inQuotes := false
	fieldStarted := false
	i := 0
	n := len(text)
	endField := func() {
		rec = append(rec, field.String())
		field.Reset()
		fieldStarted = false
	}
	endRecord := func() {
		table = append(table, rec)
		rec = nil
	}
	for i < n {
		c := text[i]
		if inQuotes {
			if c == '"' {
				if i+1 < n && text[i+1] == '"' {
					field.WriteByte('"')
					i += 2
					continue
				}
				inQuotes = false
				i++
				continue
			}
			field.WriteByte(c)
			i++
			continue
		}
		switch c {
		case '"':
			if !fieldStarted || field.Len() == 0 {
				inQuotes = true
				fieldStarted = true
			} else {
				field.WriteByte(c)
			}
			i++
		case ',':
			endField()
			i++
		case '\n':
			if fieldStarted || len(rec) > 0 {
				endField()
			}
			endRecord()
			i++
		default:
			field.WriteByte(c)
			fieldStarted = true
			i++
		}
	}
	if inQuotes || fieldStarted || len(rec) > 0 {
		endField()
		endRecord()
	}
	return table
}

type Skipped struct {
	Row     int    `json:"row"`
	Message string `json:"message"`
	Raw     string `json:"raw"`
}

type Report struct {
	Format         string           `json:"format"`
	Activities     []store.Activity `json:"activities"`
	Skipped        []Skipped        `json:"skipped"`
	FooterStripped bool             `json:"footerStripped"`
	CountsByType   map[string]int   `json:"countsByType"`
	RowCount       int              `json:"rowCount"`
}

func allBlank(cells []string) bool {
	for _, c := range cells {
		if py.Strip(c) != "" {
			return false
		}
	}
	return true
}

func rowJSON(r row, norms []string) string {
	m := map[string]string{}
	for _, k := range norms {
		m[k] = r[k]
	}
	b, _ := json.Marshal(m)
	return string(b)
}

func ParseCSV(text, name string) Report {
	text = strings.ReplaceAll(text, "\ufeff", "")
	footer := false
	var lines []string
	for _, line := range py.Lines(text) {
		if IsFooterLine(line) {
			footer = true
			continue
		}
		lines = append(lines, line)
	}
	table := readCSV(strings.Join(lines, "\n"))
	for len(table) > 0 && allBlank(table[len(table)-1]) {
		table = table[:len(table)-1]
	}
	rep := Report{Format: "unknown", Activities: []store.Activity{}, Skipped: []Skipped{}, FooterStripped: footer, CountsByType: map[string]int{}}
	if len(table) == 0 {
		rep.Skipped = []Skipped{{Row: 1, Message: "Empty file"}}
		return rep
	}
	headers := make([]string, len(table[0]))
	for i, h := range table[0] {
		headers[i] = py.Strip(strings.ReplaceAll(h, "\ufeff", ""))
	}
	fmtName := DetectFormat(headers)
	rep.Format = fmtName
	rep.RowCount = len(table) - 1
	norms := make([]string, len(headers))
	for i, h := range headers {
		norms[i] = NormalizeHeader(h)
	}
	if fmtName == "unknown" {
		rep.Skipped = append(rep.Skipped, Skipped{Row: 1, Message: "Unrecognized CSV format. Expected a Wealthsimple activities export, a statement export with date/transaction/description/amount columns, or a Date/Action/Symbol file.", Raw: strings.Join(headers, ",")})
		return rep
	}
	book := BookIDFromFileName(name)
	for i, cells := range table[1:] {
		rowNo := i + 2
		r := row{}
		for j, k := range norms {
			if j < len(cells) {
				r[k] = cells[j]
			} else {
				r[k] = ""
			}
		}
		vals := make([]string, 0, len(norms))
		for _, k := range norms {
			vals = append(vals, r[k])
		}
		if allBlank(vals) {
			continue
		}
		if IsFooterLine(strings.Join(vals, " ")) {
			footer = true
			rep.FooterStripped = true
			continue
		}
		var activity *store.Activity
		issue := ""
		switch fmtName {
		case "statement":
			activity, issue = MapStatement(r, book)
		case "legacy":
			activity = MapLegacy(r)
		default:
			activity = MapCanonical(r)
		}
		if issue != "" {
			rep.Skipped = append(rep.Skipped, Skipped{Row: rowNo, Message: issue, Raw: rowJSON(r, norms)})
		}
		if activity == nil {
			rep.Skipped = append(rep.Skipped, Skipped{Row: rowNo, Message: "Unparsed row (missing or invalid date)", Raw: rowJSON(r, norms)})
			continue
		}
		rep.Activities = append(rep.Activities, *activity)
		key := activity.ActivityType
		if key == "" {
			key = activity.Category
		}
		rep.CountsByType[key]++
	}
	return rep
}

type ImportResult struct {
	OK             bool           `json:"ok"`
	File           string         `json:"file"`
	Format         string         `json:"format"`
	Rows           int            `json:"rows"`
	Added          int            `json:"added"`
	Duplicates     int            `json:"duplicates"`
	Skipped        []Skipped      `json:"skipped"`
	SkippedCount   int            `json:"skippedCount"`
	FooterStripped bool           `json:"footerStripped"`
	CountsByType   map[string]int `json:"countsByType"`
}

func ImportText(st *store.Store, name, text string) ImportResult {
	rep := ParseCSV(text, name)
	added, dups := 0, 0
	if len(rep.Activities) > 0 {
		merged := st.MergeLocalRows(rep.Activities)
		added, dups = merged.Added, merged.Duplicates
	}
	parts := strings.Split(name, "/")
	skipped := rep.Skipped
	if len(skipped) > 20 {
		skipped = skipped[:20]
	}
	return ImportResult{OK: true, File: parts[len(parts)-1], Format: rep.Format, Rows: rep.RowCount, Added: added, Duplicates: dups, Skipped: skipped, SkippedCount: len(rep.Skipped), FooterStripped: rep.FooterStripped, CountsByType: rep.CountsByType}
}

func IsJunkName(name string) bool {
	return strings.HasPrefix(name, "._") || strings.Contains(strings.ToUpper(name), "__MACOSX")
}

func IsCSVName(name string) bool { return strings.HasSuffix(strings.ToLower(name), ".csv") }

type csvFile struct {
	Path  string
	Name  string
	Size  int64
	Mtime int64
}

func ListCSVFiles(folder string) []csvFile {
	out := []csvFile{}
	entries, err := os.ReadDir(folder)
	if err != nil {
		return out
	}
	sort.Slice(entries, func(i, j int) bool { return entries[i].Name() < entries[j].Name() })
	for _, e := range entries {
		n := e.Name()
		p := filepath.Join(folder, n)
		st, err := os.Stat(p)
		if err != nil || !st.Mode().IsRegular() || !IsCSVName(n) || IsJunkName(n) {
			continue
		}
		if st.Size() == 0 {
			continue
		}
		out = append(out, csvFile{Path: p, Name: n, Size: st.Size(), Mtime: st.ModTime().Unix()})
	}
	return out
}

func expandUser(p string) string {
	if strings.HasPrefix(p, "~") {
		if home, err := os.UserHomeDir(); err == nil {
			return home + p[1:]
		}
	}
	return p
}

func WatchFolder(st *store.Store) string { return st.GetMeta(WatchMeta) }

func SetWatchFolder(st *store.Store, path string) map[string]any {
	p := expandUser(py.Strip(path))
	if p == "" {
		return map[string]any{"ok": false, "error": "Folder path required"}
	}
	if info, err := os.Stat(p); err != nil || !info.IsDir() {
		return map[string]any{"ok": false, "error": "Not a folder: " + p}
	}
	st.SetMeta(WatchMeta, p)
	return map[string]any{"ok": true, "path": p}
}

func ClearWatchFolder(st *store.Store) {
	st.SetMeta(WatchMeta, "")
	st.SetMeta(WatchFilesMeta, "")
	st.SetMeta(WatchLastMeta, "")
}

type seenFile struct {
	Size       int64  `json:"size"`
	Mtime      int64  `json:"mtime"`
	Added      int    `json:"added"`
	Duplicates int    `json:"duplicates"`
	Format     string `json:"format"`
	ScannedAt  string `json:"scannedAt"`
}

func seenFiles(st *store.Store) map[string]seenFile {
	out := map[string]seenFile{}
	if raw := st.GetMeta(WatchFilesMeta); raw != "" {
		json.Unmarshal([]byte(raw), &out)
	}
	if out == nil {
		out = map[string]seenFile{}
	}
	return out
}

func ScanFolder(st *store.Store, folder string, force bool) map[string]any {
	if folder == "" {
		folder = WatchFolder(st)
	}
	path := expandUser(py.Strip(folder))
	if path == "" {
		return map[string]any{"ok": false, "error": "No folder is being watched"}
	}
	if info, err := os.Stat(path); err != nil || !info.IsDir() {
		return map[string]any{"ok": false, "error": "Folder not found: " + path, "path": path}
	}
	seen := seenFiles(st)
	files := []map[string]any{}
	added, dups := 0, 0
	for _, f := range ListCSVFiles(path) {
		prev, had := seen[f.Path]
		if !force && had && prev.Size == f.Size && prev.Mtime == f.Mtime {
			files = append(files, map[string]any{"file": f.Name, "unchanged": true, "added": prev.Added, "duplicates": prev.Duplicates, "format": prev.Format})
			continue
		}
		data, err := os.ReadFile(f.Path)
		if err != nil {
			files = append(files, map[string]any{"file": f.Name, "error": err.Error()})
			continue
		}
		text := strings.TrimPrefix(strings.ToValidUTF8(string(data), "�"), "\ufeff")
		rep := ImportText(st, f.Name, text)
		added += rep.Added
		dups += rep.Duplicates
		seen[f.Path] = seenFile{Size: f.Size, Mtime: f.Mtime, Added: rep.Added, Duplicates: rep.Duplicates, Format: rep.Format, ScannedAt: py.NowStamp()}
		files = append(files, map[string]any{"file": f.Name, "unchanged": false, "added": rep.Added, "duplicates": rep.Duplicates, "format": rep.Format, "rows": rep.Rows, "skippedCount": rep.SkippedCount})
	}
	now := py.NowStamp()
	kept := map[string]seenFile{}
	for k, v := range seen {
		if _, err := os.Stat(k); err == nil {
			kept[k] = v
		}
	}
	b, _ := json.Marshal(kept)
	st.SetMeta(WatchFilesMeta, string(b))
	st.SetMeta(WatchLastMeta, now)
	return map[string]any{"ok": true, "path": path, "added": added, "duplicates": dups, "files": files, "scannedAt": now}
}

func Status(st *store.Store) map[string]any {
	path := WatchFolder(st)
	seen := seenFiles(st)
	keys := make([]string, 0, len(seen))
	for k := range seen {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	files := []map[string]any{}
	for _, k := range keys {
		v := seen[k]
		files = append(files, map[string]any{"file": filepath.Base(k), "added": v.Added, "duplicates": v.Duplicates, "format": v.Format, "scannedAt": v.ScannedAt})
	}
	return map[string]any{"ok": true, "path": path, "watching": path != "", "lastScan": st.GetMeta(WatchLastMeta), "files": files}
}
