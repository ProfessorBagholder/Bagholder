package disclosures

import (
	"bytes"
	"encoding/json"
	"flag"
	"os"
	"path/filepath"
	"regexp"
	"testing"
)

var updateGolden = flag.Bool("update", false, "rewrite testdata/sedar_golden.json from the current parser")

var goldenFixtures = []string{"search_documents.html", "reporting_issuers.html", "issuer_menu.html", "issuer_profile.html"}

var goldenNames = []string{"", "Shopify Inc. / Shopify Inc.", "Franco-Nevada Corporation", "no such issuer"}

type goldenPage struct {
	Filings   []Filing          `json:"filings"`
	Profiles  []Profile         `json:"profiles"`
	MenuNodes map[string]string `json:"menuNodes"`
	DocsNode  string            `json:"docsNode"`
	Action    *searchAction     `json:"action"`
	Fields    [][2]string       `json:"fields"`
	Params    [][2]string       `json:"params"`
}

func goldenOf(doc string) goldenPage {
	g := goldenPage{Filings: ParseFilings(doc), Profiles: ParseReportingIssuers(doc), MenuNodes: map[string]string{}, DocsNode: docsMenuNode(doc), Action: findSearchAction(doc), Fields: [][2]string{}, Params: [][2]string{}}
	for _, n := range goldenNames {
		g.MenuNodes[n] = issuerMenuNode(doc, n)
	}
	for _, f := range formFields(doc) {
		g.Fields = append(g.Fields, [2]string{f.k, f.v})
	}
	for _, f := range viParams(doc) {
		g.Params = append(g.Params, [2]string{f.k, f.v})
	}
	return g
}

func TestSedarParsersMatchTheGoldenFile(t *testing.T) {
	got := map[string]goldenPage{}
	for _, name := range goldenFixtures {
		got[name] = goldenOf(fixture(t, name))
	}
	data, err := json.MarshalIndent(got, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	data = append(data, '\n')
	path := filepath.Join("testdata", "sedar_golden.json")
	if *updateGolden {
		if err := os.WriteFile(path, data, 0o644); err != nil {
			t.Fatal(err)
		}
	}
	want, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(data, want) {
		t.Errorf("the parsers no longer produce the golden output; diff %s against the current output:\n%s", path, data)
	}
}

func BenchmarkParseFilings(b *testing.B) {
	doc := benchFixture(b, "search_documents.html")
	b.ReportAllocs()
	for i := 0; i < b.N; i++ {
		ParseFilings(doc)
	}
}

func BenchmarkParseReportingIssuers(b *testing.B) {
	doc := benchFixture(b, "reporting_issuers.html")
	b.ReportAllocs()
	for i := 0; i < b.N; i++ {
		ParseReportingIssuers(doc)
	}
}

func BenchmarkIssuerMenuNode(b *testing.B) {
	doc := benchFixture(b, "search_documents.html")
	b.ReportAllocs()
	for i := 0; i < b.N; i++ {
		issuerMenuNode(doc, "no such issuer")
	}
}

func benchFixture(b *testing.B, name string) string {
	b.Helper()
	data, err := os.ReadFile(filepath.Join("..", "..", "tests", "fixtures", name))
	if err != nil {
		b.Fatal(err)
	}
	return string(data)
}

var sizeSweepRE = regexp.MustCompile(`(?i)(\d[\d.,]* ?(?:KB|MB|bytes))`)

func TestSizeInMatchesTheWindowSweepItReplaced(t *testing.T) {
	windows := []string{
		"", "abc", "1.2 MB", "<td>1.2 MB</td>", "x 12,345 bytes y", "5 mbytes", "5 kbytes", "1a2 KB", "12 x 3KB",
		"id=\"nodeW792\" kb", "792 kb", "12  KB", "12.KB", ".5 KB", "5 Kb", "5Kb", "9 Mb 10 KB", "1,000,000 bytes",
		"a1b2c3 KB 4 MB", "0 bytes", "12 bytesx", "12 KBB", "12 byte", "3\n KB", "3 \nKB", "７ KB", "3 kilobytes", "3 mb.",
	}
	for _, w := range windows {
		if got, want := sizeIn(w), sizeSweepRE.FindString(w); got != want {
			t.Errorf("sizeIn(%q) = %q, the sweep found %q", w, got, want)
		}
	}
}
