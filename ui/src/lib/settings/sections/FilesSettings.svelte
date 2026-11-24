<script lang="ts">
  import { open as openDialog } from '@tauri-apps/plugin-dialog';
  import Select from '../../ui/Select.svelte';
  import Switch from '../../ui/Switch.svelte';
  import { t } from '../../i18n';
  import type { FilePrefs } from '../../ipc';
  import { isMobile } from '../../platform';
  import { prefs } from '../prefs.svelte';
  import { editDevice, editPrefs } from '../edit.svelte';

  const files = $derived(prefs.value.files);
  const conflicts = $derived<{ value: FilePrefs['default_conflict']; label: string }[]>([
    { value: 'ask', label: t('settings.conflictAsk') },
    { value: 'rename', label: t('files.keepBoth') },
    { value: 'overwrite', label: t('files.overwrite') },
    { value: 'skip', label: t('files.skip') },
  ]);

  async function chooseDownloadDir(): Promise<void> {
    const picked = await openDialog({ directory: true, multiple: false });
    if (typeof picked === 'string') await editDevice((draft) => (draft.download_dir = picked));
  }
</script>

<p class="section-label">{t('settings.group.browsing')}</p>
<div class="form-group">
  <div class="form-row">
    <span class="form-label">{t('files.showHidden')}</span>
    <Switch label={t('files.showHidden')} checked={files.show_hidden} onchange={(checked) => void editPrefs((draft) => (draft.files.show_hidden = checked))} />
  </div>
  <div class="form-row">
    <span class="form-label">{t('settings.confirmDelete')}</span>
    <Switch label={t('settings.confirmDelete')} checked={files.confirm_delete} onchange={(checked) => void editPrefs((draft) => (draft.files.confirm_delete = checked))} />
  </div>
</div>

<p class="section-label">{t('settings.group.transfers')}</p>
<div class="form-group">
  <div class="form-row">
    <span class="form-label">{t('settings.defaultConflict')}</span>
    <div class="control">
      <Select
        label={t('settings.defaultConflict')}
        value={files.default_conflict}
        options={conflicts}
        onchange={(value) => void editPrefs((draft) => (draft.files.default_conflict = value))}
      />
    </div>
  </div>
  <div class="form-row">
    <span class="form-label">
      {t('settings.downloadDir')}
      {#if isMobile}
        <small>{t('settings.downloadDirMobile')}</small>
      {:else}
        <small class="path" title={prefs.device.download_dir}>{prefs.device.download_dir || t('settings.downloadDirDefault')}</small>
      {/if}
    </span>
    {#if !isMobile}
      {#if prefs.device.download_dir}
        <button type="button" class="plain" onclick={() => void editDevice((draft) => (draft.download_dir = ''))}>{t('settings.reset')}</button>
      {/if}
      <button type="button" class="secondary" onclick={() => void chooseDownloadDir()}>{t('settings.choose')}</button>
    {/if}
  </div>
  <div class="form-row">
    <span class="form-label">{t('settings.maxTransfers')}</span>
    <div class="control narrow">
      <Select
        label={t('settings.maxTransfers')}
        value={String(prefs.device.max_transfers)}
        options={[1, 2, 3, 4, 5, 6, 7, 8].map((n) => ({ value: String(n), label: String(n) }))}
        onchange={(value) => void editDevice((draft) => (draft.max_transfers = Number(value)))}
      />
    </div>
  </div>
</div>
<p class="form-note">{t('settings.filesDeviceNote')}</p>

<style>
  .control {
    width: 180px;
  }
  .control.narrow {
    width: 80px;
  }
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
  }
</style>
