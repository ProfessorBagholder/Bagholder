package app

import (
	"errors"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/exposure"
	"github.com/ProfessorBagholder/Bagholder/internal/instruments"
	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/model"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
	"github.com/ProfessorBagholder/Bagholder/internal/ws"
)

func notAuthorized(err error) bool { return errors.Is(err, ws.ErrNotAuthorized) }

func publicSyncError(err error) string { return ws.PublicSyncError(errText(err)) }

func (a *App) fetchNicknameNavHistory(sess ws.Session, accounts []ws.Item) ([]store.NavPoint, []string) {
	points := []store.NavPoint{}
	errs := []string{}
	lastBy := a.st.NavLastDates()
	groups, _ := ws.NavAccountGroups(accounts)
	nicks := make([]string, 0, len(groups))
	for nick := range groups {
		nicks = append(nicks, nick)
	}
	sort.Strings(nicks)
	for _, nick := range nicks {
		a.setSyncStep("Fetching equity history for " + nick + "…")
		var series [][]store.NavPoint
		var failed error
		for _, aid := range groups[nick] {
			pts, err := a.ws.FetchAccountNavHistory(sess, aid, lastBy[nick])
			if err != nil {
				failed = err
				break
			}
			series = append(series, pts)
		}
		if failed != nil {
			public := publicSyncError(failed)
			errs = append(errs, nick+": "+public)
			a.logf("NAV history failed for %s: %s\n", nick, public)
			continue
		}
		for _, rec := range ws.MergeNavPoints(series) {
			rec.AccountID = nick
			points = append(points, rec)
		}
	}
	return points, errs
}

func (a *App) refreshNavOnly(allowRefresh bool) map[string]any {
	_ = a.st.Ensure()
	sess := a.loadSession()
	if sess == nil || sess.Str("access_token") == "" {
		return map[string]any{"ok": false, "error": "not connected"}
	}
	identity := ws.IdentityFrom(sess)
	if identity == "" {
		identity = ws.IdentityFrom(a.ws.TokenInfo(sess))
	}
	if identity == "" {
		return map[string]any{"ok": false, "error": "no identity"}
	}
	accounts := a.loadBook().Accounts
	if len(accounts) == 0 {
		return map[string]any{"ok": false, "error": "no accounts stored"}
	}
	items := make([]ws.Item, 0, len(accounts))
	for _, acc := range accounts {
		items = append(items, accountItem(acc))
	}
	lastBy := a.st.NavLastDates()
	navHistory, err := a.ws.FetchNavHistory(sess, identity, lastBy[""])
	if err != nil {
		if notAuthorized(err) {
			if allowRefresh && a.ws.RefreshSession(a.sessionOrEmpty(), true) {
				return a.refreshNavOnly(false)
			}
			return map[string]any{"ok": false, "error": "Session expired. Connect again."}
		}
		navHistory = nil
	}
	combined := []store.NavPoint{}
	for _, rec := range navHistory {
		rec.AccountID = ""
		combined = append(combined, rec)
	}
	nicknamePts, navErrors := a.fetchNicknameNavHistory(sess, items)
	combined = append(combined, nicknamePts...)
	a.st.UpsertNav(combined)
	nickSet := map[string]bool{}
	allDays := 0
	for _, p := range combined {
		if p.AccountID != "" {
			nickSet[p.AccountID] = true
		} else {
			allDays++
		}
	}
	return map[string]any{"ok": true, "allDays": allDays, "accounts": len(nickSet), "errors": navErrors}
}

func accountItem(acc store.Account) ws.Item {
	item := ws.Item{"id": acc.ID, "nickname": acc.Nickname, "unifiedAccountType": acc.UnifiedAccountType, "currency": acc.Currency, "status": acc.Status, "type": acc.Type, "marginAccountId": acc.MarginAccountID}
	if acc.NetLiquidationValue != nil {
		item["netLiquidationValue"] = *acc.NetLiquidationValue
	}
	return item
}

