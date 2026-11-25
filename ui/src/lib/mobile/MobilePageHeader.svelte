<script lang="ts">
  import type { Snippet } from 'svelte';
  import { t } from '../i18n';
  import IconButton from '../ui/IconButton.svelte';

  let { title, large = false, onback, actions }: {
    title: string;
    large?: boolean;
    onback?: () => void;
    actions?: Snippet;
  } = $props();
</script>

<header class:large>
  {#if onback}
    <IconButton icon="back" label={t('mobile.back')} onclick={onback} />
  {/if}
  <h1>{title}</h1>
  {#if actions}
    <div class="actions">{@render actions()}</div>
  {/if}
</header>

<style>
  header {
    position: sticky;
    top: 0;
    z-index: 1;
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--space-2);
    min-height: 48px;
    padding-inline: var(--space-4);
    background: var(--material-bar);
    box-shadow: inset 0 -0.5px 0 var(--separator);
  }
  h1 {
    flex: 1;
    min-width: 0;
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-title);
    font-weight: 700;
  }
  .large {
    padding-block: var(--space-2);
  }
  .large h1 {
    font-size: var(--text-large);
  }
  .actions {
    display: flex;
    flex: none;
    gap: var(--space-1);
  }
  header :global(.icon-button) {
    width: var(--touch-target, 44px);
    height: var(--touch-target, 44px);
  }
</style>
