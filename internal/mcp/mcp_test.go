package mcp

import (
	"bytes"
	"encoding/json"
	"net"
	"strings"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/disclosures"
)

type fakeProvider struct {
	calls []fetchCall
	items []disclosures.Item
}

type fetchCall struct {
	symbol, name, exchange, currency string
	limit                            int
	profileNo                        string
}

func (f *fakeProvider) Source() string                                { return "SEC" }
func (f *fakeProvider) Available() bool                               { return true }
func (f *fakeProvider) Covers(symbol, exchange, currency string) bool { return true }
func (f *fakeProvider) Fetch(symbol, name, exchange, currency string, limit int, profileNo string) ([]disclosures.Item, error) {
	f.calls = append(f.calls, fetchCall{symbol, name, exchange, currency, limit, profileNo})
	return f.items, nil
}
func (f *fakeProvider) Document(row disclosures.Row) ([]byte, string, error) { return nil, "", nil }
func (f *fakeProvider) HasFiler(symbol, name, exchange, currency string) (bool, bool) {
	return false, false
}
func (f *fakeProvider) Enrichment(row disclosures.Row) *disclosures.Enrichment { return nil }
func (f *fakeProvider) Categorize(row disclosures.Row) string                  { return "" }
func (f *fakeProvider) Content(row disclosures.Row) ([]byte, string, error)    { return nil, "", nil }

func serve(t *testing.T, s *Server, lines ...string) []map[string]any {
	t.Helper()
	var out bytes.Buffer
	s.Serve(strings.NewReader(strings.Join(lines, "\n")+"\n"), &out)
	var responses []map[string]any
	for _, line := range strings.Split(strings.TrimSpace(out.String()), "\n") {
		if line == "" {
			continue
		}
		var msg map[string]any
		if err := json.Unmarshal([]byte(line), &msg); err != nil {
			t.Fatalf("response is not JSON: %v: %q", err, line)
		}
		responses = append(responses, msg)
	}
	return responses
}

func one(t *testing.T, s *Server, line string) map[string]any {
	t.Helper()
	responses := serve(t, s, line)
	if len(responses) != 1 {
		t.Fatalf("got %d responses, want 1: %v", len(responses), responses)
	}
	return responses[0]
}

func result(t *testing.T, r map[string]any) map[string]any {
	t.Helper()
	res, ok := r["result"].(map[string]any)
	if !ok {
		t.Fatalf("no result in %v", r)
	}
	return res
}

func toolPayload(t *testing.T, r map[string]any) map[string]any {
	t.Helper()
	content, _ := result(t, r)["content"].([]any)
	if len(content) == 0 {
		t.Fatalf("no content in %v", r)
	}
	first, _ := content[0].(map[string]any)
	text, _ := first["text"].(string)
	var payload map[string]any
	if err := json.Unmarshal([]byte(text), &payload); err != nil {
		t.Fatalf("content text is not JSON: %v: %q", err, text)
	}
	return payload
}

func closedProxy(t *testing.T) {
	t.Helper()
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	addr := "http://" + l.Addr().String()
	l.Close()
	t.Setenv("HTTPS_PROXY", addr)
	t.Setenv("HTTP_PROXY", addr)
}

func newServer() *Server {
	s := disclosures.NewSedar()
	return &Server{Pipeline: &disclosures.Pipeline{Providers: []disclosures.Provider{s}, Sedar: s}, Sedar: s}
}

func TestInitializeReportsTheProtocolAndToolCapability(t *testing.T) {
	r := one(t, newServer(), `{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}`)
	res := result(t, r)
	if res["protocolVersion"] != ProtocolVersion {
		t.Errorf("protocolVersion = %v, want %q", res["protocolVersion"], ProtocolVersion)
	}
	caps, _ := res["capabilities"].(map[string]any)
	if _, ok := caps["tools"]; !ok {
		t.Errorf("capabilities lack tools: %v", res["capabilities"])
	}
	info, _ := res["serverInfo"].(map[string]any)
	if info["name"] != "disclosures" {
		t.Errorf("serverInfo.name = %v, want disclosures", info["name"])
	}
}

func TestInitializedNotificationGetsNoResponse(t *testing.T) {
	responses := serve(t, newServer(), `{"jsonrpc": "2.0", "method": "notifications/initialized"}`)
	if len(responses) != 0 {
		t.Errorf("the initialized notification was answered: %v", responses)
	}
}

func TestToolsListOffersTheDisclosureTools(t *testing.T) {
	r := one(t, newServer(), `{"jsonrpc": "2.0", "id": 2, "method": "tools/list"}`)
	tools, _ := result(t, r)["tools"].([]any)
	names := map[string]bool{}
	for _, raw := range tools {
		tool, _ := raw.(map[string]any)
		name, _ := tool["name"].(string)
		names[name] = true
		if _, ok := tool["inputSchema"]; !ok {
			t.Errorf("tool %s has no inputSchema", name)
		}
		if desc, _ := tool["description"].(string); desc == "" {
			t.Errorf("tool %s has no description", name)
		}
	}
	want := map[string]bool{"disclosures_list": true, "disclosures_document": true, "sedar_resolve_profile": true}
	if len(names) != len(want) {
		t.Errorf("tool names = %v, want %v", names, want)
	}
	for n := range want {
		if !names[n] {
			t.Errorf("tool names = %v, want %v", names, want)
		}
	}
}

func TestResolveWithoutTheSedarDependencyIsACleanToolError(t *testing.T) {
	closedProxy(t)
	r := one(t, newServer(), `{"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "sedar_resolve_profile", "arguments": {"query": "Shopify"}}}`)
	payload := toolPayload(t, r)
	if msg, _ := payload["error"].(string); msg == "" {
		t.Errorf("error is empty: %v", payload)
	}
}

func TestDisclosuresListCallsThePipeline(t *testing.T) {
	fake := &fakeProvider{items: []disclosures.Item{{ID: "sec:1", Source: "SEC"}}}
	s := &Server{Pipeline: &disclosures.Pipeline{Providers: []disclosures.Provider{fake}}}
	r := one(t, s, `{"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "disclosures_list", "arguments": {"symbol": "NVDA"}}}`)
	payload := toolPayload(t, r)
	items, _ := payload["items"].([]any)
	if len(items) == 0 {
		t.Fatalf("no items in %v", payload)
	}
	first, _ := items[0].(map[string]any)
	if first["id"] != "sec:1" {
		t.Errorf("items[0].id = %v, want sec:1", first["id"])
	}
	if len(fake.calls) != 1 {
		t.Fatalf("the pipeline was fetched %d times, want 1", len(fake.calls))
	}
	if got, want := fake.calls[0], (fetchCall{symbol: "NVDA", limit: 100}); got != want {
		t.Errorf("fetch arguments = %+v, want %+v", got, want)
	}
}

func TestAnUnknownMethodReturnsAJSONRPCError(t *testing.T) {
	r := one(t, newServer(), `{"jsonrpc": "2.0", "id": 4, "method": "no/such"}`)
	e, _ := r["error"].(map[string]any)
	if e["code"] != float64(-32601) {
		t.Errorf("error.code = %v, want -32601", e["code"])
	}
}