func (a *App) sessionOrEmpty() ws.Session {
	sess := a.loadSession()
	if sess == nil {
		return ws.Session{}
	}
	return sess
}

func (a *App) refreshPortfolio() map[string]any {
	a.mu.Lock()
	if a.state.syncing {
		a.mu.Unlock()
		return map[string]any{"ok": false, "skipped": "sync running"}
	}
	if !a.state.connected {
		a.mu.Unlock()
		return map[string]any{"ok": false, "skipped": "not connected"}
	}
	a.mu.Unlock()
	sess := a.loadSession()
	identity := ""
	if sess != nil {
		identity = ws.IdentityFrom(sess)
	}
	if sess == nil || sess.Str("access_token") == "" || identity == "" {
		a.logf("bagholder portfolio: no session to read with\n")
		return map[string]any{"ok": false, "skipped": "no session"}
	}
	accounts, err := a.ws.FetchAllAccounts(sess, identity)
	if err != nil {
		a.logf("bagholder portfolio: failed: %s\n", errText(err))
		return map[string]any{"ok": false, "skipped": "error"}
	}
	var ids []string
	for _, acc := range accounts {
		if id := py.S(acc["id"]); id != "" {
			ids = append(ids, id)
		}
	}
	if len(ids) == 0 {
		a.logf("bagholder portfolio: Wealthsimple returned no accounts\n")
		return map[string]any{"ok": false, "skipped": "no accounts"}
	}
	balances, err := a.ws.FetchBalances(sess, ids)
	if err != nil {
		a.logf("bagholder portfolio: failed: %s\n", errText(err))
		return map[string]any{"ok": false, "skipped": "error"}
	}
	margin := a.ws.FetchMargin(sess, ws.MarginAccountIDs(accounts))
	a.st.ReplaceAccounts(ws.SlimAccounts(accounts))
	a.st.ReplaceBalances(balances)
	a.st.ReplaceMargin(margin)
	a.st.SetMeta("balances_read_at", nowStamp())
	a.invalidate(false)
	available := 0
	for _, m := range margin {
		if m.BuyingPower != nil {
			available++
		}
	}
	a.logf("bagholder portfolio: %d accounts, %d balances, buying power for %d of %d margin accounts\n", len(ids), len(balances), available, len(margin))
	return map[string]any{"ok": true, "accounts": len(ids), "balances": len(balances), "margin": len(margin)}
}

type exposureJob struct {
	kind     string
	sec      store.Security
	symbol   string
	exchange string
	currency string
}

