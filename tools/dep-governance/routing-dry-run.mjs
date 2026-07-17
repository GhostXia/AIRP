// Dry-run routing demo (#192 acceptance #1).
//
// Reads fixtures/routing-samples.json, runs each sample through
// classifyUpgrade(), and prints the routing decision. Exits non-zero if any
// sample's actual class/routing does not match the expected value declared
// in the fixture. This proves the five routing classes (patch, minor,
// major, 0x-minor, prerelease) plus the patch-sensitive override are
// implemented correctly, without needing network access or a live upstream
// registry.
//
// This is the "fixed fixture or dry-run" acceptance proof for #192.
// Real upgrade detection (comparing Cargo.lock/package-lock.json against
// the latest upstream stable versions, reading security advisories, and
// creating deduplicated GitHub issues or auto-PRs) is a documented future
// step — see README.md "Deferred".

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { classifyUpgrade, makeDedupKey, validateConfig } from "./audit-routing.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

function loadJson(p) {
  return JSON.parse(fs.readFileSync(p, "utf8"));
}

export async function main(argv) {
  const fixturePath = path.resolve(
    __dirname,
    "fixtures",
    "routing-samples.json",
  );
  const configPath = path.resolve(
    __dirname,
    "audit-routing.config.json",
  );

  const fixture = loadJson(fixturePath);
  const config = validateConfig(loadJson(configPath));

  const samples = fixture.samples ?? [];
  let pass = 0;
  let fail = 0;
  const failures = [];

  process.stdout.write(`Routing dry-run: ${samples.length} samples\n`);
  process.stdout.write("=".repeat(72) + "\n");

  for (const s of samples) {
    const decision = classifyUpgrade(
      s.current,
      s.target,
      { name: s.name, ecosystem: s.ecosystem, areaTags: s.areaTags ?? [] },
      config,
    );
    const dedup = makeDedupKey(s.ecosystem, s.name, s.target);

    const classOk = decision.class === s.expected_class;
    const routingOk = decision.routing === s.expected_routing;
    const ok = classOk && routingOk;

    process.stdout.write(
      `[${ok ? "PASS" : "FAIL"}] ${s.id.padEnd(28)} ${s.ecosystem}/${s.name} ${s.current} -> ${s.target}\n`,
    );
    process.stdout.write(
      `         class=${decision.class} (expected ${s.expected_class})  routing=${decision.routing} (expected ${s.expected_routing})  dedup=${dedup}\n`,
    );
    process.stdout.write(`         reason: ${decision.reason}\n`);

    if (ok) {
      pass++;
    } else {
      fail++;
      failures.push({
        id: s.id,
        expected: { class: s.expected_class, routing: s.expected_routing },
        actual: { class: decision.class, routing: decision.routing },
      });
    }
  }

  process.stdout.write("=".repeat(72) + "\n");
  process.stdout.write(`Result: ${pass} pass, ${fail} fail out of ${samples.length}\n`);

  if (fail > 0) {
    process.stderr.write(`\n${fail} sample(s) failed:\n`);
    for (const f of failures) {
      process.stderr.write(
        `  ${f.id}: expected ${JSON.stringify(f.expected)}, got ${JSON.stringify(f.actual)}\n`,
      );
    }
    process.exit(1);
  }
}

const isMain = (() => {
  if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(new URL(import.meta.url).pathname)) {
    return true;
  }
  return process.argv[1]?.endsWith("routing-dry-run.mjs");
})();

if (isMain) {
  main(process.argv.slice(2)).catch((e) => {
    process.stderr.write(`fatal: ${e?.stack ?? e}\n`);
    process.exit(1);
  });
}
