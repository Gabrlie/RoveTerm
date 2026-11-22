<script lang="ts">
  // Interactive sign-in dialog for local ACP agents: probes the agent's advertised `authMethods`,
  // runs protocol `authenticate(methodId)` flows, or hosts an embedded interactive PTY (`TermView`)
  // for ACP terminal-auth and installed CLI login fallbacks.
  import { onDestroy, onMount } from 'svelte';
  import Dialog from '../ui/Dialog.svelte';
  import Icon from '../ui/Icon.svelte';
  import TermView from '../terminal/TermView.svelte';
  import { notify } from '../ui/toast.svelte';
  import { errorText, t } from '../i18n';
  import {
    aiAcpAuthMethods,
    aiAcpAuthenticate,
    aiAcpCancelAuth,
    aiAcpLoginTerminal,
    sessionClose,
    type AcpAuthMethod,
    type ExitStatus,
    type SessionId,
  } from '../ipc';

  interface Props {
    agent: string;
    agentName: string;
    onclose: () => void;
    onsignedin?: () => void;
    onapikey?: () => void;
  }

  let { agent, agentName, onclose, onsignedin, onapikey }: Props = $props();

  type Stage = 'probing' | 'methods' | 'authenticating' | 'terminal';

  let stage = $state<Stage>('probing');
  let methods = $state<AcpAuthMethod[]>([]);
  let probeError = $state<string | null>(null);
  let error = $state('');
  let activeMethod = $state<AcpAuthMethod | null>(null);
  let terminalRun = $state(0);
  let exitStatus = $state<ExitStatus | null>(null);
  let completed = $state(false);
  let termSessionId: SessionId | null = null;
  let destroyed = false;
  let seq = 0;

  async function closeTerminal(): Promise<void> {
    const id = termSessionId;
    termSessionId = null;
    if (id) await sessionClose(id).catch(() => undefined);
  }

  async function probe(): Promise<void> {
    const ticket = ++seq;
    stage = 'probing';
    error = '';
    probeError = null;
    activeMethod = null;
    try {
      const info = await aiAcpAuthMethods(agent);
      if (destroyed || ticket !== seq) return;
      methods = info.methods;
      probeError = info.probe_error;
      stage = 'methods';
    } catch (failure) {
      if (destroyed || ticket !== seq) return;
      methods = [];
      probeError = errorText(failure);
      stage = 'methods';
    }
  }

  async function choose(method: AcpAuthMethod): Promise<void> {
    error = '';
    if (method.kind === 'api_key') {
      if (onapikey) {
        onclose();
        onapikey();
      } else {
        error = t('ai.login.useApiKeyHint');
      }
      return;
    }
    if (method.kind === 'terminal') {
      await closeTerminal();
      activeMethod = method;
      exitStatus = null;
      completed = false;
      terminalRun += 1;
      stage = 'terminal';
      return;
    }
    const ticket = ++seq;
    activeMethod = method;
    stage = 'authenticating';
    try {
      await aiAcpAuthenticate(agent, method.id);
      if (destroyed || ticket !== seq) return;
      notify(t('ai.login.succeeded', { name: agentName }), 'success');
      onsignedin?.();
      onclose();
    } catch (failure) {
      if (destroyed || ticket !== seq) return;
      const code = typeof failure === 'object' && failure !== null && 'code' in failure ? String((failure as { code: unknown }).code) : null;
      if (code !== 'ai.cancelled') error = errorText(failure);
      activeMethod = null;
      stage = 'methods';
    }
  }

  async function cancelAuth(): Promise<void> {
    seq += 1;
    await aiAcpCancelAuth().catch(() => undefined);
    if (destroyed) return;
    activeMethod = null;
    stage = 'methods';
  }

  async function backToMethods(): Promise<void> {
    await closeTerminal();
    activeMethod = null;
    exitStatus = null;
    stage = 'methods';
  }

  async function retryTerminal(): Promise<void> {
    if (!activeMethod) return;
    await closeTerminal();
    exitStatus = null;
    completed = false;
    terminalRun += 1;
  }

  function onTerminalExit(status: ExitStatus): void {
    exitStatus = status;
    if (status.code === 0) {
      completed = true;
      notify(t('ai.login.succeeded', { name: agentName }), 'success');
    }
  }

  function close(): void {
    const signedIn = completed;
    if (stage === 'probing' || stage === 'authenticating') {
      seq += 1;
      void aiAcpCancelAuth().catch(() => undefined);
    }
    void closeTerminal();
    if (signedIn) onsignedin?.();
    onclose();
  }

  function onTerminalKeydown(event: KeyboardEvent): void {
    // Let Escape reach interactive login TUIs while the process is still running.
    if (event.key === 'Escape' && !exitStatus) event.stopPropagation();
  }

  function methodIcon(kind: AcpAuthMethod['kind']) {
    if (kind === 'terminal') return 'terminal' as const;
    if (kind === 'api_key') return 'key' as const;
    return 'logIn' as const;
  }

  function methodSubtitle(method: AcpAuthMethod): string {
    if (method.description) return method.description;
    if (method.fallback) return t('ai.login.fallbackHint');
    if (method.kind === 'api_key') return t('ai.login.apiKeyHint');
    if (method.kind === 'terminal') return t('ai.login.terminalHint');
    return t('ai.login.agentHint');
  }

  onMount(() => {
    void probe();
  });

  onDestroy(() => {
    destroyed = true;
    seq += 1;
    if (stage === 'probing' || stage === 'authenticating') {
      void aiAcpCancelAuth().catch(() => undefined);
    }
    void closeTerminal();
  });
