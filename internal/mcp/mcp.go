package mcp

import (
	"bufio"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	"github.com/ProfessorBagholder/Bagholder/internal/disclosures"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

const ProtocolVersion = "2024-11-05"

var ServerInfo = map[string]any{"name": "disclosures", "version": "1.0.0"}

var meta = map[string]any{
	"name":     map[string]any{"type": "string", "description": "The issuer's name, to seed the lookup and guard against ticker collisions."},
	"exchange": map[string]any{"type": "string", "description": "The listing exchange, if known (e.g. NASDAQ, TSX)."},
	"currency": map[string]any{"type": "string", "description": "The listing currency, if known (USD, CAD)."},
}

func withMeta(props map[string]any) map[string]any {
	out := map[string]any{}
	for k, v := range props {
		out[k] = v
	}
	for k, v := range meta {
		out[k] = v
	}
	return out
}

var Tools = []map[string]any{
	{
		"name":        "disclosures_list",
		"description": "List a company's regulatory filings from every source that covers it (SEDAR+ Canada, SEC EDGAR US), merged newest-first and tagged by source and category (Financials, Material events, Governance, Offerings, Insider & ownership, News release). Give a ticker; add name/exchange/currency when known for accuracy.",
		"inputSchema": map[string]any{"type": "object", "properties": withMeta(map[string]any{
			"symbol": map[string]any{"type": "string", "description": "The ticker, e.g. SHOP or NVDA."},
			"limit":  map[string]any{"type": "integer", "description": "Maximum items to return (default 100)."},
		}), "required": []string{"symbol"}},
	},
	{
		"name":        "disclosures_document",
		"description": "Download one filing to a local file, given the ticker and the item's id from disclosures_list. Returns the saved path.",
		"inputSchema": map[string]any{"type": "object", "properties": withMeta(map[string]any{
			"symbol": map[string]any{"type": "string", "description": "The ticker the item belongs to."},
			"id":     map[string]any{"type": "string", "description": "The item's id from disclosures_list (e.g. 'sec:0001-…' or 'sedar:drm:…')."},
			"dest":   map[string]any{"type": "string", "description": "Where to save the file. Defaults to a temp file named after the id."},
		}), "required": []string{"symbol", "id"}},
	},
	{
		"name":        "sedar_resolve_profile",
		"description": "Find the SEDAR+ reporting-issuer profile number(s) for a Canadian company by name or ticker.",
		"inputSchema": map[string]any{"type": "object", "properties": map[string]any{"query": map[string]any{"type": "string", "description": "Issuer name or nine-digit profile number."}}, "required": []string{"query"}},
	},
}

type Server struct {
	Pipeline *disclosures.Pipeline
	Sedar    *disclosures.Sedar
}

func (s *Server) call(name string, args map[string]any) (any, error) {
	metaOf := func() (string, string, string) {
		return py.S(args["name"]), py.S(args["exchange"]), py.S(args["currency"])
	}
	switch name {
	case "disclosures_list":
		n, e, c := metaOf()
		limit := 100
		if v, ok := py.NumOK(args["limit"]); ok {
			limit = int(v)
		}
		return s.Pipeline.Fetch(py.S(args["symbol"]), n, e, c, limit, ""), nil
	case "disclosures_document":
		n, e, c := metaOf()
		result := s.Pipeline.Fetch(py.S(args["symbol"]), n, e, c, 200, "")
		var row *disclosures.Item
		for i := range result.Items {
			if result.Items[i].ID == py.S(args["id"]) {
				row = &result.Items[i]
				break
			}
		}
		if row == nil {
			return map[string]any{"error": fmt.Sprintf("no item %q for %s", py.S(args["id"]), py.S(args["symbol"]))}, nil
		}
		data, ct, err := s.Pipeline.Document(disclosures.Row{ID: row.ID, Source: row.Source, Category: row.Category, Type: row.Type, URL: row.URL, ProfileNo: row.ProfileNo, Issuer: row.Issuer})
		if err != nil {
			return nil, err
		}
		dest := py.S(args["dest"])
		if dest == "" {
			var base strings.Builder
			for _, ch := range py.S(args["id"]) {
				if (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') || (ch >= '0' && ch <= '9') {
					base.WriteRune(ch)
				}
			}
			b := base.String()
			if b == "" {
				b = "filing"
			}
			ext := ".bin"
			if strings.Contains(ct, "pdf") {
				ext = ".pdf"
			} else if strings.Contains(ct, "html") {
				ext = ".html"
			}
			dest = filepath.Join(os.TempDir(), b+ext)
		}
		if err := os.WriteFile(dest, data, 0o644); err != nil {
			return nil, err
		}
		return map[string]any{"path": dest, "contentType": ct, "bytes": len(data)}, nil
	case "sedar_resolve_profile":
		profiles, err := s.Sedar.ResolveProfile(py.S(args["query"]))
		if err != nil {
			return nil, err
		}
		return map[string]any{"profiles": profiles}, nil
	}
	return nil, fmt.Errorf("unknown tool %q", name)
}

func resultContent(payload any) map[string]any {
	b, _ := json.MarshalIndent(payload, "", "  ")
	return map[string]any{"content": []map[string]any{{"type": "text", "text": string(b)}}}
}

func errorContent(msg string) map[string]any {
	b, _ := json.Marshal(map[string]any{"error": msg})
	return map[string]any{"content": []map[string]any{{"type": "text", "text": string(b)}}, "isError": true}
}

func (s *Server) Handle(msg map[string]any) map[string]any {
	method := py.S(msg["method"])
	mid, hasID := msg["id"]
	switch method {
	case "initialize":
		return map[string]any{"jsonrpc": "2.0", "id": mid, "result": map[string]any{"protocolVersion": ProtocolVersion, "capabilities": map[string]any{"tools": map[string]any{}}, "serverInfo": ServerInfo}}
	case "notifications/initialized", "initialized":
		return nil
	case "tools/list":
		return map[string]any{"jsonrpc": "2.0", "id": mid, "result": map[string]any{"tools": Tools}}
	case "tools/call":
		params, _ := msg["params"].(map[string]any)
		args, _ := params["arguments"].(map[string]any)
		if args == nil {
			args = map[string]any{}
		}
		out, err := s.call(py.S(params["name"]), args)
		if err != nil {
			if disclosures.IsUnavailable(err) {
				return map[string]any{"jsonrpc": "2.0", "id": mid, "result": errorContent(err.Error())}
			}
			return map[string]any{"jsonrpc": "2.0", "id": mid, "result": errorContent(fmt.Sprintf("%T: %s", err, err))}
		}
		return map[string]any{"jsonrpc": "2.0", "id": mid, "result": resultContent(out)}
	}
	if hasID && mid != nil {
		return map[string]any{"jsonrpc": "2.0", "id": mid, "error": map[string]any{"code": -32601, "message": "method not found: " + method}}
	}
	return nil
}

func (s *Server) Serve(in io.Reader, out io.Writer) {
	scanner := bufio.NewScanner(in)
	scanner.Buffer(make([]byte, 1<<20), 64<<20)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" {
			continue
		}
		var msg map[string]any
		if err := json.Unmarshal([]byte(line), &msg); err != nil {
			continue
		}
		if resp := s.Handle(msg); resp != nil {
			b, _ := json.Marshal(resp)
			out.Write(append(b, '\n'))
		}
	}
}
