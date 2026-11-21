<script lang="ts">
  import Switch from '../../ui/Switch.svelte';
  import { ask } from '../../ui/confirm.svelte';
  import { notify } from '../../ui/toast.svelte';
  import { writeClipboard } from '../../ui/clipboard';
  import { errorText, t } from '../../i18n';
  import { editDevice } from '../edit.svelte';
  import { prefs } from '../prefs.svelte';
  import { tailscale } from '../../tailscale/tailscale.svelte';

  interface Props { onopentailscale?: () => void }
  let { onopentailscale }: Props = $props();

  let authKey = $state('');
  let hostname = $state(prefs.device.tailscale.hostname);
  let controlUrl = $state(prefs.device.tailscale.control_url);

  async function connect(): Promise<void> {
    try {
      await tailscale.connect({ authKey: authKey.trim() || null, controlUrl: controlUrl.trim(), hostname: hostname.trim() });
      authKey = '';
    } catch (error) {
      notify(errorText(error), 'error');
    }
  }

  async function saveDeviceDetails(): Promise<void> {
    await editDevice((draft) => {
      draft.tailscale.hostname = hostname.trim();
      draft.tailscale.control_url = controlUrl.trim();
    });
  }

  async function copyAuthUrl(): Promise<void> {
    if (!tailscale.status.auth_url) return;
    try {
      await writeClipboard(tailscale.status.auth_url);
      notify(t('tailscale.copied'), 'success');
    } catch (error) {
      notify(errorText(error), 'error');
    }
  }

  async function openAuth(): Promise<void> {
    await tailscale.openAuthUrl().catch((error: unknown) => notify(errorText(error), 'error'));
  }

  async function cancelLogin(): Promise<void> {
    await tailscale.disconnect().catch((error: unknown) => notify(errorText(error), 'error'));
  }

  async function logout(): Promise<void> {
    if (!await ask({ title: t('tailscale.logout'), message: t('tailscale.confirmLogout'), confirmLabel: t('tailscale.logout'), danger: true })) return;
    try {
      await tailscale.logout();
      hostname = prefs.device.tailscale.hostname;
      controlUrl = prefs.device.tailscale.control_url;
      notify(t('tailscale.loggedOut'), 'success');
    } catch (error) {
      notify(errorText(error), 'error');
    }
  }

  async function toggleConnection(): Promise<void> {
    try {
      if (tailscale.active) await tailscale.disconnect();
      else await tailscale.connect({ controlUrl: controlUrl.trim(), hostname: hostname.trim() });
    } catch (error) {
      notify(errorText(error), 'error');
    }
  }
</script>

