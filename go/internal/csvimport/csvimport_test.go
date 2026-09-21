package csvimport

import (
	"math"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/model"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

const canonicalCSV = `transaction_date,activity_type,activity_sub_type,symbol,quantity,unit_price,net_cash_amount,currency,account_id
2026-01-05,Trade,BUY,AAA,10,5.00,-50.00,CAD,acct-1
2026-02-05,Trade,SELL,AAA,-10,6.00,60.00,CAD,acct-1
2026-02-06,Dividend,DIVIDEND,AAA,,,1.50,CAD,acct-1
`

const statementCSV = `date,transaction,description,amount,balance,currency
2026-01-06,BUY,"AAA - Alpha Inc: Bought 10 shares (executed at 2026-01-05) at $5.00 per share",-50.00,950.00,CAD
2026-02-06,SELL,"AAA - Alpha Inc: Sold 10 shares (executed at 2026-02-05) at $6.00 per share",60.00,1010.00,CAD
2026-02-10,SELL,"LUNR 15JAN27 12.00 CALL: Sold 2 contracts (executed at 2026-02-10)",1200.00,2210.00,USD
2026-03-01,DIV,"AAA - Alpha Inc: Dividend",1.50,2211.50,CAD
As of 2026-03-02
`

const legacyCSV = `Date,Action,Symbol,Quantity,Price,Amount,Currency
2026-01-05,Buy,AAA,10,5.00,-50.00,CAD
2026-02-05,Sell,AAA,10,6.00,60.00,CAD
`

func tempStore(t *testing.T) *store.Store {
	t.Helper()
	st := store.MustOpen(t.TempDir())
	if err := st.Ensure(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	return st
}

func writeFile(t *testing.T, path, text string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(text), 0o644); err != nil {
		t.Fatal(err)
	}
}

func scanFiles(r map[string]any) []map[string]any {
	files, _ := r["files"].([]map[string]any)
	return files
}

func TestHelpers(t *testing.T) {
	numbers := []struct {
		raw  string
		want float64
	}{
		{"($1,234.50)", -1234.5},
		{"CAD 12", 12.0},
		{"n/a", 0.0},
	}
	for _, c := range numbers {
		if got := ParseNumber(c.raw); got != c.want {
			t.Errorf("ParseNumber(%q) = %v, want %v", c.raw, got, c.want)
		}
	}
	dates := []struct{ raw, want string }{
		{"2026-01-05T14:00:00Z", "2026-01-05"},
		{"05/01/2026", "2026-05-01"},
		{"25/01/2026", "2026-01-25"},
		{"5-Jan-2026", "2026-01-05"},
		{"Jan 5, 2026", "2026-01-05"},
		{"46027", "2026-01-05"},
	}
	for _, c := range dates {
		if got := ParseDate(c.raw); got != c.want {
			t.Errorf("ParseDate(%q) = %q, want %q", c.raw, got, c.want)
		}
	}
	formats := []struct {
		headers []string
		want    string
	}{
		{[]string{"transaction_date", "activity_type", "symbol"}, "canonical"},
		{[]string{"Date", "Transaction", "Description", "Amount"}, "statement"},
		{[]string{"Date", "Action", "Symbol", "Quantity", "Price", "Amount"}, "legacy"},
		{[]string{"foo", "bar"}, "unknown"},
	}
	for _, c := range formats {
		if got := DetectFormat(c.headers); got != c.want {
			t.Errorf("DetectFormat(%v) = %q, want %q", c.headers, got, c.want)
		}
	}
	if got := BookIDFromFileName("monthly-statement-ABC12345CAD-2026-01-31.csv"); got != "ABC12345CAD" {
		t.Errorf("BookIDFromFileName = %q, want %q", got, "ABC12345CAD")
	}
}

func TestCanonical(t *testing.T) {
	r := ParseCSV(canonicalCSV, "activities.csv")
	if r.Format != "canonical" {
		t.Fatalf("format = %q, want %q", r.Format, "canonical")
	}
	if len(r.Activities) != 3 {
		t.Fatalf("%d activities, want 3", len(r.Activities))
	}
	buy, sell, div := r.Activities[0], r.Activities[1], r.Activities[2]
	if buy.Category != "trade" || buy.ActivitySubType != "BUY" || buy.Quantity != 10.0 || buy.UnitPrice != 5.0 {
		t.Errorf("buy = (%q, %q, %v, %v), want (trade, BUY, 10, 5)", buy.Category, buy.ActivitySubType, buy.Quantity, buy.UnitPrice)
	}
	if sell.Category != "trade" || sell.Quantity != -10.0 || sell.NetCashAmount != 60.0 {
		t.Errorf("sell = (%q, %v, %v), want (trade, -10, 60)", sell.Category, sell.Quantity, sell.NetCashAmount)
	}
	if div.Category != "dividend" {
		t.Errorf("dividend category = %q, want %q", div.Category, "dividend")
	}
	if want := map[string]int{"Trade": 2, "Dividend": 1}; !reflect.DeepEqual(r.CountsByType, want) {
		t.Errorf("countsByType = %v, want %v", r.CountsByType, want)
	}
}

func TestStatementReadsFillsFromDescriptions(t *testing.T) {
	r := ParseCSV(statementCSV, "monthly-statement-ABC12345CAD-2026-03-31.csv")
	if r.Format != "statement" {
		t.Fatalf("format = %q, want %q", r.Format, "statement")
	}
	if !r.FooterStripped {
		t.Error("footer not stripped")
	}
	if len(r.Skipped) != 0 {
		t.Errorf("skipped = %v, want none", r.Skipped)
	}
	if len(r.Activities) != 4 {
		t.Fatalf("%d activities, want 4", len(r.Activities))
	}
	buy, sell, opt, div := r.Activities[0], r.Activities[1], r.Activities[2], r.Activities[3]
	if buy.Symbol != "AAA" || buy.Name != "Alpha Inc" || buy.Quantity != 10.0 || buy.UnitPrice != 5.0 || buy.TransactionDate != "2026-01-05" || buy.SettlementDate != "2026-01-06" {
		t.Errorf("buy = (%q, %q, %v, %v, %q, %q), want (AAA, Alpha Inc, 10, 5, 2026-01-05, 2026-01-06)", buy.Symbol, buy.Name, buy.Quantity, buy.UnitPrice, buy.TransactionDate, buy.SettlementDate)
	}
	if sell.ActivitySubType != "SELL" || sell.Quantity != -10.0 || sell.NetCashAmount != 60.0 {
		t.Errorf("sell = (%q, %v, %v), want (SELL, -10, 60)", sell.ActivitySubType, sell.Quantity, sell.NetCashAmount)
	}
	if opt.Symbol != "LUNR 15JAN27 12.00 CALL" || opt.Quantity != -2.0 || opt.Currency != "USD" {
		t.Errorf("option = (%q, %v, %q), want (LUNR 15JAN27 12.00 CALL, -2, USD)", opt.Symbol, opt.Quantity, opt.Currency)
	}
	if math.Abs(opt.UnitPrice-6.0) > 1e-7 {
		t.Errorf("option unit price = %v, want 6", opt.UnitPrice)
	}
	if div.Category != "dividend" {
		t.Errorf("dividend category = %q, want %q", div.Category, "dividend")
	}
	if buy.BookID != "ABC12345CAD" {
		t.Errorf("book id = %q, want %q", buy.BookID, "ABC12345CAD")
	}
}

func TestLegacyAndUnknown(t *testing.T) {
	r := ParseCSV(legacyCSV, "old.csv")
	if r.Format != "legacy" {
		t.Fatalf("format = %q, want %q", r.Format, "legacy")
	}
	subs := make([]string, 0, len(r.Activities))
	for _, a := range r.Activities {
		subs = append(subs, a.ActivitySubType)
	}
	if want := []string{"BUY", "SELL"}; !reflect.DeepEqual(subs, want) {
		t.Errorf("sub types = %v, want %v", subs, want)
	}
	if len(r.Activities) < 2 || r.Activities[1].Quantity != -10.0 {
		t.Errorf("sell quantity, want -10: %v", r.Activities)
	}
	r = ParseCSV("foo,bar\n1,2\n", "x.csv")
	if r.Format != "unknown" {
		t.Errorf("format = %q, want %q", r.Format, "unknown")
	}
	if len(r.Skipped) != 1 {
		t.Errorf("%d skipped, want 1", len(r.Skipped))
	}
}

func TestImportTextMergesAndDedups(t *testing.T) {
	st := tempStore(t)
	r := ImportText(st, "activities.csv", canonicalCSV)
	if r.Format != "canonical" || r.Added != 3 || r.Duplicates != 0 {
		t.Fatalf("(format, added, duplicates) = (%q, %d, %d), want (canonical, 3, 0)", r.Format, r.Added, r.Duplicates)
	}
	r = ImportText(st, "activities.csv", canonicalCSV)
	if r.Added != 0 || r.Duplicates != 3 {
		t.Fatalf("(added, duplicates) = (%d, %d), want (0, 3)", r.Added, r.Duplicates)
	}
	v := model.BuildView(model.New(st).Base(), nil)
	if v.KPI.Count != 1 {
		t.Errorf("kpi count = %d, want 1", v.KPI.Count)
	}
	if math.Abs(v.KPI.Realized-10) > 1e-7 {
		t.Errorf("kpi realized = %v, want 10", v.KPI.Realized)
	}
}

func TestFolderScanSkipsJunkAndUnchangedFiles(t *testing.T) {
	st := tempStore(t)
	folder := filepath.Join(t.TempDir(), "csv")
	if err := os.MkdirAll(filepath.Join(folder, "nested"), 0o755); err != nil {
		t.Fatal(err)
	}
	writeFile(t, filepath.Join(folder, "a.csv"), legacyCSV)
	writeFile(t, filepath.Join(folder, "._a.csv"), legacyCSV)
	writeFile(t, filepath.Join(folder, "notes.txt"), "hi")
	writeFile(t, filepath.Join(folder, "nested", "b.csv"), canonicalCSV)
	if SetWatchFolder(st, filepath.Join(folder, "missing"))["ok"] != false {
		t.Error("a missing folder is accepted")
	}
	if SetWatchFolder(st, folder)["ok"] != true {
		t.Fatal("the folder is refused")
	}
	r := ScanFolder(st, "", false)
	names := []string{}
	for _, f := range scanFiles(r) {
		names = append(names, f["file"].(string))
	}
	if want := []string{"a.csv"}; !reflect.DeepEqual(names, want) {
		t.Errorf("files = %v, want %v", names, want)
	}
	if r["added"] != 2 {
		t.Errorf("added = %v, want 2", r["added"])
	}
	r = ScanFolder(st, "", false)
	if files := scanFiles(r); len(files) == 0 || files[0]["unchanged"] != true {
		t.Errorf("a.csv is not unchanged: %v", files)
	}
	if r["added"] != 0 {
		t.Errorf("added = %v, want 0", r["added"])
	}
	writeFile(t, filepath.Join(folder, "c.csv"), canonicalCSV)
	r = ScanFolder(st, "", false)
	unchanged := map[string]any{}
	for _, f := range scanFiles(r) {
		unchanged[f["file"].(string)] = f["unchanged"]
	}
	if want := map[string]any{"a.csv": true, "c.csv": false}; !reflect.DeepEqual(unchanged, want) {
		t.Errorf("unchanged = %v, want %v", unchanged, want)
	}
	if r["added"] != 3 {
		t.Errorf("added = %v, want 3", r["added"])
	}
	r = ScanFolder(st, "", true)
	if r["added"] != 0 || r["duplicates"] != 5 {
		t.Errorf("(added, duplicates) = (%v, %v), want (0, 5)", r["added"], r["duplicates"])
	}
	status := Status(st)
	if status["watching"] != true {
		t.Error("not watching")
	}
	if files := scanFiles(status); len(files) != 2 {
		t.Errorf("%d files, want 2", len(files))
	}
	ClearWatchFolder(st)
	if Status(st)["watching"] != false {
		t.Error("still watching")
	}
}
