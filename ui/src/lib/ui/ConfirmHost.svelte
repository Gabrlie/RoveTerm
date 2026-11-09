<script lang="ts">
  // Renders the pending in-app confirmation, one at a time.
  import Dialog from './Dialog.svelte';
  import { t } from '../i18n';
  import { confirmQueue } from './confirm.svelte';
</script>

{#if confirmQueue.current}
  {@const request = confirmQueue.current}
  <Dialog title={request.title} onclose={() => confirmQueue.answer(false)} width={400}>
    <p class="message">{request.message}</p>
    {#snippet actions()}
      <button type="button" class="secondary" onclick={() => confirmQueue.answer(false)}>{t('common.cancel')}</button>
      <button type="button" class="primary" class:danger={request.danger} onclick={() => confirmQueue.answer(true)}>{request.confirmLabel}</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .message {
    margin: 0;
    color: var(--fg);
    line-height: 1.5;
    white-space: pre-line;
  }
</style>
