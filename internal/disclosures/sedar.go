package disclosures

import (
	"crypto/sha1"
	"encoding/hex"
	"fmt"
	"html"
	"net/url"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/browserhttp"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

const (
	SedarBase        = "https://www.sedarplus.ca"
	SedarPaceSeconds = 2.0
	SedarTimeout     = 90
	SedarDocTimeout  = 180
	SedarSearchLimit = 100
	SedarSource      = "SEDAR+"
	ScopeTTL         = 120
)

type ProfileNotFound struct{ Msg string }

func (e *ProfileNotFound) Error() string { return e.Msg }

func notFound(format string, args ...any) error { return &ProfileNotFound{fmt.Sprintf(format, args...)} }

func IsProfileNotFound(err error) bool {
	_, ok := err.(*ProfileNotFound)
	return ok
}

type scopeHit struct {
	expires time.Time
	html    string
}

type Sedar struct {
	mu       sync.Mutex
	session  *browserhttp.Session
	last     time.Time
	scope    map[string]scopeHit
	scopeMu  sync.Mutex
	docSess  *browserhttp.Session
}

func NewSedar() *Sedar { return &Sedar{scope: map[string]scopeHit{}} }

func (s *Sedar) Source() string { return SedarSource }

func (s *Sedar) Available() bool { return true }

func (s *Sedar) pace() {
	wait := s.last.Add(time.Duration(SedarPaceSeconds * float64(time.Second))).Sub(time.Now())
	if wait > 0 {
		time.Sleep(wait)
	}
	s.last = time.Now()
}

func (s *Sedar) getSession() (*browserhttp.Session, error) {
	if s.session == nil {
		sess, err := browserhttp.New(SedarDocTimeout, true)
		if err != nil {
			return nil, Unavailable("browser session could not be opened: %s", err)
		}
		s.session = sess
	}
	return s.session, nil
}

func (s *Sedar) Reset() {
	s.mu.Lock()
	s.session = nil
	s.mu.Unlock()
}

var (
	fieldRE     = regexp.MustCompile(`(?i)<(input|select|textarea)\b([^>]*)>`)
	nameRE      = regexp.MustCompile(`name="([^"]*)"`)
	typeRE      = regexp.MustCompile(`type="([^"]*)"`)
	valueRE     = regexp.MustCompile(`value="([^"]*)"`)
	selectedRE  = regexp.MustCompile(`<option[^>]*selected[^>]*value="([^"]*)"|value="([^"]*)"[^>]*selected`)
	viParamRE   = regexp.MustCompile(`(?i)<input\b([^>]*class="[^"]*viewInstanceFormParameter[^"]*"[^>]*)>`)
	searchActRE = regexp.MustCompile(`(?s)(?:appSearchButton|-searchButton)[^>]*?onclick="[^"]*?cat\w*Callback\('(W\d+)','(\w+)'[^"]*?containerNodeId:'(W\d+)'`)
	menuAnchor  = regexp.MustCompile(`(?s)<a[^>]*?catCallback\('(W\d+)','invokeMenuCb'[^>]*>(.*?)</a>`)
	catCallback = regexp.MustCompile(`catCallback\('(W\d+)'`)
	instRE      = regexp.MustCompile(`viewInstance/view\.html\?id=([0-9a-f]+)`)
	instUpdRE   = regexp.MustCompile(`update\.html\?id=([0-9a-f]+)`)
	keyRE       = regexp.MustCompile(`viewInstanceKey:'([^']+)'`)
	sidRE       = regexp.MustCompile(`sessionId:'([^']+)'`)
	appRE       = regexp.MustCompile(`/(csa-\w+)/viewInstance`)
	tagRE       = regexp.MustCompile(`<[^>]+>`)
	issuerRE    = regexp.MustCompile(`appReceiveFocus">\s*([^<]*?\((\d{9})\))\s*</span>`)
	docLinkRE   = regexp.MustCompile(`(?s)<a class="appDocumentView appResourceLink appDocumentLink" href="([^"]+)"[^>]*>\s*<span>(.*?)</span>`)
	submittedRE = regexp.MustCompile(`<span aria-hidden="true">\s*(\d{1,2} \w{3} \d{4}[^<]*?)\s*</span>`)
	sizeRE      = regexp.MustCompile(`(?i)(\d[\d.,]* ?(?:KB|MB|bytes))`)
	riRowRE     = regexp.MustCompile(`(?s)<tr[^>]*appTblRow[^>]*>(.*?)</tr>`)
	tdRE        = regexp.MustCompile(`(?s)<td[^>]*>(.*?)</td>`)
	drmRE       = regexp.MustCompile(`drmKey=([0-9a-f]+)`)
	nineRE      = regexp.MustCompile(`^\d{9}$`)
	submitRE    = regexp.MustCompile(`^(\d{1,2}) (\w{3}) (\d{4})(?:\s+(\d{1,2}):(\d{2}))?`)
	langTailRE  = regexp.MustCompile(`(?i)[-–]\s*(English|French)\s*$`)
	langParenRE = regexp.MustCompile(`(?i)\((English|French)\)\s*$`)
	pdfExtRE    = regexp.MustCompile(`(?i)\.pdf$`)
)

const docsMenuText = "search and download documents for this profile"

type field struct{ k, v string }

func formFields(doc string) []field {
	var out []field
	for _, m := range fieldRE.FindAllStringSubmatchIndex(doc, -1) {
		tag := strings.ToLower(doc[m[2]:m[3]])
		attrs := doc[m[4]:m[5]]
		name := nameRE.FindStringSubmatch(attrs)
		if name == nil || strings.HasPrefix(name[1], "_CB") {
			continue
		}
		nm := name[1]
		if tag == "input" {
			typ := "text"
			if t := typeRE.FindStringSubmatch(attrs); t != nil {
				typ = strings.ToLower(t[1])
			}
			if typ == "submit" || typ == "button" || typ == "file" {
				continue
			}
			if (typ == "checkbox" || typ == "radio") && !strings.Contains(attrs, "checked") {
				continue
			}
			val := ""
			if v := valueRE.FindStringSubmatch(attrs); v != nil {
				val = html.UnescapeString(v[1])
			}
			out = append(out, field{nm, val})
		} else if tag == "select" {
			end := strings.Index(doc[m[1]:], "</select>")
			body := ""
			if end >= 0 {
				body = doc[m[1] : m[1]+end]
			} else {
				body = doc[m[1]:]
			}
			val := ""
			if sel := selectedRE.FindStringSubmatch(body); sel != nil {
				v := sel[1]
				if v == "" {
					v = sel[2]
				}
				val = html.UnescapeString(v)
			}
			out = append(out, field{nm, val})
		}
	}
	return out
}

func viParams(doc string) []field {
	var out []field
	for _, m := range viParamRE.FindAllStringSubmatch(doc, -1) {
		name := nameRE.FindStringSubmatch(m[1])
		if name == nil {
			continue
		}
		val := ""
		if v := valueRE.FindStringSubmatch(m[1]); v != nil {
			val = html.UnescapeString(v[1])
		}
		out = append(out, field{name[1], val})
	}
	return out
}

type searchAction struct{ node, name, container string }

func findSearchAction(page string) *searchAction {
	m := searchActRE.FindStringSubmatch(page)
	if m == nil {
		return nil
	}
	return &searchAction{m[1], m[2], m[3]}
}

func sedarText(s string) string {
	return py.Strip(py.CollapseSpace(html.UnescapeString(tagRE.ReplaceAllString(s, " "))))
}

func issuerMenuNode(doc, name string) string {
	fallback := ""
	want := strings.ToLower(cutRunes(sedarText(name), 20))
	for _, m := range menuAnchor.FindAllStringSubmatch(doc, -1) {
		t := sedarText(m[2])
		if t == "" || strings.Contains(strings.ToLower(t), "search for profiles") {
			continue
		}
		if want != "" && strings.Contains(strings.ToLower(t), want) {
			return m[1]
		}
		if fallback == "" {
			fallback = m[1]
		}
	}
	return fallback
}

func docsMenuNode(doc string) string {
	idx := strings.Index(strings.ToLower(doc), docsMenuText)
	if idx < 0 {
		return ""
	}
	start := strings.LastIndex(doc[:idx], "<a ")
	if start < 0 {
		return ""
	}
	if m := catCallback.FindStringSubmatch(doc[start:idx]); m != nil {
		return m[1]
	}
	return ""
}

type view struct {
	s       *Sedar
	service string
	app     string
	inst    string
	key     string
	sid     string
	page    string
	ref     string
}

func (s *Sedar) openView(service string) (*view, error) {
	sess, err := s.getSession()
	if err != nil {
		return nil, err
	}
	s.pace()
	resp, err := sess.Get(SedarBase+"/csa-party/service/create.html?targetAppCode=csa-party&service="+service, nil)
	if err != nil {
		return nil, Unavailable("could not open %s: %s", service, err)
	}
	page := string(resp.Body)
	head := page
	if len(head) > 2000 {
		head = head[:2000]
	}
	if strings.Contains(resp.URL, "validate.perfdrive.com") || strings.Contains(head, "validate.perfdrive.com") {
		return nil, Unavailable("the SEDAR+ bot gate turned the request away")
	}
	mInst := instRE.FindStringSubmatch(resp.URL)
	if mInst == nil {
		mInst = instUpdRE.FindStringSubmatch(page)
	}
	mKey := keyRE.FindStringSubmatch(page)
	mSid := sidRE.FindStringSubmatch(page)
	mApp := appRE.FindStringSubmatch(resp.URL)
	if mInst == nil || mKey == nil || mSid == nil {
		return nil, Unavailable("SEDAR+ did not return the %s form", service)
	}
	v := &view{s: s, service: service, app: "csa-party", inst: mInst[1], key: mKey[1], sid: mSid[1], page: page}
	if mApp != nil {
		v.app = mApp[1]
	}
	v.ref = SedarBase + "/" + v.app + "/viewInstance/view.html?id=" + v.inst
	return v, nil
}

func (v *view) headers(async bool) map[string]string {
	h := map[string]string{
		"x-catalyst-session-global": v.sid,
		"x-security-token":          "null",
		"Referer":                   v.ref,
		"Origin":                    SedarBase,
		"Content-Type":              "application/x-www-form-urlencoded; charset=UTF-8",
	}
	if async {
		h["x-catalyst-async"] = "true"
		h["x-catalyst-secured"] = "true"
		h["X-Requested-With"] = "XMLHttpRequest"
	}
	return h
}

func encodeForm(data []field) string {
	var b strings.Builder
	for i, f := range data {
		if i > 0 {
			b.WriteByte('&')
		}
		b.WriteString(url.QueryEscape(f.k))
		b.WriteByte('=')
		b.WriteString(url.QueryEscape(f.v))
	}
	return b.String()
}

type callbackOpts struct {
	value     *string
	extra     []field
	container string
	jsonFrag  bool
	html      *string
}

func (v *view) callback(node, name string, o callbackOpts) (string, error) {
	page := v.page
	if o.html != nil {
		page = *o.html
	}
	extraKeys := map[string]bool{}
	for _, f := range o.extra {
		extraKeys[f.k] = true
	}
	var data []field
	if o.jsonFrag {
		data = viParams(page)
	} else {
		for _, f := range formFields(page) {
			if !extraKeys[f.k] {
				data = append(data, f)
			}
		}
	}
	data = append(data, field{"_CBNODE_", node}, field{"_CBNAME_", name}, field{"_VIKEY_", v.key})
	if o.value != nil {
		data = append(data, field{"_CBVALUE_", *o.value})
	}
	if o.container != "" {
		data = append(data, field{"_CBHTMLFRAG_", "true"}, field{"_CBHTMLFRAGID_", strconv.FormatInt(time.Now().UnixMilli(), 10)}, field{"_CBHTMLFRAGNODEID_", o.container}, field{"_CBASYNCUPDATE_", "true"})
	}
	if o.jsonFrag {
		data = append(data, field{"_CBJSONFRAG_", "true"})
	}
	data = append(data, o.extra...)
	sess, err := v.s.getSession()
	if err != nil {
		return "", err
	}
	v.s.pace()
	resp, err := sess.Post(SedarBase+"/"+v.app+"/viewInstance/update.html?id="+v.inst, v.headers(o.container != "" || o.jsonFrag), []byte(encodeForm(data)))
	if err != nil {
		return "", Unavailable("callback %s/%s failed: %s", node, name, err)
	}
	return string(resp.Body), nil
}

func (v *view) refreshIdentity(doc string) bool {
	mInst := instRE.FindStringSubmatch(doc)
	if mInst == nil {
		mInst = instUpdRE.FindStringSubmatch(doc)
	}
	mKey := keyRE.FindStringSubmatch(doc)
	mSid := sidRE.FindStringSubmatch(doc)
	if mInst != nil && mKey != nil {
		v.inst, v.key = mInst[1], mKey[1]
		if mSid != nil {
			v.sid = mSid[1]
		}
		v.ref = SedarBase + "/" + v.app + "/viewInstance/view.html?id=" + v.inst
		v.page = doc
		return true
	}
	return false
}

func FilingID(rawURL, profileNo, file, submitted string) string {
	if m := drmRE.FindStringSubmatch(rawURL); m != nil {
		return "drm:" + m[1]
	}
	h := sha1.Sum([]byte(strings.Join([]string{profileNo, file, submitted, rawURL}, "|")))
	return "h:" + hex.EncodeToString(h[:])[:16]
}

type Filing struct {
	ID          string `json:"id"`
	Issuer      string `json:"issuer"`
	ProfileNo   string `json:"profileNo"`
	File        string `json:"file"`
	Submitted   string `json:"submitted"`
	SubmittedAt string `json:"submittedAt"`
	Size        string `json:"size"`
	URL         string `json:"url"`
}

func ParseFilings(doc string) []Filing {
	out := []Filing{}
	for _, m := range docLinkRE.FindAllStringSubmatchIndex(doc, -1) {
		rawURL := html.UnescapeString(doc[m[2]:m[3]])
		start := m[0] - 2600
		if start < 0 {
			start = 0
		}
		before := doc[start:m[0]]
		end := m[1] + 1400
		if end > len(doc) {
			end = len(doc)
		}
		after := doc[m[1]:end]
		var issuer []string
		for _, im := range issuerRE.FindAllStringSubmatch(before, -1) {
			issuer = im
		}
		sub := submittedRE.FindStringSubmatch(after)
		size := sizeRE.FindStringSubmatch(after)
		profileNo := ""
		issuerName := ""
		if issuer != nil {
			profileNo = issuer[2]
			issuerName = sedarText(issuer[1])
		}
		file := sedarText(doc[m[4]:m[5]])
		submitted := ""
		if sub != nil {
			submitted = strings.TrimSpace(sub[1])
		}
		sz := ""
		if size != nil {
			sz = size[1]
		}
		out = append(out, Filing{ID: FilingID(rawURL, profileNo, file, submitted), Issuer: issuerName, ProfileNo: profileNo, File: file, Submitted: submitted, SubmittedAt: sedarISO(submitted), Size: sz, URL: rawURL})
	}
	return out
}

var sedarMonths = map[string]int{"Jan": 1, "Feb": 2, "Mar": 3, "Apr": 4, "May": 5, "Jun": 6, "Jul": 7, "Aug": 8, "Sep": 9, "Oct": 10, "Nov": 11, "Dec": 12}

func sedarISO(submitted string) string {
	m := submitRE.FindStringSubmatch(submitted)
	if m == nil {
		return ""
	}
	mon, ok := sedarMonths[m[2]]
	if !ok {
		return ""
	}
	d, _ := strconv.Atoi(m[1])
	y, _ := strconv.Atoi(m[3])
	hh, _ := strconv.Atoi(m[4])
	mm, _ := strconv.Atoi(m[5])
	return fmt.Sprintf("%04d-%02d-%02dT%02d:%02d", y, mon, d, hh, mm)
}

type Profile struct {
	Name         string `json:"name"`
	ProfileNo    string `json:"profileNo"`
	Provinces    string `json:"provinces"`
	Jurisdiction string `json:"jurisdiction"`
	Type         string `json:"type"`
}

func ParseReportingIssuers(doc string) []Profile {
	out := []Profile{}
	for _, row := range riRowRE.FindAllStringSubmatch(doc, -1) {
		var cells []string
		for _, c := range tdRE.FindAllStringSubmatch(row[1], -1) {
			cells = append(cells, sedarText(c[1]))
		}
		idx := -1
		for i, c := range cells {
			if nineRE.MatchString(c) {
				idx = i
				break
			}
		}
		if idx < 0 {
			continue
		}
		cell := func(i int) string {
			if i >= 0 && i < len(cells) {
				return cells[i]
			}
			return ""
		}
		out = append(out, Profile{Name: cell(idx - 1), ProfileNo: cells[idx], Provinces: cell(idx + 3), Jurisdiction: cell(idx + 4), Type: cell(idx + 5)})
	}
	return out
}

func (s *Sedar) ResolveProfile(query string) ([]Profile, error) {
	q := strings.TrimSpace(query)
	if q == "" {
		return nil, notFound("empty query")
	}
	s.mu.Lock()
	v, err := s.openView("searchReportingIssuers")
	if err != nil {
		s.mu.Unlock()
		return nil, err
	}
	action := findSearchAction(v.page)
	if action == nil {
		s.mu.Unlock()
		return nil, Unavailable("could not find the reporting-issuer search control on the page")
	}
	doc, err := v.callback(action.node, action.name, callbackOpts{extra: []field{{"QueryString", q}}, container: action.container})
	s.mu.Unlock()
	if err != nil {
		return nil, err
	}
	rows := ParseReportingIssuers(doc)
	if len(rows) == 0 {
		return nil, notFound("no SEDAR+ profile matched %q", q)
	}
	ql := strings.ToLower(q)
	rank := func(r Profile) (int, int, int) {
		a, b, c := 1, 1, 1
		if r.ProfileNo == q {
			a = 0
		}
		if strings.Contains(strings.ToLower(r.Name), ql) {
			b = 0
		}
		if strings.HasPrefix(strings.ToLower(r.Name), ql) {
			c = 0
		}
		return a, b, c
	}
	sort.SliceStable(rows, func(i, j int) bool {
		a1, b1, c1 := rank(rows[i])
		a2, b2, c2 := rank(rows[j])
		if a1 != a2 {
			return a1 < a2
		}
		if b1 != b2 {
			return b1 < b2
		}
		return c1 < c2
	})
	return rows, nil
}

type ListResult struct {
	Profile *Profile `json:"profile"`
	Scoped  *bool    `json:"scoped"`
	Filings []Filing `json:"filings"`
}

func (s *Sedar) ListFilings(query, profileNo string, limit int) (*ListResult, error) {
	var profile *Profile
	if profileNo == "" && query != "" {
		matches, err := s.ResolveProfile(query)
		if err != nil {
			return nil, err
		}
		p := matches[0]
		profile = &p
		profileNo = p.ProfileNo
	}
	var scoped *bool
	var doc string
	s.mu.Lock()
	if profileNo != "" {
		name := query
		if profile != nil && profile.Name != "" {
			name = profile.Name
		}
		html, ok := s.scopedDocuments(profileNo, name)
		scoped = &ok
		if !ok {
			v, err := s.openView("searchDocuments")
			if err != nil {
				s.mu.Unlock()
				return nil, err
			}
			html = v.page
		}
		doc = html
	} else {
		v, err := s.openView("searchDocuments")
		if err != nil {
			s.mu.Unlock()
			return nil, err
		}
		doc = v.page
	}
	s.mu.Unlock()
	filings := ParseFilings(doc)
	if profileNo != "" {
		kept := []Filing{}
		for _, f := range filings {
			if f.ProfileNo == "" || f.ProfileNo == profileNo {
				kept = append(kept, f)
			}
		}
		filings = kept
	}
	if limit < 1 {
		limit = 1
	}
	if len(filings) > limit {
		filings = filings[:limit]
	}
	out := &ListResult{Profile: profile, Scoped: scoped, Filings: filings}
	if profile == nil && profileNo != "" {
		out.Profile = &Profile{ProfileNo: profileNo}
	}
	return out, nil
}

func (s *Sedar) scopedDocuments(profileNo, name string) (string, bool) {
	s.scopeMu.Lock()
	hit, ok := s.scope[profileNo]
	s.scopeMu.Unlock()
	if ok && hit.expires.After(time.Now()) {
		return hit.html, true
	}
	doc, ok := s.scopedDocumentsUncached(profileNo, name)
	if ok {
		s.scopeMu.Lock()
		s.scope[profileNo] = scopeHit{time.Now().Add(ScopeTTL * time.Second), doc}
		s.scopeMu.Unlock()
	}
	return doc, ok
}

func (s *Sedar) scopedDocumentsUncached(profileNo, name string) (string, bool) {
	v, err := s.openView("searchReportingIssuers")
	if err != nil {
		return "", false
	}
	action := findSearchAction(v.page)
	if action == nil {
		return "", false
	}
	ri, err := v.callback(action.node, action.name, callbackOpts{extra: []field{{"QueryString", profileNo}}, container: action.container})
	if err != nil {
		return "", false
	}
	issuerNode := issuerMenuNode(ri, name)
	if issuerNode == "" {
		return "", false
	}
	profilePage, err := v.callback(issuerNode, "invokeMenuCb", callbackOpts{html: &ri})
	if err != nil {
		return "", false
	}
	v.refreshIdentity(profilePage)
	docsNode := docsMenuNode(v.page)
	if docsNode == "" {
		return "", false
	}
	page := v.page
	docs, err := v.callback(docsNode, "invokeMenuCb", callbackOpts{html: &page})
	if err != nil {
		return "", false
	}
	v.refreshIdentity(docs)
	if strings.Contains(v.page, "appDocumentLink") {
		return v.page, true
	}
	if act := findSearchAction(v.page); act != nil {
		page := v.page
		out, err := v.callback(act.node, act.name, callbackOpts{container: act.container, html: &page})
		if err != nil {
			return "", false
		}
		return out, true
	}
	return v.page, true
}

func (s *Sedar) Newest(limit int) ([]Filing, error) {
	s.mu.Lock()
	v, err := s.openView("searchDocuments")
	s.mu.Unlock()
	if err != nil {
		return nil, err
	}
	out := ParseFilings(v.page)
	if limit < 1 {
		limit = 1
	}
	if len(out) > limit {
		out = out[:limit]
	}
	return out, nil
}

func isDocument(resp *browserhttp.Response) bool {
	ct := ""
	for k, v := range resp.Header {
		if strings.EqualFold(k, "content-type") && len(v) > 0 {
			ct = strings.ToLower(v[0])
		}
	}
	if resp.Status != 200 || len(resp.Body) == 0 {
		return false
	}
	if strings.Contains(ct, "text/html") {
		return false
	}
	trimmed := strings.TrimLeft(string(resp.Body), " \t\r\n\x0b\x0c")
	return !strings.HasPrefix(trimmed, "<")
}

func contentTypeOf(resp *browserhttp.Response, def string) string {
	for k, v := range resp.Header {
		if strings.EqualFold(k, "content-type") && len(v) > 0 {
			return v[0]
		}
	}
	return def
}

func (s *Sedar) DownloadBytes(profileNo, docID, name string) ([]byte, string, error) {
	parts := strings.Split(docID, ":")
	key := parts[len(parts)-1]
	s.mu.Lock()
	doc, ok := s.scopedDocuments(profileNo, name)
	if !ok {
		s.mu.Unlock()
		return nil, "", Unavailable("could not open the profile's documents to download from")
	}
	var row *Filing
	for _, f := range ParseFilings(doc) {
		if key != "" && strings.Contains(f.URL, key) {
			ff := f
			row = &ff
			break
		}
	}
	if row == nil {
		s.mu.Unlock()
		return nil, "", notFound("no document %q in profile %s", docID, profileNo)
	}
	sess, err := s.getSession()
	if err != nil {
		s.mu.Unlock()
		return nil, "", err
	}
	resp, err := sess.Get(html.UnescapeString(row.URL), map[string]string{"Referer": SedarBase + "/csa-party/viewInstance/view.html"})
	s.mu.Unlock()
	if err != nil {
		return nil, "", Unavailable("document fetch failed: %s", err)
	}
	if !isDocument(resp) {
		s.scopeMu.Lock()
		delete(s.scope, profileNo)
		s.scopeMu.Unlock()
		return nil, "", Unavailable("document did not download (status %d)", resp.Status)
	}
	return resp.Body, contentTypeOf(resp, "application/pdf"), nil
}

var sedarCAExchanges = map[string]bool{"TSX": true, "TSXV": true, "TSX-V": true, "CSE": true, "CNSX": true, "NEO": true, "NEO EXCHANGE": true, "CBOE CANADA": true, "AQL": true, "TSX VENTURE": true, "CANADIAN SECURITIES EXCHANGE": true}

func (s *Sedar) Covers(symbol, exchange, currency string) bool {
	ex := strings.ToUpper(exchange)
	cur := strings.ToUpper(currency)
	if cur == "USD" || ex == "NASDAQ" || ex == "NYSE" || ex == "AMEX" || ex == "ARCA" || ex == "US" {
		return false
	}
	return cur == "CAD" || sedarCAExchanges[ex] || (ex == "" && cur == "")
}

func containsAny(s string, needles ...string) bool {
	for _, n := range needles {
		if strings.Contains(s, n) {
			return true
		}
	}
	return false
}

func SedarCategory(file string) string {
	f := strings.ToLower(file)
	switch {
	case strings.Contains(f, "news release") || strings.Contains(f, "press release"):
		return News
	case containsAny(f, "md&a", "financial statement", "annual report", "interim", "certification", "52-109", "financial report"):
		return Financials
	case strings.Contains(f, "material change"):
		return Events
	case containsAny(f, "circular", "proxy", "voting results", "meeting", "information circular"):
		return Governance
	case containsAny(f, "prospectus", "offering", "45-106", "exempt distribution", "45-102", "rights offering", "45-108"):
		return Offerings
	case containsAny(f, "insider", "early warning", "45-101", "issuer bid"):
		return Insider
	}
	return Other
}

func (s *Sedar) Categorize(row Row) string { return SedarCategory(row.Type) }

func SplitTypeTitle(file string) (string, string) {
	name := strings.TrimSpace(pdfExtRE.ReplaceAllString(file, ""))
	for _, rx := range []*regexp.Regexp{langTailRE, langParenRE} {
		if loc := rx.FindStringSubmatchIndex(name); loc != nil {
			lang := name[loc[2]:loc[3]]
			return strings.Trim(name[:loc[0]], " -–"), "(" + py.Title(lang) + ")"
		}
	}
	return name, ""
}

func toItem(raw Filing, profileNo string) Item {
	typ, title := SplitTypeTitle(raw.File)
	pn := raw.ProfileNo
	if pn == "" {
		pn = profileNo
	}
	return Item{ID: "sedar:" + raw.ID, Source: SedarSource, Category: SedarCategory(raw.File), Date: raw.SubmittedAt, DateText: raw.Submitted, Type: typ, Title: title, Size: raw.Size, URL: raw.URL, Issuer: raw.Issuer, ProfileNo: pn}
}

func (s *Sedar) Fetch(symbol, name, exchange, currency string, limit int, profileNo string) ([]Item, error) {
	query := name
	if query == "" {
		query = symbol
	}
	if limit <= 0 {
		limit = SedarSearchLimit
	}
	result, err := s.ListFilings(query, profileNo, limit)
	if err != nil {
		if IsProfileNotFound(err) {
			return []Item{}, nil
		}
		return nil, err
	}
	pn := ""
	if result.Profile != nil {
		pn = result.Profile.ProfileNo
	}
	out := []Item{}
	for _, r := range result.Filings {
		out = append(out, toItem(r, pn))
	}
	return out, nil
}

func (s *Sedar) HasFiler(symbol, name, exchange, currency string) (bool, bool) { return false, false }

func (s *Sedar) Enrichment(row Row) *Enrichment { return nil }

func (s *Sedar) Content(row Row) ([]byte, string, error) { return s.Document(row) }

func (s *Sedar) Document(row Row) ([]byte, string, error) {
	return s.DownloadBytes(row.ProfileNo, row.ID, row.Issuer)
}