{#if !tailscale.configured}
  <p class="form-note intro">{t('tailscale.intro')}</p>
  <p class="section-label">{t('tailscale.title')}</p>
  <div class="form-group">
    <div class="form-row">
      <label class="form-label" for="ts-hostname">{t('tailscale.hostname')}</label>
      <input id="ts-hostname" class="field" bind:value={hostname} placeholder={t('tailscale.hostnamePlaceholder')} autocomplete="off" spellcheck="false" />
    </div>
    <div class="form-row">
      <label class="form-label" for="ts-control-url">{t('tailscale.controlUrl')}</label>
      <input id="ts-control-url" class="field" bind:value={controlUrl} placeholder={t('tailscale.controlUrlPlaceholder')} autocomplete="url" spellcheck="false" />
    </div>
    <div class="form-row">
      <label class="form-label" for="ts-auth-key">{t('tailscale.authKey')}<small>{t('tailscale.authKeyHint')}</small></label>
      <input id="ts-auth-key" class="field" type="password" bind:value={authKey} autocomplete="new-password" />
    </div>
  </div>
  {#if tailscale.needsLogin && tailscale.status.auth_url}
    <div class="auth-card">
      <p>{t('tailscale.state.needs_login')}</p>
      <button type="button" class="primary" onclick={() => void openAuth()}>{t('tailscale.openLogin')}</button>
      <button type="button" class="secondary" onclick={() => void copyAuthUrl()}>{t('tailscale.copyLogin')}</button>
      <button type="button" class="secondary" onclick={() => void cancelLogin()}>{t('tailscale.cancel')}</button>
    </div>
  {:else}
    <div class="actions">
      <button type="button" class="primary" disabled={tailscale.busy} onclick={() => void connect()}>{tailscale.busy ? t('tailscale.connecting') : t('tailscale.signInConnect')}</button>
    </div>
  {/if}
{:else}
  <p class="section-label">{t('tailscale.status')}</p>
  <div class="form-group">
    <div class="form-row">
      <span class="form-label">{t('tailscale.status')}<small>{t(tailscale.stateLabelKey())}</small></span>
      <span class="status-dot" class:connected={tailscale.connected}></span>
      <button type="button" class={tailscale.active ? 'secondary' : 'primary'} disabled={tailscale.busy} onclick={() => void toggleConnection()}>{tailscale.active ? t('tailscale.disconnect') : t('tailscale.reconnect')}</button>
    </div>
    {#if tailscale.needsLogin && tailscale.status.auth_url}
      <div class="form-row auth-row">
        <div class="auth-card">
          <p>{t('tailscale.state.needs_login')}</p>
          <button type="button" class="primary" onclick={() => void openAuth()}>{t('tailscale.openLogin')}</button>
          <button type="button" class="secondary" onclick={() => void copyAuthUrl()}>{t('tailscale.copyLogin')}</button>
          <button type="button" class="secondary" onclick={() => void cancelLogin()}>{t('tailscale.cancel')}</button>
        </div>
      </div>
    {/if}
    {#if tailscale.status.self_node}
      <div class="form-row"><span class="form-label">{t('tailscale.nodeName')}</span><span class="value">{tailscale.status.self_node.name || tailscale.status.self_node.dns_name}</span></div>
      <div class="form-row"><span class="form-label">{t('tailscale.ips')}</span><span class="value">{tailscale.status.self_node.addresses.join(', ') || t('tailscale.noAddresses')}</span></div>
      <div class="form-row"><span class="form-label">{t('tailscale.peerCount')}</span><span class="value">{tailscale.status.peer_count}</span></div>
    {/if}
    <div class="form-row window-row"><button type="button" class="secondary" onclick={() => onopentailscale?.()}>{t('tailscale.openWindow')}</button></div>
  </div>

  <p class="section-label">{t('tailscale.preferences')}</p>
  <div class="form-group">
    <div class="form-row">
      <span class="form-label">{t('tailscale.autoConnect')}</span>
      <Switch checked={prefs.device.tailscale.auto_connect} label={t('tailscale.autoConnect')} onchange={(checked) => void editDevice((draft) => { draft.tailscale.auto_connect = checked; })} />
    </div>
    <div class="form-row">
      <label class="form-label" for="ts-hostname-config">{t('tailscale.hostname')}</label>
      <input id="ts-hostname-config" class="field" bind:value={hostname} onchange={() => void saveDeviceDetails()} autocomplete="off" spellcheck="false" />
    </div>
    <div class="form-row">
      <label class="form-label" for="ts-control-url-config">{t('tailscale.controlUrl')}</label>
      <input id="ts-control-url-config" class="field" bind:value={controlUrl} onchange={() => void saveDeviceDetails()} autocomplete="url" spellcheck="false" />
    </div>
  </div>
  <p class="form-note">{t('tailscale.applyNextConnect')}</p>
  <div class="actions"><button type="button" class="secondary danger-text" onclick={() => void logout()}>{t('tailscale.logout')}</button></div>
{/if}

{#if tailscale.error}<p class="form-note failed" role="alert">{tailscale.error}</p>{/if}

<style>
  .intro { margin: 0; }
  .field { width: min(280px, 52%); }
  .value { max-width: 56%; color: var(--fg-muted); overflow-wrap: anywhere; text-align: right; }
  .status-dot { width: 8px; height: 8px; flex: none; border-radius: 50%; background: var(--danger); }
  .status-dot.connected { background: var(--success); box-shadow: 0 0 0 2px var(--success-soft); }
  .auth-row { padding: var(--space-2) var(--space-3); }
  .auth-card { width: 100%; display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); padding: var(--space-3); border-radius: var(--radius-md); background: var(--fill); }
  .auth-card p { flex: 1 1 100%; margin: 0; color: var(--fg-muted); font-size: var(--text-footnote); }
  .actions { display: flex; justify-content: flex-end; gap: var(--space-2); margin-top: var(--space-2); }
  .window-row { justify-content: flex-end; }
  .danger-text, .failed { color: var(--danger); }
</style>
