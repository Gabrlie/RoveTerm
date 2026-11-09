<script lang="ts">
  // Port forwarding rules of one host: list with live status, start/stop, and an editor.
  import { onMount } from 'svelte';
  import Dialog from '../ui/Dialog.svelte';
  import Icon from '../ui/Icon.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import {
    forwardCreate,
    forwardDelete,
    forwardList,
    forwardStart,
    forwardStop,
    forwardUpdate,
    isCommandError,
    type ForwardDraft,
    type ForwardEntry,
    type ForwardSpec,
    type HostEntry,
  } from '../ipc';
  import { errorText, t } from '../i18n';
  import { describeSpec, forwardStatuses, isPublicBind } from './forwards.svelte';

  interface Props {
    host: HostEntry;
    onclose: () => void;
  }

  let { host, onclose }: Props = $props();

  type Kind = ForwardSpec['kind'];

  let rules = $state<ForwardEntry[]>([]);
  let error = $state('');
  let busy = $state<string | null>(null);
  /** `null` while no editor is open; `''` for a new rule. */
  let editing = $state<string | null>(null);
  let kind = $state<Kind>('local');
  let label = $state('');
  let bindHost = $state('127.0.0.1');
  let bindPort = $state<number | null>(null);
  let destHost = $state('127.0.0.1');
  let destPort = $state<number | null>(null);
  let autoStart = $state(false);
  /** Rule waiting for a password before it can start. */
  let needsPassword = $state<string | null>(null);
  let password = $state('');
  let remember = $state(false);

  const publicWarning = $derived(kind !== 'remote' && isPublicBind(bindHost));

  async function load(): Promise<void> {
    try {
      rules = await forwardList(host.id);
    } catch (caught) {
      error = errorText(caught);
    }
  }

  function statusOf(rule: ForwardEntry) {
    return forwardStatuses.byId[rule.id] ?? null;
  }

  function openEditor(rule: ForwardEntry | null): void {
    error = '';
    editing = rule?.id ?? '';
    const spec = rule?.spec;
    kind = spec?.kind ?? 'local';
    label = rule?.label ?? '';
    bindHost = spec?.bind_host ?? (kind === 'remote' ? 'localhost' : '127.0.0.1');
    bindPort = spec?.bind_port ?? null;
    destHost = spec && spec.kind !== 'dynamic' ? spec.dest_host : '127.0.0.1';
    destPort = spec && spec.kind !== 'dynamic' ? spec.dest_port : null;
    autoStart = rule?.auto_start ?? false;
  }

  function changeKind(next: Kind): void {
    if (next === 'remote' && bindHost === '127.0.0.1') bindHost = 'localhost';
    if (next !== 'remote' && bindHost === 'localhost') bindHost = '127.0.0.1';
    kind = next;
  }

  function draft(): ForwardDraft {
    const port = (value: number | null) => Number(value ?? 0);
    const spec: ForwardSpec =
      kind === 'dynamic'
        ? { kind, bind_host: bindHost.trim(), bind_port: port(bindPort) }
        : { kind, bind_host: bindHost.trim(), bind_port: port(bindPort), dest_host: destHost.trim(), dest_port: port(destPort) };
    return { label: label.trim(), spec, auto_start: autoStart };
  }

  async function save(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (editing === null) return;
    error = '';
    try {
      if (editing) await forwardUpdate(editing, draft());
      else await forwardCreate(host.id, draft());
      editing = null;
      await load();
    } catch (caught) {
      error = errorText(caught);
    }
  }

  async function start(rule: ForwardEntry, typed: string | null = null): Promise<void> {
    busy = rule.id;
    error = '';
    try {
      await forwardStart(rule.id, typed, typed !== null && remember);
      needsPassword = null;
      password = '';
    } catch (caught) {
      if (isCommandError(caught) && caught.code === 'ssh.credentials_required') {
        needsPassword = rule.id;
      } else {
        error = errorText(caught);
      }
    } finally {
      busy = null;
    }
  }

  async function stop(rule: ForwardEntry): Promise<void> {
    busy = rule.id;
    try {
      await forwardStop(rule.id);
    } catch (caught) {
      error = errorText(caught);
    } finally {
      busy = null;
    }
  }

  async function remove(rule: ForwardEntry): Promise<void> {
    if (!confirm(t('forward.confirmDelete', { rule: rule.label || describeSpec(rule.spec) }))) return;
    try {
      await forwardDelete(rule.id);
      await load();
    } catch (caught) {
      error = errorText(caught);
    }
  }

  function submitPassword(event: SubmitEvent): void {
    event.preventDefault();
    const rule = rules.find((candidate) => candidate.id === needsPassword);
    if (rule) void start(rule, password);
  }

  onMount(() => {
    forwardStatuses.start().catch((caught: unknown) => console.warn('forward events unavailable', caught));
    void load();
  });
</script>

