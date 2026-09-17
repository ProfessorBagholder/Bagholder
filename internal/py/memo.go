package py

import (
	"sync"
	"sync/atomic"
)

const memoMax = 50000

type Memo[T any] struct {
	m sync.Map
	n atomic.Int32
}

func (c *Memo[T]) Get(key string, f func(string) T) T {
	if v, ok := c.m.Load(key); ok {
		return v.(T)
	}
	v := f(key)
	if c.n.Add(1) > memoMax {
		c.m.Clear()
		c.n.Store(0)
	}
	c.m.Store(key, v)
	return v
}
