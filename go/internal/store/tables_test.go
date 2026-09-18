package store

import (
	"database/sql"
	"encoding/json"
	"reflect"
	"testing"
)

func tablesStore(t *testing.T) *Store {
	t.Helper()
	s := MustOpen(t.TempDir())
	if err := s.Ensure(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { s.Close() })
	return s
}

func TestLegacySpyMetaMigratesIntoTable(t *testing.T) {
	s := tablesStore(t)
	raw, err := json.Marshal(map[string]any{"2020-01-02": 3200.5, "junk": "x"})
	if err != nil {
		t.Fatal(err)
	}
	s.SetMeta("spy_by_date", string(raw))
	err = s.tx(func(tx *sql.Tx) error {
		if _, err := tx.Exec("DELETE FROM benchmark_prices"); err != nil {
			return err
		}
		return migrateSpyMeta(tx)
	})
	if err != nil {
		t.Fatal(err)
	}
	if got, want := s.BenchmarkPrices(BenchmarkSymbol), (map[string]float64{"2020-01-02": 3200.5}); !reflect.DeepEqual(got, want) {
		t.Errorf("benchmark prices = %v, want %v", got, want)
	}
}
