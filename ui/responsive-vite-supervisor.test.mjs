import assert from 'node:assert/strict';
import { execFile, spawn } from 'node:child_process';
import { once } from 'node:events';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { terminateDetachedProcessGroup } from './process-group-cleanup.mjs';

const supervisorPath = fileURLToPath(new URL('./responsive-vite-supervisor.mjs', import.meta.url));

async function forceTreeExit(child) {
  if (child.exitCode !== null || !child.pid) return;
  if (process.platform === 'win32') {
    await new Promise(resolve => {
      execFile('taskkill', ['/pid', String(child.pid), '/t', '/f'], { windowsHide: true }, () => resolve());
    });
  } else {
    try { process.kill(-child.pid, 'SIGKILL'); } catch (error) {
      if (error?.code !== 'ESRCH') throw error;
    }
  }
  if (child.exitCode === null) await once(child, 'exit');
}

test('supervisor keeps its process identity after an early Vite exit', async t => {
  const supervisor = spawn(process.execPath, [
    supervisorPath,
    '--host', '127.0.0.1',
    '--port', 'not-a-port',
    '--strictPort',
  ], {
    cwd: fileURLToPath(new URL('.', import.meta.url)),
    stdio: ['ignore', 'ignore', 'pipe'],
  });
  t.after(() => {
    if (supervisor.exitCode === null) supervisor.kill('SIGKILL');
  });

  let stderr = '';
  await new Promise((resolve, reject) => {
    const timeout = setTimeout(
      () => reject(new Error(`supervisor did not report Vite exit: ${stderr}`)),
      5_000,
    );
    supervisor.stderr.on('data', chunk => {
      stderr += chunk.toString();
      if (!stderr.includes('AIRP_VITE_EXIT')) return;
      clearTimeout(timeout);
      resolve();
    });
    supervisor.on('error', error => {
      clearTimeout(timeout);
      reject(error);
    });
  });

  assert.equal(supervisor.exitCode, null);
  assert.match(stderr, /AIRP_VITE_EXIT code=1 signal=null/);
});

test('supervisor keeps the group identity until full-tree forced cleanup', {
  skip: process.platform === 'win32',
}, async t => {
  const supervisor = spawn(process.execPath, [
    supervisorPath,
    '--host', '127.0.0.1',
    '--port', '0',
  ], {
    cwd: fileURLToPath(new URL('.', import.meta.url)),
    detached: process.platform !== 'win32',
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  t.after(() => forceTreeExit(supervisor));

  let output = '';
  await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error(`Vite did not start: ${output}`)), 5_000);
    const collect = chunk => {
      output += chunk.toString();
      if (!output.includes('http://127.0.0.1:')) return;
      clearTimeout(timeout);
      resolve();
    };
    supervisor.stdout.on('data', collect);
    supervisor.stderr.on('data', collect);
    supervisor.on('error', error => {
      clearTimeout(timeout);
      reject(error);
    });
  });

  const outcome = await terminateDetachedProcessGroup(supervisor.pid, {
    termTimeoutMs: 1_000,
    killTimeoutMs: 1_000,
    hasTerminated: () => output.includes('AIRP_VITE_EXIT'),
  });

  assert.equal(outcome, 'terminated-tree-forced');
  assert.throws(
    () => process.kill(-supervisor.pid, 0),
    error => error?.code === 'ESRCH',
  );
});
