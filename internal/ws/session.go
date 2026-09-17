package ws

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"sync"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

type Session map[string]any

func (s Session) Str(key string) string {
	if s == nil {
		return ""
	}
	return py.S(s[key])
}

func (s Session) Has(key string) bool {
	if s == nil {
		return false
	}
	v, ok := s[key]
	if !ok || v == nil {
		return false
	}
	if str, ok := v.(string); ok {
		return str != ""
	}
	return true
}

func (s Session) Clone() Session {
	out := Session{}
	for k, v := range s {
		out[k] = v
	}
	return out
}

type Files struct {
	Home string
	mu   sync.Mutex
}

func (f *Files) SessionPath() string  { return filepath.Join(f.Home, "session.json") }
func (f *Files) ClientIDPath() string { return filepath.Join(f.Home, "client_id") }
func (f *Files) UAPath() string       { return filepath.Join(f.Home, "user_agent") }

func (f *Files) EnsureHome() {
	os.MkdirAll(f.Home, 0o700)
	os.Chmod(f.Home, 0o700)
}

func (f *Files) atomicWrite(path string, data []byte, mode os.FileMode) error {
	f.EnsureHome()
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, data, mode); err != nil {
		return err
	}
	os.Chmod(tmp, mode)
	if err := os.Rename(tmp, path); err != nil {
		return err
	}
	os.Chmod(path, mode)
	return nil
}

func (f *Files) LoadSession() Session {
	f.mu.Lock()
	defer f.mu.Unlock()
	raw, err := os.ReadFile(f.SessionPath())
	if err != nil {
		return nil
	}
	var sess Session
	if err := json.Unmarshal(raw, &sess); err != nil {
		return nil
	}
	return sess
}

func (f *Files) SaveSession(sess Session) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	data, err := json.MarshalIndent(sess, "", "  ")
	if err != nil {
		return err
	}
	return f.atomicWrite(f.SessionPath(), data, 0o600)
}

func (f *Files) DeleteSession() {
	f.mu.Lock()
	defer f.mu.Unlock()
	os.Remove(f.SessionPath())
}

func (f *Files) CachedClientID() string {
	raw, err := os.ReadFile(f.ClientIDPath())
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(raw))
}

func (f *Files) SaveClientID(cid string) {
	if cid == "" {
		return
	}
	f.EnsureHome()
	if err := os.WriteFile(f.ClientIDPath(), []byte(cid), 0o600); err == nil {
		os.Chmod(f.ClientIDPath(), 0o600)
	}
}

func (f *Files) CachedUserAgent() string {
	if v := strings.TrimSpace(f.LoadSession().Str("user_agent")); v != "" {
		return v
	}
	raw, err := os.ReadFile(f.UAPath())
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(raw))
}

func (f *Files) SaveUserAgent(ua string) {
	if ua == "" {
		return
	}
	f.EnsureHome()
	if err := os.WriteFile(f.UAPath(), []byte(ua), 0o600); err == nil {
		os.Chmod(f.UAPath(), 0o600)
	}
}
