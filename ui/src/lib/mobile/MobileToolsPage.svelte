<script lang="ts">
  import { forwardStatuses } from '../forwards/forwards.svelte';
  import HostIcon from '../hosts/HostIcon.svelte';
  import { hosts } from '../hosts/hosts.svelte';
  import { hostSubtitle } from '../hosts/route';
  import { t } from '../i18n';
  import type { HostEntry } from '../ipc';
  import { keys } from '../keys/keys.svelte';
  import Dialog from '../ui/Dialog.svelte';
  import Icon from '../ui/Icon.svelte';
  import type { IconName } from '../ui/icons';
  import MobilePageHeader from './MobilePageHeader.svelte';

  interface Props {
    activeTransfers: number;
    activeForwards: number;
    onkeys: () => void;
    onimport: () => void;
    onquick: () => void;
    ontransfers: () => void;
    onforwards: (host: HostEntry) => void;
    onsnippets: () => void;
    onnotice: (message: string) => void;
  }

  let { activeTransfers, activeForwards, onkeys, onimport, onquick, ontransfers, onforwards, onsnippets, onnotice }: Props = $props();
  let choosingHost = $state(false);

  function chooseForwards(): void {
    const [first, second] = hosts.items;
    if (!first) onnotice(t('mobile.tools.noHostsForwards'));
    else if (!second) onforwards(first);
    else choosingHost = true;
  }
</script>

{#snippet countBadge(count: number)}
  {#if count > 0}
    <span class="badge" role="img" aria-label={t('mobile.tools.activeCount', { n: count })}>{count}</span>
  {/if}
{/snippet}

{#snippet tool(icon: IconName, tint: string, title: string, description: string, action: () => void, summary = '', count = 0)}
  <button type="button" class="row" onclick={action}>
    <span class="glyph" style:--tint={tint}><Icon name={icon} size={16} /></span>
    <span class="copy">
      <strong>{title}</strong>
      <small>{description}</small>
    </span>
    {#if summary}<span class="summary">{summary}</span>{/if}
    {@render countBadge(count)}
    <span class="chevron"><Icon name="chevron" size={15} /></span>
  </button>
{/snippet}

<div class="page">
  <MobilePageHeader title={t('mobile.tools.title')} large />
  <div class="content">
    <section>
      <h2 class="section-label">{t('mobile.tools.groupAssets')}</h2>
      <div class="form-group">
        {@render tool('key', 'var(--sys-orange)', t('keys.title'), t('mobile.tools.keysDesc'), onkeys, t('mobile.tools.keysCount', { n: keys.items.length }))}
        {@render tool('importConfig', 'var(--sys-blue)', t('action.host.import'), t('mobile.tools.importDesc'), onimport)}
      </div>
    </section>
    <section>
      <h2 class="section-label">{t('mobile.tools.groupConnect')}</h2>
      <div class="form-group">
        {@render tool('quickConnect', 'var(--sys-green)', t('action.host.quickConnect'), t('mobile.tools.quickDesc'), onquick)}
        {@render tool('transfer', 'var(--sys-teal)', t('action.panel.transfers'), t('mobile.tools.transfersDesc'), ontransfers, '', activeTransfers)}
        {@render tool('forwards', 'var(--sys-purple)', t('host.forwards'), t('mobile.tools.forwardsDesc'), chooseForwards, '', activeForwards)}
        {@render tool('snippets', 'var(--sys-pink)', t('action.panel.snippets'), t('mobile.tools.snippetsDesc'), onsnippets)}
      </div>
    </section>
  </div>
</div>

{#if choosingHost}
  <Dialog presentation="sheet" title={t('mobile.tools.chooseHostForwards')} onclose={() => (choosingHost = false)}>
    <div class="form-group">
      {#each hosts.items as host (host.id)}
        <button type="button" class="row" onclick={() => { choosingHost = false; onforwards(host); }}>
          <HostIcon os={host.os} size={28} />
          <span class="copy">
            <strong>{host.name}</strong>
            <small>{hostSubtitle(host)}</small>
          </span>
          {@render countBadge(forwardStatuses.running(host.id))}
          <span class="chevron"><Icon name="chevron" size={15} /></span>
        </button>
      {/each}
    </div>
    {#snippet actions()}
      <button type="button" class="secondary" onclick={() => (choosingHost = false)}>{t('common.cancel')}</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .content {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--space-5);
    min-height: 0;
    padding: var(--space-3) var(--space-4) var(--space-6);
    overflow-y: auto;
  }
  section {
    flex: none;
  }
  .section-label {
    margin-bottom: var(--space-2);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    min-height: 58px;
    padding: var(--space-2) var(--space-3);
    border: 0;
    background: transparent;
    color: var(--fg);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .row + .row {
    box-shadow: inset 0 0.5px 0 var(--separator);
  }
  .row:active {
    background: var(--fill-pressed);
  }
  .row:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }
  .glyph {
    display: grid;
    flex: none;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
    background: var(--tint);
    color: white;
  }
  .copy {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
    overflow-wrap: anywhere;
  }
  small, .summary, .chevron {
    color: var(--fg-muted);
    font-size: var(--text-footnote);
  }
  .summary {
    text-align: end;
  }
  .chevron, .badge {
    flex: none;
  }
  .badge {
    min-width: 16px;
    padding-inline: var(--space-1);
    border-radius: var(--radius-pill);
    background: var(--accent-soft);
    color: var(--accent);
    font-size: var(--text-caption);
    font-variant-numeric: tabular-nums;
    text-align: center;
  }
</style>
