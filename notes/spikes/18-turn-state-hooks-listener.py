import json, sys, time
from http.server import BaseHTTPRequestHandler, HTTPServer
LOG = sys.argv[2]
class H(BaseHTTPRequestHandler):
    def do_POST(self):
        n = int(self.headers.get('content-length', 0)); body = json.loads(self.rfile.read(n) or b'{}')
        keep = {k: body.get(k) for k in ('hook_event_name','session_id','notification_type','message','tool_name','agent_id','agent_type','stop_hook_active','source','reason') if k in body}
        if 'tool_input' in body: keep['tool_input'] = str(body['tool_input'])[:120]
        with open(LOG, 'a') as f: f.write(json.dumps({'t': round(time.time(), 2), **keep}) + '\n')
        self.send_response(200); self.end_headers()
    def log_message(self, *a): pass
HTTPServer(('127.0.0.1', int(sys.argv[1])), H).serve_forever()
