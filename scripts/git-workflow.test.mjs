import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { validateBranchName, validateCommitMessage } from './git-workflow.mjs';

const sourceRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const zero = '0'.repeat(40);
const object = '1'.repeat(40);

function execute(command, args, cwd, env, input) {
  const result = spawnSync(command, args, { cwd, env, input, encoding: 'utf8' });
  assert.ifError(result.error);
  return result;
}

function succeeded(result) {
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  return result.stdout.trim();
}

function failed(result) {
  assert.notEqual(result.status, 0, `${result.stdout}\n${result.stderr}`);
}

function copyWorkflow(root) {
  mkdirSync(join(root, 'scripts'), { recursive: true });
  for (const name of ['git-workflow.mjs', 'install-git-hooks.mjs']) {
    copyFileSync(join(sourceRoot, 'scripts', name), join(root, 'scripts', name));
  }
  cpSync(join(sourceRoot, '.githooks'), join(root, '.githooks'), { recursive: true });
}

function fixture(t, { git = true } = {}) {
  const directory = mkdtempSync(join(tmpdir(), 'rove workflow-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const root = join(directory, 'repo with spaces');
  mkdirSync(root);
  const env = { ...process.env };
  for (const key of Object.keys(env)) {
    if (/^GIT_(?:DIR|WORK_TREE|INDEX_FILE|COMMON_DIR|CONFIG(?:_|$)|AUTHOR_|COMMITTER_)/.test(key)) delete env[key];
  }
  Object.assign(env, {
    GIT_CONFIG_NOSYSTEM: '1',
    GIT_CONFIG_GLOBAL: join(directory, 'gitconfig'),
    GIT_TERMINAL_PROMPT: '0',
    GIT_AUTHOR_NAME: 'Testing',
    GIT_AUTHOR_EMAIL: 'fixture@example.invalid',
    GIT_COMMITTER_NAME: 'Testing',
    GIT_COMMITTER_EMAIL: 'fixture@example.invalid',
  });
  writeFileSync(env.GIT_CONFIG_GLOBAL, '');
  copyWorkflow(root);
  const runGit = (...args) => execute('git', args, root, env);
  const script = (name, args = [], options = {}) => execute(
    process.execPath,
    [join(root, 'scripts', name), ...args],
    options.cwd ?? root,
    env,
    options.input,
  );
  if (git) {
    succeeded(runGit('init', '-b', 'chore/fixture'));
    succeeded(runGit('config', 'commit.gpgsign', 'false'));
    succeeded(runGit('config', 'core.autocrlf', 'false'));
    writeFileSync(join(root, 'tracked.txt'), 'original\n');
    succeeded(runGit('add', '.'));
    const disabledHooks = join(directory, 'disabled-hooks');
    mkdirSync(disabledHooks);
    succeeded(runGit('-c', `core.hooksPath=${disabledHooks}`, 'commit', '-m', 'chore(test): 初始化测试仓库'));
  }
  return {
    root, directory, env, git: runGit, script,
    install: () => script('install-git-hooks.mjs'),
    workflow: (args, options) => script('git-workflow.mjs', args, options),
    change: (content = 'changed\n') => {
      writeFileSync(join(root, 'tracked.txt'), content);
      succeeded(runGit('add', 'tracked.txt'));
    },
  };
}

test('分支规范接受任务类型、版本和主分支', () => {
  for (const name of ['main', 'feat/text-editor', 'fix/sftp-save', 'hotfix/security-patch', 'release/v0.1.0', 'chore/git-workflow', 'feat/issue-123-editor']) {
    assert.deepEqual(validateBranchName(name), [], name);
  }
});

test('分支规范拒绝项目白名单外的前缀和非法描述', () => {
  for (const name of ['', 'master', 'develop', 'dsh-35df0831-7f60', 'feature/editor', 'bugfix/save', 'codex/editor', 'claude/editor', 'ai/editor', 'feat/', 'feat/Editor', 'feat/text_editor', 'feat/text editor', 'feat/a--b', 'feat/a..b', 'feat/a.-b', 'feat/a-.b', 'feat/-a', 'feat/a-', 'feat/.a', 'feat/a.', 'feat/a/b', 'feat/a\nb', 'feat/task\n', 'feat/task\r', 'feat/task\t', ' feat/task', 'main\n']) {
    assert.ok(validateBranchName(name).length > 0, name);
  }
});

test('提交消息接受中文、作用域、破坏性标记和列表正文', () => {
  for (const message of [
    'fix(rove-fs): 修复保存失败',
    'feat(ui/files): 支持文本编辑\n',
    'refactor(rove-fs)!: 调整文件接口\n\n- 同步调用方\n- 更新接口文档\n',
    'chore(workflow): 规范开发交付\n\n- 校验分支\n\n- 校验提交格式\n',
    'test(rove_fs): 增加逻辑测试\r\n\r\n- 覆盖失败分支\r\n',
  ]) {
    assert.deepEqual(validateCommitMessage(message), [], message);
  }
});

test('提交消息拒绝缺失字段和非列表正文', () => {
  for (const message of [
    '', 'fix: 修复保存', 'fix(): 修复保存', 'Fix(ui): 修复保存', 'fix(UI): 修复保存', 'fix(ui): ',
    'fix(ui): 修复保存\n- 缺少正文分隔空行',
    'fix(ui): 修复保存\n\n正文不是列表',
    'fix(ui): 修复保存\n\n- ',
    'fix(ui): 修复保存\n\n- 第一条\n  第二行',
  ]) {
    assert.ok(validateCommitMessage(message).length > 0, message);
  }
});

test('提交消息拒绝列表内外的署名 trailers', () => {
  for (const signature of ['Co-Authored-By: Testing <fixture@example.invalid>', 'signed-off-by: Testing <fixture@example.invalid>', '- Co-Authored-By: Testing <fixture@example.invalid>', '- Signed-off-by: Testing <fixture@example.invalid>']) {
    assert.ok(validateCommitMessage(`chore(workflow): 校验提交\n\n${signature}`).length > 0, signature);
  }
});

test('CLI 读取实际工作区分支，也支持显式名称', t => {
  const f = fixture(t);
  succeeded(f.workflow(['branch']));
  succeeded(f.workflow(['branch', 'fix/save']));
  failed(f.workflow(['branch', 'dsh-invalid']));
  succeeded(f.git('switch', '-c', 'dsh-invalid'));
  failed(f.workflow(['branch']));
  succeeded(f.workflow(['branch', 'feat/explicit']));
  succeeded(f.git('checkout', '--detach'));
  failed(f.workflow(['branch']));
});

test('提交 CLI 在校验前按 Git 规则清理注释', t => {
  const f = fixture(t);
  const message = join(f.directory, 'message.txt');
  const defaultComments = '# 头部注释\nfix(ui): 修复保存\n\n# 正文注释\n- 保留中文正文\n';
  writeFileSync(message, defaultComments);
  succeeded(f.workflow(['commit-msg', message]));
  assert.equal(readFileSync(message, 'utf8'), defaultComments);
  succeeded(f.git('config', 'core.commentChar', ';'));
  const customComments = '; 头部注释\nfix(ui): 修复保存\n\n; 正文注释\n- 使用仓库指定注释字符\n';
  writeFileSync(message, customComments);
  succeeded(f.workflow(['commit-msg', message]));
  assert.equal(readFileSync(message, 'utf8'), customComments);
  writeFileSync(message, '修复保存\n');
  failed(f.workflow(['commit-msg', message]));
});

for (const { name, prefix, config } of [
  { name: '默认注释字符', prefix: '#', config: [] },
  { name: '自定义 core.commentChar', prefix: ';', config: [['core.commentChar', ';']] },
  { name: '多字符 core.commentString', prefix: ';;', config: [['core.commentChar', '#'], ['core.commentString', ';;']] },
]) {
  test(`真实提交按 ${name} 的 scissors 标记丢弃尾部 diff`, t => {
    const f = fixture(t);
    if (prefix.length > 1) {
      const probe = execute('git', ['-c', 'core.commentString=;;', 'stripspace', '--strip-comments'], f.root, f.env, ';; comment\n');
      if (succeeded(probe) !== '') return t.skip('当前 Git 未实现 core.commentString');
    }
    for (const [key, value] of config) succeeded(f.git('config', key, value));
    succeeded(f.install());
    f.change();
    f.env.GIT_EDITOR = 'true';
    const messageFile = join(f.directory, 'scissors-message.txt');
    const message = 'fix(ui): 修复保存\n\n- 保留合法正文';
    const withDiff = `${message}\n\n${prefix} ------------------------ >8 ------------------------\ndiff --git a/tracked.txt b/tracked.txt\n--- a/tracked.txt\n+++ b/tracked.txt\n@@ -1 +1 @@\n-original\n+changed\n`;
    writeFileSync(messageFile, withDiff);
    succeeded(f.workflow(['commit-msg', messageFile]));
    assert.equal(readFileSync(messageFile, 'utf8'), withDiff);
    succeeded(f.git('commit', '--cleanup=scissors', '--edit', '-F', messageFile));
    assert.equal(succeeded(f.git('log', '-1', '--format=%B')), message);
  });
}

test('非编辑 -F 提交只写入钩子已验证的正文', t => {
  const f = fixture(t);
  succeeded(f.install());
  f.change();
  const messageFile = join(f.directory, 'message-with-comments.txt');
  const message = 'fix(ui): 修复保存\n\n- 保留合法正文';
  writeFileSync(messageFile, `${message}\n\n# 这段注释不属于提交正文\n# ------------------------ >8 ------------------------\ndiff --git a/tracked.txt b/tracked.txt\n+这段尾部 diff 不属于提交正文\n`);
  succeeded(f.git('commit', '-F', messageFile));
  assert.equal(succeeded(f.git('log', '-1', '--format=%B')), message);
});

test('安装器可重复运行，并仅设置当前仓库配置', t => {
  const f = fixture(t);
  const originalGlobalConfig = readFileSync(f.env.GIT_CONFIG_GLOBAL, 'utf8');
  succeeded(f.install());
  succeeded(f.install());
  assert.equal(succeeded(f.git('config', '--local', '--get', 'core.hooksPath')), '.githooks');
  assert.equal(readFileSync(f.env.GIT_CONFIG_GLOBAL, 'utf8'), originalGlobalConfig);
});

test('通过符号链接目录启动时仍执行 CLI 和安装器', t => {
  const f = fixture(t);
  const alias = join(f.directory, 'checkout alias');
  symlinkSync(f.root, alias, process.platform === 'win32' ? 'junction' : 'dir');
  failed(execute(process.execPath, [join(alias, 'scripts', 'git-workflow.mjs'), 'branch', 'dsh-invalid'], alias, f.env));
  succeeded(execute(process.execPath, [join(alias, 'scripts', 'install-git-hooks.mjs')], alias, f.env));
  assert.equal(succeeded(f.git('config', '--local', '--get', 'core.hooksPath')), '.githooks');
});

test('安装器保留已有自定义 hooksPath', t => {
  const f = fixture(t);
  succeeded(f.git('config', '--local', 'core.hooksPath', '.custom-hooks'));
  failed(f.install());
  assert.equal(succeeded(f.git('config', '--local', '--get', 'core.hooksPath')), '.custom-hooks');
});

test('安装器不覆盖全局指定的 hooksPath', t => {
  const f = fixture(t);
  succeeded(f.git('config', '--global', 'core.hooksPath', '.global-hooks'));
  const original = readFileSync(f.env.GIT_CONFIG_GLOBAL, 'utf8');
  failed(f.install());
  assert.equal(f.git('config', '--local', '--get', 'core.hooksPath').status, 1);
  assert.equal(readFileSync(f.env.GIT_CONFIG_GLOBAL, 'utf8'), original);
});

test('安装器保留默认目录中的有效钩子', t => {
  const f = fixture(t);
  const hook = join(f.root, '.git', 'hooks', 'pre-commit');
  const content = '#!/bin/sh\nexit 0\n';
  writeFileSync(hook, content);
  chmodSync(hook, 0o755);
  failed(f.install());
  assert.equal(readFileSync(hook, 'utf8'), content);
  assert.equal(f.git('config', '--local', '--get', 'core.hooksPath').status, 1);
});

test('安装器允许没有 Git 元数据的源码归档', t => {
  const f = fixture(t, { git: false });
  succeeded(f.install());
});

test('源码归档嵌在其他仓库内时不会修改外层仓库', t => {
  const f = fixture(t);
  const archive = join(f.root, 'archive');
  mkdirSync(archive);
  copyWorkflow(archive);
  succeeded(execute(process.execPath, [join(archive, 'scripts', 'install-git-hooks.mjs')], archive, f.env));
  assert.equal(f.git('config', '--local', '--get', 'core.hooksPath').status, 1);
});

test('真实钩子拒绝非法分支，允许合规提交', t => {
  const f = fixture(t);
  succeeded(f.install());
  succeeded(f.git('switch', '-c', 'dsh-invalid'));
  f.change();
  failed(f.git('commit', '-m', 'fix(ui): 修复保存'));
  succeeded(f.git('branch', '-m', 'fix/save'));
  succeeded(f.git('commit', '-m', 'fix(ui): 修复保存\n\n- 保留保存结果'));
  assert.equal(succeeded(f.git('status', '--porcelain')), '');
});

test('真实钩子拒绝错误提交消息和署名', t => {
  const f = fixture(t);
  succeeded(f.install());
  f.change();
  failed(f.git('commit', '-m', '修复保存'));
  failed(f.git('commit', '-m', 'fix(ui): 修复保存\n\n- Co-Authored-By: Testing <fixture@example.invalid>'));
  succeeded(f.git('commit', '-m', 'fix(ui): 修复保存'));
});

test('真实 pre-commit 拒绝暂存差异中的空白错误', t => {
  const f = fixture(t);
  succeeded(f.install());
  f.change('trailing whitespace \t\n');
  failed(f.git('commit', '-m', 'fix(ui): 修复保存'));
  f.change('clean whitespace\n');
  succeeded(f.git('commit', '-m', 'fix(ui): 修复保存'));
});

test('linked worktree 在自己的目录安装和检查真实分支', t => {
  const f = fixture(t);
  const worktree = join(f.directory, 'linked worktree');
  succeeded(f.git('worktree', 'add', '-b', 'fix/linked', worktree, 'HEAD'));
  succeeded(execute(process.execPath, [join(worktree, 'scripts', 'install-git-hooks.mjs')], worktree, f.env));
  assert.equal(succeeded(f.git('config', '--local', '--get', 'core.hooksPath')), '.githooks');
  succeeded(execute('git', ['branch', '-m', 'dsh-linked'], worktree, f.env));
  failed(execute(process.execPath, [join(worktree, 'scripts', 'git-workflow.mjs'), 'branch'], worktree, f.env));
  writeFileSync(join(worktree, 'tracked.txt'), 'linked change\n');
  succeeded(execute('git', ['add', 'tracked.txt'], worktree, f.env));
  failed(execute('git', ['commit', '-m', 'fix(ui): 修复保存'], worktree, f.env));
  succeeded(execute('git', ['branch', '-m', 'fix/linked-save'], worktree, f.env));
  succeeded(execute('git', ['commit', '-m', 'fix(ui): 修复保存'], worktree, f.env));
  assert.equal(succeeded(f.git('branch', '--show-current')), 'chore/fixture');
});

test('pre-push 检查推送引用而非当前分支，并校验目标名称', t => {
  const f = fixture(t);
  succeeded(f.install());
  const remote = join(f.directory, 'remote.git');
  succeeded(execute('git', ['init', '--bare', remote], f.directory, f.env));
  succeeded(f.git('branch', 'dsh-source'));
  failed(f.git('push', remote, 'refs/heads/dsh-source:refs/heads/feat/target'));
  failed(f.git('push', remote, 'refs/heads/chore/fixture:refs/heads/dsh-target'));
  succeeded(f.git('push', remote, 'refs/heads/chore/fixture:refs/heads/chore/fixture'));
});

test('pre-push 解析 HEAD 的真实分支，同时允许推送其他合规分支', t => {
  const f = fixture(t);
  succeeded(f.install());
  const remote = join(f.directory, 'remote.git');
  succeeded(execute('git', ['init', '--bare', remote], f.directory, f.env));
  succeeded(f.git('switch', '-c', 'dsh-source'));
  failed(f.git('push', remote, 'HEAD:refs/heads/feat/target'));
  succeeded(f.git('push', remote, 'refs/heads/chore/fixture:refs/heads/chore/fixture'));
});

test('pre-push 在 detached HEAD 下允许合规目标并拒绝非法目标', t => {
  const f = fixture(t);
  succeeded(f.install());
  const remote = join(f.directory, 'remote.git');
  succeeded(execute('git', ['init', '--bare', remote], f.directory, f.env));
  succeeded(f.git('checkout', '--detach'));
  succeeded(f.git('push', remote, 'HEAD:refs/heads/feat/detached'));
  failed(f.git('push', remote, 'HEAD:refs/heads/dsh-detached'));
});

test('pre-push 放行标签及删除不合规的旧分支', t => {
  const f = fixture(t);
  succeeded(f.install());
  const remote = join(f.directory, 'remote.git');
  succeeded(execute('git', ['init', '--bare', remote], f.directory, f.env));
  succeeded(f.git('push', remote, 'refs/heads/chore/fixture:refs/heads/chore/fixture'));
  const head = succeeded(f.git('rev-parse', 'HEAD'));
  succeeded(execute('git', ['--git-dir', remote, 'update-ref', 'refs/heads/dsh-legacy', head], f.directory, f.env));
  succeeded(f.git('switch', '-c', 'dsh-current'));
  succeeded(f.git('tag', 'v0.1.0'));
  succeeded(f.git('push', remote, 'refs/tags/v0.1.0:refs/tags/v0.1.0'));
  succeeded(f.git('push', remote, ':refs/heads/dsh-legacy'));
});

test('pre-push 支持空输入及 SHA-256 引用', t => {
  const f = fixture(t);
  succeeded(f.workflow(['pre-push'], { input: '' }));
  const sha256 = 'a'.repeat(64);
  succeeded(f.workflow(['pre-push'], { input: `refs/heads/fix/save ${sha256} refs/heads/fix/save ${'0'.repeat(64)}\n` }));
  succeeded(f.workflow(['pre-push'], { input: `(delete) ${zero} refs/heads/dsh-old ${object}\n` }));
  failed(f.workflow(['pre-push'], { input: `refs/heads/chore/fixture ${object} refs/heads/dsh-target ${zero}\n` }));
});
