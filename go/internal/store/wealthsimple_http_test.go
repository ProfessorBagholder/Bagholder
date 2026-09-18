package store

import (
	"strconv"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

func navDates(points []NavPoint) []string {
	out := make([]string, 0, len(points))
	for _, p := range points {
		out = append(out, p.Date)
	}
	return out
}

func sameStrings(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

func TestNavHistoryMigratesDatePkToAccountDate(t *testing.T) {
	s := temp(t)
	db := s.DB()
	for _, q := range []string{
		"DROP TABLE nav_history",
		"CREATE TABLE nav_history (date TEXT PRIMARY KEY, equity REAL, currency TEXT, net_deposits REAL)",
	} {
		if _, err := db.Exec(q); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := db.Exec("INSERT INTO nav_history (date, equity, currency, net_deposits) VALUES (?, ?, ?, ?)", "2024-01-02", 1000.0, "CAD", 100.0); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec("INSERT OR REPLACE INTO meta(key, value) VALUES (?, ?)", "schema_version", "1"); err != nil {
		t.Fatal(err)
	}
	if err := s.Ensure(); err != nil {
		t.Fatal(err)
	}
	snap := s.Snapshot(true)
	if got := s.GetMeta("schema_version"); got != strconv.Itoa(SchemaVersion) {
		t.Errorf("schema version %q", got)
	}
	if len(snap.NavHistory) != 1 {
		t.Fatalf("%+v", snap.NavHistory)
	}
	p := snap.NavHistory[0]
	if p.Date != "2024-01-02" || p.Equity != 1000.0 || p.NetDeposits == nil || *p.NetDeposits != 100.0 {
		t.Errorf("%+v", p)
	}
	if p.AccountID != "" {
		t.Errorf("accountId on the identity row: %q", p.AccountID)
	}
	if len(snap.NavByAccount) != 0 {
		t.Errorf("%+v", snap.NavByAccount)
	}
	rows, err := db.Query("PRAGMA table_info(nav_history)")
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	pk := map[string]bool{}
	for rows.Next() {
		var cid, notnull, pkv int
		var name, typ string
		var dflt any
		if err := rows.Scan(&cid, &name, &typ, &notnull, &dflt, &pkv); err != nil {
			t.Fatal(err)
		}
		if pkv != 0 {
			pk[name] = true
		}
	}
	if len(pk) != 2 || !pk["account_id"] || !pk["date"] {
		t.Errorf("primary key %v", pk)
	}
}

func TestReplaceNavByAccountAndSnapshot(t *testing.T) {
	s := temp(t)
	s.ReplaceNav([]NavPoint{
		{Date: "2024-01-01", Equity: 10, Currency: "CAD", NetDeposits: py.Ptr(1), AccountID: ""},
		{Date: "2024-01-02", Equity: 11, NetDeposits: py.Ptr(2)},
		{Date: "2024-01-01", Equity: 5, Currency: "CAD", AccountID: "TFSA"},
		{Date: "2024-01-02", Equity: 6, AccountID: "TFSA", NetDeposits: py.Ptr(3)},
		{Date: "2024-01-01", Equity: 7, AccountID: "RRSP"},
	})
	snap := s.Snapshot(false)
	if !sameStrings(navDates(snap.NavHistory), []string{"2024-01-01", "2024-01-02"}) {
		t.Errorf("%+v", snap.NavHistory)
	}
	if snap.NavHistory[0].Equity != 10 {
		t.Errorf("%+v", snap.NavHistory[0])
	}
	if len(snap.NavByAccount) != 2 || snap.NavByAccount["TFSA"] == nil || snap.NavByAccount["RRSP"] == nil {
		t.Fatalf("%+v", snap.NavByAccount)
	}
	tfsa := snap.NavByAccount["TFSA"]
	if tfsa[0].Equity != 5 || tfsa[1].NetDeposits == nil || *tfsa[1].NetDeposits != 3 {
		t.Errorf("%+v", tfsa)
	}
	if tfsa[0].AccountID != "" {
		t.Errorf("accountId inside the account's own run: %q", tfsa[0].AccountID)
	}
	s.ReplaceNav([]NavPoint{
		{Date: "2024-06-01", Equity: 20, AccountID: ""},
		{Date: "2024-06-01", Equity: 8, AccountID: "TFSA"},
	})
	snap = s.Snapshot(false)
	if !sameStrings(navDates(snap.NavHistory), []string{"2024-06-01"}) {
		t.Errorf("%+v", snap.NavHistory)
	}
	if len(snap.NavByAccount) != 1 || snap.NavByAccount["TFSA"] == nil {
		t.Errorf("%+v", snap.NavByAccount)
	}
	if _, ok := snap.NavByAccount["RRSP"]; ok {
		t.Error("RRSP survived the replace")
	}
}

func TestUpsertNavKeepsExistingDays(t *testing.T) {
	s := temp(t)
	s.ReplaceNav([]NavPoint{
		{Date: "2024-01-01", Equity: 10, AccountID: ""},
		{Date: "2024-01-01", Equity: 5, AccountID: "TFSA"},
	})
	s.UpsertNav([]NavPoint{
		{Date: "2024-01-02", Equity: 11, AccountID: ""},
		{Date: "2024-01-01", Equity: 6, AccountID: "TFSA"},
	})
	snap := s.Snapshot(false)
	if !sameStrings(navDates(snap.NavHistory), []string{"2024-01-01", "2024-01-02"}) {
		t.Errorf("%+v", snap.NavHistory)
	}
	if snap.NavHistory[1].Equity != 11 {
		t.Errorf("%+v", snap.NavHistory[1])
	}
	if snap.NavByAccount["TFSA"][0].Equity != 6 {
		t.Errorf("%+v", snap.NavByAccount["TFSA"])
	}
	last := s.NavLastDates()
	if len(last) != 2 || last[""] != "2024-01-02" || last["TFSA"] != "2024-01-01" {
		t.Errorf("%v", last)
	}
}