func (a *App) refreshExposures() map[string]any {
	snap := a.st.Snapshot(false)
	secs := map[string]store.Security{}
	for _, sec := range snap.Securities {
		if sec.ID != "" {
			secs[sec.ID] = sec
		}
	}
	base := a.model.Base()
	heldSet := map[string]bool{}
	for _, p := range base.Positions {
		if p.Kind == "Shares" {
			heldSet[p.SecurityID] = true
		}
	}
	for _, b := range snap.Balances {
		if b.Quantity != nil && *b.Quantity > 0 && strings.HasPrefix(b.SecurityID, "sec-s-") {
			heldSet[b.SecurityID] = true
		}
	}
	held := make([]string, 0, len(heldSet))
	for sid := range heldSet {
		if sid != "" && !strings.HasPrefix(sid, "sec-c-") {
			held = append(held, sid)
		}
	}
	sort.Strings(held)
	var todo []string
	for _, sid := range a.expo.Stale(held) {
		if _, ok := secs[sid]; ok {
			todo = append(todo, sid)
		}
	}
	sort.SliceStable(todo, func(i, j int) bool {
		fi, fj := 0, 0
		if exposure.IsFund(secs[todo[i]].Name) {
			fi = 1
		}
		if exposure.IsFund(secs[todo[j]].Name) {
			fj = 1
		}
		return fi < fj
	})
	underSet := map[[2]string]bool{}
	for _, p := range base.Positions {
		if p.Kind == "Options" && p.Underlying != "" {
			underSet[[2]string{strings.ToUpper(p.Underlying), p.Currency}] = true
		}
	}
	var unders [][2]string
	for k := range underSet {
		unders = append(unders, k)
	}
	sort.Slice(unders, func(i, j int) bool {
		if unders[i][0] != unders[j][0] {
			return unders[i][0] < unders[j][0]
		}
		return unders[i][1] < unders[j][1]
	})
	var underJobs [][2]string
	for _, u := range unders {
		if len(a.expo.Stale([]string{exposure.ShareKey + u[0] + ":" + market.TMXFormOr("", u[1])})) > 0 {
			underJobs = append(underJobs, u)
		}
	}
	var watched [][3]string
	for _, w := range base.Watchlist {
		if instruments.Find(w.Symbol, w.Exchange) != nil || strings.ToUpper(w.Exchange) == "CRYPTO" {
			continue
		}
		if len(a.expo.Stale([]string{model.WatchExposureKey(w.Symbol, w.Exchange, w.Currency)})) > 0 {
			watched = append(watched, [3]string{w.Symbol, w.Exchange, w.Currency})
		}
	}
	one := func(j exposureJob) string {
		if a.stopped() {
			return ""
		}
		switch j.kind {
		case "sec":
			rec := a.expo.RefreshSecurity(j.sec)
			kind := "share"
			if exposure.IsFund(j.sec.Name) {
				kind = "fund"
			}
			src := rec.Source
			if src == "" {
				src = "no source"
			}
			tail := ""
			if rec.Error != "" {
				tail = ": " + rec.Error
			}
			return "bagholder exposure: " + j.sec.Symbol + " " + kind + ": " + src + " (" + strconv.Itoa(py.RoundInt(rec.Coverage*100)) + "% covered)" + tail
		case "watch":
			rec := a.expo.ShareExposure(j.symbol, j.exchange, j.currency)
			if rec.Error != "" && rec.Source == "" {
				return "bagholder exposure: " + j.symbol + " (watched): " + rec.Error
			}
			return "bagholder exposure: " + j.symbol + " (watched) classified"
		}
		rec := a.expo.ShareExposure(j.symbol, "", j.currency)
		if rec.Error != "" && rec.Source == "" {
			return "bagholder exposure: " + j.symbol + " (an option's underlying): " + rec.Error
		}
		return "bagholder exposure: " + j.symbol + " (an option's underlying) classified"
	}
	var jobs []exposureJob
	for _, sid := range todo {
		jobs = append(jobs, exposureJob{kind: "sec", sec: secs[sid]})
	}
	for _, u := range underJobs {
		jobs = append(jobs, exposureJob{kind: "under", symbol: u[0], currency: u[1]})
	}
	for _, w := range watched {
		jobs = append(jobs, exposureJob{kind: "watch", symbol: w[0], exchange: w[1], currency: w[2]})
	}
	done := 0
	if len(jobs) > 0 {
		lines := make(chan string, len(jobs))
		queue := make(chan exposureJob)
		var wg sync.WaitGroup
		for i := 0; i < ExposureWorkers; i++ {
			wg.Add(1)
			go func() {
				defer wg.Done()
				for j := range queue {
					lines <- one(j)
				}
			}()
		}
		go func() {
			for _, j := range jobs {
				queue <- j
			}
			close(queue)
		}()
		go func() {
			wg.Wait()
			close(lines)
		}()
		for line := range lines {
			if line != "" {
				done++
				a.logf("%s\n", line)
				a.invalidate(false)
			}
		}
	}
	return map[string]any{"ok": true, "held": len(held), "refreshed": done}
}

func (a *App) exposureLoop() {
	wait := time.Duration(ExposureFirstSec) * time.Second
	for !a.wait(wait) {
		wait = time.Duration(ExposureCheckSec) * time.Second
		a.refreshExposures()
	}
}

