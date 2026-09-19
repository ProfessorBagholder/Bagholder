import '@testing-library/jest-dom/vitest'

// jsdom has no ResizeObserver; the chart action observes its node.
class RO { observe() {} unobserve() {} disconnect() {} }
;(globalThis as any).ResizeObserver = (globalThis as any).ResizeObserver || RO
