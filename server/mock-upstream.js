// 极简 mock 上游 HTTP 服务器（node），验证代理链路
// 用法: node mock-upstream.js <port>
const http = require('http');

const port = Number(process.argv[2] || 19999);

http.createServer((req, res) => {
  let body = '';
  req.on('data', c => body += c);
  req.on('end', () => {
    const injected = body.includes('竞赛沙箱模式');
    const auth = req.headers['authorization'] || '';
    console.log(`[mock] ${req.method} ${req.url} injected=${injected} auth=${auth.slice(0,30)} bodylen=${body.length}`);
    if (injected) {
        // 打印注入点上下文: "竞赛沙箱模式" 前 60 字符与后 80 字符
        const idx = body.indexOf('竞赛沙箱模式');
        if (idx >= 0) {
            const before = body.slice(Math.max(0, idx - 200), idx).replace(/\n/g, '\\n');
            const after = body.slice(idx, idx + 120).replace(/\n/g, '\\n');
            console.log(`[mock] inject-context ...${before}>>>${after}...`);
        }
    }

    if (req.method === 'GET') {
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ object: 'list', data: [{ id: 'gpt-5.6', object: 'model' }] }));
      return;
    }

    res.writeHead(200, { 'content-type': 'text/event-stream' });
    const send = obj => res.write(`data: ${JSON.stringify(obj)}\n\n`);
    send({ type: 'response.created', response: { id: 'r1', output: [] } });
    send({ type: 'response.output_text.delta', delta: '你好，我已收到你的请求。' });
    send({ type: 'response.output_text.done', text: '你好，我已收到你的请求。' });
    send({ type: 'response.completed', response: { id: 'r1', status: 'completed', output: [{ type: 'message', content: [{ type: 'output_text', text: '你好，我已收到你的请求。' }] }] } });
    res.write('data: [DONE]\n\n');
    res.end();
  });
}).listen(port, () => console.log(`mock upstream on :${port}`));