func (a *App) portfolioLoop() {
	a.refreshPortfolio()
	for !a.wait(60 * PortfolioRefreshMinutes * time.Second) {
		a.refreshPortfolio()
	}
}

func (a *App) collectSecurityIDs() []string {
	var ids []string
	seen := map[string]bool{}
	snap := a.st.Snapshot(true)
	for _, act := range snap.Activities {
		sid := py.Strip(act.SecurityID)
		if sid != "" && !seen[sid] {
			seen[sid] = true
			ids = append(ids, sid)
		}
	}
	for _, b := range snap.Balances {
		sid := py.Strip(b.SecurityID)
		if sid != "" && !seen[sid] {
			seen[sid] = true
			ids = append(ids, sid)
		}
	}
	return ids
}

func (a *App) accountIDsForBackfill() []string {
	snap := a.st.Snapshot(true)
	var ids []string
	seen := map[string]bool{}
	for _, acc := range snap.Accounts {
		aid := py.Strip(acc.ID)
		if aid != "" && !seen[aid] {
			seen[aid] = true
			ids = append(ids, aid)
		}
	}
	if len(ids) > 0 {
		return ids
	}
	for _, act := range snap.Activities {
		aid := py.Strip(act.AccountID)
		if aid != "" && !seen[aid] {
			seen[aid] = true
			ids = append(ids, aid)
		}
	}
	return ids
}

func accountItemsByID(accounts []store.Account) []ws.Item {
	out := make([]ws.Item, 0, len(accounts))
	for _, acc := range accounts {
		if acc.ID != "" {
			out = append(out, accountItem(acc))
		}
	}
	return out
}

func (a *App) fillListings(sess ws.Session, fromSync bool) bool {
	_ = a.st.Ensure()
	if sess == nil || sess.Str("access_token") == "" {
		return false
	}
	a.mu.Lock()
	if a.state.listingsFilling || (a.state.syncing && !fromSync) {
		a.mu.Unlock()
		return false
	}
	a.state.listingsFilling = true
	a.state.syncStep = "Attaching listing ids…"
	a.mu.Unlock()
	defer func() {
		a.mu.Lock()
		a.state.listingsFilling = false
		a.state.syncStep = ""
		a.mu.Unlock()
	}()
	if a.st.NeedsSecurityIDBackfill() {
		walkOK := true
		var mapped []store.Activity
		a.setSyncStep("Attaching listing ids…")
		for _, aid := range a.accountIDsForBackfill() {
			rawItems, err := a.ws.FetchActivitiesForAccount(sess, aid, "")
			if err != nil {
				walkOK = false
				continue
			}
			accItems := accountItemsByID(a.st.Snapshot(false).Accounts)
			for _, it := range rawItems {
				mapped = append(mapped, ws.MapActivityRows(it, accItems)...)
			}
		}
		if len(mapped) > 0 {
			a.st.ApplyWealthsimpleMapped(mapped)
		}
		if walkOK {
			a.st.SetMeta("security_id_backfill_done", "1")
		}
	}
	wanted := a.collectSecurityIDs()
	pending := a.st.MissingSecurityIDs(wanted)
	seen := map[string]bool{}
	var toUpsert []store.Security
	for len(pending) > 0 {
		a.setSyncStep("Looking up company names, " + strconv.Itoa(len(pending)) + " left")
		var batch []string
		for _, sid := range pending {
			if !seen[sid] {
				batch = append(batch, sid)
			}
		}
		for _, sid := range batch {
			seen[sid] = true
		}
		pending = nil
		if len(batch) == 0 {
			break
		}
		recs := a.ws.FetchSecurities(sess, batch)
		toUpsert = append(toUpsert, recs...)
		var under []string
		for _, r := range recs {
			u := py.Strip(r.UnderlyingID)
			if u != "" && !seen[u] {
				under = append(under, u)
			}
		}
		if len(under) > 0 {
			pending = a.st.MissingSecurityIDs(under)
		}
	}
	if len(toUpsert) > 0 {
		a.st.UpsertSecurities(toUpsert)
	}
	return true
}

