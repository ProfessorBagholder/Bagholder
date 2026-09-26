<script lang="ts">
  // ledger's tkIconSvg(path): a 256-viewBox phosphor glyph, currentColor, 12px
  // by default.
  let { d, size = 12 }: { d: string; size?: number } = $props()
</script>

<svg width={size} height={size} viewBox="0 0 256 256" fill="currentColor"><path {d} /></svg>
