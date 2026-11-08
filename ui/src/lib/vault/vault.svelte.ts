// Vault lock state shared by the UI. Unlock is only requested by user actions that need secrets.
import { onVaultState, vaultResetSecrets, vaultStatus, vaultUnlock, type LockState } from '../ipc';

class VaultState {
  state = $state<LockState>({ state: 'locked', reason: { kind: 'not_attempted' } });

  get unlocked(): boolean {
    return this.state.state === 'unlocked';
  }

  get keyLost(): boolean {
    return this.state.state === 'locked' && this.state.reason.kind === 'key_lost';
  }

  async init(): Promise<void> {
    await onVaultState((next) => (this.state = next));
    this.state = await vaultStatus();
  }

  async unlock(): Promise<LockState> {
    this.state = await vaultUnlock();
    return this.state;
  }

  async reset(): Promise<LockState> {
    this.state = await vaultResetSecrets();
    return this.state;
  }
}

export const vault = new VaultState();
