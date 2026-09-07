import { createHash, X509Certificate } from 'node:crypto';
import { performance } from 'node:perf_hooks';
import { setTimeout as delay } from 'node:timers/promises';
import tls from 'node:tls';

const transient = 'ERR_CERT_VERIFIER_CHANGED';
const deadlineError = () => new Error('Initial navigation deadline exceeded');

// Never log Playwright messages: they include the navigation URL and its query.
function navigationCode(error) {
  return /^page\.goto: net::(ERR_[A-Z0-9_]+)(?: at https:\/\/[^\r\n]*)?(?:\r?\n|$)/.exec(error?.message)?.[1] ?? 'NAVIGATION_FAILED';
}

function verifiedIdentity(target, timeoutMs) {
  return new Promise((resolve, reject) => {
    // Default Node trust includes the smoke's NODE_EXTRA_CA_CERTS. Do not use
    // the Chromium SPKI exception to bypass this independent chain/name check.
    const socket = tls.connect({ host: target.hostname, port: Number(target.port || 443),
      servername: target.hostname, rejectUnauthorized: true });
    const timer = setTimeout(() => finish(deadlineError()), timeoutMs);
    function finish(error, identity) {
      clearTimeout(timer);
      socket.destroy();
      if (error) reject(error);
      else resolve(identity);
    }
    socket.once('error', error => {
      const code = /^[A-Z0-9_]+$/.test(error.code ?? '') ? error.code : 'TLS_FAILED';
      finish(new Error(`Gateway verification failed: ${code}`));
    });
    socket.once('secureConnect', () => {
      try {
        if (!socket.authorized) throw new Error('Gateway verification failed: TLS_UNAUTHORIZED');
        const cert = new X509Certificate(socket.getPeerCertificate().raw);
        finish(null, { leafSha256: cert.fingerprint256,
          spki: createHash('sha256').update(cert.publicKey.export({ type: 'spki', format: 'der' })).digest('base64') });
      } catch { finish(new Error('Gateway verification failed: CERTIFICATE_INVALID')); }
    });
  });
}

// Only the smoke's first GET belongs here; never wrap chat actions/history in
// this retry. A successful exact-head rerun suggests transience, not a proven
// verifier root cause. Two attempts maximum, sharing a monotonic 10s deadline.
export async function navigateInitialGet(page, url, expectedSpki, {
  timeoutMs = 10_000,
  diagnose = event => console.error('initial-navigation ' + JSON.stringify(event)),
  probe = verifiedIdentity,
} = {}) {
  const target = new URL(url);
  if (target.protocol !== 'https:' || target.username || target.password) throw new Error('Initial navigation requires credential-free HTTPS');
  if (!/^[A-Za-z0-9+/]{43}=$/.test(expectedSpki)) throw new Error('Invalid expected gateway SPKI');
  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) throw new Error('Invalid navigation deadline');
  const started = performance.now();
  const deadline = started + timeoutMs;
  const remaining = () => {
    const ms = deadline - performance.now();
    if (ms <= 0) throw deadlineError();
    return ms;
  };
  // Playwright gets the remaining timeout too; the outer race also bounds a
  // stalled driver. On failure the caller closes the browser, never continues.
  async function bounded(operation) {
    const ms = remaining();
    let timer;
    try {
      const result = await Promise.race([Promise.resolve().then(() => operation(ms)),
        new Promise((_, reject) => { timer = setTimeout(() => reject(deadlineError()), ms); })]);
      remaining();
      return result;
    } finally { clearTimeout(timer); }
  }
  let previousLeaf;
  for (let attempt = 1; attempt <= 2; attempt++) {
    const identity = await bounded(ms => probe(new URL(target.origin), ms));
    // Diagnostics contain only validated hashes, a comparison, and counters.
    if (!/^(?:[A-F0-9]{2}:){31}[A-F0-9]{2}$/.test(identity.leafSha256) ||
        !/^[A-Za-z0-9+/]{43}=$/.test(identity.spki)) throw new Error('Invalid gateway identity diagnostic');
    diagnose({ attempt, leafSha256: identity.leafSha256, spki: identity.spki,
      expectedSpkiMatches: identity.spki === expectedSpki,
      leafChanged: previousLeaf === undefined ? null : previousLeaf !== identity.leafSha256 });
    if (identity.spki !== expectedSpki) throw new Error('Gateway SPKI identity changed');
    previousLeaf = identity.leafSha256;
    let response;
    try {
      // Stop the retry boundary at document commit. Loading after commit stays
      // outside this catch, so its failure cannot replay page initialization.
      response = await bounded(ms => page.goto(url, { waitUntil: 'commit', timeout: ms }));
    } catch (error) {
      const code = navigationCode(error);
      diagnose({ attempt, code, elapsedMs: Math.round(performance.now() - started) });
      if (error.message === 'Initial navigation deadline exceeded') throw deadlineError();
      if (code !== transient || attempt === 2) throw new Error(`Initial navigation failed: ${code} (attempt ${attempt})`);
      const retryDelay = new AbortController();
      try { await bounded(() => delay(250, undefined, { signal: retryDelay.signal })); }
      finally { retryDelay.abort(); }
      continue;
    }
    try {
      await bounded(ms => page.waitForLoadState('domcontentloaded', { timeout: ms }));
    } catch { throw new Error('Initial document load failed after commit (no retry)'); }
    diagnose({ attempt, outcome: 'loaded', elapsedMs: Math.round(performance.now() - started) });
    return response;
  }
}
