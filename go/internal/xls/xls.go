package xls

import (
	"encoding/binary"
	"errors"
	"math"
	"unicode/utf16"
)

const (
	recLabel    = 0x0204
	recNumber   = 0x0203
	recRK       = 0x027E
	recMulRK    = 0x00BD
	recSST      = 0x00FC
	recContinue = 0x003C
	recLabelSST = 0x00FD
)

var oleMagic = []byte{0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1}

func i32(b []byte, off int) int32 {
	if off+4 > len(b) || off < 0 {
		return 0
	}
	return int32(binary.LittleEndian.Uint32(b[off:]))
}

func u16(b []byte, off int) int {
	if off+2 > len(b) || off < 0 {
		return 0
	}
	return int(binary.LittleEndian.Uint16(b[off:]))
}

func Streams(raw []byte) (map[string][]byte, error) {
	if len(raw) < 512 || string(raw[:8]) != string(oleMagic) {
		return nil, errors.New("not an OLE compound file")
	}
	ssz := 1 << u16(raw, 30)
	msz := 1 << u16(raw, 32)
	nfat := int(i32(raw, 44))
	dirstart := i32(raw, 48)
	minicut := int(i32(raw, 56))
	ministart := i32(raw, 60)
	difstart := i32(raw, 68)
	ndif := int(i32(raw, 72))
	sector := func(i int32) []byte {
		start := 512 + int(i)*ssz
		end := start + ssz
		if start < 0 || start > len(raw) {
			return nil
		}
		if end > len(raw) {
			end = len(raw)
		}
		return raw[start:end]
	}
	var difat []int32
	for i := 0; i < 109; i++ {
		difat = append(difat, i32(raw, 76+i*4))
	}
	node, left := difstart, ndif
	for left > 0 && node >= 0 {
		blk := sector(node)
		if len(blk) < ssz {
			break
		}
		for i := 0; i < ssz/4-1; i++ {
			difat = append(difat, i32(blk, i*4))
		}
		node = i32(blk, ssz-4)
		left--
	}
	var fat []int32
	for i := 0; i < nfat && i < len(difat); i++ {
		f := difat[i]
		if f < 0 {
			continue
		}
		blk := sector(f)
		for j := 0; j+4 <= len(blk); j += 4 {
			fat = append(fat, i32(blk, j))
		}
	}
	chain := func(start int32) []int32 {
		var out []int32
		seen := map[int32]bool{}
		i := start
		for i >= 0 && int(i) < len(fat) && !seen[i] {
			seen[i] = true
			out = append(out, i)
			i = fat[i]
		}
		return out
	}
	join := func(ids []int32) []byte {
		var out []byte
		for _, i := range ids {
			out = append(out, sector(i)...)
		}
		return out
	}
	type entry struct {
		name  string
		kind  byte
		start int32
		size  uint32
	}
	var entries []entry
	dirdata := join(chain(dirstart))
	for off := 0; off+128 <= len(dirdata); off += 128 {
		e := dirdata[off : off+128]
		nlen := u16(e, 64)
		n := nlen - 2
		if n < 0 {
			n = 0
		}
		if n > 64 {
			n = 64
		}
		name := decodeUTF16(e[:n])
		entries = append(entries, entry{name, e[66], i32(e, 116), binary.LittleEndian.Uint32(e[120:])})
	}
	var root *entry
	for i := range entries {
		if entries[i].kind == 5 {
			root = &entries[i]
			break
		}
	}
	var mini []byte
	if root != nil && root.start >= 0 {
		mini = join(chain(root.start))
	}
	var minifat []int32
	if ministart >= 0 {
		blk := join(chain(ministart))
		for j := 0; j+4 <= len(blk); j += 4 {
			minifat = append(minifat, i32(blk, j))
		}
	}
	out := map[string][]byte{}
	for _, e := range entries {
		if e.kind != 2 {
			continue
		}
		var buf []byte
		if int(e.size) < minicut && len(minifat) > 0 {
			seen := map[int32]bool{}
			i := e.start
			for i >= 0 && int(i) < len(minifat) && !seen[i] {
				seen[i] = true
				a, b := int(i)*msz, int(i+1)*msz
				if a > len(mini) {
					a = len(mini)
				}
				if b > len(mini) {
					b = len(mini)
				}
				buf = append(buf, mini[a:b]...)
				i = minifat[i]
			}
		} else {
			buf = join(chain(e.start))
		}
		if int(e.size) < len(buf) {
			buf = buf[:e.size]
		}
		out[e.name] = buf
	}
	return out, nil
}

func decodeUTF16(b []byte) string {
	u := make([]uint16, 0, len(b)/2)
	for i := 0; i+1 < len(b); i += 2 {
		u = append(u, binary.LittleEndian.Uint16(b[i:]))
	}
	return string(utf16.Decode(u))
}

