package store

import (
	"database/sql"
	"fmt"
	"testing"
)

func TestTheCountsMatchTheSnapshotWithoutReadingTheRows(t *testing.T) {
	s := temp(t)
	for i := 0; i < 4; i++ {
		if _, err := s.InsertLocal(Activity{ID: fmt.Sprintf("m%d", i), TransactionDate: fmt.Sprintf("2026-01-0%d", i+1), Symbol: "AAA", Category: "trade", ActivitySubType: "BUY", Quantity: 1, UnitPrice: 2.0, NetCashAmount: -2.0, Currency: "CAD"}); err != nil {
			t.Fatal(err)
		}
	}
	s.ReplaceAccounts([]Account{{ID: "a1", Nickname: "One"}, {ID: "a2", Nickname: "Two"}})
	s.SetMeta("synced_at", "2026-09-12T10:00:00Z")
	snap := s.Snapshot(true)
	counts := s.StatusCounts()
	if counts.ActivityCount != len(snap.Activities) {
		t.Errorf("activities %d vs %d", counts.ActivityCount, len(snap.Activities))
	}
	if counts.AccountCount != len(snap.Accounts) {
		t.Errorf("accounts %d vs %d", counts.AccountCount, len(snap.Accounts))
	}
	if counts.SyncedAt != snap.SyncedAt {
		t.Errorf("syncedAt %q vs %q", counts.SyncedAt, snap.SyncedAt)
	}
}

func TestTheOptionRelabelRunsOnceUntilTheRowsChange(t *testing.T) {
	s := temp(t)
	s.DeleteMeta(OptionRelabelMeta)
	relabel := func() bool {
		var ran bool
		err := s.tx(func(tx *sql.Tx) error {
			var err error
			ran, err = relabelWhenRowsChanged(tx)
			return err
		})
		if err != nil {
			t.Fatal(err)
		}
		return ran
	}
	runs := 0
	for i := 0; i < 3; i++ {
		if relabel() {
			runs++
		}
	}
	if runs != 1 {
		t.Errorf("an unchanged table is relabelled once: %d", runs)
	}
	if _, err := s.InsertLocal(Activity{ID: "o1", TransactionDate: "2026-02-02", Symbol: "AAA", Category: "trade", ActivitySubType: "BUY", Quantity: 1, UnitPrice: 2.0, NetCashAmount: -2.0, Currency: "CAD"}); err != nil {
		t.Fatal(err)
	}
	if relabel() {
		runs++
	}
	if runs != 2 {
		t.Errorf("a new row is relabelled: %d", runs)
	}
}
