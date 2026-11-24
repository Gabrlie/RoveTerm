<script lang="ts">
  import { onMount } from 'svelte';
  import { errorText, t } from '../../i18n';
  import { vaultBiometricStatus, vaultSetBiometric, vaultStatus, type VaultBiometricStatus } from '../../ipc';
  import { ask } from '../../ui/confirm.svelte';
  import Switch from '../../ui/Switch.svelte';
  import { vault } from '../../vault/vault.svelte';
  import { settingsError } from '../edit.svelte';
  import { prefs } from '../prefs.svelte';

  let status = $state<VaultBiometricStatus>();
  let checked = $state(false);
  let busy = $state(false);

  async function run(action: () => Promise<void>): Promise<void> {
    if (busy) return;
    busy = true;
    settingsError.text = '';
    try {
      await action();
    } catch (error) {
      settingsError.text ||= errorText(error);
    } finally {
      busy = false;
    }
  }

  async function refreshBiometricStatus(): Promise<void> {
    try {
      status = await vaultBiometricStatus();
      checked = status.enabled;
    } catch (error) {
      status = undefined;
      settingsError.text ||= errorText(error);
    }
  }

  onMount(() => {
    checked = prefs.device.vault_biometric ?? false;
    void run(refreshBiometricStatus);
  });

  function setBiometric(enabled: boolean): Promise<void> {
    checked = status?.enabled ?? prefs.device.vault_biometric ?? false;
    return run(async () => {
      try {
        prefs.device = await vaultSetBiometric(enabled);
        checked = prefs.device.vault_biometric;
      } catch (error) {
        settingsError.text = errorText(error);
      }
      await refreshBiometricStatus();
      vault.state = await vaultStatus();
    });
  }

  function unlock(): Promise<void> {
    return run(async () => {
      const state = await vault.unlock();
      if (state.state === 'locked') {
        settingsError.text = t(state.reason.kind === 'denied' ? 'error.vault.denied' : 'vault.unlockFailed');
      }
    });
  }

  function reset(): Promise<void> {
    return run(async () => {
      const confirmed = await ask({
        title: t('vault.state.keyLost'),
        message: t('vault.confirmReset'),
        confirmLabel: t('vault.reset'),
        danger: true,
      });
      if (!confirmed || !vault.keyLost) return;
      await vault.reset();
      await refreshBiometricStatus();
    });
  }
</script>

<p class="section-label">{t('vault.title')}</p>
<div class="form-group">
  <div class="form-row">
    <span class="form-label">
      {t('vault.status')}
      <small>{t(vault.unlocked ? 'vault.state.unlocked' : vault.keyLost ? 'vault.state.keyLost' : 'vault.state.locked')}</small>
    </span>
    {#if vault.keyLost}
      <button type="button" class="secondary" disabled={busy} onclick={() => void reset()}>{t('vault.reset')}</button>
    {:else if !vault.unlocked}
      <button type="button" class="secondary" disabled={busy} onclick={() => void unlock()}>{t('vault.unlock')}</button>
    {/if}
  </div>
  <div class="form-row">
    <span class="form-label">
      {t('vault.biometric')}
      {#if status}
        <small>{t(!status.supported ? 'error.vault.biometric_unsupported' : !status.enrolled ? 'error.vault.biometric_unavailable' : 'vault.biometricHint')}</small>
      {/if}
    </span>
    <Switch bind:checked label={t('vault.biometric')} disabled={busy || !status?.supported || (!status?.enrolled && !status?.enabled)} onchange={(enabled) => void setBiometric(enabled)} />
  </div>
</div>
