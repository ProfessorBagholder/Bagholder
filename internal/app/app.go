package app

import (
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/disclosures"
	"github.com/ProfessorBagholder/Bagholder/internal/enrich"
	"github.com/ProfessorBagholder/Bagholder/internal/exposure"
	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/model"
	"github.com/ProfessorBagholder/Bagholder/internal/notify"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/shorts"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
	"github.com/ProfessorBagholder/Bagholder/internal/ws"
)

const (
	AppVersion        = "1.43.2"
	Repo              = "ProfessorBagholder/Bagholder"
	RepoURL           = "https://github.com/" + Repo
	ReleaseURL        = "https://api.github.com/repos/" + Repo + "/releases/latest"
	RestartCode       = 3
	UpdateHealthySec  = 20
	UpdateMaxBytes    = 50 * 1024 * 1024
	UpdateCheckHours  = 1
	UpdatesOffMessage = "This copy is updated with docker compose pull; a new release is a new image."
	ImagePage         = RepoURL + "/pkgs/container/bagholder"
	Protocol          = "2026-09-16.1"
	EnrichVersion     = 11
	LoginURL          = "https://my.wealthsimple.com/app/login"

	TokenCheckSec         = 30
	TokenRefreshMarginSec = 300
	ActivityPullSec       = 24 * 60 * 60
	CaptureWaitSec        = 180
	OAuthCookie           = "_oauth2_access_v2"
	DeviceCookie          = "wssdi"

	PortfolioRefreshMinutes = 5
	ExposureCheckSec        = 30 * 60
	ExposureFirstSec        = 20
	ExposureWorkers         = 4
	SyncFailsTold           = 3
)

var (
	Ports          = []int{8765, 8766, 8767}
	DebugPorts     = []int{18765, 18766, 18767}
	LoginViewSize  = [2]int{960, 1000}
	StartedAt      = py.NowStamp()
	BrowserMissing = "Install Chrome, Brave, Edge, or another Chromium browser. Passkey login has to happen on Wealthsimple’s site."
)

type Config struct {
	Home       string
	AppDir     string
	Exe        string
	BindHost   string
	UpdatesOff bool
	LoginView  bool
	OrdersLive bool
	NoBrowser  bool
	Ports      []int
	Static     fs.FS
	Stderr     *os.File
}

func ConfigFromEnv() Config {
	cfg := Config{Home: store.Home(), OrdersLive: strings.TrimSpace(os.Getenv("BAGHOLDER_DRY_ORDERS")) != "1"}
	cfg.BindHost = strings.TrimSpace(os.Getenv("BAGHOLDER_BIND"))
	if cfg.BindHost == "" {
		cfg.BindHost = "127.0.0.1"
	}
	cfg.UpdatesOff = strings.TrimSpace(os.Getenv("BAGHOLDER_NO_UPDATE")) != ""
	cfg.LoginView = strings.TrimSpace(os.Getenv("BAGHOLDER_LOGIN_VIEW")) != ""
	cfg.NoBrowser = strings.TrimSpace(os.Getenv("BAGHOLDER_NO_BROWSER")) != ""
	if exe, err := os.Executable(); err == nil {
		if real, err := filepath.EvalSymlinks(exe); err == nil {
			exe = real
		}
		cfg.Exe = exe
		cfg.AppDir = filepath.Dir(exe)
	} else {
		cfg.AppDir, _ = os.Getwd()
	}
	cfg.Ports = portChoices()
	return cfg
}

func portChoices() []int {
	env := strings.TrimSpace(os.Getenv("BAGHOLDER_PORT"))
	if env != "" {
		n := 0
		digits := true
		for _, r := range env {
			if r < '0' || r > '9' {
				digits = false
				break
			}
			n = n*10 + int(r-'0')
			if n > 65535 {
				break
			}
		}
		if digits && n >= 1024 && n <= 65535 {
			return []int{n}
		}
	}
	return append([]int(nil), Ports...)
}

