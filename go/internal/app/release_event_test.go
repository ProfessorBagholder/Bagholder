package app

import (
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

// The bell rang for a release from weeks earlier because events were counted by the id a
// source gave them: TMX, Yahoo, Seeking Alpha and Google carry one release under four ids,
// a week apart in their own timestamps, and it drops in and out of a search between passes.
func releaseApp(t *testing.T) *App {
	t.Helper()
	a := newTestApp(t)
	a.notify.SetSettings(map[string]any{"releasesAll": true})
	return a
}

func wire(id, headline, at string) store.WireItem {
	return store.WireItem{ID: id, Headline: headline, Source: "GlobeNewswire", PublishedAt: at, Kind: "release"}
}

func notesOf(a *App) []store.Notification { return a.st.ListNotifications(0, "", false, 100, true) }

// the first read of a source carries a back catalogue; it is absorbed, never rung
func settle(a *App, rows ...store.WireItem) {
	a.noteWireReleases("QNC", "TSX-V", rows, map[string]bool{})
}

func TestABackCatalogueAbsorbedOnAFirstReadNeverRings(t *testing.T) {
	a := releaseApp(t)
	history := []store.WireItem{wire("tmx:1", "Older release", "2026-07-01T12:00:00Z"), wire("tmx:2", "Second older release", "2026-07-02T12:00:00Z")}
	settle(a, history...)
	if got := len(notesOf(a)); got != 0 {
		t.Fatalf("history absorbed on a first read rang %d times", got)
	}
	ids := map[string]bool{"yahoo:8": true, "yahoo:9": true}
	returned := []store.WireItem{wire("yahoo:8", "Older release", "2026-07-08T12:00:00Z"), wire("yahoo:9", "Second older release", "2026-07-09T12:00:00Z")}
	a.noteWireReleases("QNC", "TSX-V", returned, ids)
	if got := len(notesOf(a)); got != 0 {
		t.Errorf("the same history returning under fresh ids and later dates rang %d times", got)
	}
}

func TestAReleaseThatJustAppearedIsToldOnceWhateverIdItReturnsUnder(t *testing.T) {
	a := releaseApp(t)
	settle(a, wire("tmx:1", "Older release", "2026-07-01T12:00:00Z"))

	fresh := []store.WireItem{wire("tmx:1", "Older release", "2026-07-01T12:00:00Z"), wire("tmx:7", "Aegis Announces August 2026 Distributions", "2026-09-18T12:00:00Z")}
	a.noteWireReleases("QNC", "TSX-V", fresh, map[string]bool{"tmx:7": true})
	notes := notesOf(a)
	if len(notes) != 1 {
		t.Fatalf("a release that just appeared told %d times, want 1", len(notes))
	}

	again := []store.WireItem{wire("yahoo:42", "Aegis Announces August 2026 Distributions", "2026-09-25T08:00:00Z")}
	a.noteWireReleases("QNC", "TSX-V", again, map[string]bool{"yahoo:42": true})
	if got := len(notesOf(a)); got != 1 {
		t.Errorf("the same release under another id and a later date told again: %d notifications", got)
	}
}

func TestOneReleaseIsOneKeyWhateverIdsCarryIt(t *testing.T) {
	head := "Aegis Announces August 2026 Distributions"
	byTMX := releaseKeyWire("QNC", []store.WireItem{wire("tmx:7", head, "2026-09-18T12:00:00Z")})
	byYahoo := releaseKeyWire("QNC", []store.WireItem{wire("yahoo:42", head, "2026-09-25T08:00:00Z")})
	if byTMX != byYahoo {
		t.Errorf("one release keyed two ways: %q and %q", byTMX, byYahoo)
	}
}