<Dialog title={t('forward.title', { host: host.name })} {onclose} width={560}>
  {#if rules.length === 0 && editing === null}
    <p class="empty">{t('forward.empty')}</p>
  {/if}
  {#if rules.length > 0}
  <ul class="rules">
    {#each rules as rule (rule.id)}
      {@const status = statusOf(rule)}
      <li class="rule">
        <span class="kind {rule.spec.kind}">{t(`forward.kind.${rule.spec.kind}.short`)}</span>
        <div class="text">
          <span class="title">
            {rule.label || describeSpec(rule.spec)}
            {#if rule.public}<span class="warn" title={t('forward.publicWarning')}><Icon name="warning" size={13} /></span>{/if}
            {#if rule.auto_start}<span class="tag">{t('forward.autoStartTag')}</span>{/if}
          </span>
          <span class="detail">
            {#if rule.label}{describeSpec(rule.spec)} · {/if}
            {#if status?.state === 'running'}
              <span class="running">{t('forward.running', { active: status.active })}</span>
              {#if status.bound_port !== rule.spec.bind_port}· {t('forward.boundPort', { port: status.bound_port })}{/if}
            {:else if status?.state === 'failed'}
              <span class="failed">{t('forward.failed', { error: status.error ?? '' })}</span>
            {:else}
              {t('forward.stopped')}
            {/if}
          </span>
          {#if needsPassword === rule.id}
            <form class="password" onsubmit={submitPassword}>
              <input type="password" bind:value={password} placeholder={t('host.password')} autocomplete="current-password" />
              {#if host.auth.method === 'password'}
                <label class="check"><input type="checkbox" bind:checked={remember} />{t('host.rememberPassword')}</label>
              {/if}
              <button type="submit" class="secondary">{t('forward.start')}</button>
            </form>
          {/if}
        </div>
        {#if status?.state === 'running'}
          <IconButton icon="stop" label={t('forward.stop')} small disabled={busy === rule.id} onclick={() => void stop(rule)} />
        {:else}
          <IconButton icon="play" label={t('forward.start')} small disabled={busy === rule.id} onclick={() => void start(rule)} />
        {/if}
        <IconButton icon="edit" label={t('forward.edit')} small onclick={() => openEditor(rule)} />
        <IconButton icon="remove" label={t('common.delete')} small danger onclick={() => void remove(rule)} />
      </li>
    {/each}
  </ul>
  {/if}

  {#if editing !== null}
    <form id="forward-form" class="editor" class:separated={rules.length > 0} onsubmit={save}>
      <label
        >{t('forward.kind')}
        <select value={kind} onchange={(event) => changeKind(event.currentTarget.value as Kind)}>
          <option value="local">{t('forward.kind.local')}</option>
          <option value="remote">{t('forward.kind.remote')}</option>
          <option value="dynamic">{t('forward.kind.dynamic')}</option>
        </select>
      </label>
      <p class="hint">{t(`forward.hint.${kind}`)}</p>
      <div class="row">
        <label>{kind === 'remote' ? t('forward.serverBindHost') : t('forward.bindHost')}<input bind:value={bindHost} required spellcheck="false" /></label>
        <label class="port">{t('host.port')}<input type="number" min={kind === 'remote' ? 0 : 1} max="65535" bind:value={bindPort} required /></label>
      </div>
      {#if kind !== 'dynamic'}
        <div class="row">
          <label>{kind === 'remote' ? t('forward.localDestHost') : t('forward.destHost')}<input bind:value={destHost} required spellcheck="false" /></label>
          <label class="port">{t('host.port')}<input type="number" min="1" max="65535" bind:value={destPort} required /></label>
        </div>
      {/if}
      <label>{t('forward.label')}<input bind:value={label} autocomplete="off" /></label>
      <label class="check"><input type="checkbox" bind:checked={autoStart} />{t('forward.autoStart')}</label>
      {#if publicWarning}
        <p class="warning"><Icon name="warning" size={14} />{t('forward.publicWarning')}</p>
      {/if}
    </form>
  {/if}
  {#if error}<p class="error">{error}</p>{/if}

  {#snippet actions()}
    {#if editing === null}
      <button type="button" class="secondary" onclick={onclose}>{t('common.close')}</button>
      <button type="button" class="primary" onclick={() => openEditor(null)}>{t('forward.add')}</button>
    {:else}
      <button type="button" class="secondary" onclick={() => (editing = null)}>{t('common.cancel')}</button>
      <button type="submit" class="primary" form="forward-form">{t('common.save')}</button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .empty {
    margin: 0;
    color: var(--fg-muted);
  }
  .rules {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .rule {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }
  .kind {
    flex: none;
    width: 22px;
    margin-top: 1px;
    border-radius: var(--radius-sm);
    background: var(--bg-sunken);
    color: var(--fg-muted);
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    font-weight: 600;
    text-align: center;
    line-height: 18px;
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .title {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--fg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .detail {
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .running {
    color: var(--accent);
  }
  .failed,
  .warn,
  .warning {
    color: var(--danger);
  }
  .tag {
    padding: 0 var(--space-1);
    border-radius: var(--radius-sm);
    background: var(--bg-sunken);
    color: var(--fg-muted);
    font-size: var(--text-caption);
  }
  .password {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-1);
  }
  .password input[type='password'] {
    flex: 1;
    min-width: 0;
  }
  .editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .editor.separated {
    padding-top: var(--space-2);
    border-top: 1px solid var(--border);
  }
  .hint {
    margin: calc(-1 * var(--space-2)) 0 0;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .port {
    flex: 0 0 96px !important;
  }
  .warning {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin: 0;
    font-size: var(--text-footnote);
  }
</style>
