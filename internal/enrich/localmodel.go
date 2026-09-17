package enrich

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"time"
)

const (
	DefaultLlamafileURL    = "https://huggingface.co/Mozilla/Llama-3.2-1B-Instruct-llamafile/resolve/main/Llama-3.2-1B-Instruct-Q4_K_M.llamafile"
	DefaultLlamafileSHA256 = "ac1c2864000bad7f62ee56ee908d3f55dd051a267d015b15fa6e831e69767b55"
	ManagedHost            = "127.0.0.1"
	DownloadTimeout        = 60 * 30
	StartTimeout           = 120
)

var allowedHosts = []string{"huggingface.co", "cdn-lfs.huggingface.co", "cdn-lfs-us-1.huggingface.co"}

func envOr(key, def string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return def
}

type LocalModel struct {
	Home     string
	mu       sync.Mutex
	phase    string
	detail   string
	proc     *exec.Cmd
	endpoint string
	model    string
	http     *http.Client
}

func NewLocalModel(home string) *LocalModel {
	return &LocalModel{Home: home, phase: "off", http: &http.Client{Timeout: 3 * time.Second}}
}

func (l *LocalModel) userURL() string { return strings.TrimRight(os.Getenv("BAGHOLDER_LLM_URL"), "/") }
func (l *LocalModel) ollamaURL() string {
	return strings.TrimRight(envOr("BAGHOLDER_OLLAMA_URL", "http://127.0.0.1:11434"), "/")
}
func (l *LocalModel) ollamaModel() string {
	return envOr("BAGHOLDER_OLLAMA_MODEL", "llama3.2")
}
func (l *LocalModel) managedPort() int {
	n, err := strconv.Atoi(envOr("BAGHOLDER_LLM_PORT", "8121"))
	if err != nil {
		return 8121
	}
	return n
}
func (l *LocalModel) chatTimeout() time.Duration {
	f, err := strconv.ParseFloat(envOr("BAGHOLDER_LLM_CHAT_TIMEOUT", "40"), 64)
	if err != nil {
		f = 40
	}
	return time.Duration(f * float64(time.Second))
}
func (l *LocalModel) llamafileURL() string {
	return envOr("BAGHOLDER_LLAMAFILE_URL", DefaultLlamafileURL)
}
func (l *LocalModel) llamafileSHA() string {
	return envOr("BAGHOLDER_LLAMAFILE_SHA256", DefaultLlamafileSHA256)
}

func (l *LocalModel) modelsDir() string {
	d := filepath.Join(l.Home, "models")
	os.MkdirAll(d, 0o755)
	return d
}

func (l *LocalModel) llamafilePath() string {
	return filepath.Join(l.modelsDir(), "summarizer.llamafile")
}

func (l *LocalModel) getOK(rawURL string, timeout time.Duration) bool {
	client := &http.Client{Timeout: timeout}
	req, err := http.NewRequest(http.MethodGet, rawURL, nil)
	if err != nil {
		return false
	}
	req.Header.Set("User-Agent", "Bagholder")
	resp, err := client.Do(req)
	if err != nil {
		return false
	}
	defer resp.Body.Close()
	io.Copy(io.Discard, resp.Body)
	return resp.StatusCode < 400
}

func (l *LocalModel) detectRunning() (string, string, bool) {
	if u := l.userURL(); u != "" && l.getOK(u+"/v1/models", 2*time.Second) {
		return u, envOr("BAGHOLDER_LLM_MODEL", "local"), true
	}
	if l.getOK(l.ollamaURL()+"/api/tags", 2*time.Second) {
		return l.ollamaURL(), l.ollamaModel(), true
	}
	return "", "", false
}

func (l *LocalModel) Status() string {
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.endpoint != "" {
		return "ready"
	}
	return l.phase
}

func (l *LocalModel) Available() bool { return l.Endpoint() != "" }

func (l *LocalModel) WaitReady(seconds float64) bool {
	l.Endpoint()
	deadline := time.Now().Add(time.Duration(seconds * float64(time.Second)))
	for time.Now().Before(deadline) {
		if l.Available() {
			return true
		}
		if st := l.Status(); st != "detecting" && st != "starting" {
			return false
		}
		time.Sleep(500 * time.Millisecond)
	}
	return l.Available()
}

func (l *LocalModel) Endpoint() string {
	l.mu.Lock()
	if l.endpoint != "" {
		e := l.endpoint
		l.mu.Unlock()
		return e
	}
	l.mu.Unlock()
	if base, model, ok := l.detectRunning(); ok {
		l.mu.Lock()
		l.endpoint, l.model, l.phase = base, model, "ready"
		l.mu.Unlock()
		return base
	}
	l.Ensure()
	return ""
}

func (l *LocalModel) Ensure() {
	l.mu.Lock()
	if l.phase == "detecting" || l.phase == "downloading" || l.phase == "starting" || l.endpoint != "" {
		l.mu.Unlock()
		return
	}
	l.phase = "detecting"
	l.mu.Unlock()
	go l.provision()
}

func (l *LocalModel) set(phase, detail string) {
	l.mu.Lock()
	l.phase, l.detail = phase, detail
	l.mu.Unlock()
}

