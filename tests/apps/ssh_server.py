"""Offline SSH peer for the real OpenSSH client's reverse-forward test.
Only loopback forwarding and the test username are accepted; no shell/exec API.
"""
import json
import os
from pathlib import Path
import select
import socket
import sys
import threading
import paramiko

forward_file = Path(sys.argv[1])
host_key = paramiko.RSAKey.generate(2048)
stop = threading.Event()
listeners = []
transports = []

def pipe(connection, channel):
    try:
        while not stop.is_set():
            readable, _, _ = select.select([connection, channel], [], [], 0.1)
            for source in readable:
                data = source.recv(4096)
                if not data:
                    return
                (channel if source is connection else connection).sendall(data)
    finally:
        connection.close()
        channel.close()

class Server(paramiko.ServerInterface):
    def __init__(self, transport):
        self.transport = transport
    def check_auth_none(self, username):
        return paramiko.AUTH_SUCCESSFUL if username == "strayd-fixture" else paramiko.AUTH_FAILED
    def check_port_forward_request(self, address, port):
        if address not in ("localhost", "127.0.0.1") or port != 0:
            return False
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen(8)
        listener.settimeout(0.2)
        listeners.append(listener)
        allocated = listener.getsockname()[1]
        forward_file.with_suffix('.pending').write_text(json.dumps({"port": allocated}))
        forward_file.with_suffix('.pending').replace(forward_file)
        def accept():
            while not stop.is_set():
                try:
                    connection, origin = listener.accept()
                except socket.timeout:
                    continue
                except OSError:
                    return
                try:
                    channel = self.transport.open_forwarded_tcpip_channel(src_addr=origin, dest_addr=(address, allocated))
                except paramiko.SSHException:
                    connection.close()
                    continue
                threading.Thread(target=pipe, args=(connection, channel), daemon=True).start()
        threading.Thread(target=accept, daemon=True).start()
        return allocated

listener = socket.socket()
listener.bind(("127.0.0.1", 0))
listener.listen(8)
listeners.append(listener)
print(json.dumps({"event": "ready", "pid": os.getpid(), "ports": [listener.getsockname()[1]], "children": []}), flush=True)

def accept():
    while not stop.is_set():
        try:
            connection, _ = listener.accept()
            transport = paramiko.Transport(connection)
            transports.append(transport)
            transport.add_server_key(host_key)
            transport.start_server(server=Server(transport))
        except (OSError, paramiko.SSHException):
            if stop.is_set():
                return

threading.Thread(target=accept, daemon=True).start()
watchdog = threading.Timer(120, lambda: os._exit(0))
watchdog.daemon = True
watchdog.start()
try:
    sys.stdin.read()
finally:
    stop.set()
    for transport in transports:
        transport.close()
    for listener in listeners:
        listener.close()
