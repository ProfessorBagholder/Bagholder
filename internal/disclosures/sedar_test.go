package disclosures

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"sort"
	"strings"
	"sync"
	"testing"
)

func fixture(t *testing.T, name string) string {
	t.Helper()
	data, err := os.ReadFile(filepath.Join("..", "..", "tests", "fixtures", name))
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}

var nineDigits = regexp.MustCompile(`^\d{9}$`)
var fourDigits = regexp.MustCompile(`\d{4}`)

func TestParseFilingsEveryRowHasIssuerProfileFileDateAndADocumentURL(t *testing.T) {
	rows := ParseFilings(fixture(t, "search_documents.html"))
	if len(rows) == 0 {
		t.Fatal("the fixture has result rows")
	}
	for _, r := range rows {
		if !nineDigits.MatchString(r.ProfileNo) {
			t.Errorf("a nine-digit profile number: %q", r.ProfileNo)
		}
		if r.Issuer == "" {
			t.Error("an issuer name")
		}
		if r.File == "" {
			t.Error("a document file name")
		}
		if !fourDigits.MatchString(r.Submitted) {
			t.Errorf("a submitted date carrying a year: %q", r.Submitted)
		}
		if !strings.HasPrefix(r.URL, "https://www.sedarplus.ca/") {
			t.Errorf("a same-site document url: %q", r.URL)
		}
		if !strings.Contains(r.URL, "resource.html") {
			t.Errorf("the url is a document resource link: %q", r.URL)
		}
	}
}

func TestParseFilingsTheFirstRowIsReadExactly(t *testing.T) {
	rows := ParseFilings(fixture(t, "search_documents.html"))
	if len(rows) == 0 {
		t.Fatal("no rows")
	}
	first := rows[0]
	if first.ProfileNo != "000026091" {
		t.Errorf("profileNo = %q", first.ProfileNo)
	}
	if first.Issuer != "Franco-Nevada Corporation (000026091)" {
		t.Errorf("issuer = %q", first.Issuer)
	}
	if first.File != "News release - English.pdf" {
		t.Errorf("file = %q", first.File)
	}
	if !strings.HasPrefix(first.Submitted, "13 Sep 2026") {
		t.Errorf("submitted = %q", first.Submitted)
	}
	if !strings.Contains(first.URL, "drmKey=") {
		t.Errorf("url = %q", first.URL)
	}
}

func TestParseFilingsRowsKeepThePageOrder(t *testing.T) {
	rows := ParseFilings(fixture(t, "search_documents.html"))
	dates := make([]string, 0, len(rows))
	for _, r := range rows {
		dates = append(dates, r.Submitted)
	}
	sorted := slices.Clone(dates)
	sort.Sort(sort.Reverse(sort.StringSlice(sorted)))
	if !slices.Equal(dates, sorted) {
		t.Errorf("the page lists newest first and we keep it: %v", dates)
	}
}

func TestParseReportingIssuersEachRowMapsANameToAProfileNumber(t *testing.T) {
	rows := ParseReportingIssuers(fixture(t, "reporting_issuers.html"))
	if len(rows) == 0 {
		t.Fatal("no rows")
	}
	for _, r := range rows {
		if !nineDigits.MatchString(r.ProfileNo) {
			t.Errorf("profileNo = %q", r.ProfileNo)
		}
		if r.Name == "" {
			t.Errorf("empty name for %q", r.ProfileNo)
		}
	}
}

func TestParseReportingIssuersAKnownIssuerIsReadWithItsFields(t *testing.T) {
	rows := ParseReportingIssuers(fixture(t, "reporting_issuers.html"))
	byNo := map[string]Profile{}
	for _, r := range rows {
		byNo[r.ProfileNo] = r
	}
	r, ok := byNo["000010658"]
	if !ok {
		t.Fatal("000010658 not found")
	}
	if !strings.Contains(r.Name, "01 Quantum") {
		t.Errorf("name = %q", r.Name)
	}
	if !strings.Contains(r.Provinces, "ON") {
		t.Errorf("provinces = %q", r.Provinces)
	}
	if r.Type != "Company" {
		t.Errorf("the type column is read, not an eligibility flag: %q", r.Type)
	}
}

func TestParsersReturnEmptyOnABlankOrErroredPage(t *testing.T) {
	if got := ParseFilings(""); len(got) != 0 {
		t.Errorf("ParseFilings(\"\") = %v", got)
	}
	if got := ParseReportingIssuers(""); len(got) != 0 {
		t.Errorf("ParseReportingIssuers(\"\") = %v", got)
	}
	if got := ParseFilings("<div>There has been an unexpected system error.</div>"); len(got) != 0 {
		t.Errorf("ParseFilings(error page) = %v", got)
	}
}

