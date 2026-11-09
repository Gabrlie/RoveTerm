<script lang="ts">
  import Dialog from '../ui/Dialog.svelte';
  import { t } from '../i18n';
  import type { CredentialKind } from '../ipc';

  interface Props {
    title: string;
    /** Labels the field and the remember option. */
    secret?: CredentialKind;
    allowRemember: boolean;
    onsubmit: (password: string, remember: boolean) => void;
    onclose: () => void;
  }

  let { title, secret = 'password', allowRemember, onsubmit, onclose }: Props = $props();
  let password = $state('');
  let remember = $state(false);

  function submit(event: SubmitEvent): void {
    event.preventDefault();
    onsubmit(password, remember);
  }
</script>

<Dialog {title} {onclose} width={380}>
  <form id="password-form" onsubmit={submit}>
    <label
      >{secret === 'password' ? t('host.password') : t('host.passphrase')}
      <!-- svelte-ignore a11y_autofocus -->
      <input type="password" bind:value={password} autocomplete={secret === 'password' ? 'current-password' : 'off'} autofocus />
    </label>
    {#if allowRemember}
      <label class="check"><input type="checkbox" bind:checked={remember} />{secret === 'password' ? t('host.rememberPassword') : t('host.rememberPassphrase')}</label>
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
