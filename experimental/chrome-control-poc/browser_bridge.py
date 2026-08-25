#!/usr/bin/env python3
"""Disposable AgenticGPT spike for OpenAI bundled Chrome control.

This intentionally lives outside the production runtime. It keeps one standalone
Codex node_repl MCP process alive and exposes a tiny local connect/exec surface.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import select
import socket
import subprocess
import sys
import time
import tomllib
import uuid

REGISTRY = Path.home() / ".local/state/openai-codex/chrome-native-hosts-v2.json"
CODEX_CONFIG = Path.home() / ".codex/config.toml"
SOCKET_PATH = Path(f"/tmp/agentic-chrome-control-poc-{os.getuid()}.sock")
LOG_PATH = Path(f"/tmp/agentic-chrome-control-poc-{os.getuid()}.log")


class BridgeError(RuntimeError):
    pass


def _latest_runtime() -> dict:
    registry = json.loads(REGISTRY.read_text())
    entries = registry.get("entries") or []
    if not entries:
        raise BridgeError(f"no Chrome runtime entries in {REGISTRY}")
    return max(entries, key=lambda item: item.get("updatedAt", ""))


def _node_repl_env(entry: dict) -> dict[str, str]:
    with CODEX_CONFIG.open("rb") as fh:
        config = tomllib.load(fh)
    try:
        configured = config["mcp_servers"]["node_repl"].get("env", {})
    except KeyError as exc:
        raise BridgeError("Codex node_repl MCP config is missing") from exc

    paths = entry["paths"]
    env = os.environ.copy()
    env.update({str(key): str(value) for key, value in configured.items()})
    # Registry paths/version are authoritative for the currently installed runtime.
    env.update(
        {
            "NODE_REPL_NODE_PATH": paths["nodePath"],
            "CODEX_HOME": paths["codexHome"],
            "CODEX_CLI_PATH": paths["codexCliPath"],
            "BROWSER_USE_CODEX_APP_VERSION": entry["appVersion"],
            "BROWSER_USE_CODEX_APP_BUILD_FLAVOR": entry.get("channel", "prod"),
        }
    )
    return env


class NodeReplBridge:
    def __init__(self) -> None:
        self.entry = _latest_runtime()
        paths = self.entry["paths"]
        self.browser_client = paths["browserClientPath"]
        self.node_repl = paths["nodeReplPath"]
        self.cwd = str(Path(self.browser_client).parent.parent)
        self.session_id = f"agentic-browser-poc-{uuid.uuid4().hex}"
        self.turn_id = f"turn-{uuid.uuid4().hex}"
        self.proc: subprocess.Popen[str] | None = None
        self.next_id = 1
        self.connected = False

    @property
    def meta(self) -> dict:
        return {
            "x-codex-turn-metadata": {
                "session_id": self.session_id,
                "turn_id": self.turn_id,
            }
        }

    def start(self) -> None:
        if self.proc is not None:
            return
        self.proc = subprocess.Popen(
            [self.node_repl],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
            env=_node_repl_env(self.entry),
            cwd=self.cwd,
        )
        init = self._rpc(
            "initialize",
            {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "agentic-chrome-control-poc", "version": "0.1"},
            },
            timeout=10,
        )
        if "error" in init:
            raise BridgeError(f"node_repl initialize failed: {init['error']}")
        self._send({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}})

    def _send(self, message: dict) -> None:
        if self.proc is None or self.proc.stdin is None:
            raise BridgeError("node_repl is not running")
        self.proc.stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
        self.proc.stdin.flush()

    def _read_message(self, timeout: float) -> dict:
        if self.proc is None or self.proc.stdout is None:
            raise BridgeError("node_repl is not running")
        ready, _, _ = select.select([self.proc.stdout], [], [], timeout)
        if not ready:
            raise BridgeError("timed out waiting for node_repl")
        line = self.proc.stdout.readline().rstrip("\n")
        if not line:
            raise BridgeError("node_repl closed stdout")
        return json.loads(line)

    def _rpc(self, method: str, params: dict, timeout: float = 30) -> dict:
        request_id = self.next_id
        self.next_id += 1
        self._send({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise BridgeError(f"timed out waiting for {method}")
            message = self._read_message(remaining)
            if message.get("id") == request_id:
                return message
            # The simple spike does not implement optional MCP client-side requests.
            if "id" in message and "method" in message:
                self._send(
                    {
                        "jsonrpc": "2.0",
                        "id": message["id"],
                        "error": {"code": -32601, "message": "PoC client request unsupported"},
                    }
                )

    def js(self, code: str, timeout_ms: int = 20_000) -> dict:
        self.start()
        response = self._rpc(
            "tools/call",
            {
                "name": "js",
                "arguments": {"code": code, "timeout_ms": timeout_ms},
                "_meta": self.meta,
            },
            timeout=max(30, timeout_ms / 1000 + 5),
        )
        if "error" in response:
            raise BridgeError(f"node_repl tools/call failed: {response['error']}")
        return response.get("result", {})

    def connect(self) -> dict:
        browser_client = json.dumps(self.browser_client)
        code = f"""
