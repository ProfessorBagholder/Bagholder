<script lang="ts">
  // The header's update notice (SPEC.md versions-and-the-update-check-14): a
  // button where this copy can install the release itself or through its
  // updater; a container with none says the one command that updates it, with a
  // copy button, and links nowhere.
  import type { Status } from './model'
  import { copyText } from './clipboard'
  import { updateNow, tell } from './ui.svelte'

  let { status }: { status: Status } = $props()
  const COMMAND = 'docker compose pull && docker compose up -d'
  function copy() {
    copyText(COMMAND).then(
      () => tell('Copied: ' + COMMAND),
      () => tell('The clipboard is not available here: select the command and copy it.', 'err'),
    )
  }
</script>

<!-- nothing to press while an update installs: its progress is in the header's line -->
{#if status.updateAvailable && !status.updating}
  {#if status.canUpdate}
    <button class="pill" style="font-size:11px;margin-left:8px;padding:1px 8px;width:auto;color:var(--accent-300)" onclick={updateNow}>Update to {status.latestVersion}</button>
  {:else if status.updateBy === 'image'}
    <span class="muted" style="font-size:11px;margin-left:8px;white-space:nowrap" data-testid="update-command">{status.latestVersion} available: <code>{COMMAND}</code></span>
    <button class="pill" style="font-size:11px;margin-left:6px;padding:1px 8px;width:auto" onclick={copy}>Copy</button>
  {:else}
    <a href={status.updateUrl || '#'} target="_blank" rel="noopener" style="font-size:11px;margin-left:8px;color:var(--accent-300);text-decoration:none">Update available</a>
  {/if}
{/if}