func (l *LocalModel) provision() {
	defer func() {
		if e := recover(); e != nil {
			l.set("failed", fmt.Sprint(e))
		}
	}()
	if base, model, ok := l.detectRunning(); ok {
		l.mu.Lock()
		l.endpoint, l.model, l.phase = base, model, "ready"
		l.mu.Unlock()
		return
	}
	path := l.llamafilePath()
	if !l.verified(path) {
		l.set("downloading", "")
		if !l.download(path) {
			l.set("failed", "download failed")
			return
		}
	}
	if !l.verified(path) {
		l.set("failed", "checksum mismatch")
		os.Remove(path)
		return
	}
	l.set("starting", "")
	if l.spawn(path) && l.waitReady() {
		l.mu.Lock()
		l.endpoint = fmt.Sprintf("http://%s:%d", ManagedHost, l.managedPort())
		l.model = "local"
		l.phase = "ready"
		l.mu.Unlock()
	} else {
		l.set("failed", "server did not start")
	}
}

func (l *LocalModel) verified(path string) bool {
	want := strings.ToLower(l.llamafileSHA())
	if want == "" {
		return false
	}
	f, err := os.Open(path)
	if err != nil {
		return false
	}
	defer f.Close()
	h := sha256.New()
	if _, err := io.Copy(h, f); err != nil {
		return false
	}
	return hex.EncodeToString(h.Sum(nil)) == want
}

func (l *LocalModel) download(path string) bool {
	u, err := url.Parse(l.llamafileURL())
	if err != nil {
		return false
	}
	allowed := false
	for _, h := range allowedHosts {
		if u.Hostname() == h {
			allowed = true
		}
	}
	if !allowed {
		return false
	}
	tmp := strings.TrimSuffix(path, filepath.Ext(path)) + ".part"
	client := &http.Client{Timeout: DownloadTimeout * time.Second}
	req, err := http.NewRequest(http.MethodGet, l.llamafileURL(), nil)
	if err != nil {
		return false
	}
	req.Header.Set("User-Agent", "Bagholder")
	resp, err := client.Do(req)
	if err != nil {
		return false
	}
	defer resp.Body.Close()
	out, err := os.Create(tmp)
	if err != nil {
		return false
	}
	if _, err := io.Copy(out, resp.Body); err != nil {
		out.Close()
		os.Remove(tmp)
		return false
	}
	out.Close()
	if err := os.Rename(tmp, path); err != nil {
		os.Remove(tmp)
		return false
	}
	if info, err := os.Stat(path); err == nil {
		os.Chmod(path, info.Mode()|0o110)
	}
	if runtime.GOOS == "darwin" {
		exec.Command("xattr", "-d", "com.apple.quarantine", path).Run()
	}
	return true
}

func (l *LocalModel) spawn(path string) bool {
	port := strconv.Itoa(l.managedPort())
	cmd := exec.Command(path, "--server", "--nobrowser", "--host", ManagedHost, "--port", port, "-ngl", "0", "--log-disable")
	if err := cmd.Start(); err != nil {
		cmd = exec.Command("sh", path, "--server", "--nobrowser", "--host", ManagedHost, "--port", port, "--log-disable")
		if err := cmd.Start(); err != nil {
			return false
		}
	}
	l.mu.Lock()
	l.proc = cmd
	l.mu.Unlock()
	go cmd.Wait()
	return true
}

func (l *LocalModel) waitReady() bool {
	base := fmt.Sprintf("http://%s:%d", ManagedHost, l.managedPort())
	deadline := time.Now().Add(StartTimeout * time.Second)
	for time.Now().Before(deadline) {
		l.mu.Lock()
		proc := l.proc
		l.mu.Unlock()
		if proc != nil && proc.ProcessState != nil && proc.ProcessState.Exited() {
			return false
		}
		if l.getOK(base+"/health", 2*time.Second) || l.getOK(base+"/v1/models", 2*time.Second) {
			return true
		}
		time.Sleep(2 * time.Second)
	}
	return false
}

func (l *LocalModel) Shutdown() {
	l.mu.Lock()
	proc := l.proc
	l.proc = nil
	l.mu.Unlock()
	if proc != nil && proc.Process != nil && (proc.ProcessState == nil || !proc.ProcessState.Exited()) {
		proc.Process.Signal(os.Interrupt)
		done := make(chan struct{})
		go func() { proc.Wait(); close(done) }()
		select {
		case <-done:
		case <-time.After(5 * time.Second):
			proc.Process.Kill()
		}
	}
}

func (l *LocalModel) Chat(prompt string, maxTokens int) string {
	base := l.Endpoint()
	if base == "" {
		return ""
	}
	l.mu.Lock()
	model := l.model
	l.mu.Unlock()
	if model == "" {
		model = "local"
	}
	body, _ := json.Marshal(map[string]any{"model": model, "messages": []map[string]string{{"role": "user", "content": prompt}}, "temperature": 0.1, "max_tokens": maxTokens, "stream": false})
	client := &http.Client{Timeout: l.chatTimeout()}
	resp, err := client.Post(base+"/v1/chat/completions", "application/json", strings.NewReader(string(body)))
	if err != nil {
		return ""
	}
	defer resp.Body.Close()
	var parsed struct {
		Choices []struct {
			Message struct {
				Content string `json:"content"`
			} `json:"message"`
		} `json:"choices"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&parsed); err != nil || len(parsed.Choices) == 0 {
		return ""
	}
	return strings.TrimSpace(parsed.Choices[0].Message.Content)
}
