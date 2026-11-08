<script lang="ts">
  import Dialog from '../ui/Dialog.svelte';
  import { t } from '../i18n';

  interface Props {
    title: string;
    allowRemember: boolean;
    onsubmit: (password: string, remember: boolean) => void;
    onclose: () => void;
  }

  let { title, allowRemember, onsubmit, onclose }: Props = $props();
  let password = $state('');
  let remember = $state(false);

  function submit(event: SubmitEvent): void {
    event.preventDefault();
    onsubmit(password, remember);
  }
</script>

<Dialog {title} {onclose} width={380}>
  <form id="password-form" onsubmit={submit}>
    <label>{t('host.password')}<input type="password" bind:value={password} autocomplete="current-password" /></label>
    {#if allowRemember}
      <label class="check"><input type="checkbox" bind:checked={remember} />{t('host.rememberPassword')}</label>
    {/if}
  </form>
  {#snippet actions()}
    <button type="button" class="secondary" onclick={onclose}>{t('common.cancel')}</button>
    <button type="submit" class="primary" form="password-form">{t('host.connect')}</button>
  {/snippet}
</Dialog>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
</style>
