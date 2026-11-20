<script lang="ts">
  import { open as openDialog } from '@tauri-apps/plugin-dialog';
  import Select from '../../ui/Select.svelte';
  import Switch from '../../ui/Switch.svelte';
  import { t } from '../../i18n';
  import type { FilePrefs } from '../../ipc';
  import { prefs } from '../prefs.svelte';
  import { editDevice, editPrefs, settingsError } from '../edit.svelte';
  import { isMobile } from '../../platform';
  import { isMac } from '../../actions';
  import { editorArgs, editorProgram } from '../externaleditor';

  const files = $derived(prefs.value.files);
  const conflicts = $derived<{ value: FilePrefs['default_conflict']; label: string }[]>([
    { value: 'ask', label: t('settings.conflictAsk') },
    { value: 'rename', label: t('files.keepBoth') },
    { value: 'overwrite', label: t('files.overwrite') },
    { value: 'skip', label: t('files.skip') },
  ]);

  let program = $state(prefs.device.external_editor?.program ?? '');
  let argv = $state((prefs.device.external_editor?.args ?? []).join('\n'));
  const appBundle = $derived(isMac && /\.app\/?$/i.test(program.trim()));
  let saveRevision = 0;
  async function saveEditor(): Promise<void> {
    const revision = ++saveRevision;
    const next = editorProgram(program, editorArgs(argv));
    await editDevice((draft) => (draft.external_editor = next));
    if (revision !== saveRevision || settingsError.text) return;
    program = prefs.device.external_editor.program;
    argv = prefs.device.external_editor.args.join('\n');
  }
  async function chooseEditor(bundle = false): Promise<void> {
    const picked = await openDialog({ directory: false, multiple: false,
      ...(bundle ? { defaultPath: '/Applications', filters: [{ name: t('settings.externalChooseApp'), extensions: ['app'] }] }
        : /Windows/.test(navigator.userAgent) ? { filters: [{ name: t('settings.externalExecutable'), extensions: ['exe'] }] } : {}) });
    if (typeof picked !== 'string') return;
    program = picked;
    await saveEditor();
  }
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
      <small class="path" title={prefs.device.download_dir}>{prefs.device.download_dir || t('settings.downloadDirDefault')}</small>
    </span>
    {#if prefs.device.download_dir}
      <button type="button" class="plain" onclick={() => void editDevice((draft) => (draft.download_dir = ''))}>{t('settings.reset')}</button>
    {/if}
    <button type="button" class="secondary" onclick={() => void chooseDownloadDir()}>{t('settings.choose')}</button>
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

{#if !isMobile}
  <p class="section-label">{t('settings.externalEditor')}</p>
  <div class="form-group">
    <div class="form-row editor-row">
      <label class="form-label" for="external-editor-program">{t('settings.externalProgram')}<small>{t('settings.externalDeviceHint')}</small></label>
      <input id="external-editor-program" class="program" bind:value={program} onblur={() => void saveEditor()} placeholder={t('settings.externalUnconfigured')} spellcheck="false" />
    </div>
    <div class="form-row">
      <span class="form-label">{t('settings.externalExecutable')}</span>
      {#if isMac}<button class="secondary" onclick={() => void chooseEditor(true)}>{t('settings.externalChooseApp')}</button>{/if}
      <button class="secondary" onclick={() => void chooseEditor()}>{t('settings.choose')}</button>
    </div>
    <div class="form-row editor-row">
      <label class="form-label" for="external-editor-args">{t('settings.externalArgs')}<small>{t(appBundle ? 'settings.externalAppHint' : 'settings.externalArgsHint')}</small></label>
      <textarea id="external-editor-args" bind:value={argv} disabled={appBundle} onblur={() => void saveEditor()} spellcheck="false" rows="4"></textarea>
    </div>
    {#if !appBundle}<div class="form-row"><span class="form-label">{t('settings.externalFileHint')}</span><button class="plain" onclick={() => { argv = [argv, '{file}'].filter(Boolean).join('\n'); void saveEditor(); }}>{t('settings.externalAddFile')}</button></div>{/if}
  </div>
{/if}

<style>
  .editor-row { flex-wrap: wrap; }
  .program, textarea { flex: 1; min-width: 180px; font-family: var(--font-mono); font-size: var(--text-footnote); }
  textarea { resize: vertical; }
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
