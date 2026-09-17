package store

import (
	"database/sql"
	"path/filepath"
	"testing"
)

func TestATableFromTheVersionBeforeGainsTheColumnAndKeepsItsRows(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, "bagholder.db")
	old, err := sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	for _, q := range []string{
		"CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT)",
		"INSERT INTO meta VALUES ('schema_version', '12')",
		"CREATE TABLE shorts (symbol TEXT NOT NULL, exchange TEXT NOT NULL DEFAULT '', market TEXT, as_of TEXT, shares REAL, previous REAL, previous_of TEXT, change REAL, float_shares REAL, of_float REAL, average_volume REAL, days_to_cover REAL, volume_of TEXT, volume_span TEXT, short_volume REAL, total_volume REAL, volume_pct REAL, series TEXT, fetched_at TEXT, PRIMARY KEY (symbol, exchange))",
		"INSERT INTO shorts (symbol, exchange, shares, fetched_at) VALUES ('QNC','TSX-V',2667164,'2026-09-15T20:00:00Z')",
	} {
		if _, err := old.Exec(q); err != nil {
			t.Fatal(err)
		}
	}
	if err := old.Close(); err != nil {
		t.Fatal(err)
	}
	s, err := Open(home)
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	row := s.ShortsFor("QNC", "TSX-V")
	if row == nil || row.Shares == nil || *row.Shares != 2667164.0 {
		t.Fatalf("%+v", row)
	}
	if row.ReadVersion != 0 {
		t.Errorf("unmarked, so it is read again once: %d", row.ReadVersion)
	}
}
