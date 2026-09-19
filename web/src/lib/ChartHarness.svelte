<script lang="ts">
  import { equityChart } from './actions/equityChart'
  import type { EquityPoint } from './model'
  let { points }: { points: EquityPoint[] } = $props()
</script>

<div data-testid="chart" use:equityChart={points}></div>
