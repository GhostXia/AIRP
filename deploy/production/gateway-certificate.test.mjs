import test, { before, after } from 'node:test';
import assert from 'node:assert/strict';
import { execFile, execFileSync } from 'node:child_process';
import { createHash, X509Certificate } from 'node:crypto';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import net from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import tls from 'node:tls';
import { performance } from 'node:perf_hooks';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { waitForGatewayLeaf } from './gateway-certificate.mjs';

let fixture;
let valid;
let wrongHost;
async function certificate(name, hostname) {
  const keyPath = join(fixture, `${name}.key`);
  const certPath = join(fixture, `${name}.crt`);
  execFileSync('openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1',
    '-keyout', keyPath, '-out', certPath, '-subj', `/CN=${hostname}`,
    '-addext', `subjectAltName=DNS:${hostname}`], { stdio: 'ignore', timeout: 15_000 });
  return { key: await readFile(keyPath), cert: await readFile(certPath) };
}
before(async () => {
  fixture = await mkdtemp(join(tmpdir(), 'airp-gateway-cert-test-'));
  valid = await certificate('valid', 'localhost');
  wrongHost = await certificate('wrong-host', 'elsewhere.invalid');
});
after(async () => { if (fixture) await rm(fixture, { recursive: true, force: true }); });

async function listen(server) {
  const sockets = new Set();
  server.on('connection', (socket) => {
    sockets.add(socket);
    socket.once('close', () => sockets.delete(socket));
  });
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  return {
    origin: `https://localhost:${server.address().port}`,
    sockets,
    close: () => new Promise((resolve) => {
      server.close(resolve);
      for (const socket of sockets) socket.destroy();
    }),
  };
}

test('CA exists before delayed leaf readiness; return the verified served certificate', async () => {
  let attempts = 0;
  let ready = false;
  const context = tls.createSecureContext(valid);
  const server = tls.createServer({ SNICallback: (_name, callback) => {
    attempts++;
    callback(ready ? null : new Error('leaf not issued yet'), ready ? context : undefined);
  } });
  const gateway = await listen(server);
  const timer = setTimeout(() => { ready = true; }, 180);
  try {
    const pem = await waitForGatewayLeaf({ origin: gateway.origin, ca: valid.cert, timeoutMs: 3000, pollIntervalMs: 30 });
    assert.equal(new X509Certificate(pem).fingerprint256, new X509Certificate(valid.cert).fingerprint256);
    assert.ok(attempts >= 2, 'must retry the not-yet-issued leaf');
  } finally { clearTimeout(timer); await gateway.close(); }
});

test('root-only trust verifies an intermediate chain and returns the localhost leaf and its SPKI', async () => {
  const openssl = (args) => execFileSync('openssl', args, { cwd: fixture, stdio: 'ignore', timeout: 15_000 });
  openssl(['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1',
    '-keyout', 'root.key', '-out', 'root.crt', '-subj', '/CN=Fixture Root',
    '-addext', 'basicConstraints=critical,CA:TRUE,pathlen:1', '-addext', 'keyUsage=critical,keyCertSign,cRLSign']);
  for (const [name, issuer, extensions] of [
    ['intermediate', 'root', 'basicConstraints=critical,CA:TRUE,pathlen:0\nkeyUsage=critical,keyCertSign,cRLSign'],
    ['leaf', 'intermediate', 'basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:localhost'],
  ]) {
    openssl(['req', '-new', '-newkey', 'rsa:2048', '-nodes', '-keyout', `${name}.key`,
      '-out', `${name}.csr`, '-subj', `/CN=${name === 'leaf' ? 'localhost' : 'Fixture Intermediate'}`]);
    await writeFile(join(fixture, `${name}.ext`), extensions);
    openssl(['x509', '-req', '-in', `${name}.csr`, '-CA', `${issuer}.crt`, '-CAkey', `${issuer}.key`,
      '-set_serial', name === 'leaf' ? '3' : '2', '-days', '1', '-extfile', `${name}.ext`, '-out', `${name}.crt`]);
  }
  const [root, intermediate, leaf, key] = await Promise.all(
    ['root.crt', 'intermediate.crt', 'leaf.crt', 'leaf.key'].map((name) => readFile(join(fixture, name))));
  const gateway = await listen(tls.createServer({ key, cert: Buffer.concat([leaf, intermediate]) }));
  const spki = (cert) => createHash('sha256').update(
    new X509Certificate(cert).publicKey.export({ type: 'spki', format: 'der' })).digest('base64');
  try {
    const pem = await waitForGatewayLeaf({ origin: gateway.origin, ca: root, timeoutMs: 3000, pollIntervalMs: 30 });
    assert.equal((pem.match(/BEGIN CERTIFICATE/g) ?? []).length, 1, 'return only the leaf, not the chain');
    assert.equal(new X509Certificate(pem).fingerprint256, new X509Certificate(leaf).fingerprint256);
    assert.equal(spki(pem), spki(leaf));
    assert.notEqual(spki(pem), spki(intermediate));
    assert.notEqual(spki(pem), spki(root));
  } finally { await gateway.close(); }
});

