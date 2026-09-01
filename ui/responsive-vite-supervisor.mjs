import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const viteCli = fileURLToPath(new URL('./node_modules/vite/bin/vite.js', import.meta.url));
const vite = spawn(process.execPath, [viteCli, ...process.argv.slice(2)], {
  cwd: process.cwd(),
  env: process.env,
  stdio: 'inherit',
});

// Keep the process-group leader alive after Vite exits. The smoke runner can
// then terminate the recorded tree/group without acting on a reusable PID.
const keepAlive = setInterval(() => {}, 1_000);
let reported = false;

function reportExit(message) {
  if (reported) return;
  reported = true;
  process.stderr.write(`AIRP_VITE_EXIT ${message}\n`);
}

vite.on('error', error => reportExit(`spawn-error=${JSON.stringify(error.message)}`));
vite.on('exit', (code, signal) => {
  reportExit(`code=${code ?? 'null'} signal=${signal ?? 'null'}`);
});

// Group-wide TERM must not release the stable process-group identity. The
// parent always finishes with a group-wide KILL after the TERM grace period,
// so resistant descendants are cleared while this process still owns the id.
process.on('SIGTERM', () => {});

process.on('exit', () => clearInterval(keepAlive));