type state struct {
	connected       bool
	capturing       bool
	syncing         bool
	syncStep        string
	email           string
	lastSync        string
	err             string
	chrome          *browserProc
	loginAttempt    int
	updating        string
	updateError     string
	listingsFilling bool
	syncFails       int
	syncFirstFail   string
}

type job struct {
	running bool
	until   time.Time
}

type App struct {
	cfg      Config
	st       *store.Store
	model    *model.Model
	mk       *market.Client
	ws       *ws.Client
	files    *ws.Files
	notify   *notify.Notifier
	expo     *exposure.Client
	shorts   *shorts.Client
	pipeline *disclosures.Pipeline
	sedar    *disclosures.Sedar
	enricher *enrich.Enricher

	mu    sync.Mutex
	state state

	stopOnce sync.Once
	stopCh   chan struct{}
	exitCode int

	jobsMu sync.Mutex
	jobs   map[string]*job

	searchMu    sync.Mutex
	searchCache map[string][]SearchRow

	ordersRefreshedAt string
	ordersRefreshing  sync.Mutex
	ordersMu          sync.Mutex

	bracketMu     sync.Mutex
	bracketBusy   bool
	bracketSaidMu sync.Mutex
	bracketSaid   map[string]bool
	stopAllowedMu sync.Mutex
	stopAllowed   map[string]bool

	shortsLeft   int
	shortsMu     sync.Mutex
	universeKick chan struct{}

	viewMu      sync.Mutex
	view        *miniWS
	viewTarget  string
	cast        screencast
	reachedOnce sync.Once

	serverMu sync.Mutex
	server   *serverHandle
}

var cooldown = map[string]float64{"quotes": 60.0, "market": 300.0}

func New(cfg Config) (*App, error) {
	if cfg.Home == "" {
		cfg.Home = store.Home()
	}
	if err := ensureHome(cfg.Home); err != nil {
		return nil, err
	}
	st, err := store.Open(cfg.Home)
	if err != nil {
		return nil, err
	}
	if err := st.Ensure(); err != nil {
		return nil, err
	}
	a := &App{cfg: cfg, st: st, stopCh: make(chan struct{}), jobs: map[string]*job{}, searchCache: map[string][]SearchRow{}, bracketSaid: map[string]bool{}, stopAllowed: map[string]bool{}, universeKick: make(chan struct{}, 1)}
	a.model = model.New(st)
	a.mk = market.NewClient(st)
	a.ws = ws.NewClient(cfg.Home)
	a.files = a.ws.Files
	a.ws.OnError = func(msg string) {
		a.mu.Lock()
		a.state.err = msg
		a.mu.Unlock()
	}
	a.notify = notify.New(st)
	a.expo = exposure.NewClient(a.mk)
	a.expo.SymbolSearch = func(text string) []exposure.SearchMatch {
		r := a.symbolSearch(text)
		out := make([]exposure.SearchMatch, 0, len(r.Matches))
		for _, m := range r.Matches {
			out = append(out, exposure.SearchMatch{Symbol: m.Symbol, Exchange: m.Exchange, Currency: m.Currency})
		}
		return out
	}
	a.shorts = shorts.NewClient(a.mk)
	a.sedar = disclosures.NewSedar()
	a.pipeline = &disclosures.Pipeline{Providers: []disclosures.Provider{a.sedar, disclosures.NewEdgar(a.mk)}, Sedar: a.sedar}
	a.enricher = &enrich.Enricher{Model: enrich.NewLocalModel(cfg.Home)}
	a.cast.cond = sync.NewCond(&a.cast.mu)
	return a, nil
}

func (a *App) Store() *store.Store        { return a.st }
func (a *App) Model() *model.Model        { return a.model }
func (a *App) Market() *market.Client     { return a.mk }
func (a *App) WS() *ws.Client             { return a.ws }
func (a *App) Notifier() *notify.Notifier { return a.notify }
func (a *App) Pipeline() *disclosures.Pipeline {
	return a.pipeline
}
func (a *App) Config() Config { return a.cfg }

