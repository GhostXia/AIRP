import { X509Certificate } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { performance } from 'node:perf_hooks';
import { setTimeout as delay } from 'node:timers/promises';
import tls from 'node:tls';
import { pathToFileURL } from 'node:url';

// Read the certificate actually served by the local smoke gateway, validating
// both its chain against our copied CA and its localhost identity.
function readLeaf(port, ca, timeoutMs) {
  return new Promise((resolve, reject) => {
    let socket;
    let timer;
    let settled = false;
    const finish = (error, pem) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      socket?.destroy();
      if (error) reject(error);
      else resolve(pem);
    };
    try {
      socket = tls.connect({ host: '127.0.0.1', port, servername: 'localhost', ca, rejectUnauthorized: true });
      timer = setTimeout(() => finish(new Error('TLS handshake timed out')), timeoutMs);
      socket.once('error', (error) => {
        if (socket.authorizationError) error.certificateVerificationFailed = true;
        finish(error);
      });
      socket.once('close', () => finish(new Error('TLS connection closed before certificate readiness')));
      socket.once('secureConnect', () => {
        try {
          if (!socket.authorized) throw new Error('Gateway certificate is not authorized');
          finish(null, new X509Certificate(socket.getPeerCertificate().raw).toString());
        } catch (error) { finish(error); }
      });
    } catch (error) { finish(error); }
  });
}

// A root CA can exist before Caddy has a usable leaf. Retry the real handshake
// within one monotonic deadline, including stalled attempts and retry delays.
export async function waitForGatewayLeaf({ origin, ca, timeoutMs = 60_000, pollIntervalMs = 250 }) {
  const target = new URL(origin);
  if (target.protocol !== 'https:' || target.hostname !== 'localhost' || target.username || target.password) {
    throw new Error('Gateway certificate probe requires an HTTPS localhost origin');
  }
  for (const value of [timeoutMs, pollIntervalMs]) {
    if (!Number.isFinite(value) || value <= 0) throw new Error('Certificate readiness timing must be positive and finite');
  }
  const deadline = performance.now() + timeoutMs;
  let lastError;
  while (performance.now() < deadline) {
    try {
      return await readLeaf(Number(target.port || 443), ca, Math.min(1000, deadline - performance.now()));
    } catch (error) {
      // Bad trust/identity is not a readiness race. Keep that diagnostic rather
      // than replacing it with a later short attempt's deadline error.
      if (error.certificateVerificationFailed) {
        throw new Error(`Gateway certificate verification failed: ${error.message}`, { cause: error });
      }
      lastError = error;
    }
    const remaining = deadline - performance.now();
    if (remaining > 0) await delay(Math.min(pollIntervalMs, remaining));
  }
  throw new Error(`Gateway leaf certificate not ready within ${timeoutMs}ms: ${lastError?.message ?? 'deadline expired'}`, { cause: lastError });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const [origin, caFile, destination] = process.argv.slice(2);
    if (!origin || !caFile || !destination) throw new Error('Usage: gateway-certificate.mjs ORIGIN CA_FILE OUTPUT_FILE');
    const pem = await waitForGatewayLeaf({ origin, ca: await readFile(caFile) });
    await writeFile(destination, pem);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
