import hashlib
import json
import socket
import ssl
import sys
import time

certificate, key, ready, requests, accepted, sent, mode, closed, payload, media_type, status, location, max_connections = sys.argv[1:]
max_connections = int(max_connections)
with open(payload, "rb") as stream:
    body = stream.read()
listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
listener.bind(("127.0.0.1", 0))
listener.listen(4)
with open(ready, "w", encoding="ascii") as stream:
    stream.write(str(listener.getsockname()[1]))

def mark(path):
    with open(path, "a", encoding="ascii") as stream:
        stream.write("1\n")
        stream.flush()

def receive_request(connection):
    connection.settimeout(2.0)
    data = b""
    while b"\r\n\r\n" not in data:
        piece = connection.recv(4096)
        if not piece:
            return None
        data += piece
    header_bytes, body_bytes = data.split(b"\r\n\r\n", 1)
    lines = header_bytes.split(b"\r\n")
    method, path, _ = lines[0].decode("iso-8859-1").split(" ", 2)
    headers = {}
    for line in lines[1:]:
        name, value = line.split(b":", 1)
        headers[name.decode("iso-8859-1").lower()] = value.strip().decode("iso-8859-1")
    length = int(headers.get("content-length", "0"))
    while len(body_bytes) < length:
        piece = connection.recv(4096)
        if not piece:
            break
        body_bytes += piece
    authorization = headers.pop("authorization", None)
    record = {
        "method": method,
        "path": path,
        "headers": headers,
        "body": body_bytes[:length].decode("utf-8", "replace"),
    }
    if authorization is not None:
        record["authorization_sha256"] = hashlib.sha256(
            authorization.encode("utf-8")
        ).hexdigest()
    with open(requests, "a", encoding="utf-8") as stream:
        stream.write(json.dumps(record, sort_keys=True) + "\n")
        stream.flush()
    return record

def wait_for_close(connection):
    connection.settimeout(2.0)
    try:
        while True:
            if not connection.recv(4096):
                mark(closed)
                return
    except TimeoutError:
        return
    except (ConnectionError, OSError):
        mark(closed)

def send_response(connection, code, response_body, response_type, close=True):
    reasons = {200: "OK", 307: "Temporary Redirect", 500: "Internal Server Error"}
    headers = [
        f"HTTP/1.1 {code} {reasons.get(code, 'Status')}\r\n",
        f"Content-Type: {response_type}\r\n",
    ]
    if close:
        headers.append(f"Content-Length: {len(response_body)}\r\n")
        headers.append("Connection: close\r\n")
    else:
        headers.append("Connection: keep-alive\r\n")
    headers.append("\r\n")
    connection.sendall("".join(headers).encode("ascii") + response_body)
    mark(sent)

def serve(connection, index):
    if mode == "tls-stall":
        wait_for_close(connection)
        return
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(certificate, key)
    try:
        connection = context.wrap_socket(connection, server_side=True)
    except (TimeoutError, ConnectionError, OSError):
        return
    if receive_request(connection) is None:
        connection.close()
        return
    if mode == "header-stall":
        wait_for_close(connection)
        return
    if mode == "success":
        send_response(connection, 200, body, media_type)
    elif mode == "status":
        send_response(connection, int(status), body, media_type)
    elif mode == "redirect" and index == 0:
        connection.sendall(
            f"HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\n"
            "Content-Length: 0\r\nConnection: close\r\n\r\n".encode("ascii")
        )
    elif mode == "redirect":
        send_response(connection, 200, body, media_type)
    elif mode == "delayed-redirect-chain":
        time.sleep(int(status) / 1000)
        if index < 2:
            try:
                connection.sendall(
                    f"HTTP/1.1 307 Temporary Redirect\r\nLocation: /v1/redirect-{index + 1}\r\n"
                    "Content-Length: 0\r\nConnection: close\r\n\r\n".encode("ascii")
                )
            except (ConnectionError, OSError):
                mark(closed)
                return
        else:
            try:
                send_response(connection, 200, body, media_type)
            except (ConnectionError, OSError):
                mark(closed)
                return
    elif mode == "redirect-loop":
        connection.sendall(
            f"HTTP/1.1 307 Temporary Redirect\r\nLocation: /v1/redirect-{index + 1}\r\n"
            "Content-Length: 0\r\nConnection: close\r\n\r\n".encode("ascii")
        )
    elif mode == "declared-oversize":
        connection.sendall(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n"
            b"Content-Length: 8388609\r\nConnection: keep-alive\r\n\r\n"
        )
        mark(sent)
        wait_for_close(connection)
        return
    elif mode == "unframed-success":
        send_response(connection, 200, body, media_type, close=False)
    elif mode == "headers-stall":
        connection.sendall(
            f"HTTP/1.1 200 OK\r\nContent-Type: {media_type}\r\n"
            "Connection: keep-alive\r\n\r\n".encode("ascii")
        )
        mark(sent)
        wait_for_close(connection)
        return
    elif mode == "event-stall":
        send_response(connection, 200, body, media_type, close=False)
        wait_for_close(connection)
        return
    elif mode == "error-body-stall":
        send_response(connection, int(status), body, media_type, close=False)
        wait_for_close(connection)
        return
    elif mode == "heartbeat-stall":
        send_response(connection, 200, b"", media_type, close=False)
        for _ in range(4):
            try:
                connection.sendall(b": heartbeat\n\n")
            except (ConnectionError, OSError):
                mark(closed)
                return
            time.sleep(0.05)
        wait_for_close(connection)
        return
    connection.close()

for index in range(max_connections):
    try:
        connection, _ = listener.accept()
        mark(accepted)
        serve(connection, index)
    except (TimeoutError, ConnectionError, OSError):
        break
listener.close()