func TestFormFieldsDropsCallbackAndButtonInputs(t *testing.T) {
	html := `<form><input name="Keep" value="v"/>` +
		`<input type="submit" name="Drop"/>` +
		`<input type="hidden" name="_CBNODE_" value="x"/>` +
		`<select name="Pick"><option value="a">A</option><option value="b" selected>B</option></select>` +
		`<input type="checkbox" name="Off"/><input type="checkbox" name="On" value="y" checked/></form>`
	got := map[string]string{}
	for _, f := range formFields(html) {
		got[f.k] = f.v
	}
	if got["Keep"] != "v" {
		t.Errorf("Keep = %q", got["Keep"])
	}
	if got["Pick"] != "b" {
		t.Errorf("the selected option is taken: %q", got["Pick"])
	}
	if got["On"] != "y" {
		t.Errorf("a checked box is kept: %q", got["On"])
	}
	if _, ok := got["Drop"]; ok {
		t.Error("Drop was kept")
	}
	if _, ok := got["Off"]; ok {
		t.Error("an unchecked box is dropped")
	}
	if _, ok := got["_CBNODE_"]; ok {
		t.Error("callback fields are set by the caller, not carried")
	}
}

func TestSearchActionIsReadFromEachService(t *testing.T) {
	_ = findSearchAction(fixture(t, "search_documents.html"))
	got := findSearchAction(`<button class="appSearchButton" onclick="x catHtmlFragmentCallback('W766','buttonPush',null,{containerNodeId:'W706'})">`)
	if got == nil || *got != (searchAction{"W766", "buttonPush", "W706"}) {
		t.Errorf("appSearchButton action = %v", got)
	}
	got = findSearchAction(`<button id="nodeW557-searchButton" onclick="x catHtmlFragmentCallback('W557','fireOnChange',null,{containerNodeId:'W553'})">`)
	if got == nil || *got != (searchAction{"W557", "fireOnChange", "W553"}) {
		t.Errorf("-searchButton action = %v", got)
	}
}

func TestTheIssuerMenuNodeIsTheIssuerNotAHeaderAction(t *testing.T) {
	html := fixture(t, "issuer_menu.html")
	if got := issuerMenuNode(html, "Shopify Inc. / Shopify Inc."); got != "W1118" {
		t.Errorf("issuerMenuNode(named) = %q", got)
	}
	if got := issuerMenuNode(html, ""); got != "W1118" {
		t.Errorf("issuerMenuNode(unnamed) = %q", got)
	}
}

func TestTheDocumentsMenuNodeIsFoundOnTheProfile(t *testing.T) {
	if got := docsMenuNode(fixture(t, "issuer_profile.html")); got != "W733" {
		t.Errorf("docsMenuNode(profile) = %q", got)
	}
	if got := docsMenuNode("<div>no menu here</div>"); got != "" {
		t.Errorf("docsMenuNode(no menu) = %q", got)
	}
}

func TestRefreshIdentityAdoptsANewViewInstance(t *testing.T) {
	v := &view{app: "csa-party", inst: "oldid", key: "oldkey"}
	if !v.refreshIdentity(fixture(t, "issuer_profile.html")) {
		t.Fatal("refreshIdentity returned false on the profile page")
	}
	if v.inst == "oldid" {
		t.Error("the id is taken from the navigated page")
	}
	if v.key == "oldkey" {
		t.Error("the key is taken from the navigated page")
	}
	if v.refreshIdentity("<div>a fragment with no instance</div>") {
		t.Error("refreshIdentity returned true on a fragment with no instance")
	}
}

func stubSedarProxy(t *testing.T) *int {
	t.Helper()
	var mu sync.Mutex
	connects := 0
	proxy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == http.MethodConnect {
			mu.Lock()
			connects++
			mu.Unlock()
		}
		w.WriteHeader(http.StatusBadGateway)
	}))
	t.Cleanup(proxy.Close)
	t.Setenv("HTTPS_PROXY", proxy.URL)
	t.Setenv("HTTP_PROXY", proxy.URL)
	return &connects
}

func TestScopeCacheASecondCallUsesTheCacheAndDoesNotRewalk(t *testing.T) {
	t.Skip("needs a live SEDAR+ session: the issuer walk goes through the browser TLS client, which cannot be answered by a stub, so a walk that succeeds once cannot be staged")
}

func TestScopeCacheAFailedWalkIsNotCached(t *testing.T) {
	connects := stubSedarProxy(t)
	s := NewSedar()
	if _, ok := s.scopedDocuments("000099999", ""); ok {
		t.Fatal("a failed walk reported a document page")
	}
	if _, ok := s.scopedDocuments("000099999", ""); ok {
		t.Fatal("a failed walk reported a document page on the second call")
	}
	if *connects != 2 {
		t.Errorf("a None result is retried, never cached: %d walks", *connects)
	}
	if _, cached := s.scope["000099999"]; cached {
		t.Error("a None result is retried, never cached: the failure was cached")
	}
}
