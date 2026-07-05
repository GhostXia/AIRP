// 零依赖纯 node 静态文件服务器 —— 只为 webui/ 用，避免依赖 npx serve / python。
// 不进 Cargo/npm workspace（PLAN §7.2）。
const http = require('http');
const fs = require('fs');
const path = require('path');

const PORT = parseInt(process.env.WEBUI_PORT || '9001', 10);
const HOST = process.env.WEBUI_HOST || '127.0.0.1'; // 跨设备访问改 0.0.0.0
const ROOT = __dirname;

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'application/javascript; charset=utf-8',
  '.mjs': 'application/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.svg': 'image/svg+xml',
  '.ico': 'image/x-icon',
};

http.createServer((req, res) => {
  // F3 (审计 fix): decodeURIComponent 对 malformed % 抛 URIError 会杀进程。
  // try/catch 兜住，返回 400 而非崩溃整个 server。
  let urlPath;
  try {
    urlPath = decodeURIComponent(req.url.split('?')[0]);
  } catch {
    res.writeHead(400); res.end('bad url encoding'); return;
  }
  if (urlPath === '/') urlPath = '/index.html';
  // 防路径穿越：归一化后必须仍在 ROOT 下
  const safe = path.normalize(path.join(ROOT, urlPath));
  if (safe !== ROOT && !safe.startsWith(ROOT + path.sep)) {
    res.writeHead(403); res.end('forbidden'); return;
  }
  fs.readFile(safe, (err, data) => {
    if (err) { res.writeHead(404); res.end('not found: ' + urlPath); return; }
    res.writeHead(200, { 'Content-Type': MIME[path.extname(safe).toLowerCase()] || 'application/octet-stream' });
    res.end(data);
  });
}).listen(PORT, HOST, () => {
  console.log('WebUI serving on http://' + HOST + ':' + PORT);
  console.log('Engine 默认 http://127.0.0.1:8000 （在浏览器顶部 Engine URL 框可改）');
  console.log('按 Ctrl+C 停止');
});