if (globalThis.agent == null) {{
  const {{ setupBrowserRuntime }} = await import({browser_client});
  globalThis.agent = await setupBrowserRuntime();
}}
if (globalThis.chrome == null) {{
  globalThis.chrome = await agent.browsers.get("chrome");
}}
const __browsers = await agent.browsers.list();
nodeRepl.write(JSON.stringify({{
  browserId: chrome.browserId,
  browsers: __browsers
}}));
"""
        result = self.js(code)
        self.connected = not bool(result.get("isError"))
        return result

    def exec(self, code: str, timeout_ms: int = 20_000) -> dict:
        if not self.connected:
            connected = self.connect()
            if connected.get("isError"):
                return connected
        wrapped = """{
  const __agenticPocValue = await (async () => {
%s
  })();
  if (__agenticPocValue !== undefined) {
    nodeRepl.write(
      typeof __agenticPocValue === "string"
        ? __agenticPocValue
        : JSON.stringify(__agenticPocValue)
    );
  }
}""" % code
        return self.js(wrapped, timeout_ms=timeout_ms)

    def close(self) -> None:
        if self.proc is None:
            return
        try:
            self.proc.terminate()
            self.proc.wait(timeout=2)
        except Exception:
            self.proc.kill()
        self.proc = None


def _recv_line(conn: socket.socket) -> dict:
    chunks: list[bytes] = []
    while True:
        chunk = conn.recv(65536)
        if not chunk:
            break
        chunks.append(chunk)
        if b"\n" in chunk:
            break
    data = b"".join(chunks).split(b"\n", 1)[0]
    if not data:
        raise BridgeError("empty local request")
    return json.loads(data)


def _send_line(conn: socket.socket, payload: dict) -> None:
    conn.sendall(json.dumps(payload, ensure_ascii=False).encode() + b"\n")


def serve() -> None:
    if SOCKET_PATH.exists():
        SOCKET_PATH.unlink()
    bridge = NodeReplBridge()
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(str(SOCKET_PATH))
    os.chmod(SOCKET_PATH, 0o600)
    server.listen(8)
    try:
        stopping = False
        while not stopping:
            conn, _ = server.accept()
            with conn:
                try:
                    request = _recv_line(conn)
                    command = request.get("command")
                    if command == "ping":
                        payload = {"ok": True, "connected": bridge.connected}
                    elif command == "connect":
                        payload = {"ok": True, "result": bridge.connect()}
                    elif command == "exec":
                        payload = {
                            "ok": True,
                            "result": bridge.exec(
                                str(request.get("code", "")),
                                int(request.get("timeoutMs", 20_000)),
                            ),
                        }
                    elif command == "stop":
                        payload = {"ok": True}
                        stopping = True
                    else:
                        raise BridgeError(f"unknown command: {command!r}")
                except Exception as exc:
                    payload = {"ok": False, "error": f"{type(exc).__name__}: {exc}"}
                _send_line(conn, payload)
    finally:
        bridge.close()
        server.close()
        try:
            SOCKET_PATH.unlink()
        except FileNotFoundError:
            pass


def _request(payload: dict, timeout: float = 35) -> dict:
    conn = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    conn.settimeout(timeout)
    conn.connect(str(SOCKET_PATH))
    try:
        _send_line(conn, payload)
        return _recv_line(conn)
    finally:
        conn.close()


def _server_alive() -> bool:
    if not SOCKET_PATH.exists():
        return False
    try:
        return bool(_request({"command": "ping"}, timeout=1).get("ok"))
    except Exception:
        return False


def _ensure_server() -> None:
    if _server_alive():
        return
    try:
        SOCKET_PATH.unlink()
    except FileNotFoundError:
        pass
    log = LOG_PATH.open("ab", buffering=0)
    subprocess.Popen(
        [sys.executable, str(Path(__file__).resolve()), "serve"],
        stdin=subprocess.DEVNULL,
        stdout=log,
        stderr=log,
        start_new_session=True,
        close_fds=True,
    )
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if _server_alive():
            return
        time.sleep(0.05)
    raise BridgeError(f"bridge daemon did not start; see {LOG_PATH}")


def main() -> int:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("serve")
    sub.add_parser("connect")
    sub.add_parser("status")
    sub.add_parser("stop")
    exec_parser = sub.add_parser("exec")
    exec_parser.add_argument("code")
    exec_parser.add_argument("--timeout-ms", type=int, default=20_000)
    args = parser.parse_args()

    if args.command == "serve":
        serve()
        return 0
    if args.command == "status":
        if not _server_alive():
            print(json.dumps({"ok": True, "running": False}, ensure_ascii=False))
            return 0
        result = _request({"command": "ping"})
        result["running"] = True
        print(json.dumps(result, ensure_ascii=False, indent=2))
        return 0
    if args.command == "stop":
        if not _server_alive():
            print(json.dumps({"ok": True, "running": False}, ensure_ascii=False))
            return 0
        print(json.dumps(_request({"command": "stop"}), ensure_ascii=False, indent=2))
        return 0

    _ensure_server()
    if args.command == "connect":
        payload = {"command": "connect"}
    else:
        payload = {"command": "exec", "code": args.code, "timeoutMs": args.timeout_ms}
    result = _request(payload, timeout=max(35, getattr(args, "timeout_ms", 20_000) / 1000 + 10))
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0 if result.get("ok") else 1


if __name__ == "__main__":
    raise SystemExit(main())
