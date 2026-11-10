<script lang="ts">
  // Replaces the WebView's native context menu everywhere. Views with their own menu prevent
  // the default first; text fields get cut / copy / paste / select all; elsewhere right-click
  // does nothing.
  import ContextMenu, { type MenuItem } from './ContextMenu.svelte';
  import { t } from '../i18n';
  import { readClipboard } from './clipboard';

  type Field = HTMLInputElement | HTMLTextAreaElement;

  let menu = $state<{ x: number; y: number; field: Field } | null>(null);

  const TEXT_INPUTS = new Set(['text', 'search', 'url', 'email', 'tel', 'password', 'number', '']);

  function editable(target: EventTarget | null): Field | null {
    if (target instanceof HTMLTextAreaElement) return target;
    if (target instanceof HTMLInputElement && TEXT_INPUTS.has(target.type)) return target;
    return null;
  }

  function selected(field: Field): string {
    // `number` inputs have no selection API; they copy their whole value.
    if (field.selectionStart === null || field.selectionEnd === null) return field.value;
    return field.value.slice(field.selectionStart, field.selectionEnd);
  }

  /** Replaces the selection and tells bindings about it. */
  function replace(field: Field, text: string): void {
    if (field.selectionStart === null || field.selectionEnd === null) field.value = text;
    else field.setRangeText(text, field.selectionStart, field.selectionEnd, 'end');
    field.dispatchEvent(new Event('input', { bubbles: true }));
  }

  function items(field: Field): MenuItem[] {
    const text = selected(field);
    const writable = !field.readOnly && !field.disabled;
    const secret = field instanceof HTMLInputElement && field.type === 'password';
    const list: MenuItem[] = [];
    if (writable && text && !secret) {
      list.push({
        label: t('edit.cut'),
        onselect: () => {
          void navigator.clipboard.writeText(text).then(() => replace(field, ''));
        },
      });
    }
    if (text && !secret) {
      list.push({ label: t('edit.copy'), icon: 'copy', onselect: () => void navigator.clipboard.writeText(text) });
    }
    if (writable) {
      list.push({
        label: t('edit.paste'),
        icon: 'paste',
        onselect: () => {
          void readClipboard().then((pasted) => {
            field.focus();
            if (pasted) replace(field, pasted);
          });
        },
      });
    }
    list.push({ label: t('edit.selectAll'), separated: list.length > 0, onselect: () => (field.focus(), field.select()) });
    return list;
  }

  function onContextMenu(event: MouseEvent): void {
    if (event.defaultPrevented) return;
    event.preventDefault();
    const field = editable(event.target);
    menu = field ? { x: event.clientX, y: event.clientY, field } : null;
  }
</script>

<svelte:window oncontextmenu={onContextMenu} />

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={items(menu.field)} onclose={() => (menu = null)} />
{/if}
