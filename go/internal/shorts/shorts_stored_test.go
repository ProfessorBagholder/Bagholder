package shorts

import (
	"reflect"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

const appLayer = "bagholder.%s lives in internal/app as an unexported method on *App; not reachable from internal/shorts"

func TestAMarketNoOneReportsAnswersThatItIsNotCovered(t *testing.T) {
	t.Skipf(appLayer, "shorts_payload")
}

func TestAListingWithFiguresHandsThemOver(t *testing.T) {
	t.Skipf(appLayer, "shorts_payload")
}

func TestNoSymbolIsAnErrorRatherThanAnEmptyCard(t *testing.T) {
	t.Skipf(appLayer, "shorts_payload")
}

func stored(over func(*store.Short)) store.Short {
	rec := store.Short{Market: "ca", AsOf: "2026-08-31", Shares: p(2667164.0), Previous: p(2603087.0), PreviousOf: "2026-08-15",
		Change: p(64077.0), Float: p(212448707.0), OfFloat: p(1.2554), AverageVolume: p(510698.0), DaysToCover: p(5.2),
		VolumeOf: "2026-08-16/2026-08-31", VolumeSpan: "period", ShortVolume: p(1197633.0),
		TotalVolume: p(5617679.0), VolumePct: p(21.319), Series: []store.ShortPoint{{Date: "2026-08-15", Shares: p(2603087.0)}}}
	if over != nil {
		over(&rec)
	}
	return rec
}

func save(st *store.Store, symbol, exchange string, rec store.Short) {
	st.SaveShorts(symbol, exchange, rec, "", 0, rec.Series != nil)
}

func TestAReadingComesBackAsItWentIn(t *testing.T) {
	c, _ := newClient(t)
	save(c.Store, "QNC", "TSX-V", stored(nil))
	held := c.Store.ShortsFor("QNC", "TSX-V")
	if held == nil {
		t.Fatal("nothing stored")
	}
	if !is(held.Shares, 2667164.0) {
		t.Errorf("shares = %v", show(held.Shares))
	}
	if !is(held.OfFloat, 1.2554) {
		t.Errorf("ofFloat = %v", show(held.OfFloat))
	}
	if held.VolumeSpan != "period" {
		t.Errorf("volumeSpan = %q", held.VolumeSpan)
	}
	if !reflect.DeepEqual(held.Series, []store.ShortPoint{{Date: "2026-08-15", Shares: p(2603087.0)}}) {
		t.Errorf("series = %v", held.Series)
	}
	if held.FetchedAt == "" {
		t.Error("fetchedAt is empty")
	}
}

func TestALaterReadingWithoutARunOfReportsKeepsTheOneStored(t *testing.T) {
	c, _ := newClient(t)
	save(c.Store, "QNC", "TSX-V", stored(nil))
	save(c.Store, "QNC", "TSX-V", stored(func(r *store.Short) { r.Series = nil; r.Shares = p(99.0) }))
	held := c.Store.ShortsFor("QNC", "TSX-V")
	if held == nil {
		t.Fatal("nothing stored")
	}
	if !is(held.Shares, 99.0) {
		t.Errorf("shares = %v", show(held.Shares))
	}
	if len(held.Series) != 1 {
		t.Errorf("series = %v", held.Series)
	}
}

func TestTheTwoListingsOfOneCompanyAreKeptApart(t *testing.T) {
	c, _ := newClient(t)
	save(c.Store, "QNC", "TSX-V", stored(func(r *store.Short) { r.Shares = p(2667164.0) }))
	save(c.Store, "QNC", "NYSE", stored(func(r *store.Short) { r.Market = "us"; r.Shares = p(7058199.0) }))
	if held := c.Store.ShortsFor("QNC", "TSX-V"); held == nil || !is(held.Shares, 2667164.0) {
		t.Errorf("TSX-V = %+v", held)
	}
	if held := c.Store.ShortsFor("QNC", "NYSE"); held == nil || !is(held.Shares, 7058199.0) {
		t.Errorf("NYSE = %+v", held)
	}
	if n := len(c.Store.AllShorts()); n != 2 {
		t.Errorf("stored %d", n)
	}
}

func TestAListingNeverReadIsNotInTheStore(t *testing.T) {
	c, _ := newClient(t)
	if held := c.Store.ShortsFor("NOSUCH", "TSX"); held != nil {
		t.Errorf("got %+v", held)
	}
}

func TestWhatIsStoredIsAnsweredWithoutReadingAgain(t *testing.T) {
	t.Skipf(appLayer, "shorts_payload")
}

func TestAListingNotStoredYetIsReadAndKept(t *testing.T) {
	t.Skipf(appLayer, "shorts_payload")
}

func TestAMarketNoOneReportsIsNeverReadOrKept(t *testing.T) {
	t.Skipf(appLayer, "shorts_payload")
}

func TestAStaleReadingIsStillAnsweredAtOnce(t *testing.T) {
	t.Skipf(appLayer, "shorts_payload and kick")
}

func TestOnlyListingsTheBookHoldsOrWatchesAreListed(t *testing.T) {
	t.Skipf(appLayer, "shorts_feed")
}

func TestEachRowSaysWhetherItIsHeldOrWatched(t *testing.T) {
	t.Skipf(appLayer, "shorts_feed")
}

func TestARowCarriesItsNameAndTheHoldingItOpens(t *testing.T) {
	t.Skipf(appLayer, "shorts_feed")
}

func TestTheListIsToldWhileASweepStillHasListingsToRead(t *testing.T) {
	t.Skipf(appLayer, "shorts_feed and sweep_shorts")
}

func TestAListingReadButCarryingNoPositionIsLeftOut(t *testing.T) {
	t.Skipf(appLayer, "shorts_feed")
}

func TestARowFromOlderLogicIsReadAgainHoweverFreshItIs(t *testing.T) {
	t.Skipf(appLayer, "_shorts_stale")
}

func TestARowAtTheCurrentVersionStandsUntilItsHoursAreUp(t *testing.T) {
	t.Skipf(appLayer, "_shorts_stale")
}

func TestTheSweepReadsARowFromOlderLogic(t *testing.T) {
	t.Skipf(appLayer, "sweep_shorts")
}

func TestTheListShowsTheVenueTheWayTheBookDoes(t *testing.T) {
	t.Skipf(appLayer, "shorts_feed")
}
