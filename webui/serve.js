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

function send(res, status, body, type) {
  const headers = type ? { 'Content-Type': type } : undefined;
  res.writeHead(status, headers);
  res.end(body);
}

http.createServer((req, res) => {
  // F3 (审计 fix): decodeURIComponent 对 malformed % 抛 URIError 会杀进程。
  // try/catch 兜住，返回 400 而非崩溃整个 server。
  let urlPath;
  try {
    urlPath = decodeURIComponent(req.url.split('?')[0]);
  } catch {
    send(res, 400, 'bad url encoding'); return;
  }
  // null byte 会让 fs.readFile 抛同步 TypeError，提前拦截。
  if (urlPath.includes('\0')) {
    send(res, 400, 'bad url encoding'); return;
  }
  // Windows 上 path.resolve(ROOT, '/foo') 会把 /foo 当成当前盘根目录，
  // 导致 /app.js 被解析到 d:\app.js。因此统一把 urlPath 当相对路径处理。
  const filePath = urlPath === '/' ? 'index.html' : urlPath.replace(/^\/+/, '');
  // F2-bis 加固：用 resolve + relative 做路径穿越检查，比 normalize + startsWith 更严格。
  const target = path.resolve(ROOT, filePath);
  const rel = path.relative(ROOT, target);
  if (rel.startsWith('..' + path.sep) || rel === '..' || rel === '') {
    send(res, 403, 'forbidden'); return;
  }
  // fs.readFile 对非法 path 仍可能抛同步异常（如 null byte 漏检），兜住保进程。
  try {
    fs.readFile(target, (err, data) => {
      if (err) { send(res, 404, 'not found: ' + urlPath); return; }
      send(res, 200, data, MIME[path.extname(target).toLowerCase()] || 'application/octet-stream');
    });
  } catch (e) {
    send(res, 500, 'internal error');
  }
}).listen(PORT, HOST, () => {
  console.log('WebUI serving on http://' + HOST + ':' + PORT);
  console.log('Engine 默认 http://127.0.0.1:8000 （在浏览器顶部 Engine URL 框可改）');
  console.log('按 Ctrl+C 停止');
});