func ensureHome(home string) error {
	if err := os.MkdirAll(home, 0o700); err != nil {
		return err
	}
	_ = os.Chmod(home, 0o700)
	return nil
}

func (a *App) logf(format string, args ...any) {
	fmt.Fprintf(os.Stderr, format, args...)
}

func errText(err error) string {
	if err == nil {
		return ""
	}
	return err.Error()
}

func (a *App) stopped() bool {
	select {
	case <-a.stopCh:
		return true
	default:
		return false
	}
}

func (a *App) wait(d time.Duration) bool {
	if d <= 0 {
		return a.stopped()
	}
	t := time.NewTimer(d)
	defer t.Stop()
	select {
	case <-a.stopCh:
		return true
	case <-t.C:
		return false
	}
}

func (a *App) setStop() {
	a.stopOnce.Do(func() { close(a.stopCh) })
}

func (a *App) job(name string) *job {
	j, ok := a.jobs[name]
	if !ok {
		j = &job{}
		a.jobs[name] = j
	}
	return j
}

func (a *App) singleFlightStart(name string) bool {
	a.jobsMu.Lock()
	defer a.jobsMu.Unlock()
	j := a.job(name)
	if j.running {
		return false
	}
	j.running = true
	return true
}

func (a *App) singleFlightEnd(name string) {
	a.jobsMu.Lock()
	defer a.jobsMu.Unlock()
	j := a.job(name)
	j.running = false
	j.until = time.Now().Add(time.Duration(cooldown[name] * float64(time.Second)))
}

func (a *App) kick(name string, fn func()) bool {
	a.jobsMu.Lock()
	j := a.job(name)
	if j.running || time.Now().Before(j.until) {
		a.jobsMu.Unlock()
		return false
	}
	a.jobsMu.Unlock()
	go fn()
	return true
}

func (a *App) loadSession() ws.Session {
	return a.files.LoadSession()
}

func (a *App) saveSession(sess ws.Session) {
	_ = a.files.SaveSession(sess)
}

func (a *App) deleteSessionAndBook() {
	a.ws.ResetRefused()
	a.files.DeleteSession()
	a.mu.Lock()
	a.state.connected = false
	a.state.email = ""
	a.state.lastSync = ""
	a.state.capturing = false
	a.state.err = ""
	a.mu.Unlock()
}

func (a *App) loadBook() store.Snapshot {
	_ = a.st.Ensure()
	return a.st.Snapshot(true)
}

type accountsSnapshot struct {
	accounts   []store.Account
	balances   []store.Balance
	margin     []store.Margin
	hasMargin  bool
	navHistory []store.NavPoint
	syncedAt   string
}

func (a *App) saveAccountsSnapshot(book accountsSnapshot) {
	a.st.ReplaceAccounts(book.accounts)
	a.st.ReplaceBalances(book.balances)
	if book.hasMargin {
		a.st.ReplaceMargin(book.margin)
	}
	a.st.UpsertNav(book.navHistory)
	if book.syncedAt != "" {
		a.st.SetMeta("synced_at", book.syncedAt)
	}
}

func (a *App) setSyncStep(msg string) {
	a.mu.Lock()
	a.state.syncStep = msg
	a.mu.Unlock()
}

func (a *App) setError(msg string) {
	a.mu.Lock()
	a.state.err = msg
	a.mu.Unlock()
}

func (a *App) connectedIdle() bool {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.state.connected && !a.state.syncing
}

func (a *App) invalidate(book bool) { a.model.Invalidate(book) }

func nowStamp() string { return py.NowStamp() }

func today() string { return time.Now().UTC().Format("2006-01-02") }

func which(names ...string) string {
	for _, n := range names {
		if p, err := exec.LookPath(n); err == nil && p != "" {
			return p
		}
	}
	return ""
}

func isFile(p string) bool {
	if p == "" {
		return false
	}
	st, err := os.Stat(p)
	return err == nil && st.Mode().IsRegular()
}

func isWindows() bool { return runtime.GOOS == "windows" }
