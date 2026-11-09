<script lang="ts">
  // A host's icon: its operating system's logo on the brand colour, or a server glyph on grey
  // until the system is detected (or when it is not recognised).
  import Icon from '../ui/Icon.svelte';
  import type { HostOs } from '../ipc';
  import { osIcon } from './osIcons';

  let { os, size = 26 }: { os: HostOs | null; size?: number } = $props();

  const icon = $derived(osIcon(os));
</script>

<span
  class="host-icon"
  class:light={icon?.light}
  style:--tint={icon?.color ?? 'var(--sys-graphite)'}
  style:--size="{size}px"
  title={os?.name || undefined}
  aria-hidden="true"
>
  {#if icon}
    <svg viewBox="0 0 24 24" width={Math.round(size * 0.58)} height={Math.round(size * 0.58)}><path d={icon.path} /></svg>
  {:else}
    <Icon name="server" size={Math.round(size * 0.58)} />
  {/if}
</span>

<style>
  .host-icon {
    flex: none;
    display: grid;
    place-items: center;
    width: var(--size);
    height: var(--size);
    border-radius: calc(var(--size) * 0.27);
    background: linear-gradient(to bottom, color-mix(in srgb, var(--tint) 84%, #fff), var(--tint));
    /* The ring keeps black and very dark logos' tiles visible on dark surfaces. */
    box-shadow:
      inset 0 0 0 0.5px rgb(255 255 255 / 0.18),
      0 0.5px 1.5px rgb(0 0 0 / 0.2);
    color: #fff;
  }
  .host-icon.light {
    color: #1d1d1f;
  }
  svg {
    fill: currentColor;
  }
</style>
