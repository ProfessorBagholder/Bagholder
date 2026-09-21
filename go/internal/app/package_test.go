package app

import (
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"testing"
)

const shipped = "ledger.html lightweight-charts.js favicon.png"

func repoRoot(t *testing.T) string {
	t.Helper()
	// the module lives in go/ and the page, the Dockerfile's context and the tracked file
	// list are the repository's, one level above it
	root, err := filepath.Abs(filepath.Join("..", "..", ".."))
	if err != nil {
		t.Fatal(err)
	}
	return root
}

func readRoot(t *testing.T, name string) string {
	t.Helper()
	raw, err := os.ReadFile(filepath.Join(repoRoot(t), name))
	if err != nil {
		t.Fatal(err)
	}
	return string(raw)
}

func copiedFiles(t *testing.T) []string {
	t.Helper()
	out := []string{}
	re := regexp.MustCompile(`^COPY\s+(.*?)\s+\./?\s*$`)
	for _, line := range strings.Split(readRoot(t, filepath.Join("go", "Dockerfile")), "\n") {
		line = strings.TrimSpace(line)
		if strings.HasPrefix(line, "COPY --from=") {
			continue
		}
		if m := re.FindStringSubmatch(line); m != nil {
			out = append(out, strings.Fields(m[1])...)
		}
	}
	return out
}

func dockerIgnored(t *testing.T) []string {
	t.Helper()
	out := []string{}
	for _, line := range strings.Split(readRoot(t, ".dockerignore"), "\n") {
		line = strings.TrimSpace(line)
		if line != "" && !strings.HasPrefix(line, "#") {
			out = append(out, line)
		}
	}
	return out
}

// the Go module's own root, go/ in the repository; the Dockerfile copies it in under that name
func moduleRoot(t *testing.T) string {
	t.Helper()
	root, err := filepath.Abs(filepath.Join("..", ".."))
	if err != nil {
		t.Fatal(err)
	}
	return root
}

func goFilesOf(t *testing.T) map[string][]string {
	t.Helper()
	cmd := exec.Command("go", "list", "-deps", "-f", "{{.ImportPath}}\t{{.Dir}}\t{{range .GoFiles}}{{.}} {{end}}", "./cmd/bagholder")
	cmd.Dir = moduleRoot(t)
	raw, err := cmd.CombinedOutput()
	if err != nil {
		t.Skipf("go list is needed to know what the app imports: %v\n%s", err, raw)
	}
	out := map[string][]string{}
	for _, line := range strings.Split(strings.TrimSpace(string(raw)), "\n") {
		parts := strings.SplitN(line, "\t", 3)
		if len(parts) != 3 || !strings.HasPrefix(parts[0], "github.com/ProfessorBagholder/Bagholder") {
			continue
		}
		rel, err := filepath.Rel(moduleRoot(t), parts[1])
		if err != nil || strings.HasPrefix(rel, "..") {
			continue
		}
		out[filepath.ToSlash(rel)] = strings.Fields(parts[2])
	}
	if len(out) == 0 {
		t.Fatal("the app imports at least its own packages")
	}
	return out
}

func trackedFiles(t *testing.T) map[string]bool {
	t.Helper()
	cmd := exec.Command("git", "-C", repoRoot(t), "ls-files")
	raw, err := cmd.Output()
	if err != nil {
		t.Skip("not a git checkout")
	}
	out := map[string]bool{}
	for _, name := range strings.Fields(string(raw)) {
		out[name] = true
	}
	return out
}

func TestTheImageCarriesEveryPackageTheAppImports(t *testing.T) {
	copied := copiedFiles(t)
	if len(copied) == 0 {
		t.Fatal("the Dockerfile copies the app in")
	}
	takesAll := false
	for _, p := range copied {
		if p == "." {
			takesAll = true
		}
	}
	named := map[string]bool{}
	for _, p := range copied {
		p = strings.TrimSuffix(p, "/")
		if p == "go" {
			takesAll = true // COPY go/ ./ is the whole module
		}
		named[p] = true
	}
	missing := []string{}
	for dir := range goFilesOf(t) {
		if takesAll || named[dir] {
			continue
		}
		missing = append(missing, dir)
	}
	sort.Strings(missing)
	if len(missing) != 0 {
		t.Errorf("packages the app imports but the image would not have: %v", missing)
	}
}

func TestTheImageCarriesThePageAndItsChartLibrary(t *testing.T) {
	// the page, the chart library and the icon are the repository's, shared by every build;
	// go generate brings them into the module so the binary carries them and needs nothing beside it
	embedded := readRoot(t, filepath.Join("go", "static.go"))
	ignored := dockerIgnored(t)
	for _, needed := range strings.Fields(shipped) {
		if _, err := os.Stat(filepath.Join(repoRoot(t), needed)); err != nil {
			t.Errorf("%s is served by the app: %v", needed, err)
		}
		if !strings.Contains(embedded, needed) {
			t.Errorf("%s is served by the app but the binary would not carry it", needed)
		}
		for _, pattern := range ignored {
			if pattern == needed {
				t.Errorf("%s is served by the app but kept out of the image", needed)
			}
		}
	}
}

func TestNothingTheImageNeedsIsKeptOutOfIt(t *testing.T) {
	ignored := map[string]bool{}
	for _, p := range dockerIgnored(t) {
		ignored[strings.Trim(p, "/")] = true
	}
	if ignored["*.go"] {
		t.Error("the image is built from the Go sources")
	}
	for _, needed := range []string{"go.mod", "go.sum"} {
		if ignored[needed] {
			t.Errorf("%s is needed to build the image", needed)
		}
	}
	for dir := range goFilesOf(t) {
		for part := dir; part != "."; part = filepath.Dir(part) {
			if ignored[filepath.ToSlash(part)] {
				t.Errorf("%s is imported by the app but kept out of the image", dir)
			}
		}
	}
}

func TestTheReleaseArchiveCarriesEveryFileTheAppImports(t *testing.T) {
	tracked := trackedFiles(t)
	missing := []string{}
	for dir, files := range goFilesOf(t) {
		for _, f := range files {
			name := dir + "/" + f
			if dir == "." {
				name = f
			}
			// git lists the repository's paths and the module sits in go/
			if !tracked["go/"+name] {
				missing = append(missing, name)
			}
		}
	}
	sort.Strings(missing)
	if len(missing) != 0 {
		t.Errorf("files the app imports that the release archive would not carry: %v", missing)
	}
	for _, needed := range strings.Fields(shipped) {
		if !tracked[needed] {
			t.Errorf("%s is not tracked", needed)
		}
	}
}

func TestTheAppBuildsFromWhatShips(t *testing.T) {
	if testing.Short() {
		t.Skip("building the app is slow")
	}
	out := filepath.Join(t.TempDir(), "bagholder")
	cmd := exec.Command("go", "build", "-o", out, "./cmd/bagholder")
	cmd.Dir = moduleRoot(t)
	if raw, err := cmd.CombinedOutput(); err != nil {
		text := string(raw)
		if len(text) > 600 {
			text = text[len(text)-600:]
		}
		t.Errorf("the app does not build from its own files: %v\n%s", err, text)
	}
}
