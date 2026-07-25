// tls-args.mjs — Chrome 启动参数构造（TLS 自签证书处理）
//
// 独立成模块（不 import playwright-core），便于单元测试和静态扫描。
// runner.mjs 和 runner-tls.test.mjs 都 import 这个模块。
//
// 背景：agent-browser-exploration.yml 的 runner 在 GitHub Actions 上连续 3 次 run
// 因 net::ERR_CERT_AUTHORITY_INVALID 失败。根因是 runner 没把 Caddy 自签证书的
// SPKI 传给 Chrome，导致 page.goto 拒绝 https://localhost:9443。
//
// 修复方案（对齐 pr-gate.yml 的 production-browser-smoke.mjs:34 模式）：
//   1. bootstrap-topology.sh 从 gateway leaf cert 算出 SPKI hash，写入 $GITHUB_ENV
//   2. workflow 把 AIRP_CHROME_SPKI 传给 runner
//   3. runner 把 --ignore-certificate-errors-spki-list=<spki> 加入 launch args，
//      Chrome 精确信任该 SPKI 对应的证书，不无脑 ignoreHTTPSErrors

// Chrome SPKI 格式：base64 编码的 SHA-256 公钥哈希，固定 32 字节
// → base64 编码 43 字符 + 1 个 '=' 填充
const SPKI_PATTERN = /^[A-Za-z0-9+/]{43}=$/;

/**
 * 构造 Chrome launch args，根据 spki 决定是否加 --ignore-certificate-errors-spki-list。
 *
 * @param {string} [spki] - base64 编码的 SPKI（可空，本地 http 场景可不传）
 * @returns {string[]} launch args 数组（空数组表示不加任何 args）
 */
export function buildLaunchArgs(spki) {
  const launchArgs = [];
  if (spki && SPKI_PATTERN.test(spki)) {
    launchArgs.push('--ignore-certificate-errors-spki-list=' + spki);
  } else if (spki) {
    console.warn('[runner] AIRP_CHROME_SPKI format invalid (expected base64 SPKI ending with =); ignoring');
  }
  // 无 spki 时不加 args，保持与旧版行为一致（同源 http 或本地无 TLS 场景仍可用）
  return launchArgs;
}