test('never accepts an untrusted chain', async () => {
  const gateway = await listen(tls.createServer(valid));
  try {
    await assert.rejects(waitForGatewayLeaf({ origin: gateway.origin, ca: wrongHost.cert, timeoutMs: 200, pollIntervalMs: 30 }),
      (error) => /verification failed/.test(error.message) && error.cause?.code === 'DEPTH_ZERO_SELF_SIGNED_CERT');
  } finally { await gateway.close(); }
});

test('never accepts a trusted certificate for the wrong hostname', async () => {
  const gateway = await listen(tls.createServer(wrongHost));
  try {
    await assert.rejects(waitForGatewayLeaf({ origin: gateway.origin, ca: wrongHost.cert, timeoutMs: 200, pollIntervalMs: 30 }),
      (error) => /verification failed/.test(error.message) && error.cause?.code === 'ERR_TLS_CERT_ALTNAME_INVALID');
  } finally { await gateway.close(); }
});

test('stalled TLS handshake exhausts one bounded deadline and retains its cause', async () => {
  let connections = 0;
  const gateway = await listen(net.createServer((socket) => {
    connections++;
    socket.resume(); // Drain ClientHello so the peer's subsequent FIN is observable.
  }));
  try {
    const started = performance.now();
    await assert.rejects(waitForGatewayLeaf({ origin: gateway.origin, ca: valid.cert, timeoutMs: 180, pollIntervalMs: 30 }),
      /not ready within 180ms: TLS handshake timed out/);
    assert.ok(performance.now() - started < 1500, 'hanging peer must not extend the deadline');
    assert.ok(connections > 0, 'fixture must accept a real client connection');
    const closedBy = performance.now() + 1000;
    while (gateway.sockets.size && performance.now() < closedBy) await delay(10);
    assert.equal(gateway.sockets.size, 0, 'client must close every connection before server cleanup');
  } finally { await gateway.close(); }
});

test('the smoke probe rejects nonlocal origins and invalid timing', async () => {
  for (const origin of ['http://localhost:9443', 'https://example.com', 'https://user:password@localhost']) {
    await assert.rejects(waitForGatewayLeaf({ origin, ca: valid.cert }), /HTTPS localhost/);
  }
  for (const timeoutMs of [0, -1, NaN, Infinity]) {
    await assert.rejects(waitForGatewayLeaf({ origin: 'https://localhost:9443', ca: valid.cert, timeoutMs }), /positive and finite/);
  }
});

test('CLI writes only the verified served leaf for subsequent SPKI derivation', async () => {
  const gateway = await listen(tls.createServer(valid));
  const destination = join(fixture, 'served-leaf.crt');
  try {
    await promisify(execFile)(process.execPath, [fileURLToPath(new URL('./gateway-certificate.mjs', import.meta.url)),
      gateway.origin, join(fixture, 'valid.crt'), destination], { timeout: 5000 });
    assert.equal(new X509Certificate(await readFile(destination)).fingerprint256, new X509Certificate(valid.cert).fingerprint256);
  } finally { await gateway.close(); }
});

test('production smoke waits for the served leaf before deriving browser SPKI', async () => {
  const script = await readFile(new URL('./smoke-ci.sh', import.meta.url), 'utf8');
  const probe = script.indexOf('node "$deploy/gateway-certificate.mjs" "$origin" "$root_ca" "$gateway_leaf"');
  assert.ok(probe > 0);
  assert.ok(probe < script.indexOf('chrome_spki=$(openssl x509'));
  assert.ok(!script.includes('find /data/caddy/certificates/local'));
  assert.ok(script.includes('--cacert $root_ca'), 'later HTTPS requests still require CA verification');
});
