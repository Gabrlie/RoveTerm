<script lang="ts">
  import { viewport } from '../layout/viewport.svelte';
  import Dialog from '../ui/Dialog.svelte';
  import { t } from '../i18n';

  let { onsubmit, onclose }: { onsubmit: (address: string, password: string | null) => void; onclose: () => void } = $props();
  let address = $state('');
  let password = $state('');

  function submit(event: SubmitEvent): void {
    event.preventDefault();
    if (address.trim()) onsubmit(address.trim(), password || null);
  }
</script>

<Dialog presentation="page" title={t('action.host.quickConnect')} {onclose} width={420} autofocus={true}>
  <form id="quick-form" onsubmit={submit}>
    <label>{t('quick.address')}<input bind:value={address} placeholder="user@host:22" required spellcheck="false" autocomplete="off" /></label>
    <label>{t('host.password')}<input type="password" bind:value={password} placeholder={t('quick.passwordHint')} autocomplete="off" /></label>
  </form>
  {#snippet actions()}
    <button class:page-action={viewport.compact} type="button" class="secondary" onclick={onclose}>{t('common.cancel')}</button>
    <button class:page-action={viewport.compact} type="submit" class="primary" form="quick-form">{t('host.connect')}</button>
  {/snippet}
</Dialog>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .page-action {
    min-height: var(--touch-target, 44px);
    height: auto;
    white-space: normal;
  }
</style>