</script>

<Dialog title={t('ai.login.title', { name: agentName })} width={stage === 'terminal' ? 680 : 480} onclose={close}>
  {#if stage === 'probing'}
    <div class="state-box" role="status">
      <span class="pulse-dot"></span>
      <div class="state-copy">
        <p class="state-title">{t('ai.login.probing')}</p>
        <p class="state-detail">{t('ai.login.probingHint')}</p>
      </div>
    </div>
  {:else if stage === 'authenticating'}
    <div class="state-box" role="status">
      <span class="pulse-dot"></span>
      <div class="state-copy">
        <p class="state-title">{t('ai.login.authenticating', { method: activeMethod?.name ?? agentName })}</p>
        <p class="state-detail">{t('ai.login.authenticatingHint')}</p>
      </div>
    </div>
  {:else if stage === 'terminal' && activeMethod}
    <div class="term-bar">
      {#if activeMethod.command_preview}
        <code class="mono cmd-chip" title={activeMethod.command_preview}>{activeMethod.command_preview}</code>
      {/if}
      <span class="term-status" class:done={exitStatus?.code === 0} class:failed={!!exitStatus && exitStatus.code !== 0}>
        {#if !exitStatus}
          <span class="pulse-dot small"></span>{t('ai.login.terminalRunning')}
        {:else if exitStatus.code === 0}
          <Icon name="check" size={13} />{t('ai.login.terminalDone')}
        {:else}
          <Icon name="warning" size={13} />{exitStatus.code !== null ? t('ai.login.terminalFailed', { code: exitStatus.code }) : t('ai.login.terminalStopped')}
        {/if}
      </span>
    </div>
    <div class="terminal-box" onkeydown={onTerminalKeydown} role="presentation">
      {#key terminalRun}
        <TermView
          open={(size, onOutput) => aiAcpLoginTerminal(agent, activeMethod!.id, size, onOutput)}
          onsession={(id) => (termSessionId = id)}
          onexit={onTerminalExit}
          onerror={(failure) => (error = errorText(failure))}
        />
      {/key}
    </div>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  {:else}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    {#if probeError}
      <p class="warn-note" role="status">{t('ai.login.probeWarning', { error: probeError })}</p>
    {/if}
    {#if methods.length > 0}
      <p class="intro">{t('ai.login.chooseHint')}</p>
      <div class="method-list">
        {#each methods as method (method.id)}
          <button type="button" class="method-card" onclick={() => void choose(method)}>
            <span class="method-icon" class:accent={method.kind !== 'api_key'}>
              <Icon name={methodIcon(method.kind)} size={15} />
            </span>
            <span class="method-copy">
              <span class="method-head">
                <span class="method-name">{method.name}</span>
                <span class="method-tag">{t(`ai.login.kind.${method.kind}`)}</span>
              </span>
              <span class="method-desc">{methodSubtitle(method)}</span>
              {#if method.command_preview}
                <code class="mono method-cmd">{method.command_preview}</code>
              {/if}
            </span>
            <span class="method-chevron"><Icon name="chevron" size={13} /></span>
          </button>
        {/each}
      </div>
    {:else}
      <div class="empty-box">
        <p class="empty-title">{t('ai.login.noMethods')}</p>
        <p class="state-detail">{t('ai.login.noMethodsHint')}</p>
        {#if onapikey}
          <button type="button" class="secondary" onclick={() => { onclose(); onapikey(); }}>{t('ai.login.configureKey')}</button>
        {/if}
      </div>
    {/if}
  {/if}

  {#snippet actions()}
    {#if stage === 'probing'}
      <button type="button" class="secondary" onclick={close}>{t('ai.cancel')}</button>
    {:else if stage === 'authenticating'}
      <button type="button" class="secondary" onclick={() => void cancelAuth()}>{t('ai.cancel')}</button>
    {:else if stage === 'terminal'}
      {#if methods.length > 1}
        <button type="button" class="plain left-action" onclick={() => void backToMethods()}>{t('ai.back')}</button>
      {/if}
      {#if exitStatus && exitStatus.code !== 0}
        <button type="button" class="secondary" onclick={() => void retryTerminal()}>{t('ai.sidebar.retryStartup')}</button>
      {/if}
      <button type="button" class={completed ? 'primary' : 'secondary'} onclick={close}>{t(completed ? 'ai.login.done' : 'common.close')}</button>
    {:else}
      <button type="button" class="plain left-action" onclick={() => void probe()}>{t('ai.login.refresh')}</button>
      <button type="button" class="secondary" onclick={close}>{t('common.close')}</button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .intro {
    margin: 0;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .warn-note {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--warning-soft);
    color: var(--warning);
    font-size: var(--text-footnote);
    overflow-wrap: anywhere;
  }
  .state-box {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: 0 0 0 0.5px var(--separator);
  }
  .state-copy {
    display: grid;
    gap: 4px;
    min-width: 0;
  }
  .state-title {
    margin: 0;
    color: var(--fg);
    font-weight: 600;
  }
  .state-detail {
    margin: 0;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    line-height: 1.45;
  }
  .pulse-dot {
    flex: none;
    width: 8px;
    height: 8px;
    margin-top: 5px;
    border-radius: 50%;
    background: var(--accent);
    animation: pulse 1.2s ease-in-out infinite;
  }
  .pulse-dot.small {
    width: 6px;
    height: 6px;
    margin-top: 0;
  }
  @keyframes pulse {
    50% { opacity: 0.3; }
  }
  @media (prefers-reduced-motion: reduce) {
    .pulse-dot { animation: none; }
  }
  .method-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .method-card {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    padding: var(--space-3);
    border: none;
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: 0 0 0 0.5px var(--border), var(--shadow-sm);
    color: var(--fg);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition:
      background-color var(--dur-fast) var(--ease-out),
      box-shadow var(--dur-fast) var(--ease-out);
  }
  .method-card:hover {
    background: var(--fill-hover);
    box-shadow: 0 0 0 0.5px color-mix(in srgb, var(--accent) 45%, var(--border)), var(--shadow-sm);
  }
  .method-icon {
    display: grid;
    place-items: center;
    flex: none;
    width: 32px;
    height: 32px;
    border-radius: var(--radius-sm);
    background: var(--fill);
    color: var(--fg-muted);
  }
  .method-icon.accent {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .method-copy {
    display: grid;
    gap: 3px;
    flex: 1;
    min-width: 0;
  }
  .method-head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
  }
  .method-name {
    font-weight: 600;
    color: var(--fg);
  }
  .method-tag {
    padding: 1px 6px;
    border-radius: var(--radius-xs);
    background: var(--fill);
    color: var(--fg-muted);
    font-size: var(--text-caption);
    font-weight: 500;
  }
  .method-desc {
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    overflow-wrap: anywhere;
  }
  .method-cmd {
    justify-self: start;
    max-width: 100%;
    padding: 2px 6px;
    border-radius: var(--radius-xs);
    background: var(--fill);
    color: var(--fg-muted);
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .method-chevron {
    display: flex;
    flex: none;
    color: var(--fg-subtle);
  }
  .empty-box {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: 0 0 0 0.5px var(--separator);
  }
  .empty-title {
    margin: 0;
    font-weight: 600;
  }
  .term-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: var(--space-2);
    min-width: 0;
  }
  .cmd-chip {
    min-width: 0;
    max-width: 100%;
    padding: 2px 8px;
    border-radius: var(--radius-xs);
    background: var(--fill);
    color: var(--fg-muted);
    font-size: var(--text-caption);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .term-status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--fg-muted);
    font-size: var(--text-footnote);
    font-weight: 500;
  }
  .term-status.done {
    color: var(--success);
  }
  .term-status.failed {
    color: var(--danger);
  }
  .terminal-box {
    height: 340px;
    border-radius: var(--radius-sm);
    overflow: hidden;
    box-shadow: 0 0 0 0.5px var(--border);
  }
  .mono {
    font-family: var(--font-mono);
  }
  .left-action {
    margin-right: auto;
  }
</style>
