// Window appearance that depends on the platform.
import { windowSetMaterial } from '../ipc';

/** Shows the system window material behind the translucent surfaces when the platform has one;
 *  `<html data-material>` tells the styles whether it is shown. */
export async function applyWindowMaterial(enabled: boolean): Promise<void> {
  const shown = await windowSetMaterial(enabled).catch(() => false);
  document.documentElement.dataset.material = shown ? 'on' : 'off';
}
