<script lang="ts">
  // The page with nothing to show yet, in three states: not connected (connect),
  // connected and syncing (the first pull is under way), connected with nothing
  // back (sync again). A sync error is shown here in full, under the button, and
  // so not in the header.
  import type { Status } from './model'
  import { connect, syncNow } from './ui.svelte'

  let { status }: { status: Status } = $props()
</script>

<div class="empty" style="min-height:60vh">
  <div style="font-size:16px;font-weight:500">{status.connected && status.syncing ? 'Pulling your history' : 'No activity yet'}</div>
  <div class="dim" style="font-size:12.5px;max-width:420px">
    {#if !status.connected}
      Connect your Wealthsimple account and Bagholder will pull your full history, then keep it up to date every weekday after the close. Nothing leaves this machine.
    {:else if status.syncing}
      Your full Wealthsimple history is on its way. The first sync can take a minute.
    {:else}
      Connected to Wealthsimple, but no activity has come back yet.
    {/if}
  </div>
  {#if !status.connected}
    <button class="btn btn-primary" onclick={connect}>Connect Wealthsimple</button>
  {:else if !status.syncing}
    <button class="btn btn-primary" onclick={syncNow}>Sync now</button>
  {/if}
  {#if status.error}<div class="status-err" style="font-size:12px">{status.error}</div>{/if}
</div>
