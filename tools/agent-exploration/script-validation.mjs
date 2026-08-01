// Generated exploration scripts are ES modules. Validate syntax in a separate
// Node process so the runner never executes a malformed candidate just to
// discover a parser or export-contract error.

import { spawn } from 'node:child_process';

const REQUIRED_RUN_EXPORT = /^\s*export\s+async\s+function\s+run\s*\(\s*ctx\s*\)/m;

/**
 * Returns diagnostics for a generated exploration script without importing or
 * executing it. An empty array means it is safe to pass to the existing
 * runtime sandbox.
 * @param {string} source
 * @returns {Promise<string[]>}
 */
export async function validateScript(source) {
  const diagnostics = [];
  const syntaxDiagnostic = await moduleSyntaxDiagnostic(source);
  if (syntaxDiagnostic) diagnostics.push(syntaxDiagnostic);
  if (!REQUIRED_RUN_EXPORT.test(source)) {
    diagnostics.push('Missing required module export: export async function run(ctx)');
  }
  return diagnostics;
}

function moduleSyntaxDiagnostic(source) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, ['--input-type=module', '--check', '-'], {
      stdio: ['pipe', 'ignore', 'pipe'],
    });
    let stderr = '';
    child.stderr.setEncoding('utf8');
    child.stderr.on('data', chunk => { stderr += chunk; });
    child.on('error', reject);
    child.on('close', code => {
      if (code === 0) return resolve(null);
      const detail = stderr.trim().replace(/\s+/g, ' ').slice(0, 600);
      resolve('JavaScript module syntax check failed: ' + (detail || 'node --check exited ' + code));
    });
    child.stdin.end(source);
  });
}
