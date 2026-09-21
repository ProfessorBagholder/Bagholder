package xls

import (
	"bytes"
	"encoding/binary"
	"math"
	"reflect"
	"testing"
	"unicode/utf16"
)

func le16(v int) []byte {
	b := make([]byte, 2)
	binary.LittleEndian.PutUint16(b, uint16(v))
	return b
}

func le32(v int32) []byte {
	b := make([]byte, 4)
	binary.LittleEndian.PutUint32(b, uint32(v))
	return b
}

func utf16le(s string) []byte {
	var out []byte
	for _, u := range utf16.Encode([]rune(s)) {
		out = append(out, le16(int(u))...)
	}
	return out
}

func latin1(s string) []byte {
	out := make([]byte, 0, len(s))
	for _, r := range s {
		out = append(out, byte(r))
	}
	return out
}

func rec(rid int, body []byte) []byte {
	return append(append(le16(rid), le16(len(body))...), body...)
}

func label(row, col int, s string, wide bool) []byte {
	flag := byte(0)
	chars := latin1(s)
	if wide {
		flag = 1
		chars = utf16le(s)
	}
	body := append(append(append(le16(row), le16(col)...), le16(0)...), le16(len([]rune(s)))...)
	body = append(body, flag)
	return rec(recLabel, append(body, chars...))
}

func number(row, col int, value float64) []byte {
	b := make([]byte, 8)
	binary.LittleEndian.PutUint64(b, math.Float64bits(value))
	body := append(append(append(le16(row), le16(col)...), le16(0)...), b...)
	return rec(recNumber, body)
}

func packed(value int32, cents bool) uint32 {
	v := uint32(value<<2) | 2
	if cents {
		v |= 1
	}
	return v
}

func rk(row, col int, raw uint32) []byte {
	b := make([]byte, 4)
	binary.LittleEndian.PutUint32(b, raw)
	body := append(append(append(le16(row), le16(col)...), le16(0)...), b...)
	return rec(recRK, body)
}

func mulrk(row, first int, raws []uint32) []byte {
	body := append(le16(row), le16(first)...)
	for _, raw := range raws {
		b := make([]byte, 4)
		binary.LittleEndian.PutUint32(b, raw)
		body = append(append(body, le16(0)...), b...)
	}
	return rec(recMulRK, append(body, le16(first+len(raws)-1)...))
}

func sst(strs []string) []byte {
	body := append(le32(int32(len(strs))), le32(int32(len(strs)))...)
	for _, s := range strs {
		body = append(append(append(body, le16(len(s))...), 0), latin1(s)...)
	}
	return rec(recSST, body)
}

func labelsst(row, col int, index int32) []byte {
	body := append(append(append(le16(row), le16(col)...), le16(0)...), le32(index)...)
	return rec(recLabelSST, body)
}

func container(stream []byte, name string) []byte {
	sectors := (len(stream) + 511) / 512
	data := append(append([]byte{}, stream...), make([]byte, sectors*512-len(stream))...)
	header := make([]byte, 512)
	copy(header, oleMagic)
	copy(header[28:], le16(0xFFFE))
	copy(header[30:], le16(9))
	copy(header[32:], le16(6))
	copy(header[44:], le32(1))
	copy(header[48:], le32(1))
	copy(header[56:], le32(4096))
	copy(header[60:], le32(-2))
	copy(header[68:], le32(-2))
	copy(header[72:], le32(0))
	for i := 0; i < 109; i++ {
		v := int32(-1)
		if i == 0 {
			v = 0
		}
		copy(header[76+i*4:], le32(v))
	}
	table := bytes.Repeat([]byte{0xff}, 512)
	copy(table[0:], le32(-3))
	copy(table[4:], le32(-2))
	for i := 0; i < sectors; i++ {
		next := int32(3 + i)
		if i == sectors-1 {
			next = -2
		}
		copy(table[8+i*4:], le32(next))
	}
	directory := make([]byte, 512)
	entry := func(at int, entryName string, kind byte, start int32, size uint32) {
		raw := utf16le(entryName)
		copy(directory[at:], raw)
		copy(directory[at+64:], le16(len(raw)+2))
		directory[at+66] = kind
		copy(directory[at+116:], le32(start))
		binary.LittleEndian.PutUint32(directory[at+120:], size)
	}
	entry(0, "Root Entry", 5, -2, 0)
	entry(128, name, 2, 2, uint32(len(stream)))
	out := append([]byte{}, header...)
	out = append(out, table...)
	out = append(out, directory...)
	return append(out, data...)
}

func join(parts ...[]byte) []byte {
	var out []byte
	for _, p := range parts {
		out = append(out, p...)
	}
	return out
}