func expiresAtUnix(sess ws.Session) (float64, bool) {
	if sess == nil {
		return 0, false
	}
	raw, ok := sess["expires_at"]
	if !ok || raw == nil || raw == "" {
		return 0, false
	}
	if f, ok := raw.(float64); ok {
		return f, true
	}
	s := py.Strip(py.S(raw))
	if f, err := strconv.ParseFloat(s, 64); err == nil {
		return f, true
	}
	if t, naive, ok := py.ParseISO(s); ok {
		if naive {
			t = time.Date(t.Year(), t.Month(), t.Day(), t.Hour(), t.Minute(), t.Second(), t.Nanosecond(), time.UTC)
		}
		return float64(t.UnixNano()) / 1e9, true
	}
	return 0, false
}

func tokenRefreshNeeded(sess ws.Session, now time.Time) bool {
	exp, ok := expiresAtUnix(sess)
	if !ok {
		return true
	}
	return float64(now.UnixNano())/1e9 >= exp-TokenRefreshMarginSec
}

func (a *App) ensureFreshToken(sess ws.Session) bool {
	if sess == nil {
		sess = a.loadSession()
	}
	if sess == nil || sess.Str("refresh_token") == "" {
		a.mu.Lock()
		a.state.connected = false
		a.state.err = "missing refresh token"
		a.mu.Unlock()
		return false
	}
	a.mu.Lock()
	connected := a.state.connected
	a.mu.Unlock()
	if connected && !tokenRefreshNeeded(sess, time.Now()) {
		return true
	}
	ok := a.ws.RefreshSession(sess, true)
	a.mu.Lock()
	a.state.connected = ok
	if ok {
		a.state.err = ""
	} else if strings.TrimSpace(a.state.err) == "" {
		a.state.err = "Wealthsimple token refresh failed"
	}
	a.mu.Unlock()
	return ok
}

func (a *App) activitySyncBounds() (string, bool) {
	if a.st.ActivityCount() == 0 {
		return "", true
	}
	return a.st.IncrementalStartDate(), false
}

