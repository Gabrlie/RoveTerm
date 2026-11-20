#!/usr/bin/env node
import { spawnSync } from 'node:child_process';
import { accessSync, constants, existsSync, readdirSync, realpathSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const sourceRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

function git(root, args, missingAllowed = false) {
  const result = spawnSync('git', ['-C', root, ...args], { encoding: 'utf8' });
  if (result.error) throw result.error;
  if (missingAllowed && result.status === 1) return null;
  if (result.status !== 0) {
    throw new Error(result.stderr.trim() || result.stdout.trim() || `git ${args[0]} 执行失败。`);
  }
  return result.stdout.trim();
}

function isExecutableFile(path) {
  if (!statSync(path).isFile()) return false;
  try {
    accessSync(path, constants.X_OK);
    return true;
  } catch (error) {
    if (error.code === 'EACCES') return false;
    throw error;
  }
}

// Install only in this source checkout; linked worktrees share the local Git config.
export function installGitHooks(root = sourceRoot) {
  root = realpathSync(root);
  if (!existsSync(join(root, '.git'))) {
    return { installed: false, reason: '源码目录没有 .git，跳过 Git hooks 安装。' };
  }
  const topLevel = realpathSync(git(root, ['rev-parse', '--show-toplevel']));
  if (root !== topLevel) throw new Error('Git 工作区根目录与源码目录不一致，未修改 hooks 配置。');
  const hooksDir = join(root, '.githooks');
  for (const name of ['pre-commit', 'commit-msg', 'pre-push']) {
    if (!isExecutableFile(join(hooksDir, name))) {
      throw new Error(`项目钩子 .githooks/${name} 缺失或不可执行，未修改 hooks 配置。`);
    }
  }

  const configured = git(root, ['config', '--path', '--get', 'core.hooksPath'], true);
  if (configured !== null && resolve(root, configured) !== hooksDir) {
    throw new Error(`已有自定义 core.hooksPath=${JSON.stringify(configured)}，请先整合现有钩子；未覆盖配置。`);
  }
  if (configured === null) {
    const commonDir = resolve(root, git(root, ['rev-parse', '--git-common-dir']));
    const defaultHooksDir = join(commonDir, 'hooks');
    const conflicts = existsSync(defaultHooksDir)
      ? readdirSync(defaultHooksDir).filter((name) => !name.endsWith('.sample') && isExecutableFile(join(defaultHooksDir, name)))
      : [];
    if (conflicts.length) {
      throw new Error(`已有有效 Git hooks：${conflicts.join('、')}，请先整合现有钩子；未修改 hooks 配置。`);
    }
  }
  git(root, ['config', '--local', 'core.hooksPath', '.githooks']);
  return { installed: true };
}

if (process.argv[1] && realpathSync(resolve(process.argv[1])) === realpathSync(fileURLToPath(import.meta.url))) {
  try {
    const result = installGitHooks();
    console.log(result.installed ? 'Git hooks 已安装：.githooks' : result.reason);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
