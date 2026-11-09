<script lang="ts">
  // Asks whether to trust an unknown server key. Unanswered prompts expire in the backend.
  import Dialog from '../ui/Dialog.svelte';
  import { hostkeyRespond, type HostKeyPrompt } from '../ipc';
  import { t } from '../i18n';

  let { prompt, ondone }: { prompt: HostKeyPrompt; ondone: () => void } = $props();

  async function answer(choice: 'trust' | 'reject'): Promise<void> {
    await hostkeyRespond(prompt.request_id, choice).catch((error: unknown) => console.warn('hostkey answer failed', error));
    ondone();
  }
</script>

<Dialog title={t('hostkey.title')} onclose={() => answer('reject')} width={500}>
  <p>{t('hostkey.body', { host: `${prompt.hostname}:${prompt.port}` })}</p>
  <dl>
    <dt>{t('hostkey.algorithm')}</dt>
    <dd>{prompt.key.algorithm}</dd>
    <dt>{t('hostkey.fingerprint')}</dt>
    <dd class="fingerprint">{prompt.key.fingerprint}</dd>
    {#if prompt.route}
      <dt>{t('hostkey.route')}</dt>
      <dd class="fingerprint">{prompt.route}</dd>
    {/if}
  </dl>
  {#snippet actions()}
    <button type="button" class="secondary" onclick={() => answer('reject')}>{t('hostkey.reject')}</button>
    <button type="button" class="primary" onclick={() => answer('trust')}>{t('hostkey.trust')}</button>
  {/snippet}
</Dialog>

<style>
  p {
    margin: 0;
    line-height: 1.5;
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: var(--space-1) var(--space-3);
    margin: 0;
  }
  dt {
    color: var(--fg-muted);
  }
  dd {
    margin: 0;
  }
  .fingerprint {
    font-family: var(--font-mono);
    font-size: var(--text-footnote);
    word-break: break-all;
  }
</style>