func (a *App) runSync(allowRefresh, forceActivity bool) bool {
	_ = a.st.Ensure()
	a.mu.Lock()
	if a.state.syncing {
		a.mu.Unlock()
		return false
	}
	a.state.syncing = true
	a.state.err = ""
	a.state.syncStep = "Checking session…"
	a.mu.Unlock()
	finished := false
	finish := func() {
		if finished {
			return
		}
		finished = true
		a.mu.Lock()
		a.state.syncing = false
		a.state.syncStep = ""
		a.mu.Unlock()
	}
	defer finish()
	sess := a.loadSession()
	if sess == nil || (sess.Str("access_token") == "" && sess.Str("refresh_token") == "") || sess.Str("access_token") == "" {
		a.mu.Lock()
		a.state.connected = false
		a.mu.Unlock()
		return false
	}
	var info map[string]any
	identity := ws.IdentityFrom(sess)
	if identity == "" {
		info = a.ws.TokenInfo(sess)
		identity = ws.IdentityFrom(info)
	}
	if identity == "" {
		line := "Sync failed: " + ws.PublicSyncError("no identity_canonical_id")
		a.logf("%s\n", line)
		a.setError(line)
		a.noteSyncFailed(ws.PublicSyncError("no identity_canonical_id"))
		return false
	}
	sess["identity_canonical_id"] = identity
	email := py.S(info["email"])
	if email == "" {
		email = py.S(info["username"])
	}
	if email == "" {
		email = sess.Str("email")
	}
	if email != "" {
		sess["email"] = email
	}
	a.saveSession(sess)
	if !forceActivity && !a.st.ActivityPullDue(time.Now()) {
		a.mu.Lock()
		a.state.connected = true
		a.state.email = email
		if s := a.st.GetMeta("synced_at"); s != "" {
			a.state.lastSync = s
		}
		a.state.capturing = false
		a.state.err = ""
		a.state.syncStep = ""
		a.mu.Unlock()
		return true
	}
	fail := func(err error) bool {
		if notAuthorized(err) {
			if allowRefresh && a.ws.RefreshSession(a.sessionOrEmpty(), true) {
				finish()
				return a.runSync(false, forceActivity)
			}
			a.noteSessionExpired()
			return false
		}
		public := publicSyncError(err)
		line := "Sync failed: " + public
		a.logf("%s\n", line)
		a.setError(line)
		a.noteSyncFailed(public)
		return false
	}
	a.setSyncStep("Fetching accounts…")
	accounts, err := a.ws.FetchAllAccounts(sess, identity)
	if err != nil {
		return fail(err)
	}
	var accItems []ws.Item
	var accIDs []string
	for _, acc := range accounts {
		if id := py.S(acc["id"]); id != "" {
			accItems = append(accItems, acc)
			accIDs = append(accIDs, id)
		}
	}
	startDate, _ := a.activitySyncBounds()
	var mapped []store.Activity
	a.setSyncStep("Syncing transactions")
	for _, aid := range accIDs {
		rawItems, err := a.ws.FetchActivitiesForAccount(sess, aid, startDate)
		if err != nil {
			return fail(err)
		}
		for _, it := range rawItems {
			mapped = append(mapped, ws.MapActivityRows(it, accItems)...)
		}
	}
	pools := ws.FifoPoolIDs(accounts)
	for i := range mapped {
		aid := mapped[i].AccountID
		if p, ok := pools[aid]; ok {
			mapped[i].FifoID = p
		} else {
			mapped[i].FifoID = aid
		}
	}
	a.setSyncStep("Fetching balances…")
	balances, err := a.ws.FetchBalances(sess, accIDs)
	if err != nil {
		return fail(err)
	}
	margin := a.ws.FetchMargin(sess, ws.MarginAccountIDs(accounts))
	a.setSyncStep("Fetching equity history…")
	lastBy := a.st.NavLastDates()
	navHistory, err := a.ws.FetchNavHistory(sess, identity, lastBy[""])
	if err != nil {
		navHistory = nil
	}
	combined := []store.NavPoint{}
	for _, rec := range navHistory {
		rec.AccountID = ""
		combined = append(combined, rec)
	}
	nicknamePts, navErrors := a.fetchNicknameNavHistory(sess, accounts)
	combined = append(combined, nicknamePts...)
	a.st.ApplyWealthsimpleMapped(mapped)
	synced := nowStamp()
	a.setSyncStep("Saving…")
	a.saveAccountsSnapshot(accountsSnapshot{accounts: ws.SlimAccounts(accounts), balances: balances, margin: margin, hasMargin: true, navHistory: combined, syncedAt: synced})
	a.st.MarkActivityPulled(synced)
	a.fillListings(sess, true)
	navErrLine := ""
	if len(navErrors) > 0 {
		navErrLine = "NAV history failed for " + strings.Join(navErrors, "; ")
	}
	a.mu.Lock()
	a.state.connected = true
	a.state.syncFails = 0
	a.state.email = email
	a.state.lastSync = synced
	a.state.capturing = false
	a.state.err = navErrLine
	a.state.syncStep = ""
	a.mu.Unlock()
	return true
}

func (a *App) noteSessionExpired() {
	a.mu.Lock()
	was := a.state.connected
	a.state.connected = false
	a.state.err = "Session expired. Connect again."
	a.mu.Unlock()
	if was {
		a.notify.Emit("connection", "session:"+nowStamp(), "Sign in needed", "The Wealthsimple session expired. Connect again from the menu.", nil)
	}
}

