<script lang="ts">
  // A host's identity mark: initials on a gradient tile in a colour derived from the name.
  import { avatarColor, initials } from './avatar';

  let { name, size = 26 }: { name: string; size?: number } = $props();
</script>

<span class="avatar" style:--tint={avatarColor(name)} style:--size="{size}px" aria-hidden="true">{initials(name)}</span>

<style>
  .avatar {
    flex: none;
    display: grid;
    place-items: center;
    width: var(--size);
    height: var(--size);
    border-radius: calc(var(--size) * 0.27);
    background: linear-gradient(to bottom, color-mix(in srgb, var(--tint) 82%, #fff), var(--tint));
    box-shadow:
      inset 0 0.5px 0 rgb(255 255 255 / 0.3),
      0 0.5px 1.5px rgb(0 0 0 / 0.18);
    color: #fff;
    font-size: calc(var(--size) * 0.38);
    font-weight: 700;
    letter-spacing: 0.02em;
    user-select: none;
  }
</style>
