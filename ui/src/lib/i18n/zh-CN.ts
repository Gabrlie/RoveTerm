// Simplified Chinese UI strings. Keys must match `en.ts`.
import type { Messages } from './en';

export const zhCN: Messages = {
  'app.title': 'RoveTerm',
  'terminal.exited': '进程已退出（代码 {code}）',
  'terminal.exitedSignal': '进程被信号 {signal} 终止',
  'terminal.openFailed': '无法打开终端：{message}',
  'terminal.reconnectHint': '按 Enter 重新连接',
  'error.ssh.connect': '无法连接到服务器',
  'error.ssh.timeout': '连接超时',
  'error.ssh.auth_failed': '认证失败',
  'error.ssh.credentials_required': '需要密码',
  'error.ssh.host_key_mismatch': '服务器主机密钥已变更，连接已被拒绝',
  'error.ssh.host_key_declined': '已拒绝该主机密钥',
  'error.ssh.host_key': '主机密钥校验失败',
  'error.ssh.key': '私钥无法读取或解密',
  'error.ssh.agent': 'SSH agent 不可用',
  'error.ssh.protocol': 'SSH 协议错误',
  'error.vault.locked': '凭据库已锁定',
  'error.input.invalid': '输入无效',
  'error.unknown': '发生错误',
};