func (a *App) noteSyncFailed(reason string) {
	a.mu.Lock()
	a.state.syncFails++
	fails := a.state.syncFails
	if fails == 1 {
		a.state.syncFirstFail = nowStamp()
	}
	first := a.state.syncFirstFail
	a.mu.Unlock()
	if fails == SyncFailsTold {
		if reason == "" {
			reason = "Sync failed."
		}
		a.notify.Emit("connection", "sync:"+first, "Sync failing", reason, nil)
	}
}

func (a *App) bootSession() {
	_ = a.st.Ensure()
	sess := a.loadSession()
	if sess == nil {
		a.mu.Lock()
		a.state.connected = false
		a.state.lastSync = a.st.Snapshot(false).SyncedAt
		a.mu.Unlock()
		return
	}
	infoOK := false
	if sess.Str("access_token") != "" {
		info := a.ws.TokenInfo(sess)
		infoOK = len(info) > 0 && info["error"] == nil && info["_http_status"] == nil
		if infoOK {
			a.ws.ApplyTokenInfoClientID(sess, info)
			if id := py.JSONStr(info["identity_canonical_id"]); id != "" && sess.Str("identity_canonical_id") == "" {
				sess["identity_canonical_id"] = id
			}
			if email := py.S(info["email"]); email != "" {
				sess["email"] = email
			}
			a.saveSession(sess)
		}
	}
	ok := infoOK
	if !ok && sess.Str("refresh_token") != "" {
		ok = a.ws.RefreshSession(sess, true)
		if s := a.loadSession(); s != nil {
			sess = s
		}
	}
	a.mu.Lock()
	a.state.connected = ok
	if ok {
		a.state.err = ""
	} else if strings.TrimSpace(a.state.err) == "" {
		if sess.Str("refresh_token") == "" {
			a.state.err = "missing refresh token"
		} else {
			a.state.err = "Wealthsimple token refresh failed"
		}
	}
	a.state.email = sess.Str("email")
	a.state.lastSync = a.st.Snapshot(false).SyncedAt
	a.mu.Unlock()
}

func (a *App) refreshNow() map[string]any {
	sess := a.loadSession()
	if sess == nil || sess.Str("refresh_token") == "" {
		a.mu.Lock()
		a.state.connected = false
		a.state.err = "not connected"
		a.mu.Unlock()
		return map[string]any{"ok": false, "error": "not connected", "connected": false}
	}
	ok := a.ws.RefreshSession(sess, true)
	a.mu.Lock()
	a.state.connected = ok
	if ok {
		a.state.err = ""
	}
	err := strings.TrimSpace(a.state.err)
	a.mu.Unlock()
	return map[string]any{"ok": ok, "error": err, "connected": ok}
}

func (a *App) autoSyncLoop() {
	delay := time.Duration(TokenCheckSec) * time.Second
	failDelay := time.Duration(TokenCheckSec) * time.Second
	for !a.wait(delay) {
		sess := a.loadSession()
		if sess != nil && sess.Str("refresh_token") != "" {
			a.ensureFreshToken(sess)
		}
		a.mu.Lock()
		connected, syncing := a.state.connected, a.state.syncing
		a.mu.Unlock()
		if connected && !syncing && a.st.ActivityPullDue(time.Now()) {
			ok := a.runSync(true, true)
			a.refreshMarketData()
			if ok {
				failDelay = time.Duration(TokenCheckSec) * time.Second
			} else {
				if failDelay < time.Duration(TokenCheckSec)*time.Second {
					failDelay = time.Duration(TokenCheckSec) * time.Second
				}
				failDelay *= 2
				if failDelay > 1800*time.Second {
					failDelay = 1800 * time.Second
				}
			}
			delay = failDelay
		} else {
			delay = time.Duration(TokenCheckSec) * time.Second
			failDelay = delay
		}
	}
}

func (a *App) syncThenMarket() bool {
	ok := a.runSync(true, true)
	a.refreshMarketData()
	return ok
}
