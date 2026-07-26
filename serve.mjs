// Minimal static server for local play:  node serve.mjs   ->  http://localhost:8080
import http from 'http'; import fs from 'fs'; import path from 'path';
const MIME = { '.html':'text/html; charset=utf-8', '.js':'text/javascript; charset=utf-8', '.wasm':'application/wasm' };
const ROOT = path.join(import.meta.dirname ?? '.', 'site');
http.createServer((req, res) => {
  let u = decodeURIComponent(req.url.split('?')[0]);
  if (u === '/') u = '/index.html';
  const fp = path.join(ROOT, path.normalize(u).replace(/^(\.\.[/\\])+/, ''));
  fs.readFile(fp, (e, d) => {
    if (e) { res.statusCode = 404; res.end('Not found'); return; }
    res.setHeader('Content-Type', MIME[path.extname(fp)] || 'application/octet-stream');
    res.end(d);
  });
}).listen(8080, () => console.log('Aetherloom on http://localhost:8080'));