func decodeLatin1(b []byte) string {
	r := make([]rune, len(b))
	for i, c := range b {
		r[i] = rune(c)
	}
	return string(r)
}

type record struct {
	id   int
	body []byte
}

func records(buf []byte) []record {
	var out []record
	i := 0
	for i+4 <= len(buf) {
		rid := u16(buf, i)
		ln := u16(buf, i+2)
		end := i + 4 + ln
		if end > len(buf) {
			end = len(buf)
		}
		out = append(out, record{rid, buf[i+4 : end]})
		i += 4 + ln
	}
	return out
}

func packedNumber(v uint32) float64 {
	var x float64
	if v&2 != 0 {
		n := v >> 2
		if n&(1<<29) != 0 {
			x = float64(int64(n) - (1 << 30))
		} else {
			x = float64(n)
		}
	} else {
		x = math.Float64frombits(uint64(v&0xFFFFFFFC) << 32)
	}
	if v&1 != 0 {
		return x / 100
	}
	return x
}

func text(data []byte, off int) string {
	n := u16(data, off)
	if off+3 > len(data) {
		return ""
	}
	wide := data[off+2]&1 != 0
	width := 1
	if wide {
		width = 2
	}
	end := off + 3 + n*width
	if end > len(data) {
		end = len(data)
	}
	body := data[off+3 : end]
	if wide {
		return decodeUTF16(body)
	}
	return decodeLatin1(body)
}

func shared(chunks [][]byte) []string {
	var buf []byte
	for _, c := range chunks {
		buf = append(buf, c...)
	}
	count := int(i32(buf, 4))
	pos := 8
	var out []string
	for k := 0; k < count; k++ {
		if pos+3 > len(buf) {
			break
		}
		n := u16(buf, pos)
		flags := buf[pos+2]
		pos += 3
		rich := 0
		if flags&8 != 0 {
			rich = u16(buf, pos)
			pos += 2
		}
		far := 0
		if flags&4 != 0 {
			far = int(i32(buf, pos))
			pos += 4
		}
		width := 1
		if flags&1 != 0 {
			width = 2
		}
		end := pos + n*width
		if end > len(buf) {
			end = len(buf)
		}
		if flags&1 != 0 {
			out = append(out, decodeUTF16(buf[pos:end]))
		} else {
			out = append(out, decodeLatin1(buf[pos:end]))
		}
		pos += n*width + rich*4 + far
	}
	return out
}

type cellKey struct{ r, c int }

func Cells(buf []byte) map[cellKey]any {
	out := map[cellKey]any{}
	var strs []string
	var collecting [][]byte
	for _, rec := range records(buf) {
		if rec.id == recSST {
			collecting = [][]byte{rec.body}
			continue
		}
		if rec.id == recContinue && collecting != nil {
			collecting = append(collecting, rec.body)
			continue
		}
		if collecting != nil {
			strs = shared(collecting)
			collecting = nil
		}
		data := rec.body
		switch rec.id {
		case recLabel:
			out[cellKey{u16(data, 0), u16(data, 2)}] = text(data, 6)
		case recLabelSST:
			idx := int(i32(data, 6))
			v := ""
			if idx >= 0 && idx < len(strs) {
				v = strs[idx]
			}
			out[cellKey{u16(data, 0), u16(data, 2)}] = v
		case recNumber:
			if len(data) >= 14 {
				out[cellKey{u16(data, 0), u16(data, 2)}] = math.Float64frombits(binary.LittleEndian.Uint64(data[6:]))
			}
		case recRK:
			if len(data) >= 10 {
				out[cellKey{u16(data, 0), u16(data, 2)}] = packedNumber(binary.LittleEndian.Uint32(data[6:]))
			}
		case recMulRK:
			r, first := u16(data, 0), u16(data, 2)
			for k := 0; k < (len(data)-6)/6; k++ {
				out[cellKey{r, first + k}] = packedNumber(binary.LittleEndian.Uint32(data[6+k*6:]))
			}
		}
	}
	return out
}

func Table(raw []byte) ([][]any, error) {
	found, err := Streams(raw)
	if err != nil {
		return nil, err
	}
	buf := found["Workbook"]
	if len(buf) == 0 {
		buf = found["Book"]
	}
	if len(buf) == 0 {
		return nil, errors.New("no workbook stream")
	}
	grid := Cells(buf)
	if len(grid) == 0 {
		return [][]any{}, nil
	}
	rows, cols := 0, 0
	for k := range grid {
		if k.r > rows {
			rows = k.r
		}
		if k.c > cols {
			cols = k.c
		}
	}
	out := make([][]any, rows+1)
	for r := 0; r <= rows; r++ {
		row := make([]any, cols+1)
		for c := 0; c <= cols; c++ {
			if v, ok := grid[cellKey{r, c}]; ok {
				row[c] = v
			} else {
				row[c] = ""
			}
		}
		out[r] = row
	}
	return out, nil
}
