// The active message catalogue, held in reactive state so text rendered through `t` updates
// when the language changes.
import { en, type Messages } from './en';

export const locale = $state<{ messages: Messages }>({ messages: en });
