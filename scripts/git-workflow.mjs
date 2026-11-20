#!/usr/bin/env node
import { spawnSync } from 'node:child_process';
import { readFileSync, realpathSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

// Conventional Branch 1.1.0 grammar with this project's purpose-prefix subset.
const branchPattern = /^(?:main|(?:feat|fix|hotfix|release|chore)\/[a-z0-9]+(?:\.[a-z0-9]+)*(?:-[a-z0-9]+(?:\.[a-z0-9]+)*)*)$/u;
const headerPattern = /^[a-z][a-z0-9-]*\([a-z0-9][a-z0-9._/-]*\)!?: \S.*$/u;
const signaturePattern = /^\s*(?:-\s*)?(?:(?:co-authored|signed-off|reviewed|acked|tested|reported|suggested|helped|mentored)-by|author|committer)\s*:/iu;

export function validateBranchName(name) {
  return typeof name === 'string' && name === name.trim() && branchPattern.test(name)
    ? []
    : [`分支名 ${JSON.stringify(name)} 不符合项目约定：使用 feat/、fix/、hotfix/、release/、chore/ 加小写描述，主分支为 main。`];
}

// Input is the message body after Git removes comments and normalizes whitespace.
export function validateCommitMessage(message) {
  const lines = message.replace(/\r\n?/gu, '\n').replace(/\n+$/u, '').split('\n');
  const errors = [];
  if (!headerPattern.test(lines[0])) {
    errors.push('提交标题须为 type(scope): subject；type 和 scope 使用小写标识，scope 与 subject 必填，可在冒号前加 !。');
  }
  if (lines.some((line) => signaturePattern.test(line))) {
    errors.push('提交消息不得包含 Co-Authored-By、Signed-off-by 等署名行。');
  }
  if (lines.length > 1 && lines[1] !== '') {
    errors.push('提交标题与 body 之间须空一行。');
  }
  if (lines.slice(2).some((line) => line !== '' && !/^- \S.*$/u.test(line))) {
    errors.push('提交 body 的每个非空行须为 - 开头的列表项，每点一行。');
  }
  return errors;
}

function git(args, options = {}) {
  const result = spawnSync('git', args, { encoding: 'utf8', ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(result.stderr.trim() || result.stdout.trim() || `git ${args[0]} 执行失败。`);
  }
  return result.stdout;
}

function currentBranch(allowDetached = false) {
  const result = spawnSync('git', ['symbolic-ref', '--quiet', '--short', 'HEAD'], { encoding: 'utf8' });
  if (result.error) throw result.error;
  if (allowDetached && result.status === 1) return null;
  if (result.status !== 0) {
    throw new Error(result.stderr.trim() || '当前处于 detached HEAD；请先创建符合项目规范的任务分支。');
  }
  return result.stdout.trim();
}

function assertValid(errors) {
  if (errors.length) throw new Error(errors.join('\n'));
}

function commentPrefix() {
  for (const key of ['core.commentString', 'core.commentChar']) {
    const result = spawnSync('git', ['config', '--null', '--get', key], { encoding: 'utf8' });
    if (result.error) throw result.error;
    if (result.status === 1) continue;
    if (result.status !== 0) throw new Error(result.stderr.trim() || `无法读取 ${key}。`);
    const value = result.stdout.replace(/\0$/u, '');
    return { value, auto: key === 'core.commentChar' && value === 'auto' };
  }
  return { value: '#', auto: false };
}

function cleanCommitMessage(message) {
  const scissors = ' ------------------------ >8 ------------------------';
  const lines = message.replace(/\r\n?/gu, '\n').split('\n');
  const configured = commentPrefix();
  let prefix = configured.value;
  const args = [];
  if (configured.auto) {
    const marker = lines.find((line) => '#;@!$%^&|:'.includes(line[0]) && line.slice(1) === scissors);
    prefix = marker?.[0] ?? '#';
    args.push('-c', `core.commentChar=${prefix}`);
  }
  const cut = lines.indexOf(prefix + scissors);
  const input = (cut === -1 ? lines : lines.slice(0, cut)).join('\n');
  return git([...args, 'stripspace', '--strip-comments'], { input });
}

function checkPush(input) {
  for (const line of input.split(/\r?\n/u)) {
    if (!line.trim()) continue;
    const fields = line.trim().split(/\s+/u);
    if (fields.length !== 4 || !fields[0] || !fields[2]
      || !/^(?:[a-f0-9]{40}|[a-f0-9]{64})$/iu.test(fields[1])
      || !/^(?:[a-f0-9]{40}|[a-f0-9]{64})$/iu.test(fields[3])) {
      throw new Error('pre-push 输入须为 Git 的四字段引用记录。');
    }
    const [localRef, localOid, remoteRef] = fields;
    if (/^0+$/u.test(localOid) || !remoteRef.startsWith('refs/heads/')) continue;
    if (localRef === 'HEAD' || localRef === '@') {
      const branch = currentBranch(true);
      if (branch !== null) assertValid(validateBranchName(branch));
    }
    for (const ref of new Set([localRef, remoteRef])) {
      if (ref.startsWith('refs/heads/')) {
        assertValid(validateBranchName(ref.slice('refs/heads/'.length)));
      }
    }
  }
}

function main(args) {
  const [command, ...rest] = args;
  switch (command) {
    case 'branch':
      if (rest.length > 1) throw new Error('用法：pnpm check:branch [name]');
      assertValid(validateBranchName(rest[0] ?? currentBranch()));
      break;
    case 'commit-msg':
    case 'hook-commit-msg': {
      if (rest.length !== 1) throw new Error('用法：pnpm check:commit <message-file>');
      const message = readFileSync(rest[0], 'utf8');
      const cleaned = cleanCommitMessage(message);
      assertValid(validateCommitMessage(cleaned));
      // The committed message must match validation even with Git cleanup overrides.
      if (command === 'hook-commit-msg' && message !== cleaned) writeFileSync(rest[0], cleaned);
      break;
    }
    case 'pre-commit':
      if (rest.length) throw new Error('pre-commit 不接受参数。');
      assertValid(validateBranchName(currentBranch()));
      git(['diff', '--cached', '--check']);
      break;
    case 'pre-push':
      checkPush(readFileSync(0, 'utf8'));
      break;
    default:
      throw new Error('用法：git-workflow.mjs <branch [name] | commit-msg <file> | hook-commit-msg <file> | pre-commit | pre-push>');
  }
}

if (process.argv[1] && realpathSync(resolve(process.argv[1])) === realpathSync(fileURLToPath(import.meta.url))) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
