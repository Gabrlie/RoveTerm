import { migrateKeyBarV1, parseKeyBarLayout, serializeKeyBarLayout, type KeyBarLayout } from './keybar';

const STORAGE_KEY = 'ui.keybar.layout.v2';
const LEGACY_STORAGE_KEY = 'ui.keybar.layout.v1';

function storeLayout(layout: KeyBarLayout): void {
  try {
    localStorage.setItem(STORAGE_KEY, serializeKeyBarLayout(layout));
  } catch {
    // Keep the current layout usable when device storage is unavailable.
  }
}

function loadLayout(): KeyBarLayout {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved !== null) return parseKeyBarLayout(saved);
    const legacy = localStorage.getItem(LEGACY_STORAGE_KEY);
    if (legacy !== null) {
      const layout = migrateKeyBarV1(legacy);
      storeLayout(layout);
      return layout;
    }
  } catch {
    return parseKeyBarLayout(null);
  }
  return parseKeyBarLayout(null);
}

class KeyBarPrefs {
  #layout = $state.raw<KeyBarLayout>(loadLayout());

  get layout(): KeyBarLayout {
    return this.#layout;
  }

  setExpanded(expanded: boolean): void {
    this.save({ ...this.#layout, expanded });
  }

  save(next: KeyBarLayout): void {
    this.#layout = parseKeyBarLayout(next);
    storeLayout(this.#layout);
  }

  reset(): void {
    this.save(parseKeyBarLayout(null));
  }
}

export const keybarPrefs = new KeyBarPrefs();
