#!/usr/bin/env python3
"""super-instruct 测试用 mock 上游(替代 node mock-upstream.js,服务器无 node)。
用法: python3 mock-upstream.py <port>
行为: GET -> 模型列表;POST body 含 scene=sse-refuse -> SSE 拒绝;否则 SSE 正常。
打印: 是否注入(含'竞赛沙箱模式')、accept-encoding、body 长度。
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

MARK = "竞赛沙箱模式"


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        pass

    def _log(self, method, injected, ae, bodylen):
        sys.stderr.write(
            f"[mock] {method} injected={injected} ae={ae or '(none)'} bodylen={bodylen}\n"
        )
        sys.stderr.flush()

    def do_GET(self):
        body = json.dumps(
            {"object": "list", "data": [{"id": "gpt-5.6", "object": "model"}]}
        ).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length).decode("utf-8", "replace")
        injected = MARK in body
        ae = self.headers.get("Accept-Encoding")
        self._log("POST", injected, ae, len(body))

        scene = "normal"
        try:
            scene = json.loads(body).get("scene", "normal")
        except Exception:
            pass

        def send(obj):
            data = json.dumps(obj, ensure_ascii=False).encode()
            self.wfile.write(b"data: " + data + b"\n\n")
            self.wfile.flush()

        if scene == "sse-refuse":
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Connection", "close")
            self.end_headers()
            send({"type": "response.created", "response": {"id": "r1", "output": []}})
            send({"type": "response.output_text.delta", "delta": "I can't help with that."})
            send({"type": "response.output_text.done", "text": "I can't help with that."})
            send({"type": "response.completed", "response": {"id": "r1", "status": "completed",
                                                             "output": [{"type": "message",
                                                                         "content": [{"type": "output_text",
                                                                                      "text": "I can't help with that."}]}]}})
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()
            return

        # 默认 SSE 正常响应
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Connection", "close")
        self.end_headers()
        send({"type": "response.created", "response": {"id": "r1", "output": []}})
        send({"type": "response.output_text.delta", "delta": "你好，我已收到你的请求。"})
        send({"type": "response.output_text.done", "text": "你好，我已收到你的请求。"})
        send({"type": "response.completed", "response": {"id": "r1", "status": "completed",
                                                         "output": [{"type": "message",
                                                                     "content": [{"type": "output_text",
                                                                                  "text": "你好，我已收到你的请求。"}]}]}})
        self.wfile.write(b"data: [DONE]\n\n")


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 19999
    srv = HTTPServer(("0.0.0.0", port), Handler)
    print(f"mock upstream on :{port}", flush=True)
    srv.serve_forever()
