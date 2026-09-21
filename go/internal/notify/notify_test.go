package notify

import (
	"fmt"
	"reflect"
	"sort"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

var off = map[string]bool{"fills": false, "problems": false, "connection": false, "updates": false,
	"releasesHeld": false, "releasesWatched": false, "releasesAll": false,
	"disclosuresHeld": false, "disclosuresWatched": false, "disclosuresAll": false}

func setUp(t *testing.T) (*Notifier, *store.Store) {
	t.Helper()
	st := store.MustOpen(t.TempDir())
	if err := st.Ensure(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	t.Setenv(ModeEnv, "browser")
	return New(st), st
}

func with(base map[string]bool, key string, value bool) map[string]bool {
	out := map[string]bool{}
	for k, v := range base {
		out[k] = v
	}
	out[key] = value
	return out
}

func idsOf(rows []store.Notification) []int64 {
	out := []int64{}
	for _, r := range rows {
		out = append(out, r.ID)
	}
	return out
}

func keysOf(rows []store.Notification) []string {
	out := []string{}
	for _, r := range rows {
		out = append(out, r.Key)
	}
	return out
}

func TestEveryKindIsOffUntilTurnedOnAndTheSettingsRoundTrip(t *testing.T) {
	n, _ := setUp(t)
	if got := n.Settings(); !reflect.DeepEqual(got, off) {
		t.Fatalf("settings: %v", got)
	}
	out := n.SetSettings(map[string]any{"fills": true, "bogus": true, "updates": "yes"})
	if want := with(off, "fills", true); !reflect.DeepEqual(out, want) {
		t.Fatalf("unknown keys and non-booleans are ignored: %v", out)
	}
	if got := n.Settings(); !reflect.DeepEqual(got, out) {
		t.Fatalf("settings: %v", got)
	}
	want := map[string]any{}
	for k, v := range out {
		want[k] = v
	}
	want["native"] = ""
	want["unread"] = 0
	if got := n.Status(); !reflect.DeepEqual(got, want) {
		t.Fatalf("the status payload carries the kinds, the channel and the unread count; told to stand aside, the page is the channel: %v", got)
	}
	fakeTools(t)
	t.Setenv(ModeEnv, "")
	t.Setenv("DISPLAY", ":0")
	if got := n.NativeChannel(); got != desktop() {
		t.Fatalf("native channel: %q", got)
	}
	t.Setenv("PATH", t.TempDir())
	t.Setenv("DISPLAY", "")
	t.Setenv("WAYLAND_DISPLAY", "")
	if got := n.NativeChannel(); got != "" {
		t.Fatalf("no desktop: the page is the channel: %q", got)
	}
}

func TestAKindThatIsOffIsNotToldAndAKeyIsToldOnce(t *testing.T) {
	n, st := setUp(t)
	if got := n.Emit("fills", "order:1:filled", "Order filled · QNC", "Bought 5 at 1.75", nil); got != nil {
		t.Fatalf("off: %+v", got)
	}
	n.SetSettings(map[string]any{"fills": true})
	row := n.Emit("fills", "order:1:filled", "Order filled · QNC", "Bought 5 at 1.75", nil)
	if row == nil {
		t.Fatal("on: nothing told")
	}
	got := [5]string{row.Kind, row.Title, row.Body, row.SeenAt, row.ReadAt}
	if want := [5]string{"fills", "Order filled · QNC", "Bought 5 at 1.75", "", ""}; got != want {
		t.Fatalf("row: %q", got)
	}
	if again := n.Emit("fills", "order:1:filled", "Order filled · QNC", "again", nil); again != nil {
		t.Fatalf("the same event is never told twice: %+v", again)
	}
	if bogus := n.Emit("bogus", "x", "t", "b", nil); bogus != nil {
		t.Fatalf("an unknown kind is nothing: %+v", bogus)
	}
	if d := n.Emit("disclosures", "f1", "t", "b", nil); d != nil {
		t.Fatalf("no set of tickers chosen: disclosures are not told: %+v", d)
	}
	n.SetSettings(map[string]any{"disclosuresWatched": true})
	if got := n.DisclosureScopes(); !reflect.DeepEqual(got, map[string]bool{"watched": true}) {
		t.Fatalf("scopes: %v", got)
	}
	if d := n.Emit("disclosures", "f1", "t", "b", nil); d == nil {
		t.Fatal("any set on: the kind is told")
	}
	if test := n.TestNotification(); test == nil || test.ID <= row.ID {
		t.Fatalf("the test goes out whatever the kinds say: %+v", test)
	}
	if rows := st.ListNotifications(0, "", false, 0, false); len(rows) != 3 {
		t.Fatalf("history: %d", len(rows))
	}
}

func TestSeenRowsAreNotListedAgainAndTheOldestArePruned(t *testing.T) {
	n, st := setUp(t)
	n.SetSettings(map[string]any{"fills": true})
	ids := []int64{}
	for i := 0; i < 3; i++ {
		ids = append(ids, n.Emit("fills", fmt.Sprintf("k%d", i), "t", "b", nil).ID)
	}
	if got := st.MarkNotificationsSeen([]int64{ids[0], 0, -1}); got != 1 {
		t.Fatalf("seen: %d", got)
	}
	if got := idsOf(st.ListNotifications(0, "", true, 0, false)); !reflect.DeepEqual(got, ids[1:]) {
		t.Fatalf("unseen: %v", got)
	}
	if got := idsOf(st.ListNotifications(ids[1], "", false, 0, false)); !reflect.DeepEqual(got, ids[2:]) {
		t.Fatalf("after: %v", got)
	}
	kept := store.NotificationsKept
	for i := 3; i < kept; i++ {
		n.Emit("fills", fmt.Sprintf("f%d", i), "t", "b", nil)
	}
	n.Emit("fills", "k9", "t", "b", nil)
	if rows := st.ListNotifications(0, "", false, kept+1, false); len(rows) != kept {
		t.Fatalf("the newest are kept: %d", len(rows))
	}
	want := []string{"k9"}
	for i := kept - 1; i >= 3; i-- {
		want = append(want, fmt.Sprintf("f%d", i))
	}
	want = append(want, "k2", "k1")
	if got := keysOf(st.ListNotifications(0, "", false, kept+1, true)); !reflect.DeepEqual(got, want) {
		t.Fatalf("the history reads newest first: %v", got)
	}
	if got := st.UnreadNotifications(); got != kept {
		t.Fatalf("unread: %d", got)
	}
	if got := st.MarkNotificationsRead([]int64{ids[2], 0}, false); got != 1 {
		t.Fatalf("read: %d", got)
	}
	if got := st.UnreadNotifications(); got != kept-1 {
		t.Fatalf("unread: %d", got)
	}
	if got := st.MarkNotificationsRead(nil, true); got != int64(kept-1) {
		t.Fatalf("no ids: every unread one: %d", got)
	}
	if unread, read := st.UnreadNotifications(), st.MarkNotificationsRead(nil, true); unread != 0 || read != 0 {
		t.Fatalf("unread %d read %d", unread, read)
	}
	for _, r := range st.ListNotifications(0, "", false, kept+1, false) {
		if r.ReadAt == "" {
			t.Fatalf("unread row: %+v", r)
		}
	}
	if got := st.ClearNotifications(); got != int64(kept) {
		t.Fatalf("cleared: %d", got)
	}
	if rows, latest := st.ListNotifications(0, "", false, 0, false), st.LatestNotificationID(); len(rows) != 0 || latest != 0 {
		t.Fatalf("after clear: %v %d", rows, latest)
	}
}

type dated struct {
	id string
	at string
}

func TestAStreamMetForTheFirstTimeShowsNothingAndNeverShowsItsPast(t *testing.T) {
	_, st := setUp(t)
	at := func(i dated) string { return i.at }
	ident := func(i dated) string { return i.id }
	fresh := func(stream string, items []dated) []string {
		out := []string{}
		for _, i := range FreshSince(st, stream, items, at, ident, nil) {
			out = append(out, i.id)
		}
		return out
	}
	held := []dated{{"a", "2026-05-01"}, {"b", "2026-06-01"}}
	if got := fresh("s1", held); len(got) != 0 {
		t.Fatalf("met for the first time: nothing, whatever it holds: %v", got)
	}
	if got := st.GetMeta("notify_seen:s1"); got != "2026-06-01|b" {
		t.Fatalf("and the mark is set from it, with what stood at that moment: %q", got)
	}
	if got := fresh("s1", held); len(got) != 0 {
		t.Fatalf("the same again: still nothing: %v", got)
	}
	later := append(append([]dated{}, held...), dated{"c", "2026-07-01"})
	if got := fresh("s1", later); !reflect.DeepEqual(got, []string{"c"}) {
		t.Fatalf("what comes after the mark: %v", got)
	}
	if got := fresh("s1", later); len(got) != 0 {
		t.Fatalf("and never again, without the caller having to remember: %v", got)
	}
	both := append(append([]dated{}, later...), dated{"d", "2026-07-01"}, dated{"old", "2026-02-01"})
	if got := fresh("s1", both); !reflect.DeepEqual(got, []string{"d"}) {
		t.Fatalf("a sibling at the newest moment: %v", got)
	}
	if got := fresh("s1", both); len(got) != 0 {
		t.Fatalf("again: %v", got)
	}
	if got := fresh("s2", held); len(got) != 0 {
		t.Fatalf("s2: %v", got)
	}
	marked := []string{}
	for _, k := range []string{"notify_seen:s1", "notify_seen:s2"} {
		if st.GetMeta(k) != "" {
			marked = append(marked, k)
		}
	}
	sort.Strings(marked)
	if !reflect.DeepEqual(marked, []string{"notify_seen:s1", "notify_seen:s2"}) {
		t.Fatalf("marks: %v", marked)
	}
}
