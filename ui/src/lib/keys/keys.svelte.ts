// The key library as the UI sees it: names and public halves only.
import { keyList, type KeyId, type SshKey } from '../ipc';

class KeyStore {
  items = $state<SshKey[]>([]);

  async refresh(): Promise<void> {
    this.items = await keyList();
  }

  find(id: KeyId): SshKey | undefined {
    return this.items.find((key) => key.id === id);
  }
}

export const keys = new KeyStore();
