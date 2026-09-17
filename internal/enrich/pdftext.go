package enrich

import (
	"bytes"
	"io"
	"os"
	"os/exec"
	"strings"
	"time"

	"github.com/ledongthuc/pdf"
)

func PDFDisabled() bool { return os.Getenv("BAGHOLDER_NO_PDF") != "" }

func PDFAvailable() bool { return !PDFDisabled() }

func PDFStatus() string {
	if PDFDisabled() {
		return "off"
	}
	return "ready"
}

func PDFPending() bool { return false }

func PDFText(data []byte) string {
	if PDFDisabled() || len(data) < 5 || string(data[:5]) != "%PDF-" {
		return ""
	}
	if exe, err := exec.LookPath("pdftotext"); err == nil {
		cmd := exec.Command(exe, "-q", "-nopgbrk", "-", "-")
		cmd.Stdin = bytes.NewReader(data)
		var out bytes.Buffer
		cmd.Stdout = &out
		done := make(chan error, 1)
		if err := cmd.Start(); err == nil {
			go func() { done <- cmd.Wait() }()
			select {
			case err := <-done:
				if err == nil {
					if got := strings.TrimSpace(strings.ToValidUTF8(out.String(), "�")); got != "" {
						return got
					}
				}
			case <-time.After(30 * time.Second):
				cmd.Process.Kill()
				<-done
			}
		}
	}
	return goPDFText(data)
}

func goPDFText(data []byte) (out string) {
	defer func() {
		if e := recover(); e != nil {
			out = ""
		}
	}()
	r, err := pdf.NewReader(bytes.NewReader(data), int64(len(data)))
	if err != nil {
		return ""
	}
	var b strings.Builder
	for i := 1; i <= r.NumPage(); i++ {
		p := r.Page(i)
		if p.V.IsNull() {
			continue
		}
		rows, err := p.GetTextByRow()
		if err != nil {
			continue
		}
		for _, row := range rows {
			for _, w := range row.Content {
				b.WriteString(w.S)
				b.WriteByte(' ')
			}
			b.WriteByte('\n')
		}
	}
	if b.Len() == 0 {
		reader, err := r.GetPlainText()
		if err != nil {
			return ""
		}
		raw, _ := io.ReadAll(reader)
		return strings.TrimSpace(strings.ToValidUTF8(string(raw), "�"))
	}
	return strings.TrimSpace(strings.ToValidUTF8(b.String(), "�"))
}
