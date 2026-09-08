import json
import os
import socket
import sys
import threading

server = socket.socket()
server.bind(("127.0.0.1", 0))
server.listen(32)
print(json.dumps({"event": "ready", "pid": os.getpid(), "ports": [server.getsockname()[1]], "children": []}), flush=True)
watchdog = threading.Timer(120, lambda: os._exit(0))
watchdog.daemon = True
watchdog.start()
sys.stdin.read()
server.close()