func TestTextNumbersAndPackedRunsAllLandInTheirCells(t *testing.T) {
	halfDouble := uint32(math.Float64bits(2.0)>>32) &^ 3
	stream := join(label(0, 0, "Security Symbol", false), label(0, 1, "Exchange Code", false),
		label(1, 0, "QNC", false), label(1, 1, "TSXV", false),
		mulrk(1, 2, []uint32{packed(2667164, false), packed(64077, false)}),
		number(2, 0, 1.5), rk(2, 1, packed(250, true)), rk(2, 2, halfDouble))
	cells := Cells(stream)
	if cells[cellKey{0, 0}] != "Security Symbol" {
		t.Errorf("(0,0) = %v", cells[cellKey{0, 0}])
	}
	if cells[cellKey{1, 0}] != "QNC" {
		t.Errorf("(1,0) = %v", cells[cellKey{1, 0}])
	}
	if cells[cellKey{1, 2}] != 2667164.0 {
		t.Errorf("(1,2) = %v", cells[cellKey{1, 2}])
	}
	if cells[cellKey{1, 3}] != 64077.0 {
		t.Errorf("(1,3) = %v", cells[cellKey{1, 3}])
	}
	if cells[cellKey{2, 0}] != 1.5 {
		t.Errorf("(2,0) = %v", cells[cellKey{2, 0}])
	}
	if cells[cellKey{2, 1}] != 2.5 {
		t.Errorf("(2,1) = %v", cells[cellKey{2, 1}])
	}
	if cells[cellKey{2, 2}] != 2.0 {
		t.Errorf("(2,2) = %v", cells[cellKey{2, 2}])
	}
}

func TestANegativePackedNumberKeepsItsSign(t *testing.T) {
	if got := Cells(rk(0, 0, packed(-110130, false)))[cellKey{0, 0}]; got != -110130.0 {
		t.Errorf("got %v", got)
	}
}

func TestWideTextReadsAsWritten(t *testing.T) {
	if got := Cells(label(0, 0, "1911 GOLD", true))[cellKey{0, 0}]; got != "1911 GOLD" {
		t.Errorf("got %v", got)
	}
}

func TestSharedStringsAreReadThroughTheCellsThatPointAtThem(t *testing.T) {
	stream := join(sst([]string{"ZYUS LIFE SCIENCES", "ZYUS"}), labelsst(0, 0, 0), labelsst(0, 1, 1), labelsst(0, 2, 9))
	cells := Cells(stream)
	if cells[cellKey{0, 0}] != "ZYUS LIFE SCIENCES" {
		t.Errorf("(0,0) = %v", cells[cellKey{0, 0}])
	}
	if cells[cellKey{0, 1}] != "ZYUS" {
		t.Errorf("(0,1) = %v", cells[cellKey{0, 1}])
	}
	if cells[cellKey{0, 2}] != "" {
		t.Errorf("(0,2) = %v", cells[cellKey{0, 2}])
	}
}

func TestRecordsItDoesNotReadAreSkippedRatherThanBreakingTheRow(t *testing.T) {
	stream := join(rec(0x0208, make([]byte, 16)), label(0, 0, "ONE", false), rec(0x00E0, bytes.Repeat([]byte{1}, 20)), rk(0, 1, packed(395141, false)))
	cells := Cells(stream)
	if cells[cellKey{0, 0}] != "ONE" {
		t.Errorf("(0,0) = %v", cells[cellKey{0, 0}])
	}
	if cells[cellKey{0, 1}] != 395141.0 {
		t.Errorf("(0,1) = %v", cells[cellKey{0, 1}])
	}
}

func TestTheWorkbookStreamIsFoundAndReadAsATable(t *testing.T) {
	stream := join(label(0, 0, "Security Issue Name", false), label(0, 1, "Security Symbol", false), label(0, 2, "Exchange Code", false),
		label(1, 0, "HIGH TIDE INC.", false), label(1, 1, "HITI", false), label(1, 2, "TSXV", false),
		mulrk(1, 3, []uint32{packed(124186, false), packed(-10870, false)}))
	rows, err := Table(container(append(stream, make([]byte, 5000)...), "Workbook"))
	if err != nil {
		t.Fatal(err)
	}
	if want := []any{"Security Issue Name", "Security Symbol", "Exchange Code", "", ""}; !reflect.DeepEqual(rows[0], want) {
		t.Errorf("row 0 = %v", rows[0])
	}
	if want := []any{"HIGH TIDE INC.", "HITI", "TSXV", 124186.0, -10870.0}; !reflect.DeepEqual(rows[1], want) {
		t.Errorf("row 1 = %v", rows[1])
	}
}

func TestSomethingThatIsNotAContainerSaysSo(t *testing.T) {
	if _, err := Table([]byte("Security,Symbol\nHITI,TSXV\n")); err == nil {
		t.Error("no error")
	}
}

func TestAContainerWithoutAWorkbookSaysSo(t *testing.T) {
	if _, err := Table(container(make([]byte, 5000), "Nothing")); err == nil {
		t.Error("no error")
	}
}
