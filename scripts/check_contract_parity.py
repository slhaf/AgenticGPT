#!/usr/bin/env python3
"""Live OpenAPI/MCP contract parity gate.

This gate deliberately uses the real Agent and Hub executables.  It does not
stand up a fake MCP server and it does not claim to exercise a production
tunnel: the standalone Agent streamable-HTTP listener is the deterministic
HTTP MCP fixture used here.
"""

from __future__ import annotations

import argparse
import contextlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import http.client
import json
import os
from pathlib import Path
import queue
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
from typing import Any, Iterable
from urllib.parse import urlencode, urlsplit

import yaml
from referencing import Registry
from jsonschema import Draft202012Validator, FormatChecker


PROTOCOL_VERSION = "2025-06-18"
TIMEOUT = 8.0
POLL = 0.05
DIAGNOSTIC_BYTES = 4000

ROOM_OPERATIONS: tuple[dict[str, Any], ...] = (
    {
        "name": "room.diary.active",
        "path": "/v1/room/diary/active",
        "operation_id": "roomDiaryActive",
        "request_schema": "RoomDiaryActiveRequest",
        "response_schema": "RoomDiaryActiveResponse",
        "required": (),
        "payload": {},
    },
    {
        "name": "room.diary.read",
        "path": "/v1/room/diary/read",
        "operation_id": "roomDiaryRead",
        "request_schema": "RoomDiaryReadRequest",
        "response_schema": "RoomDiaryReadResponse",
        "required": ("layer", "period"),
        "payload": {"layer": "daily", "period": "current"},
    },
    {
        "name": "room.notebook.recent",
        "path": "/v1/room/notebook/recent",
        "operation_id": "roomNotebookRecent",
        "request_schema": "RoomNotebookRecentRequest",
        "response_schema": "RoomNotebookResultsResponse",
        "required": (),
        "payload": {},
    },
    {
        "name": "room.notebook.search",
        "path": "/v1/room/notebook/search",
        "operation_id": "roomNotebookSearch",
        "request_schema": "RoomNotebookSearchRequest",
        "response_schema": "RoomNotebookResultsResponse",
        "required": ("query",),
        "payload": {"query": "parity"},
    },
    {
        "name": "room.notebook.read",
        "path": "/v1/room/notebook/read",
        "operation_id": "roomNotebookRead",
        "request_schema": "RoomNotebookReadRequest",
        "response_schema": "RoomNotebookReadResponse",
        "required": ("path",),
        "payload": {"path": "Notebook/contract.md"},
    },
    {
        "name": "room.state.list",
        "path": "/v1/room/state/list",
        "operation_id": "roomStateList",
        "request_schema": "RoomStateListRequest",
        "response_schema": "RoomStateListResponse",
        "required": (),
        "payload": {},
    },
    {
        "name": "room.state.read",
        "path": "/v1/room/state/read",
        "operation_id": "roomStateRead",
        "request_schema": "RoomStateReadRequest",
        "response_schema": "RoomStateReadResponse",
        "required": ("entity",),
        "payload": {"entity": "parity"},
    },
    {
        "name": "room.maintenance.status",
        "path": "/v1/room/maintenance/status",
        "operation_id": "roomMaintenanceStatus",
        "request_schema": "RoomMaintenanceStatusRequest",
        "response_schema": "RoomMaintenanceStatusResponse",
        "required": (),
        "payload": {},
    },
    {
        "name": "room.maintenance.submit",
        "path": "/v1/room/maintenance/submit",
        "operation_id": "roomMaintenanceSubmit",
        "request_schema": "RoomMaintenanceSubmitRequest",
        "response_schema": "RoomMaintenanceSubmitResponse",
        "required": ("items",),
        "payload": {
            "items": [
                {
                    "slot": "notebook",
                    "payload": {
                        "path": "Notebook/mcp.md",
                        "title": "MCP parity",
                        "body": "Full MCP Room dispatch",
                    },
                }
            ],
            "mode": "local",
            "waitSeconds": 0,
        },
    },
)
ROOM_OPERATION_BY_NAME = {item["name"]: item for item in ROOM_OPERATIONS}
ROOM_OPERATION_NAMES = set(ROOM_OPERATION_BY_NAME)
ROOM_RETIRED_NAMES = {
    "room.notebook.append",
    "room.notebook.selectExact",
    "room.notebook.current",
    "room.notebook.update",
    "room.notebook.remove",
    "room.diary.append",
    "room.diary.recent",
    "room.diary.selectExact",
}
ROOM_RETIRED_PATHS = {
    "/v1/room/notebook/append",
    "/v1/room/notebook/selectExact",
    "/v1/room/notebook/current",
    "/v1/room/notebook/update",
    "/v1/room/notebook/remove",
    "/v1/room/diary/append",
    "/v1/room/diary/recent",
    "/v1/room/diary/selectExact",
}
ROOM_FIXTURE_PAYLOAD = {
    "items": [
        {
            "slot": "diary.daily",
            "payload": {
                "summary": "Remote parity daily",
                "entries": [{"text": "Hub to Agent", "tags": ["parity"]}],
            },
        },
        {
            "slot": "diary.weekly",
            "payload": {"summary": "Remote parity weekly", "entries": []},
        },
        {
            "slot": "diary.monthly",
            "payload": {"summary": "Remote parity monthly", "entries": []},
        },
        {
            "slot": "notebook",
            "payload": {
                "path": "Notebook/contract.md",
                "title": "Contract parity",
                "body": "Hub remote Room",
            },
        },
        {
            "slot": "entity",
            "payload": {"entity": "parity", "content": "Room state parity"},
        },
    ],
    "mode": "local",
    "waitSeconds": 0,
}

ACTIVE_PROCESSES: list["ManagedProcess"] = []

class GateError(RuntimeError):
    pass


def fail(scenario: str, detail: str) -> None:
    raise GateError(f"[{scenario}] {detail}")


def run_checked(args: list[str], env: dict[str, str], scenario: str) -> subprocess.CompletedProcess[str]:
    try:
        result = subprocess.run(
            args,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=TIMEOUT,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        fail(scenario, f"command failed to run: {error}")
    if result.returncode != 0:
        fail(
            scenario,
            f"command exited {result.returncode}: {result.stderr[-1200:] or result.stdout[-1200:]}",
        )
    return result


def free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def tcp_ready(port: int) -> bool:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=0.3):
            return True
    except OSError:
        return False


def wait_until(predicate: Any, scenario: str, description: str, timeout: float = TIMEOUT,
               process: "ManagedProcess | None" = None) -> None:
    deadline = time.monotonic() + timeout
    last_error = ""
    while time.monotonic() < deadline:
        try:
            if predicate():
                return
        except Exception as error:  # pragma: no cover - included in gate diagnostics
            last_error = str(error)
        time.sleep(POLL)
    suffix = f" ({last_error})" if last_error else ""
    if process is not None:
        suffix += f"; child diagnostics: {process.diagnostics()}"
    fail(scenario, f"timed out waiting for {description}{suffix}")


def json_load_stdout(result: subprocess.CompletedProcess[str], scenario: str) -> Any:
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as error:
        fail(scenario, f"invalid JSON stdout: {error}: {result.stdout[-1000:]}")


def make_env(root: Path, extra: dict[str, str] | None = None) -> dict[str, str]:
    data_home = root / "data-home"
    cache_home = root / "cache-home"
    runtime_dir = root / "runtime"
    temp_dir = root / "tmp"
    for directory in (data_home, cache_home, runtime_dir, temp_dir):
        directory.mkdir(parents=True, exist_ok=True)
    runtime_dir.chmod(0o700)
    env = os.environ.copy()
    env.update(
        {
            "HOME": str(root / "home"),
            "XDG_CONFIG_HOME": str(root / "config-home"),
            "XDG_DATA_HOME": str(data_home),
            "XDG_CACHE_HOME": str(cache_home),
            "XDG_RUNTIME_DIR": str(runtime_dir),
            "TMPDIR": str(temp_dir),
        }
    )
    if extra:
        env.update(extra)
    return env


def init_agent(binary: Path, root: Path, mode: str, profile: str, agent_id: str,
               hub_url: str | None = None, agent_secret: str | None = None) -> tuple[Path, dict[str, Any], dict[str, str]]:
    root.mkdir(parents=True, exist_ok=True)
    (root / "home").mkdir(parents=True, exist_ok=True)
    (root / "config-home").mkdir(parents=True, exist_ok=True)
    config_path = root / "config.json"
    env = make_env(root)
    run_checked(
        [str(binary), "config", "--config", str(config_path), "init", "--non-interactive"],
        env,
        f"{mode}/{profile} config init",
    )
    try:
        config = json.loads(config_path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        fail(f"{mode}/{profile} config", f"cannot read generated config: {error}")
    workspace = root / "workspace"
    workspace.mkdir(parents=True, exist_ok=True)
    config["mode"] = mode
    config["profile"] = profile
    config["agentId"] = agent_id
    config["displayName"] = f"Contract parity {profile}"
    config["workspaceRoot"] = str(workspace)
    config.setdefault("pathPolicy", {})["writeRoots"] = [str(workspace)]
    config["confirmationProvider"] = {"channels": []}
    config["toolsets"] = {
        "enabled": ["agent", "file", "mcp", "process", "skills", "tmux", "room"]
    }
    if mode == "hub":
        config["hub"] = {
            "url": hub_url,
            "transport": "websocket",
            "agentSecret": agent_secret,
        }
        config["tunnel"] = None
    else:
        config["tunnel"] = None
    config_path.write_text(json.dumps(config, indent=2) + "\n")
    return config_path, config, env


class ManagedProcess:
    def __init__(self, args: list[str], env: dict[str, str], scenario: str,
                 stdin: int | None = subprocess.DEVNULL) -> None:
        self.args = args
        self.scenario = scenario
        self.stdout_capture = tempfile.TemporaryFile(mode="w+b")
        self.stderr_capture = tempfile.TemporaryFile(mode="w+b")
        self.closed = False
        try:
            self.process = subprocess.Popen(
                args,
                env=env,
                stdin=stdin,
                stdout=self.stdout_capture,
                stderr=self.stderr_capture,
                start_new_session=True,
            )
        except OSError as error:
            self.stdout_capture.close()
            self.stderr_capture.close()
            fail(scenario, f"could not start process: {error}")
        ACTIVE_PROCESSES.append(self)

    def alive(self) -> bool:
        return self.process.poll() is None

    def diagnostics(self) -> str:
        def tail(capture: Any) -> str:
            try:
                capture.flush()
                capture.seek(0, os.SEEK_END)
                capture.seek(max(0, capture.tell() - DIAGNOSTIC_BYTES), os.SEEK_SET)
                return capture.read(DIAGNOSTIC_BYTES).decode("utf-8", errors="replace")
            except (OSError, ValueError):
                return "<capture unavailable>"

        return (
            f"args={self.args!r}; stdout={tail(self.stdout_capture)!r}; "
            f"stderr={tail(self.stderr_capture)!r}"
        )

    def _close_captures(self) -> None:
        if self.closed:
            return
        self.closed = True
        with contextlib.suppress(OSError, ValueError):
            self.stdout_capture.close()
        with contextlib.suppress(OSError, ValueError):
            self.stderr_capture.close()

    def stop(self) -> None:
        if self.process.poll() is None:
            try:
                os.killpg(self.process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                with contextlib.suppress(ProcessLookupError):
                    os.killpg(self.process.pid, signal.SIGKILL)
                with contextlib.suppress(subprocess.TimeoutExpired):
                    self.process.wait(timeout=2)
        if self.process.poll() is not None:
            with contextlib.suppress(subprocess.SubprocessError):
                self.process.wait(timeout=0)
            self._close_captures()


class HttpResponse:
    def __init__(self, status: int, headers: dict[str, str], body: bytes) -> None:
        self.status = status
        self.headers = {key.lower(): value for key, value in headers.items()}
        self.body = body

    def json(self, scenario: str) -> Any:
        try:
            return json.loads(self.body.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            fail(scenario, f"invalid JSON response (HTTP {self.status}): {error}; {self.body[:500]!r}")


def http_request(port: int, method: str, path: str, body: Any | None = None,
                 headers: dict[str, str] | None = None, scenario: str = "HTTP") -> HttpResponse:
    request_headers = {"Host": f"127.0.0.1:{port}", "Connection": "close"}
    if headers:
        request_headers.update(headers)
    payload = None
    if body is not None:
        payload = json.dumps(body, separators=(",", ":")).encode()
        request_headers.setdefault("Content-Type", "application/json")
        request_headers["Content-Length"] = str(len(payload))
    try:
        connection = http.client.HTTPConnection("127.0.0.1", port, timeout=TIMEOUT)
        connection.request(method, path, body=payload, headers=request_headers)
        response = connection.getresponse()
        result = HttpResponse(response.status, dict(response.getheaders()), response.read())
        connection.close()
        return result
    except (OSError, http.client.HTTPException) as error:
        fail(scenario, f"HTTP request failed: {error}")

class ConfirmationReceiver:
    def __init__(self, topic: str) -> None:
        self.topic = topic
        self.events: queue.Queue[dict[str, Any]] = queue.Queue()
        receiver = self

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self) -> None:  # noqa: N802 - stdlib handler hook
                try:
                    length = int(self.headers.get("Content-Length", "0"))
                except ValueError:
                    length = 0
                if length > 64 * 1024:
                    self.send_error(413)
                    return
                try:
                    payload = json.loads(self.rfile.read(length))
                except (OSError, json.JSONDecodeError):
                    self.send_error(400)
                    return
                if not isinstance(payload, dict):
                    self.send_error(400)
                    return
                receiver.events.put(payload)
                body = b"{}"
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, _format: str, *_args: Any) -> None:
                return

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.port = int(self.server.server_address[1])
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def approve_once(self, scenario: str) -> None:
        try:
            event = self.events.get(timeout=TIMEOUT)
        except queue.Empty:
            fail(scenario, "timed out waiting for the real Hub confirmation publication")
        if event.get("topic") != self.topic:
            fail(scenario, f"unexpected confirmation topic: {event}")
        actions = event.get("actions")
        if not isinstance(actions, list):
            fail(scenario, f"confirmation publication omitted actions: {event}")
        action = next(
            (item for item in actions if isinstance(item, dict) and item.get("label") == "Allow once"),
            None,
        )
        callback = action.get("url") if isinstance(action, dict) else None
        if not isinstance(callback, str):
            fail(scenario, f"confirmation publication omitted Allow once callback: {event}")
        parsed = urlsplit(callback)
        if parsed.scheme != "http" or parsed.hostname != "127.0.0.1" or parsed.port is None:
            fail(scenario, f"confirmation callback escaped loopback: {callback}")
        callback_path = parsed.path or "/"
        if parsed.query:
            callback_path += f"?{parsed.query}"
        response = http_request(
            parsed.port,
            "POST",
            callback_path,
            {},
            {"Content-Type": "application/json"},
            scenario,
        )
        if response.status != 200:
            fail(scenario, f"confirmation callback returned HTTP {response.status}: {response.body[:500]!r}")
        value = response.json(scenario)
        if value.get("status") != "accepted" or value.get("decision") != "allow_once":
            fail(scenario, f"confirmation callback did not accept allow_once: {value}")

    def close(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)


def parse_sse(body: bytes, scenario: str) -> dict[str, Any]:
    text = body.decode("utf-8", errors="replace")
    data: list[str] = []
    for line in text.splitlines():
        if line.startswith("data:"):
            data.append(line[5:].lstrip())
    if not data:
        fail(scenario, f"MCP response was not an SSE message: {text[:500]!r}")
    try:
        value = json.loads("\n".join(data))
    except json.JSONDecodeError as error:
        fail(scenario, f"MCP SSE data was invalid JSON: {error}: {text[:500]!r}")
    if not isinstance(value, dict):
        fail(scenario, f"MCP response was not an object: {value!r}")
    return value


def parse_mcp_response(response: HttpResponse, scenario: str) -> dict[str, Any]:
    media_type = response.headers.get("content-type", "").split(";", 1)[0].strip()
    if media_type == "text/event-stream":
        return parse_sse(response.body, scenario)
    if media_type != "application/json":
        fail(scenario, f"unsupported MCP response media type: {media_type}")
    value = response.json(scenario)
    if not isinstance(value, dict):
        fail(scenario, f"MCP response was not an object: {value!r}")
    return value


def mcp_exchange(port: int, token: str, request: dict[str, Any], session: str | None,
                 scenario: str) -> tuple[HttpResponse, dict[str, Any]]:
    headers = {
        "Authorization": f"Bearer {token}",
        "Accept": "application/json, text/event-stream",
        "MCP-Protocol-Version": PROTOCOL_VERSION,
    }
    if session:
        headers["Mcp-Session-Id"] = session
    response = http_request(port, "POST", "/mcp", request, headers, scenario)
    if response.status not in (200, 202):
        fail(scenario, f"MCP HTTP status {response.status}: {response.body[:500]!r}")
    if response.status == 202 or not response.body:
        return response, {}
    return response, parse_mcp_response(response, scenario)




def open_mcp_session(port: int, token: str, label: str) -> str:
    headers = {"Authorization": f"Bearer {token}", "Accept": "application/json, text/event-stream"}
    response = http_request(
        port,
        "POST",
        "/mcp",
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "contract-parity", "version": "1"},
            },
        },
        headers,
        f"{label} MCP initialize",
    )
    if response.status != 200:
        fail(f"{label} MCP initialize", f"HTTP {response.status}")
    message = parse_mcp_response(response, f"{label} MCP initialize")
    if "error" in message or not message.get("result", {}).get("protocolVersion"):
        fail(f"{label} MCP initialize", f"unexpected result: {message}")
    session = response.headers.get("mcp-session-id", "")
    initialized = http_request(
        port,
        "POST",
        "/mcp",
        {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}},
        {
            "Authorization": f"Bearer {token}",
            "Accept": "application/json, text/event-stream",
            **({"Mcp-Session-Id": session} if session else {}),
            "MCP-Protocol-Version": PROTOCOL_VERSION,
        },
        f"{label} MCP initialized",
    )
    if initialized.status not in (200, 202):
        fail(f"{label} MCP initialized", f"HTTP {initialized.status}")
    return session


def mcp_call(port: int, token: str, session: str, request_id: int, method: str,
             params: dict[str, Any], label: str) -> dict[str, Any]:
    _, message = mcp_exchange(
        port,
        token,
        {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params},
        session,
        label,
    )
    return message


def tool_names(tools: Iterable[dict[str, Any]]) -> set[str]:
    return {str(tool.get("name")) for tool in tools}


def descriptor_map(tools: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    return {str(tool.get("name")): tool for tool in tools}


def assert_room_tool_semantics(
    tools: list[dict[str, Any]], label: str, require_defaults: bool = False
) -> None:
    descriptors = descriptor_map(tools)
    missing = ROOM_OPERATION_NAMES.difference(descriptors)
    if missing:
        fail(label, f"missing current Room descriptors: {sorted(missing)}")
    retired = ROOM_RETIRED_NAMES.intersection(descriptors)
    if retired:
        fail(label, f"retired Room descriptors still advertised: {sorted(retired)}")
    for spec in ROOM_OPERATIONS:
        name = spec["name"]
        descriptor = descriptors[name]
        schema = descriptor.get("inputSchema", {})
        required = set(schema.get("required", []))
        if required != set(spec["required"]):
            fail(label, f"{name} required fields changed: {sorted(required)}")
        properties = schema.get("properties", {})
        if "agentId" in properties:
            fail(label, f"{name} unexpectedly exposes an agentId selector")
        annotations = descriptor.get("annotations", {})
        read_only = name != "room.maintenance.submit"
        if annotations.get("readOnlyHint") is not read_only:
            fail(label, f"{name} readOnlyHint is not {read_only}")
        if annotations.get("destructiveHint") is not (not read_only):
            fail(label, f"{name} destructiveHint is not {not read_only}")
        if annotations.get("openWorldHint") is not False:
            fail(label, f"{name} openWorldHint is not false")
        if name in {"room.notebook.recent", "room.notebook.search"}:
            limit = properties.get("limit", {})
            if (
                limit.get("minimum") != 1
                or limit.get("maximum") != 100
                or (require_defaults and limit.get("default") != 20)
            ):
                fail(label, f"{name} limit is not default20/clamped1..100")
        if name == "room.notebook.search":
            query = properties.get("query", {})
            if require_defaults:
                if query.get("minLength") != 1 or query.get("maxLength") != 256:
                    fail(label, "room.notebook.search query bounds are not 1..256")
            elif (
                ("minLength" in query and query.get("minLength") != 1)
                or ("maxLength" in query and query.get("maxLength") != 256)
            ):
                fail(label, "room.notebook.search declared query bounds are not 1..256")
        if name == "room.maintenance.submit":
            items = properties.get("items", {})
            if items.get("minItems") != 1 or items.get("maxItems") != 5:
                fail(label, "room.maintenance.submit items bounds are not 1..5")
            wait_seconds = properties.get("waitSeconds", {})
            if (
                wait_seconds.get("minimum") != 0
                or wait_seconds.get("maximum") != 30
                or wait_seconds.get("default") != 0
            ):
                fail(label, "room.maintenance.submit waitSeconds is not default0/clamped0..30")


def assert_tool_semantics(
    tools: list[dict[str, Any]], label: str, hub: bool = False, coordinator: bool = False
) -> None:
    descriptors = descriptor_map(tools)
    required = {
        "process.exec", "process.batch", "process.status", "process.list",
        "process.output", "process.result", "process.cancel",
    }
    if hub:
        required |= {"hub.process.status", "hub.process.list"}
    if coordinator:
        required = {"hub.process.status", "hub.process.list"}
    for name in required:
        if name not in descriptors:
            fail(label, f"missing required descriptor {name}")

    for name in ("process.status", "process.output", "process.result", "process.cancel", "hub.process.status"):
        if name not in descriptors:
            continue
        schema = descriptors[name].get("inputSchema", {})
        if "processId" not in schema.get("required", []):
            fail(label, f"{name} processId is not required")
    status_props = descriptors.get("process.status", {}).get("inputSchema", {}).get("properties", {})
    wait_seconds = status_props.get("waitSeconds", {})
    if "process.status" in descriptors and (wait_seconds.get("minimum") != 0 or wait_seconds.get("maximum") != 30):
        fail(label, "process.status waitSeconds bounds are not 0..30")
    output_props = descriptors.get("process.output", {}).get("inputSchema", {}).get("properties", {})
    output_bytes = output_props.get("maxBytes", {})
    if "process.output" in descriptors and (
        output_bytes.get("default") != 8192 or output_bytes.get("maximum") != 32768
    ):
        fail(label, "process.output maxBytes is not default8192/max32768")
    list_name = "hub.process.list" if coordinator else "process.list"
    list_schema = descriptors.get(list_name, {}).get("inputSchema", {})
    list_props = list_schema.get("properties", {})
    if coordinator:
        if "agentId" not in list_schema.get("required", []) or "limit" in list_props:
            fail(label, "hub.process.list must require agentId and omit limit")
    else:
        if list_props.get("limit", {}).get("minimum") != 1 or list_props.get("limit", {}).get("maximum") != 100:
            fail(label, "process.list limit bounds are not 1..100")
        if list_props.get("limit", {}).get("default") != 50:
            fail(label, "process.list limit default is not 50")

    retired = {"job.get", "job.list", "job.cancel", "hub.job.get", "hub.job.list"}
    if retired.intersection(descriptors):
        fail(label, f"retired Job tools are still advertised: {sorted(retired.intersection(descriptors))}")
def assert_skill_semantics(tools: list[dict[str, Any]], label: str) -> None:
    descriptors = descriptor_map(tools)
    for name in ("skills.install.get", "skills.run"):
        descriptor = descriptors.get(name)
        if descriptor is None:
            fail(label, f"missing required Skill descriptor {name}")
        wait_schema = descriptor.get("inputSchema", {}).get("properties", {}).get("waitSeconds", {})
        if wait_schema.get("minimum") != 0 or wait_schema.get("maximum") != 30 or wait_schema.get("default") != 5:
            fail(label, f"{name} waitSeconds must be default5/clamped0..30")


def prepare_skill_fixture(config_path: Path) -> None:
    try:
        workspace = Path(json.loads(config_path.read_text())["workspaceRoot"])
        scripts = workspace / "skills" / "demo" / "scripts"
        scripts.mkdir(parents=True, exist_ok=True)
        (workspace / "skills" / "demo" / "SKILL.md").write_text("# Contract parity demo\n")
        check = scripts / "check.sh"
        check.write_text("#!/bin/sh\nprintf skill-parity\\n")
        check.chmod(0o755)
    except (OSError, KeyError, TypeError, json.JSONDecodeError) as error:
        fail("Skill fixture", f"could not create local Skill fixture: {error}")
def inline_skill_install_request() -> dict[str, Any]:
    return {
        "id": "inline",
        "source": {
            "type": "files",
            "files": [
                {"path": "SKILL.md", "content": "# Inline install\n"},
                {
                    "path": "scripts/check.sh",
                    "content": "#!/bin/sh\nprintf inline-skill\\n",
                    "executable": True,
                },
            ],
        },
        "activateAfterInstall": True,
    }

PROCESS_MARKERS = (
    "contract-parity-http-completed",
    "contract-parity-hub-full",
    "contract-parity-hub-completed",
    "contract-parity-batch-one",
    "contract-parity-batch-two",
)
PROCESS_OVERFLOW_MARKER = "overflow-" + "x" * 20000


def configure_process_fixture(config_path: Path, http_port: int | None = None,
                              http_token: str | None = None) -> None:
    try:
        data = json.loads(config_path.read_text())
        data["policy"] = {
            "allow": [
                {"program": "/usr/bin/printf", "argsPrefix": [marker]}
                for marker in PROCESS_MARKERS
            ]
            + [{"program": "/usr/bin/printf", "argsPrefix": [PROCESS_OVERFLOW_MARKER]}]
            + [
                {"program": "/usr/bin/true", "argsPrefix": []},
                {"program": "/usr/bin/sleep", "argsPrefix": ["30"]},
            ],
            "confirm": [],
            "deny": [{"program": "/usr/bin/echo", "argsPrefix": []}],
        }
        if http_port is not None and http_token is not None:
            data["confirmationProvider"] = {"channels": ["ntfy"]}
            data["mcpServers"] = {
                "standalone-http": {
                    "enabled": True,
                    "transport": "streamable-http",
                    "url": f"http://127.0.0.1:{http_port}/mcp",
                    "auth": {"type": "bearer", "token": http_token},
                }
            }
        config_path.write_text(json.dumps(data, indent=2) + "\n")
    except (OSError, TypeError, json.JSONDecodeError) as error:
        fail("process fixture", f"could not configure private process fixture: {error}")

def load_openapi(path: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    try:
        document = yaml.safe_load(path.read_text())
    except (OSError, yaml.YAMLError) as error:
        fail("schema/load", f"cannot load OpenAPI YAML: {error}")
    if not isinstance(document, dict) or document.get("openapi") != "3.1.0":
        fail("schema/load", "OpenAPI document is not 3.1.0")
    schemas = document.get("components", {}).get("schemas", {})
    if not isinstance(schemas, dict):
        fail("schema/load", "components.schemas is missing")
    for name, schema in schemas.items():
        try:
            Draft202012Validator.check_schema(schema)
        except Exception as error:
            fail("schema/check", f"{name} is not a Draft 2020-12 schema: {error}")
    pending = [document]
    while pending:
        node = pending.pop()
        if isinstance(node, dict):
            if "$ref" in node:
                resolve_local_ref(document, {"$ref": node["$ref"]})
            pending.extend(node.values())
        elif isinstance(node, list):
            pending.extend(node)
    return document, schemas
def schema_validator(document: dict[str, Any], schema: dict[str, Any]) -> Draft202012Validator:
    return Draft202012Validator(
        {**schema, "components": document["components"]},
        registry=Registry(),
        format_checker=FormatChecker(),
    )


def validate_instance(document: dict[str, Any], schemas: dict[str, Any], name: str,
                      instance: Any, scenario: str) -> None:
    if name not in schemas:
        fail(scenario, f"OpenAPI schema {name} is missing")
    errors = sorted(schema_validator(document, schemas[name]).iter_errors(instance), key=str)
    if errors:
        error = errors[0]
        fail(scenario, f"{name} validation failed at {list(error.path)}: {error.message}")
def assert_rejected(document: dict[str, Any], schemas: dict[str, Any], name: str,
                    instance: Any, scenario: str) -> None:
    if name not in schemas:
        fail(scenario, f"OpenAPI schema {name} is missing")
    errors = list(schema_validator(document, schemas[name]).iter_errors(instance))
    if not errors:
        fail(scenario, f"{name} unexpectedly accepted invalid instance")


def resolve_local_ref(document: dict[str, Any], value: Any) -> Any:
    seen: set[str] = set()
    while isinstance(value, dict) and set(value) == {"$ref"}:
        reference = value["$ref"]
        if not isinstance(reference, str) or not reference.startswith("#/"):
            fail("schema/response", f"unsupported non-local schema reference: {reference!r}")
        if reference in seen:
            fail("schema/response", f"cyclic reference-only schema: {reference}")
        seen.add(reference)
        target: Any = document
        try:
            for segment in reference[2:].split("/"):
                key = segment.replace("~1", "/").replace("~0", "~")
                target = target[int(key)] if isinstance(target, list) else target[key]
        except (KeyError, IndexError, TypeError, ValueError):
            fail("schema/response", f"unresolved local schema reference: {reference}")
        value = target
    return value


def validate_operation_response(document: dict[str, Any], path: str, method: str,
                                status: int, instance: Any, scenario: str,
                                media_type: str = "application/json") -> None:
    operation = document.get("paths", {}).get(path, {}).get(method.lower(), {})
    response = operation.get("responses", {}).get(str(status))
    if response is None:
        fail(scenario, f"OpenAPI has no declared HTTP {status} response")
    response = resolve_local_ref(document, response)
    schema = response.get("content", {}).get(media_type, {}).get("schema")
    if schema is None:
        fail(scenario, f"OpenAPI HTTP {status} response has no {media_type} schema")
    schema = resolve_local_ref(document, schema)
    errors = sorted(schema_validator(document, schema).iter_errors(instance), key=str)
    if errors:
        error = errors[0]
        fail(scenario, f"declared HTTP {status} response validation failed at {list(error.path)}: {error.message}")



def require_schema_contract(document: dict[str, Any], schemas: dict[str, Any]) -> None:
    paths = document.get("paths", {})
    if any(path.startswith("/v1/jobs") for path in paths):
        fail("schema/contract", "retired /v1/jobs HTTP routes are still advertised")
    process_paths = {
        "/v1/process/exec", "/v1/process/batch", "/v1/process",
        "/v1/process/{processId}", "/v1/process/{processId}/output",
        "/v1/process/{processId}/result", "/v1/process/{processId}/cancel",
    }
    missing = process_paths - set(paths)
    if missing:
        fail("schema/contract", f"required process HTTP routes are missing: {sorted(missing)}")
    list_parameters = paths["/v1/process"]["get"].get("parameters", [])
    list_by_name = {item["name"]: item for item in list_parameters}
    list_limit = list_by_name["limit"]["schema"]
    if list_limit.get("default") != 50 or list_limit.get("minimum") != 1 or list_limit.get("maximum") != 100:
        fail("schema/contract", "HTTP process list limit is not default50/clamped1..100")
    status_parameters = paths["/v1/process/{processId}"]["get"].get("parameters", [])
    status_by_name = {item["name"]: item for item in status_parameters}
    if "waitOnly" in status_by_name:
        fail("schema/contract", "HTTP process status exposes retired waitOnly")
    wait_schema = status_by_name.get("waitSeconds", {}).get("schema", {})
    if wait_schema.get("minimum") != 0 or wait_schema.get("maximum") != 30:
        fail("schema/contract", "HTTP process status waitSeconds is not clamped0..30")
    output_parameters = paths["/v1/process/{processId}/output"]["get"].get("parameters", [])
    output_by_name = {item["name"]: item for item in output_parameters}
    if "cursor" not in output_by_name:
        fail("schema/contract", "HTTP process output does not declare a cursor")
    output_bytes = output_by_name.get("maxBytes", {}).get("schema", {})
    if output_bytes.get("default") != 8192 or output_bytes.get("maximum") != 32768:
        fail("schema/contract", "HTTP process output maxBytes is not default8192/max32768")
    for name in (
        "ProcessExecRequest", "ProcessBatchExecRequest", "ProcessResponse", "ProcessBatchResponse",
        "ProcessStatusResponse", "ProcessCacheFallbackResponse", "ProcessListResponse",
        "ProcessOutputSegment", "ProcessOutputResponse", "ProcessResultResponse",
        "ProcessCancelResponse", "ProcessUnavailableResponse",
    ):
        if not isinstance(schemas.get(name), dict):
            fail("schema/contract", f"OpenAPI process schema {name} is missing")

    for path in ROOM_RETIRED_PATHS:
        if path in paths:
            fail("schema/contract", f"retired Room HTTP path is still advertised: {path}")
    for spec in ROOM_OPERATIONS:
        path = spec["path"]
        operation = paths.get(path, {}).get("post")
        if not isinstance(operation, dict):
            fail("schema/contract", f"current Room HTTP path is missing: {path}")
        if operation.get("requestBody", {}).get("required") is not True:
            fail("schema/contract", f"{path} JSON request body is not required")
        if operation.get("operationId") != spec["operation_id"]:
            fail("schema/contract", f"{path} operationId is not {spec['operation_id']}")
        if any(parameter.get("name") == "agentId" for parameter in operation.get("parameters", [])):
            fail("schema/contract", f"{path} exposes an agentId selector parameter")
        body_schema = (
            operation.get("requestBody", {})
            .get("content", {})
            .get("application/json", {})
            .get("schema")
        )
        if body_schema is None:
            fail("schema/contract", f"{path} has no JSON request schema")
        body_schema = resolve_local_ref(document, body_schema)
        if not isinstance(body_schema, dict):
            fail("schema/contract", f"{path} request schema is malformed")
        if body_schema.get("properties", {}).get("agentId") is not None:
            fail("schema/contract", f"{path} request exposes an agentId selector")
        request_schema = schemas.get(spec["request_schema"])
        if not isinstance(request_schema, dict):
            fail("schema/contract", f"OpenAPI schema {spec['request_schema']} is missing")
        if set(body_schema.get("required", [])) != set(spec["required"]):
            fail("schema/contract", f"{path} resolved request required fields changed")
        if set(request_schema.get("required", [])) != set(spec["required"]):
            fail("schema/contract", f"{spec['request_schema']} required fields changed")
        success = resolve_local_ref(document, operation.get("responses", {}).get("200", {}))
        if not isinstance(success, dict):
            fail("schema/contract", f"{path} success response is malformed")
        success_schema = resolve_local_ref(
            document,
            success.get("content", {}).get("application/json", {}).get("schema"),
        )
        if not isinstance(success_schema, dict) or not success_schema:
            fail("schema/contract", f"{path} success response schema is missing")
        if not isinstance(schemas.get(spec["response_schema"]), dict):
            fail("schema/contract", f"OpenAPI schema {spec['response_schema']} is missing")

    recent = schemas["RoomNotebookRecentRequest"]["properties"]["limit"]
    search = schemas["RoomNotebookSearchRequest"]["properties"]
    search_limit = search["limit"]
    if (
        recent.get("default") != 20
        or recent.get("minimum") != 1
        or recent.get("maximum") != 100
        or search_limit.get("default") != 20
        or search_limit.get("minimum") != 1
        or search_limit.get("maximum") != 100
    ):
        fail("schema/contract", "Room Notebook limit is not default20/clamped1..100")
    if search["query"].get("minLength") != 1 or search["query"].get("maxLength") != 256:
        fail("schema/contract", "RoomNotebookSearchRequest query bounds are not 1..256")
    submit = schemas["RoomMaintenanceSubmitRequest"]
    items = submit["properties"]["items"]
    if items.get("minItems") != 1 or items.get("maxItems") != 5:
        fail("schema/contract", "RoomMaintenanceSubmitRequest items bounds are not 1..5")
    wait_seconds = submit["properties"]["waitSeconds"]
    if (
        wait_seconds.get("default") != 0
        or wait_seconds.get("minimum") != 0
        or wait_seconds.get("maximum") != 30
    ):
        fail("schema/contract", "RoomMaintenanceSubmitRequest waitSeconds is not default0/clamped0..30")


def json_result(value: dict[str, Any], scenario: str) -> Any:
    if "error" in value:
        fail(scenario, f"JSON-RPC error: {value['error']}")
    result = value.get("result")
    if not isinstance(result, dict):
        fail(scenario, f"JSON-RPC result missing: {value}")
    structured = result.get("structuredContent")
    if structured is not None:
        return structured
    content = result.get("content")
    if isinstance(content, list):
        for item in content:
            if isinstance(item, dict) and item.get("type") == "text":
                try:
                    return json.loads(item.get("text", ""))
                except json.JSONDecodeError:
                    pass
    return result


def local_tool(binary: Path, config: Path, name: str, args: dict[str, Any], env: dict[str, str], scenario: str) -> Any:
    result = run_checked(
        [str(binary), "local", "--config", str(config), "call", name, "--arguments", json.dumps(args)],
        env,
        scenario,
    )
    return json_load_stdout(result, scenario)


def local_surface(binary: Path, config: Path, env: dict[str, str], scenario: str) -> list[dict[str, Any]]:
    result = run_checked([str(binary), "local", "--config", str(config), "list-tools"], env, scenario)
    value = json_load_stdout(result, scenario)
    if not isinstance(value, list):
        fail(scenario, "local list-tools did not return an array")
    return value


def start_local_agent(binary: Path, root: Path, reports: list[str]) -> tuple[ManagedProcess, Path, dict[str, str], list[dict[str, Any]]]:
    config, _, env = init_agent(binary, root, "local", "normal", "parity-local")
    prepare_skill_fixture(config)
    configure_process_fixture(config)
    process = ManagedProcess([str(binary), "run", "--config", str(config)], env, "Agent local Unix MCP")
    socket_path = Path(env["HOME"]) / ".agentic_gpt" / "runtime" / "agent" / "parity-local" / "mcp.sock"
    wait_until(
        lambda: socket_path.exists() and process.alive(),
        "Agent local Unix MCP",
        "private Unix MCP socket",
        process=process,
    )
    tools = local_surface(binary, config, env, "Agent local Unix tools/list")
    info = local_tool(binary, config, "agent.info", {}, env, "Agent local agent.info")
    info = info.get("structuredContent", info)
    if info.get("identity", {}).get("transport") != "local-unix":
        fail("Agent local agent.info", f"wrong transport: {info}")
    local_exec = local_tool(
        binary,
        config,
        "process.exec",
        {"program": "/usr/bin/printf", "args": [PROCESS_MARKERS[0]], "waitSeconds": 5},
        env,
        "Agent local process.exec",
    )
    local_exec = local_exec.get("structuredContent", local_exec)
    local_process_id = local_exec.get("processId")
    inline_output = local_exec.get("inlineOutput", {})
    inline_stdout = inline_output.get("stdout", {})
    if (
        not local_process_id
        or local_exec.get("status") != "completed"
        or inline_stdout.get("data") != PROCESS_MARKERS[0]
        or inline_stdout.get("encoding") != "utf8"
        or len(json.dumps(local_exec, separators=(",", ":")).encode()) > 8192
    ):
        fail("Agent local process.exec", f"printf did not complete with full inline output: {local_exec}")
    local_status = local_tool(
        binary, config, "process.status", {"processId": local_process_id, "waitSeconds": 0}, env,
        "Agent local process.status",
    )
    local_status = local_status.get("structuredContent", local_status)
    if local_status.get("state") != "completed" or any(key in local_status for key in ("stdout", "stderr", "result")):
        fail("Agent local process.status", f"terminal status exposed body data: {local_status}")
    first_page = local_tool(
        binary, config, "process.output", {"processId": local_process_id, "maxBytes": 8}, env,
        "Agent local process.output first page",
    )
    first_page = first_page.get("structuredContent", first_page)
    segment = first_page.get("stdout", {})
    segments = [first_page.get("stdout", {}), first_page.get("stderr", {})]
    encoded_page_bytes = sum(
        len(item.get("data", "").encode("utf-8"))
        for item in segments
        if isinstance(item, dict)
    )
    if (
        first_page.get("processId") != local_process_id
        or not segment.get("data")
        or segment.get("startOffset") != "0"
        or int(segment.get("endOffset", "0")) <= 0
        or segment.get("encoding") not in {"utf8", "base64"}
        or encoded_page_bytes > 8
        or first_page.get("eof") is not False
        or first_page.get("hasMore") is not True
        or "captureStatus" not in first_page
        or not first_page.get("nextCursor")
    ):
        fail("Agent local process.output first page", f"first output page did not advance within aggregate budget: {first_page}")
    second_page = local_tool(
        binary, config, "process.output",
        {"processId": local_process_id, "cursor": first_page["nextCursor"], "maxBytes": 64}, env,
        "Agent local process.output continuation",
    )
    second_page = second_page.get("structuredContent", second_page)
    if (
        second_page.get("eof") is not True
        or second_page.get("hasMore") is not False
        or second_page.get("nextCursor") == first_page.get("nextCursor")
    ):
        fail("Agent local process.output continuation", f"terminal EOF/cursor did not advance: {second_page}")
    invalid_cursor_rejected = False
    try:
        invalid_cursor = local_tool(
            binary, config, "process.output",
            {"processId": local_process_id, "cursor": "not-a-valid-cursor", "maxBytes": 8},
            env, "Agent local process.output invalid cursor",
        )
        invalid_cursor_rejected = (
            invalid_cursor.get("isError") is True
            or "error" in invalid_cursor
            or "invalid cursor" in json.dumps(invalid_cursor).lower()
        )
    except GateError:
        invalid_cursor_rejected = True
    if not invalid_cursor_rejected:
        fail("Agent local process.output invalid cursor", "invalid cursor was silently treated as a fresh cursor")
    process_result = local_tool(
        binary, config, "process.result", {"processId": local_process_id, "maxBytes": 8192}, env, "Agent local process.result",
    )
    process_result = process_result.get("structuredContent", process_result)
    if process_result.get("status") != "unavailable" or process_result.get("resultAvailable") is not False:
        fail("Agent local process.result", f"command result was not accurately reported unavailable: {process_result}")
    reports.append("PASS Agent local Unix MCP process status/output cursor and EOF/result retrieval")
    overflow_response = local_tool(
        binary,
        config,
        "process.exec",
        {"program": "/usr/bin/printf", "args": [PROCESS_OVERFLOW_MARKER], "waitSeconds": 5},
        env,
        "Agent local process.exec overflow",
    )
    overflow_response = overflow_response.get("structuredContent", overflow_response)
    preview = overflow_response.get("outputPreview", {})
    preview_bytes = (
        len(preview.get("stdout", "").encode("utf-8"))
        + len(preview.get("stderr", "").encode("utf-8"))
    )
    overflow_process_id = overflow_response.get("processId")
    if (
        overflow_response.get("status") != "completed"
        or overflow_process_id is None
        or "inlineOutput" in overflow_response
        or preview.get("truncated") is not True
        or preview_bytes > 2048
        or len(json.dumps(overflow_response, separators=(",", ":")).encode()) > 8192
    ):
        fail("Agent local process.exec overflow", f"creation overflow preview/aggregate cap was incorrect: {overflow_response}")
    local_tool(binary, config, "skills.setActive", {"id": "demo", "active": True}, env, "Agent local Skill activate")
    skill_value = local_tool(binary, config, "skills.run", {"id": "demo", "path": "scripts/check.sh", "waitSeconds": 0}, env, "Agent local Skill run")
    skill_value = skill_value.get("structuredContent", skill_value)
    skill_id = skill_value.get("processId")
    if skill_value.get("state") not in {"starting", "running", "completed"} or not skill_id:
        fail("Agent local Skill run", f"invalid real Skill process envelope: {skill_value}")
    skill_done = local_tool(binary, config, "process.status", {"processId": skill_id, "waitSeconds": 5}, env, "Agent local Skill completion")
    skill_done = skill_done.get("structuredContent", skill_done)
    if skill_done.get("state") != "completed" or "stdout" in skill_done or "stderr" in skill_done or "result" in skill_done:
        fail("Agent local Skill completion", f"status was not metadata-only terminal state: {skill_done}")
    skill_output = local_tool(binary, config, "process.output", {"processId": skill_id, "maxBytes": 8192}, env, "Agent local Skill output")
    skill_output = skill_output.get("structuredContent", skill_output)
    if "skill-parity" not in skill_output.get("stdout", {}).get("data", ""):
        fail("Agent local Skill output", f"Skill output was not retrievable: {skill_output}")
    install_value = local_tool(binary, config, "skills.install", inline_skill_install_request(), env, "Agent local Skill install")
    install_value = install_value.get("structuredContent", install_value)
    install_id = install_value.get("installId")
    if not install_id:
        fail("Agent local Skill install", f"missing real install id: {install_value}")
    local_tool(binary, config, "skills.install.get", {"installId": install_id, "waitSeconds": 0}, env, "Agent local Skill install get zero")
    installed = local_tool(binary, config, "skills.install.get", {"installId": install_id, "waitSeconds": 5}, env, "Agent local Skill install get short")
    installed = installed.get("structuredContent", installed)
    if installed.get("status") != "completed" or not isinstance(installed.get("result"), dict):
        fail("Agent local Skill install get short", f"inline install did not complete: {installed}")
    installed_run = local_tool(binary, config, "skills.run", {"id": "inline", "path": "scripts/check.sh", "waitSeconds": 0}, env, "Agent local installed Skill run")
    installed_run = installed_run.get("structuredContent", installed_run)
    installed_id = installed_run.get("processId")
    if not installed_id:
        fail("Agent local installed Skill run", f"missing installed process id: {installed_run}")
    installed_done = local_tool(binary, config, "process.status", {"processId": installed_id, "waitSeconds": 5}, env, "Agent local installed Skill completion")
    installed_done = installed_done.get("structuredContent", installed_done)
    if installed_done.get("state") != "completed" or "stdout" in installed_done:
        fail("Agent local installed Skill completion", f"status was not terminal metadata: {installed_done}")
    installed_output = local_tool(binary, config, "process.output", {"processId": installed_id, "maxBytes": 8192}, env, "Agent local installed Skill output")
    installed_output = installed_output.get("structuredContent", installed_output)
    if "inline-skill" not in installed_output.get("stdout", {}).get("data", ""):
        fail("Agent local installed Skill output", f"installed Skill output was not retrievable: {installed_output}")
    reports.append("PASS Agent local Unix MCP: tools/list, agent.info, Skill run/completion, inline install/get(0,5), and installed run/completion")
    return process, config, env, tools


def start_http_agent(binary: Path, root: Path, reports: list[str]) -> tuple[ManagedProcess, Path, dict[str, str], int, str, list[dict[str, Any]]]:
    port = free_port()
    config, _, env = init_agent(binary, root, "standalone", "normal", "parity-http")
    prepare_skill_fixture(config)
    token = "contract-parity-http-token"
    data = json.loads(config.read_text())
    data["httpMcp"] = {
        "enabled": True,
        "host": "127.0.0.1",
        "port": port,
        "publicUrl": None,
        "bearerToken": "env:CONTRACT_PARITY_HTTP_TOKEN",
        "allowHosts": ["localhost", "127.0.0.1", "::1"],
    }
    data["tunnel"] = {
        "tunnelId": "contract-parity-http",
        "apiKey": "env:CONTRACT_PARITY_TUNNEL_KEY",
        "hubReporting": {"enabled": False, "detail": "metadata"},
    }
    config.write_text(json.dumps(data, indent=2) + "\n")
    configure_process_fixture(config)
    env = dict(env, CONTRACT_PARITY_HTTP_TOKEN=token, CONTRACT_PARITY_TUNNEL_KEY="unused-test-tunnel", AGENTIC_GPT_SUPERVISOR_TOKEN="contract-parity-supervisor")
    process = ManagedProcess(
        [str(binary), "stdio-worker", "--config", str(config), "--profile", "normal", "--supervisor-token", "contract-parity-supervisor"],
        env,
        "Agent standalone HTTP MCP",
        stdin=subprocess.PIPE,
    )
    wait_until(
        lambda: tcp_ready(port) and process.alive(),
        "Agent standalone HTTP MCP",
        "standalone HTTP MCP listener",
        process=process,
    )
    session = open_mcp_session(port, token, "Agent standalone HTTP MCP")
    message = mcp_call(port, token, session, 2, "tools/list", {}, "Agent standalone HTTP tools/list")
    tools = message.get("result", {}).get("tools")
    if not isinstance(tools, list):
        fail("Agent standalone HTTP tools/list", f"missing tools: {message}")
    activate = mcp_call(port, token, session, 3, "tools/call", {"name": "skills.setActive", "arguments": {"id": "demo", "active": True}}, "Agent HTTP Skill activate")
    if "error" in activate or activate.get("result", {}).get("isError"):
        fail("Agent HTTP Skill activate", f"real Skill activation returned an error: {activate}")
    skill_value = json_result(mcp_call(port, token, session, 4, "tools/call", {"name": "skills.run", "arguments": {"id": "demo", "path": "scripts/check.sh", "waitSeconds": 0}}, "Agent HTTP Skill run"), "Agent HTTP Skill run")
    skill_id = skill_value.get("processId") if isinstance(skill_value, dict) else None
    if not skill_id or skill_value.get("state") not in {"starting", "running", "completed"}:
        fail("Agent HTTP Skill run", f"invalid real Skill process envelope: {skill_value}")
    skill_done = json_result(mcp_call(port, token, session, 5, "tools/call", {"name": "process.status", "arguments": {"processId": skill_id, "waitSeconds": 5}}, "Agent HTTP Skill completion"), "Agent HTTP Skill completion")
    if skill_done.get("state") != "completed" or "stdout" in skill_done or "result" in skill_done:
        fail("Agent HTTP Skill completion", f"status was not metadata-only terminal state: {skill_done}")
    skill_output = json_result(mcp_call(port, token, session, 6, "tools/call", {"name": "process.output", "arguments": {"processId": skill_id, "maxBytes": 8192}}, "Agent HTTP Skill output"), "Agent HTTP Skill output")
    if "skill-parity" not in skill_output.get("stdout", {}).get("data", ""):
        fail("Agent HTTP Skill output", f"Skill output was not retrievable: {skill_output}")
    install_value = json_result(mcp_call(port, token, session, 7, "tools/call", {"name": "skills.install", "arguments": inline_skill_install_request()}, "Agent HTTP Skill install"), "Agent HTTP Skill install")
    install_id = install_value.get("installId") if isinstance(install_value, dict) else None
    if not install_id:
        fail("Agent HTTP Skill install", f"missing real install id: {install_value}")
    mcp_call(port, token, session, 8, "tools/call", {"name": "skills.install.get", "arguments": {"installId": install_id, "waitSeconds": 0}}, "Agent HTTP Skill install get zero")
    installed = json_result(mcp_call(port, token, session, 9, "tools/call", {"name": "skills.install.get", "arguments": {"installId": install_id, "waitSeconds": 5}}, "Agent HTTP Skill install get short"), "Agent HTTP Skill install get short")
    if installed.get("status") != "completed" or not isinstance(installed.get("result"), dict):
        fail("Agent HTTP Skill install get short", f"inline install did not complete: {installed}")
    installed_run = json_result(mcp_call(port, token, session, 10, "tools/call", {"name": "skills.run", "arguments": {"id": "inline", "path": "scripts/check.sh", "waitSeconds": 0}}, "Agent HTTP installed Skill run"), "Agent HTTP installed Skill run")
    installed_id = installed_run.get("processId")
    if not installed_id:
        fail("Agent HTTP installed Skill run", f"missing installed process id: {installed_run}")
    installed_done = json_result(mcp_call(port, token, session, 11, "tools/call", {"name": "process.status", "arguments": {"processId": installed_id, "waitSeconds": 5}}, "Agent HTTP installed Skill completion"), "Agent HTTP installed Skill completion")
    if installed_done.get("state") != "completed" or "stdout" in installed_done:
        fail("Agent HTTP installed Skill completion", f"status was not terminal metadata: {installed_done}")
    installed_output = json_result(mcp_call(port, token, session, 12, "tools/call", {"name": "process.output", "arguments": {"processId": installed_id, "maxBytes": 8192}}, "Agent HTTP installed Skill output"), "Agent HTTP installed Skill output")
    if "inline-skill" not in installed_output.get("stdout", {}).get("data", ""):
        fail("Agent HTTP installed Skill output", f"installed Skill output was not retrievable: {installed_output}")
    reports.append("PASS Agent standalone streamable HTTP MCP: tools/list, Skill run/status/result, inline install/get(0,5), and installed run/status/result")
    return process, config, env, port, token, tools


def start_hub(binary: Path, root: Path, profile: str, reports: list[str], db: Path | None = None,
              confirmation: ConfirmationReceiver | None = None) -> tuple[ManagedProcess, int, Path, dict[str, str], str]:
    root.mkdir(parents=True, exist_ok=True)
    (root / "home").mkdir(parents=True, exist_ok=True)
    port = free_port()
    db_path = db or root / "hub.db"
    config_path = root / "hub.json"
    env = make_env(root)
    run_checked([str(binary), "--db", str(db_path), "--config", str(config_path), "init"], env, f"Hub {profile} init")
    if confirmation is not None:
        try:
            config = json.loads(config_path.read_text())
            config["remoteConfirmation"] = {
                "enabled": True,
                "provider": "ntfy",
                "timeoutSeconds": 45,
                "ntfy": {
                    "serverUrl": f"http://127.0.0.1:{confirmation.port}",
                    "topic": confirmation.topic,
                    "callbackBaseUrl": f"http://127.0.0.1:{port}",
                },
            }
            config_path.write_text(json.dumps(config, indent=2) + "\n")
        except (OSError, TypeError, json.JSONDecodeError) as error:
            fail(f"Hub {profile} confirmation fixture", f"could not configure remote confirmation: {error}")
    api_key = f"contract-parity-{profile}-api-key"
    process = ManagedProcess(
        [str(binary), "--db", str(db_path), "--config", str(config_path), "serve", "--bind", f"127.0.0.1:{port}", "--api-key", api_key, "--mcp-profile", profile],
        env,
        f"Hub {profile} process",
    )
    wait_until(
        lambda: tcp_ready(port) and process.alive(),
        f"Hub {profile} process",
        "Hub HTTP listener",
        process=process,
    )
    reports.append(f"PASS Hub {profile} HTTP process started")
    return process, port, config_path, env, api_key


def register_hub_agent(hub_binary: Path, db: Path, config: Path, env: dict[str, str], agent_id: str, display: str, secret: str) -> None:
    run_checked(
        [str(hub_binary), "--db", str(db), "--config", str(config), "agent", "add", "--agent-id", agent_id, "--display-name", display, "--secret", secret],
        env,
        f"Hub register {agent_id}",
    )


def start_hub_agent(binary: Path, root: Path, profile: str, agent_id: str, hub_url: str, secret: str,
                    reports: list[str], downstream: tuple[int, str] | None = None) -> tuple[ManagedProcess, Path, dict[str, str]]:
    config, _, env = init_agent(binary, root, "hub", profile, agent_id, hub_url, secret)
    if downstream is not None:
        configure_process_fixture(config, *downstream)
    process = ManagedProcess([str(binary), "run", "--config", str(config)], env, f"Hub Agent {profile}")
    reports.append(f"START Hub Agent {profile} ({agent_id})")
    return process, config, env
def start_reporting_agent(
    binary: Path,
    root: Path,
    agent_id: str,
    hub_url: str,
    secret: str,
    reports: list[str],
) -> tuple[ManagedProcess, Path, dict[str, str]]:
    config, _, env = init_agent(binary, root, "standalone", "room", agent_id)
    try:
        data = json.loads(config.read_text())
        data["hub"] = {
            "url": hub_url,
            "transport": "websocket",
            "agentSecret": secret,
        }
        data["tunnel"] = {
            "tunnelId": f"contract-parity-reporting-{agent_id}",
            "apiKey": "env:CONTRACT_PARITY_REPORTING_TUNNEL_KEY",
            "client": {"autoDownload": False},
            "hubReporting": {"enabled": True, "detail": "metadata"},
        }
        config.write_text(json.dumps(data, indent=2) + "\n")
    except (OSError, TypeError, json.JSONDecodeError) as error:
        fail("Hub ReportingOnly fixture", f"could not configure reporting Agent: {error}")
    env = dict(
        env,
        CONTRACT_PARITY_REPORTING_TUNNEL_KEY="unused-local-reporting-key",
        AGENTIC_GPT_SUPERVISOR_TOKEN="contract-parity-reporting-supervisor",
    )
    process = ManagedProcess(
        [
            str(binary),
            "stdio-worker",
            "--config",
            str(config),
            "--profile",
            "room",
            "--supervisor-token",
            "contract-parity-reporting-supervisor",
        ],
        env,
        "Hub ReportingOnly Room Agent",
        stdin=subprocess.PIPE,
    )
    reports.append(f"START Hub ReportingOnly Room Agent ({agent_id})")
    return process, config, env





def hub_json(port: int, api_key: str, method: str, path: str, body: Any | None, scenario: str) -> tuple[HttpResponse, Any]:
    response = http_request(
        port,
        method,
        path,
        body,
        {"Authorization": f"Bearer {api_key}"},
        scenario,
    )
    value = response.json(scenario) if response.body else None
    return response, value
def room_repository_root(config_path: Path) -> Path:
    try:
        config = json.loads(config_path.read_text())
        workspace = Path(config["workspaceRoot"])
        configured = (config.get("room") or {}).get("repositoryRoot")
        if configured:
            candidate = Path(os.path.expanduser(str(configured)))
            return candidate if candidate.is_absolute() else workspace / candidate
        return workspace / "room"
    except (OSError, KeyError, TypeError, json.JSONDecodeError) as error:
        fail("Room repository", f"could not resolve repository root: {error}")


def git_stdout(root: Path, env: dict[str, str], args: list[str], scenario: str) -> str:
    result = run_checked(["git", "-C", str(root), *args], env, scenario)
    return result.stdout.strip()


def room_http_call(
    port: int,
    api_key: str,
    document: dict[str, Any],
    schemas: dict[str, Any],
    name: str,
    payload: dict[str, Any],
    scenario: str,
    expected_status: int = 200,
) -> tuple[HttpResponse, dict[str, Any]]:
    spec = ROOM_OPERATION_BY_NAME[name]
    validate_instance(document, schemas, spec["request_schema"], payload, f"{scenario} request")
    response, value = hub_json(port, api_key, "POST", spec["path"], payload, scenario)
    if response.status != expected_status:
        fail(scenario, f"HTTP {response.status}: {value}")
    if not isinstance(value, dict):
        fail(scenario, f"response is not an object: {value}")
    validate_operation_response(
        document,
        spec["path"],
        "post",
        expected_status,
        value,
        scenario,
    )
    if expected_status == 200:
        validate_instance(document, schemas, spec["response_schema"], value, f"{scenario} response")
    return response, value


def room_mcp_call(
    port: int,
    api_key: str,
    session: str,
    request_id: int,
    document: dict[str, Any],
    schemas: dict[str, Any],
    name: str,
    payload: dict[str, Any],
    scenario: str,
) -> dict[str, Any]:
    spec = ROOM_OPERATION_BY_NAME[name]
    validate_instance(document, schemas, spec["request_schema"], payload, f"{scenario} request")
    message = mcp_call(
        port,
        api_key,
        session,
        request_id,
        "tools/call",
        {"name": name, "arguments": payload},
        scenario,
    )
    value = json_result(message, scenario)
    if not isinstance(value, dict):
        fail(scenario, f"response is not an object: {value}")
    validate_instance(document, schemas, spec["response_schema"], value, f"{scenario} response")
    return value


def wait_for_agent_offline(port: int, api_key: str, agent_id: str, scenario: str) -> None:
    def offline() -> bool:
        response, value = hub_json(port, api_key, "GET", "/v1/agents", None, scenario)
        if response.status != 200:
            return False
        return not any(item.get("agentId") == agent_id and item.get("online") is True for item in value.get("agents", []))

    wait_until(offline, scenario, f"disconnected Agent {agent_id}")



def wait_for_agent(port: int, api_key: str, agent_id: str, scenario: str,
                   process: ManagedProcess | None = None) -> None:
    def online() -> bool:
        response, value = hub_json(port, api_key, "GET", "/v1/agents", None, scenario)
        if response.status != 200:
            return False
        return any(item.get("agentId") == agent_id and item.get("online") is True for item in value.get("agents", []))
    wait_until(online, scenario, f"connected Agent {agent_id}", process=process)


def process_request(port: int, api_key: str, agent_id: str, program: str, args: list[str], group: str,
                    wait_seconds: int | None = 0, scenario: str = "Hub process.exec") -> dict[str, Any]:
    payload: dict[str, Any] = {
        "agentId": agent_id,
        "program": program,
        "args": args,
        "group": group,
        "needConfirm": False,
    }
    if wait_seconds is not None:
        payload["waitSeconds"] = wait_seconds
    response, value = hub_json(port, api_key, "POST", "/v1/process/exec", payload, scenario)
    if response.status != 200:
        fail(scenario, f"HTTP {response.status}: {value}")
    if not isinstance(value, dict):
        fail(scenario, f"response is not an object: {value}")
    return value

def confirmed_hub_json(port: int, api_key: str, method: str, path: str, body: Any,
                       receiver: ConfirmationReceiver, scenario: str) -> tuple[HttpResponse, Any]:
    result: dict[str, Any] = {}

    def request() -> None:
        try:
            result["value"] = hub_json(port, api_key, method, path, body, scenario)
        except BaseException as error:  # propagate the real request failure below
            result["error"] = error

    thread = threading.Thread(target=request, daemon=True)
    thread.start()
    try:
        receiver.approve_once(scenario)
    finally:
        thread.join(timeout=TIMEOUT)
    if thread.is_alive():
        fail(scenario, "Hub request remained blocked after real confirmation callback")
    if "error" in result:
        error = result["error"]
        if isinstance(error, GateError):
            raise error
        fail(scenario, f"Hub request failed after confirmation: {error}")
    value = result.get("value")
    if not isinstance(value, tuple) or len(value) != 2:
        fail(scenario, f"Hub request did not return an HTTP response: {value!r}")
    return value


def run_runtime_gate(root: Path, agent_binary: Path, hub_binary: Path,
                     document: dict[str, Any], schemas: dict[str, Any], reports: list[str]) -> None:
    local_process = http_process = normal_process = room_process = reporting_process = coordinator_process = None
    hub_process = None
    confirmation: ConfirmationReceiver | None = None
    try:
        local_process, local_config, local_env, local_tools = start_local_agent(agent_binary, root / "local", reports)
        http_process, http_config, http_env, http_port, http_token, http_tools = start_http_agent(
            agent_binary, root / "http", reports
        )
        assert_tool_semantics(local_tools, "Agent local descriptor")
        assert_tool_semantics(http_tools, "Agent HTTP descriptor")
        assert_room_tool_semantics(local_tools, "Agent local current Room descriptors")
        assert_room_tool_semantics(http_tools, "Agent HTTP current Room descriptors")
        assert_skill_semantics(local_tools, "Agent local Skill descriptors")
        assert_skill_semantics(http_tools, "Agent HTTP Skill descriptors")
        if descriptor_map(local_tools).keys() != descriptor_map(http_tools).keys():
            fail("Agent local/HTTP parity", "tools/list names differ")
        for name in ("process.status", "process.list", "process.output", "process.result", "process.cancel", "skills.install.get", "skills.run"):
            if descriptor_map(local_tools)[name].get("inputSchema") != descriptor_map(http_tools)[name].get("inputSchema"):
                fail("Agent local/HTTP parity", f"{name} input schemas differ")

        http_session = open_mcp_session(http_port, http_token, "Agent HTTP process call")
        http_exec = json_result(
            mcp_call(
                http_port,
                http_token,
                http_session,
                3,
                "tools/call",
                {
                    "name": "process.exec",
                    "arguments": {
                        "program": "/usr/bin/printf",
                        "args": [PROCESS_MARKERS[0]],
                        "waitSeconds": 5,
                    },
                },
                "Agent HTTP process.exec",
            ),
            "Agent HTTP process.exec",
        )
        http_process_id = http_exec.get("processId")
        http_inline = http_exec.get("inlineOutput", {})
        if (
            not http_process_id
            or http_exec.get("status") != "completed"
            or http_exec.get("completedInline") is not True
            or http_inline.get("stdout", {}).get("data") != PROCESS_MARKERS[0]
            or http_inline.get("stdout", {}).get("encoding") != "utf8"
        ):
            fail("Agent HTTP process.exec", f"printf did not complete with full inline output: {http_exec}")
        if len(json.dumps(http_exec, separators=(",", ":")).encode()) > 8192:
            fail("Agent HTTP process.exec", f"creation response exceeded the inline response budget: {http_exec}")
        reports.append("PASS Agent local/HTTP descriptor parity and HTTP process printf dispatch")

        confirmation = ConfirmationReceiver(
            f"contract-parity-{os.getpid()}-{time.monotonic_ns()}",
        )
        hub_root = root / "hub"
        hub_process, hub_port, hub_config, hub_env, hub_key = start_hub(
            hub_binary, hub_root, "full", reports, confirmation=confirmation
        )
        normal_id, room_id = "parity-normal", "parity-room"
        normal_secret, room_secret = "parity-normal-secret", "parity-room-secret"
        register_hub_agent(hub_binary, hub_root / "hub.db", hub_config, hub_env, normal_id, "Parity normal", normal_secret)
        register_hub_agent(hub_binary, hub_root / "hub.db", hub_config, hub_env, room_id, "Parity room", room_secret)
        normal_process, normal_config, normal_env = start_hub_agent(
            agent_binary,
            root / "hub-normal",
            "normal",
            normal_id,
            f"http://127.0.0.1:{hub_port}",
            normal_secret,
            reports,
            downstream=(http_port, http_token),
        )
        room_process, room_config, room_env = start_hub_agent(
            agent_binary, root / "hub-room", "room", room_id, f"http://127.0.0.1:{hub_port}", room_secret, reports
        )
        wait_for_agent(hub_port, hub_key, normal_id, "Hub normal Agent connection", normal_process)
        wait_for_agent(hub_port, hub_key, room_id, "Hub Room Agent connection", room_process)
        reports.append("PASS Hub Full with connected normal and active Room Agent")

        full_session = open_mcp_session(hub_port, hub_key, "Hub Full")
        _, full_tools_message = mcp_exchange(
            hub_port,
            hub_key,
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
            full_session,
            "Hub Full tools/list",
        )
        full_tools = full_tools_message.get("result", {}).get("tools", [])
        assert_tool_semantics(full_tools, "Hub Full descriptor", hub=True)
        assert_room_tool_semantics(full_tools, "Hub Full current Room descriptor", require_defaults=True)
        assert_skill_semantics(full_tools, "Hub Full Skill descriptors")
        reports.append("PASS Hub Full MCP tools/list metadata")

        full_exec = json_result(
            mcp_call(
                hub_port,
                hub_key,
                full_session,
                3,
                "tools/call",
                {
                    "name": "process.exec",
                    "arguments": {
                        "agentId": normal_id,
                        "program": "/usr/bin/printf",
                        "args": [PROCESS_MARKERS[1]],
                        "needConfirm": False,
                        "waitSeconds": 5,
                    },
                },
                "Hub Full process.exec printf",
            ),
            "Hub Full process.exec printf",
        )
        full_id = full_exec.get("processId") if isinstance(full_exec, dict) else None
        full_inline = full_exec.get("inlineOutput", {}) if isinstance(full_exec, dict) else {}
        if (
            not full_id
            or full_exec.get("status") != "completed"
            or full_exec.get("completedInline") is not True
            or full_inline.get("stdout", {}).get("data") != PROCESS_MARKERS[1]
            or full_inline.get("stdout", {}).get("encoding") != "utf8"
            or len(json.dumps(full_exec, separators=(",", ":")).encode()) > 8192
        ):
            fail("Hub Full process.exec printf", f"missing complete inline process creation response: {full_exec}")
        full_status = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 4, "tools/call",
                {"name": "process.status", "arguments": {"agentId": normal_id, "processId": full_id, "waitSeconds": 1}},
                "Hub Full process.status",
            ),
            "Hub Full process.status",
        )
        if (
            full_status.get("processId") != full_id
            or full_status.get("state") != "completed"
            or any(key in full_status for key in ("stdout", "stderr", "result"))
        ):
            fail("Hub Full process.status", f"status did not return metadata-only completion: {full_status}")
        full_result = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 5, "tools/call",
                {"name": "process.result", "arguments": {"agentId": normal_id, "processId": full_id, "maxBytes": 8192}},
                "Hub Full process.result",
            ),
            "Hub Full process.result",
        )
        if full_result.get("status") != "unavailable" or full_result.get("resultAvailable") is not False:
            fail("Hub Full process.result", f"command result was not accurately reported unavailable: {full_result}")

        hub_output = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 73, "tools/call",
                {"name": "process.output", "arguments": {"agentId": normal_id, "processId": full_id, "maxBytes": 8192}},
                "Hub Full process.output",
            ),
            "Hub Full process.output",
        )
        if (
            hub_output.get("processId") != full_id
            or hub_output.get("stdout", {}).get("data") != PROCESS_MARKERS[1]
            or hub_output.get("eof") is not True
        ):
            fail("Hub Full process.output", f"retained command output was not retrieved: {hub_output}")

        http_first_response, http_first = hub_json(
            hub_port, hub_key, "GET",
            f"/v1/process/{full_id}/output?" + urlencode({"agentId": normal_id, "maxBytes": 8}),
            None, "Hub HTTP process.output first page",
        )
        if http_first_response.status != 200:
            fail("Hub HTTP process.output first page", f"HTTP {http_first_response.status}: {http_first}")
        validate_operation_response(document, "/v1/process/{processId}/output", "get", 200, http_first, "Hub HTTP process.output first page")
        if (
            http_first.get("processId") != full_id
            or http_first.get("stdout", {}).get("startOffset") != "0"
            or not http_first.get("stdout", {}).get("data")
            or len(http_first["stdout"]["data"].encode()) > 8
            or http_first.get("hasMore") is not True
            or not http_first.get("nextCursor")
        ):
            fail("Hub HTTP process.output first page", f"bounded cursor page was not returned: {http_first}")
        http_next_response, http_next = hub_json(
            hub_port, hub_key, "GET",
            f"/v1/process/{full_id}/output?" + urlencode({"agentId": normal_id, "cursor": http_first["nextCursor"], "maxBytes": 64}),
            None, "Hub HTTP process.output continuation",
        )
        if http_next_response.status != 200:
            fail("Hub HTTP process.output continuation", f"HTTP {http_next_response.status}: {http_next}")
        validate_operation_response(document, "/v1/process/{processId}/output", "get", 200, http_next, "Hub HTTP process.output continuation")
        if (
            http_next.get("stdout", {}).get("startOffset") != http_first["stdout"]["endOffset"]
            or http_next.get("stdout", {}).get("data") == http_first["stdout"]["data"]
            or http_next.get("eof") is not True
            or http_next.get("hasMore") is not False
        ):
            fail("Hub HTTP process.output continuation", f"cursor replayed output or missed EOF: {http_next}")
        http_result_response, http_result = hub_json(
            hub_port, hub_key, "GET",
            f"/v1/process/{full_id}/result?" + urlencode({"agentId": normal_id, "maxBytes": 8192}),
            None, "Hub HTTP process.result",
        )
        if http_result_response.status != 200:
            fail("Hub HTTP process.result", f"HTTP {http_result_response.status}: {http_result}")
        validate_operation_response(document, "/v1/process/{processId}/result", "get", 200, http_result, "Hub HTTP process.result")
        if http_result.get("status") != "unavailable" or http_result.get("resultAvailable") is not False or "result" in http_result:
            fail("Hub HTTP process.result", f"command result was not accurately unavailable: {http_result}")
        reports.append("PASS Hub MCP and live HTTP process.output cursor/EOF and HTTP process.result")

        denied = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 6, "tools/call",
                {"name": "process.exec", "arguments": {
                    "agentId": normal_id, "program": "/usr/bin/echo",
                    "args": ["must-be-denied"], "needConfirm": False, "waitSeconds": 5,
                }},
                "Hub Full policy denied process",
            ),
            "Hub Full policy denied process",
        )
        if (
            denied.get("status") != "rejected"
            or not denied.get("processId")
            or denied.get("error", {}).get("code") != "policy_denied"
        ):
            fail("Hub Full policy denied process", f"policy denial lacked rejected process evidence: {denied}")
        reports.append("PASS Hub Full process.exec completion, metadata-only status, retained result, and policy denial")

        coordinator_root = root / "coordinator"
        coordinator_process, coordinator_port, coordinator_config, coordinator_env, coordinator_key = start_hub(
            hub_binary, coordinator_root, "coordinator", reports
        )
        coordinator_session = open_mcp_session(coordinator_port, coordinator_key, "Hub Coordinator")
        _, coordinator_tools_message = mcp_exchange(
            coordinator_port,
            coordinator_key,
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
            coordinator_session,
            "Hub Coordinator tools/list",
        )
        coordinator_tools = coordinator_tools_message.get("result", {}).get("tools", [])
        assert_tool_semantics(coordinator_tools, "Hub Coordinator descriptor", hub=True, coordinator=True)
        names = tool_names(coordinator_tools)
        leaked = ROOM_OPERATION_NAMES.intersection(names) | ROOM_RETIRED_NAMES.intersection(names)
        if {"process.exec", "process.batch", "process.status", "process.output", "process.result", "process.cancel", "job.get"}.intersection(names) or leaked:
            fail("Hub Coordinator profile", f"execution or Room tool leaked into tools/list: {sorted(leaked)}")
        hidden = mcp_call(
            coordinator_port,
            coordinator_key,
            coordinator_session,
            3,
            "tools/call",
            {"name": "process.exec", "arguments": {"agentId": normal_id, "program": "/usr/bin/true"}},
            "Hub Coordinator hidden call",
        )
        hidden_error = hidden.get("error", {}).get("message", "") if isinstance(hidden.get("error"), dict) else ""
        if "tool_unavailable_for_profile: process.exec" not in hidden_error:
            fail("Hub Coordinator hidden call", f"hidden execution was not rejected by profile guard: {hidden}")
        for request_id, spec in enumerate(ROOM_OPERATIONS, start=20):
            hidden_room = mcp_call(
                coordinator_port,
                coordinator_key,
                coordinator_session,
                request_id,
                "tools/call",
                {"name": spec["name"], "arguments": json.loads(json.dumps(spec["payload"]))},
                f"Hub Coordinator hidden {spec['name']}",
            )
            if f"tool_unavailable_for_profile: {spec['name']}" not in json.dumps(hidden_room, sort_keys=True):
                fail(f"Hub Coordinator hidden {spec['name']}", f"current Room call was not rejected: {hidden_room}")
        reports.append("PASS Hub Coordinator profile hides and rejects execution and all nine current Room calls")

        completed_request = {
            "agentId": normal_id,
            "program": "/usr/bin/printf",
            "args": [PROCESS_MARKERS[2]],
            "group": "parity-completed",
            "needConfirm": False,
            "waitSeconds": 5,
        }
        validate_instance(document, schemas, "ProcessExecRequest", completed_request, "Hub HTTP process.exec request")
        invalid_exec = dict(completed_request)
        invalid_exec.pop("needConfirm")
        assert_rejected(document, schemas, "ProcessExecRequest", invalid_exec, "Hub HTTP process.exec negative request")
        completed = process_request(
            hub_port,
            hub_key,
            normal_id,
            "/usr/bin/printf",
            [PROCESS_MARKERS[2]],
            "parity-completed",
            5,
            "Hub HTTP completed printf",
        )
        validate_operation_response(document, "/v1/process/exec", "post", 200, completed, "Hub HTTP completed printf")
        completed_id = completed.get("processId")
        if (
            not completed_id
            or completed.get("status") != "completed"
            or completed.get("completedInline") is not True
            or completed.get("inlineOutput", {}).get("stdout", {}).get("data") != PROCESS_MARKERS[2]
        ):
            fail("Hub HTTP completed printf", f"expected complete inline process output: {completed}")
        if len(json.dumps(completed, separators=(",", ":")).encode()) > 8192:
            fail("Hub HTTP completed printf", f"creation response exceeded 8192 bytes: {completed}")

        active = process_request(
            hub_port, hub_key, normal_id, "/usr/bin/sleep", ["30"], "parity-active", 0,
            "Hub HTTP active process",
        )
        validate_operation_response(document, "/v1/process/exec", "post", 200, active, "Hub HTTP active process")
        active_id = active.get("processId")
        if not active_id or active.get("status") in {"completed", "failed", "cancelled", "rejected", "timed_out"}:
            fail("Hub HTTP active process", f"sleep30 was not active: {active}")

        status_response, status_body = hub_json(
            hub_port, hub_key, "GET",
            f"/v1/process/{active_id}?" + urlencode({"agentId": normal_id, "waitSeconds": "0"}),
            None, "Hub HTTP process.status",
        )
        if status_response.status != 200:
            fail("Hub HTTP process.status", f"HTTP {status_response.status}: {status_body}")
        validate_operation_response(document, "/v1/process/{processId}", "get", 200, status_body, "Hub HTTP process.status")
        if (
            status_body.get("processId") != active_id
            or status_body.get("state") in {"completed", "failed", "cancelled", "rejected", "timed_out"}
            or any(key in status_body for key in ("stdout", "stderr", "result"))
        ):
            fail("Hub HTTP process.status", f"status is not active metadata-only response: {status_body}")

        cancel_response, cancel_body = hub_json(
            hub_port, hub_key, "POST",
            f"/v1/process/{active_id}/cancel?" + urlencode({"agentId": normal_id}),
            {}, "Hub HTTP process.cancel",
        )
        if cancel_response.status != 200:
            fail("Hub HTTP process.cancel", f"HTTP {cancel_response.status}: {cancel_body}")
        validate_operation_response(document, "/v1/process/{processId}/cancel", "post", 200, cancel_body, "Hub HTTP process.cancel")
        if (
            cancel_body.get("processId") != active_id
            or cancel_body.get("state") != "cancelled"
            or cancel_body.get("cancelOutcome") != "cancelled"
            or not cancel_body.get("terminationEvidence")
        ):
            fail("Hub HTTP process.cancel", f"missing observed cancellation evidence: {cancel_body}")

        batch_payload = {
            "agentId": normal_id,
            "elements": [
                {"program": "/usr/bin/printf", "args": [PROCESS_MARKERS[3]]},
                {"program": "/usr/bin/printf", "args": [PROCESS_MARKERS[4]]},
            ],
            "needConfirm": False,
            "waitSeconds": 5,
        }
        validate_instance(document, schemas, "ProcessBatchExecRequest", batch_payload, "Hub HTTP process.batch request")
        invalid_batch = dict(batch_payload)
        invalid_batch.pop("needConfirm")
        assert_rejected(document, schemas, "ProcessBatchExecRequest", invalid_batch, "Hub HTTP process.batch negative request")
        batch_response, batch_body = hub_json(
            hub_port, hub_key, "POST", "/v1/process/batch", batch_payload, "Hub HTTP process.batch"
        )
        if batch_response.status != 200:
            fail("Hub HTTP process.batch", f"HTTP {batch_response.status}: {batch_body}")
        validate_operation_response(document, "/v1/process/batch", "post", 200, batch_body, "Hub HTTP process.batch")
        batch_processes = batch_body.get("processes") if isinstance(batch_body, dict) else None
        if (
            batch_body.get("status") != "completed"
            or not isinstance(batch_processes, list)
            or len(batch_processes) != 2
            or [process.get("status") for process in batch_processes] != ["completed", "completed"]
            or not all(process.get("processId") for process in batch_processes)
            or PROCESS_MARKERS[3] not in json.dumps(batch_body)
            or PROCESS_MARKERS[4] not in json.dumps(batch_body)
        ):
            fail("Hub HTTP process.batch", f"ordered complete process output was not returned: {batch_body}")
        if len(json.dumps(batch_body, separators=(",", ":")).encode()) > 8192:
            fail("Hub HTTP process.batch", f"aggregate batch response exceeded 8192 bytes: {batch_body}")

        mcp_payload = {
            "agentId": normal_id,
            "serverId": "standalone-http",
            "toolName": "agent.info",
            "arguments": {},
            "waitSeconds": 5,
        }
        validate_instance(document, schemas, "McpCallToolRequest", mcp_payload, "Hub HTTP mcp.callTool request")
        invalid_mcp = dict(mcp_payload)
        invalid_mcp.pop("arguments")
        assert_rejected(document, schemas, "McpCallToolRequest", invalid_mcp, "Hub HTTP mcp.callTool negative request")
        mcp_response, mcp_body = confirmed_hub_json(
            hub_port, hub_key, "POST", "/v1/mcp/callTool", mcp_payload, confirmation, "Hub HTTP mcp.callTool"
        )
        if mcp_response.status != 200:
            fail("Hub HTTP mcp.callTool", f"HTTP {mcp_response.status}: {mcp_body}")
        validate_operation_response(document, "/v1/mcp/callTool", "post", 200, mcp_body, "Hub HTTP mcp.callTool")
        mcp_process_id = mcp_body.get("processId")
        if mcp_body.get("state") != "completed" or not mcp_process_id or mcp_body.get("resultAvailable") is not True:
            fail("Hub HTTP mcp.callTool", f"downstream MCP process was not completed/result-available: {mcp_body}")
        bounded_result = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 7, "tools/call",
                {"name": "process.result", "arguments": {"agentId": normal_id, "processId": mcp_process_id, "maxBytes": 8192}},
                "Hub Full bounded MCP result",
            ),
            "Hub Full bounded MCP result",
        )
        if (
            bounded_result.get("status") != "too_large"
            or bounded_result.get("resultAvailable") is not True
            or bounded_result.get("resultBytes", 0) <= 8192
            or "result" in bounded_result
        ):
            fail("Hub Full bounded MCP result", f"oversized result was not accurately reported: {bounded_result}")
        mcp_result = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 70, "tools/call",
                {"name": "process.result", "arguments": {"agentId": normal_id, "processId": mcp_process_id, "maxBytes": 524288}},
                "Hub Full explicit MCP result",
            ),
            "Hub Full explicit MCP result",
        )
        mcp_info = mcp_result.get("result", {}).get("structuredContent", {})
        mcp_identity = mcp_info.get("identity", {}) if isinstance(mcp_info, dict) else {}
        if (
            mcp_result.get("status") != "complete"
            or mcp_identity.get("agentId") != "parity-http"
            or mcp_identity.get("profile") != "normal"
            or mcp_identity.get("transport") != "tunnel-stdio"
        ):
            fail("Hub Full explicit MCP result", f"downstream identity was not returned by process.result: {mcp_result}")

        mcp_batch_payload = {
            "agentId": normal_id,
            "calls": [
                {"id": "info-1", "serverId": "standalone-http", "toolName": "agent.info", "arguments": {}},
                {"id": "skills-2", "serverId": "standalone-http", "toolName": "skills.list", "arguments": {}},
            ],
            "mode": "sequential",
            "failFast": True,
            "waitSeconds": 5,
        }
        validate_instance(document, schemas, "McpBatchRequest", mcp_batch_payload, "Hub HTTP mcp.batch request")
        invalid_mcp_batch = dict(mcp_batch_payload)
        invalid_mcp_batch.pop("calls")
        assert_rejected(document, schemas, "McpBatchRequest", invalid_mcp_batch, "Hub HTTP mcp.batch negative request")
        mcp_batch_response, mcp_batch_body = confirmed_hub_json(
            hub_port,
            hub_key,
            "POST",
            "/v1/mcp/batch",
            mcp_batch_payload,
            confirmation,
            "Hub HTTP mcp.batch",
        )
        if mcp_batch_response.status != 200:
            fail("Hub HTTP mcp.batch", f"HTTP {mcp_batch_response.status}: {mcp_batch_body}")
        validate_operation_response(document, "/v1/mcp/batch", "post", 200, mcp_batch_body, "Hub HTTP mcp.batch")
        batch_results = mcp_batch_body.get("results") if isinstance(mcp_batch_body, dict) else None
        if mcp_batch_body.get("status") != "completed" or not isinstance(batch_results, list) or len(batch_results) != 2:
            fail("Hub HTTP mcp.batch", f"batch did not complete with two ordered results: {mcp_batch_body}")
        for index, result in enumerate(batch_results):
            if result.get("state") != "completed" or not result.get("processId"):
                fail("Hub HTTP mcp.batch", f"result {index} did not complete: {mcp_batch_body}")
        first_result = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 71, "tools/call",
                {"name": "process.result", "arguments": {"agentId": normal_id, "processId": batch_results[0]["processId"], "maxBytes": 524288}},
                "Hub Full first batch MCP result",
            ),
            "Hub Full first batch MCP result",
        )
        second_result = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 72, "tools/call",
                {"name": "process.result", "arguments": {"agentId": normal_id, "processId": batch_results[1]["processId"], "maxBytes": 524288}},
                "Hub Full second batch MCP result",
            ),
            "Hub Full second batch MCP result",
        )
        first_content = first_result.get("result", {}).get("structuredContent", {})
        second_content = second_result.get("result", {}).get("structuredContent", {})
        identity = first_content.get("identity", {})
        if (
            batch_results[0]["processId"] == batch_results[1]["processId"]
            or first_result.get("status") != "complete"
            or second_result.get("status") != "complete"
            or identity.get("agentId") != "parity-http"
            or identity.get("profile") != "normal"
            or identity.get("transport") != "tunnel-stdio"
            or not any(skill.get("id") == "demo" for skill in second_content.get("skills", []))
        ):
            fail("Hub HTTP mcp.batch", f"distinct downstream info/skills results were not in request order: {mcp_batch_body}")
        reports.append("PASS Hub HTTP process.batch and real downstream mcp.callTool/mcp.batch with distinct ordered results")

        room_install_payload = inline_skill_install_request()
        validate_instance(document, schemas, "SkillInstallRequest", room_install_payload, "Hub Room skills.install request")
        room_install_response, room_install_body = hub_json(
            hub_port, hub_key, "POST", "/v1/room/skills/install", room_install_payload, "Hub Room skills.install"
        )
        if room_install_response.status != 200:
            fail("Hub Room skills.install", f"HTTP {room_install_response.status}: {room_install_body}")
        validate_operation_response(document, "/v1/room/skills/install", "post", 200, room_install_body, "Hub Room skills.install")
        room_install_id = room_install_body.get("installId")
        if not room_install_id:
            fail("Hub Room skills.install", f"missing real install id: {room_install_body}")
        room_get_payload = {"installId": room_install_id, "waitSeconds": 5}
        validate_instance(document, schemas, "SkillInstallGetRequest", room_get_payload, "Hub Room skills.install.get request")
        room_get_response, room_get_body = hub_json(
            hub_port, hub_key, "POST", "/v1/room/skills/install/get", room_get_payload, "Hub Room skills.install.get"
        )
        if room_get_response.status != 200:
            fail("Hub Room skills.install.get", f"HTTP {room_get_response.status}: {room_get_body}")
        validate_operation_response(document, "/v1/room/skills/install/get", "post", 200, room_get_body, "Hub Room skills.install.get")
        if room_get_body.get("status") != "completed" or not isinstance(room_get_body.get("result"), dict):
            fail("Hub Room skills.install.get", f"inline Room install did not complete: {room_get_body}")
        room_run_payload = {"id": "inline", "path": "scripts/check.sh", "waitSeconds": 5}
        validate_instance(document, schemas, "SkillRunRequest", room_run_payload, "Hub Room skills.run request")
        room_run_response, room_run_body = hub_json(
            hub_port, hub_key, "POST", "/v1/room/skills/run", room_run_payload, "Hub Room skills.run"
        )
        if room_run_response.status != 200:
            fail("Hub Room skills.run", f"HTTP {room_run_response.status}: {room_run_body}")
        validate_operation_response(document, "/v1/room/skills/run", "post", 200, room_run_body, "Hub Room skills.run")
        if (
            not room_run_body.get("processId")
            or room_run_body.get("state") != "completed"
            or "inline-skill" not in json.dumps(room_run_body)
        ):
            fail("Hub Room skills.run", f"active Room routing did not return completed inline process result: {room_run_body}")
        room_process_id = room_run_body["processId"]
        room_status_response, room_status = hub_json(
            hub_port, hub_key, "GET",
            f"/v1/process/{room_process_id}?" + urlencode({"agentId": room_id, "waitSeconds": "0"}),
            None, "Hub Room process.status",
        )
        if room_status_response.status != 200 or any(key in room_status for key in ("stdout", "stderr", "result")):
            fail("Hub Room process.status", f"status was not metadata-only: {room_status_response.status}, {room_status}")
        reports.append("PASS Hub Room HTTP inline skills install/get/run flat response and active-room routing")

        pagination_ids: list[str] = []
        for _ in range(101):
            value = process_request(
                hub_port, hub_key, normal_id, "/usr/bin/true", [], "parity-page", 0,
                "Hub process.list pagination setup",
            )
            validate_operation_response(document, "/v1/process/exec", "post", 200, value, "Hub process.list pagination setup")
            pagination_id = value.get("processId")
            if not pagination_id:
                fail("Hub process.list pagination setup", f"process omitted id: {value}")
            pagination_ids.append(pagination_id)

        bad_group_response, bad_group_body = hub_json(
            hub_port, hub_key, "GET",
            "/v1/process?" + urlencode({"agentId": normal_id, "group": ""}),
            None, "Hub process.list typed bad group",
        )
        if bad_group_response.status != 400:
            fail("Hub process.list typed bad group", f"expected typed 400 group error: {bad_group_response.status}, {bad_group_body}")
        validate_operation_response(document, "/v1/process", "get", 400, bad_group_body, "Hub process.list typed bad group")

        default_response, default_body = hub_json(
            hub_port, hub_key, "GET",
            "/v1/process?" + urlencode({"agentId": normal_id, "group": "parity-page"}),
            None, "Hub process.list default50",
        )
        if default_response.status != 200:
            fail("Hub process.list default50", f"HTTP {default_response.status}: {default_body}")
        validate_operation_response(document, "/v1/process", "get", 200, default_body, "Hub process.list default50")
        if len(default_body.get("processes", [])) != 50:
            fail("Hub process.list default50", f"expected exactly 50 processes, got {len(default_body.get('processes', []))}")
        cursor = default_body.get("nextCursor")
        if not cursor:
            fail("Hub process.list", "default page omitted nextCursor with 101 real processes")
        known_ids = set(pagination_ids)
        page1 = {item["processId"] for item in default_body["processes"]}
        if len(page1) != 50 or not page1.issubset(known_ids):
            fail("Hub process.list default50", f"default page was not 50 distinct known processes: {page1}")

        cursor_response, cursor_body = hub_json(
            hub_port, hub_key, "GET",
            "/v1/process?" + urlencode({"agentId": normal_id, "group": "parity-page", "cursor": cursor}),
            None, "Hub process.list cursor continuation",
        )
        if cursor_response.status != 200:
            fail("Hub process.list cursor continuation", f"HTTP {cursor_response.status}: {cursor_body}")
        validate_operation_response(document, "/v1/process", "get", 200, cursor_body, "Hub process.list cursor continuation")
        page2 = {item["processId"] for item in cursor_body.get("processes", [])}
        if len(page2) != 50 or not page2.issubset(known_ids) or page1.intersection(page2):
            fail("Hub process.list cursor continuation", f"cursor page was not 50 distinct known processes: {page2}")

        cap_response, cap_body = hub_json(
            hub_port, hub_key, "GET",
            "/v1/process?" + urlencode({"agentId": normal_id, "group": "parity-page", "limit": 101}),
            None, "Hub process.list maximum100",
        )
        if cap_response.status != 200:
            fail("Hub process.list maximum100", f"HTTP {cap_response.status}: {cap_body}")
        validate_operation_response(document, "/v1/process", "get", 200, cap_body, "Hub process.list maximum100")
        if len(cap_body.get("processes", [])) != 100:
            fail("Hub process.list maximum100", f"expected exactly 100 processes, got {len(cap_body.get('processes', []))}")

        minimum_response, minimum_body = hub_json(
            hub_port, hub_key, "GET",
            "/v1/process?" + urlencode({"agentId": normal_id, "group": "parity-page", "limit": 0}),
            None, "Hub process.list minimum1",
        )
        if minimum_response.status != 200:
            fail("Hub process.list minimum1", f"HTTP {minimum_response.status}: {minimum_body}")
        validate_operation_response(document, "/v1/process", "get", 200, minimum_body, "Hub process.list minimum1")
        if len(minimum_body.get("processes", [])) != 1:
            fail("Hub process.list minimum1", f"expected one process after lower clamp, got {len(minimum_body.get('processes', []))}")
        reports.append("PASS Hub HTTP process.status/cancel and process.list typed group error, 50/100/1 pages, and cursor")
        for retired_path in sorted(ROOM_RETIRED_PATHS):
            retired_response = http_request(
                hub_port,
                "POST",
                retired_path,
                {},
                {"Authorization": f"Bearer {hub_key}"},
                f"Retired Room HTTP path {retired_path}",
            )
            retired_text = retired_response.body.decode("utf-8", errors="replace")
            if retired_response.status != 404 or "room_legacy_surface_removed" in retired_text:
                fail(
                    f"Retired Room HTTP path {retired_path}",
                    f"legacy route remained reachable instead of being absent: HTTP {retired_response.status}, {retired_text!r}",
                )
        reports.append("PASS retired Room HTTP routes are absent while retained recent/search routes use current contracts")


        room_root = room_repository_root(room_config)
        if room_root == hub_root:
            fail("Room repository ownership", "Hub and Agent Room roots unexpectedly coincide")
        _, status_body = room_http_call(
            hub_port,
            hub_key,
            document,
            schemas,
            "room.maintenance.status",
            {},
            "Room HTTP maintenance.status before fixture",
        )
        if (
            status_body.get("repository", {}).get("initialized") is not True
            or status_body.get("schema", {}).get("ready") is not True
            or status_body.get("scaffold", {}).get("ready") is not True
        ):
            fail("Room HTTP maintenance.status before fixture", f"real scaffold status was not ready: {status_body}")

        _, fixture_body = room_http_call(
            hub_port,
            hub_key,
            document,
            schemas,
            "room.maintenance.submit",
            json.loads(json.dumps(ROOM_FIXTURE_PAYLOAD)),
            "Room HTTP maintenance.submit local fixture",
        )
        if (
            fixture_body.get("mode") != "local"
            or fixture_body.get("state") != "applied"
            or fixture_body.get("localApplied") is not True
            or fixture_body.get("sync") != "not_requested"
            or not fixture_body.get("revision")
        ):
            fail("Room HTTP maintenance.submit local fixture", f"real local apply lacked revision/sync evidence: {fixture_body}")
        fixture_head = git_stdout(room_root, room_env, ["rev-parse", "HEAD"], "Room Agent repository revision")
        if fixture_body.get("revision") != fixture_head:
            fail("Room Agent repository revision", f"submit revision did not match Agent HEAD: {fixture_body}, {fixture_head}")
        expected_room_files = {
            "Diary/Daily/current.md": "Remote parity daily",
            "Diary/Weekly/current.md": "Remote parity weekly",
            "Diary/Monthly/current.md": "Remote parity monthly",
            "Notebook/contract.md": "# Contract parity\n\nHub remote Room\n",
            "State/entities/parity.md": "Room state parity\n",
        }
        for relative, expected in expected_room_files.items():
            try:
                content = (room_root / relative).read_text()
            except OSError as error:
                fail("Room Agent repository content", f"missing {relative}: {error}")
            if expected not in content:
                fail("Room Agent repository content", f"{relative} did not contain expected fixture content: {content!r}")
            if (hub_root / relative).exists():
                fail("Hub Room ownership", f"Hub unexpectedly created Room content at {relative}")
        if any((hub_root / name).exists() for name in ("Diary", "Notebook", "State", "maintenance")):
            fail("Hub Room ownership", "Hub root contains a Room content or maintenance tree")
        reports.append("PASS Hub HTTP maintenance.submit performed real local Room apply with Agent HEAD/content ownership")

        http_room_values: dict[str, dict[str, Any]] = {}
        for spec in ROOM_OPERATIONS:
            payload = json.loads(json.dumps(spec["payload"]))
            _, value = room_http_call(
                hub_port,
                hub_key,
                document,
                schemas,
                spec["name"],
                payload,
                f"Room HTTP {spec['name']}",
            )
            http_room_values[spec["name"]] = value
        daily = http_room_values["room.diary.active"].get("daily", {})
        diary_read = http_room_values["room.diary.read"].get("document", {})
        notebook_recent = http_room_values["room.notebook.recent"].get("documents", [])
        notebook_search = http_room_values["room.notebook.search"].get("documents", [])
        malformed_path = "/v1/room/notebook/read"
        malformed_payload: dict[str, Any] = {}
        assert_rejected(
            document,
            schemas,
            "RoomNotebookReadRequest",
            malformed_payload,
            "Room HTTP notebook.read malformed request",
        )
        malformed_response = http_request(
            hub_port,
            "POST",
            malformed_path,
            malformed_payload,
            {"Authorization": f"Bearer {hub_key}"},
            "Room HTTP notebook.read malformed request",
        )
        malformed_text = malformed_response.body.decode("utf-8", errors="replace")
        malformed_media_type = malformed_response.headers.get("content-type", "").split(";", 1)[0].strip()
        if malformed_response.status != 422 or malformed_media_type != "text/plain":
            fail(
                "Room HTTP notebook.read malformed request",
                f"expected HTTP 422 text/plain: {malformed_response.status}, {malformed_media_type}, {malformed_text!r}",
            )
        validate_operation_response(
            document,
            malformed_path,
            "post",
            422,
            malformed_text,
            "Room HTTP notebook.read malformed request",
            media_type=malformed_media_type,
        )
        reports.append("PASS current Room HTTP malformed request retained declared 422 text/plain extraction proof")
        selector_path = "/v1/room/diary/active"
        selector_payload = {"agentId": "foreign"}
        assert_rejected(
            document,
            schemas,
            "RoomDiaryActiveRequest",
            selector_payload,
            "Room HTTP diary.active foreign selector",
        )
        selector_response = http_request(
            hub_port,
            "POST",
            selector_path,
            selector_payload,
            {"Authorization": f"Bearer {hub_key}"},
            "Room HTTP diary.active foreign selector",
        )
        selector_text = selector_response.body.decode("utf-8", errors="replace")
        selector_media_type = selector_response.headers.get("content-type", "").split(";", 1)[0].strip()
        if selector_response.status != 422 or selector_media_type != "text/plain":
            fail(
                "Room HTTP diary.active foreign selector",
                f"expected HTTP 422 text/plain instead of selector dispatch: {selector_response.status}, {selector_media_type}, {selector_text!r}",
            )
        validate_operation_response(
            document,
            selector_path,
            "post",
            422,
            selector_text,
            "Room HTTP diary.active foreign selector",
            media_type=selector_media_type,
        )
        reports.append("PASS current Room HTTP rejected foreign agentId selector before active-Room dispatch")

        notebook_read = http_room_values["room.notebook.read"]
        state_entities = http_room_values["room.state.list"].get("entities", [])
        state_read = http_room_values["room.state.read"]
        maintenance_status = http_room_values["room.maintenance.status"]
        if (
            daily.get("available") is not True
            or "Remote parity daily" not in daily.get("content", "")
            or "Remote parity daily" not in diary_read.get("content", "")
            or notebook_read.get("content") != expected_room_files["Notebook/contract.md"]
            or not any(item.get("path") == "Notebook/contract.md" for item in notebook_recent)
            or not any(item.get("path") == "Notebook/contract.md" for item in notebook_search)
            or not any(item.get("entity") == "parity" for item in state_entities)
            or state_read.get("content") != expected_room_files["State/entities/parity.md"]
            or maintenance_status.get("repository", {}).get("head") != fixture_head
        ):
            fail(
                "Room HTTP current operations",
                f"Agent-owned read content/revision was not returned: {http_room_values}",
            )
        reports.append("PASS Hub HTTP all nine current Room operations returned typed Agent-owned reads/status")

        invalid_read_payload = {"path": "Notebook/../outside.md"}
        _, invalid_read = room_http_call(
            hub_port,
            hub_key,
            document,
            schemas,
            "room.notebook.read",
            invalid_read_payload,
            "Room HTTP notebook.read invalid path",
            expected_status=400,
        )
        if invalid_read.get("error", {}).get("code") != "room_repository_path_invalid":
            fail("Room HTTP notebook.read invalid path", f"Agent path rejection was not preserved: {invalid_read}")

        dirty_path = room_root / "unrelated-dirty.md"
        dirty_path.write_text("dirty parity probe\n")
        try:
            dirty_payload = {
                "items": [
                    {
                        "slot": "notebook",
                        "payload": {
                            "path": "Notebook/dirty.md",
                            "title": "Dirty",
                            "body": "must reject",
                        },
                    }
                ],
                "mode": "local",
                "waitSeconds": 0,
            }
            _, dirty_body = room_http_call(
                hub_port,
                hub_key,
                document,
                schemas,
                "room.maintenance.submit",
                dirty_payload,
                "Room HTTP maintenance.submit dirty rejection",
                expected_status=400,
            )
        finally:
            dirty_path.unlink(missing_ok=True)
        if (
            dirty_body.get("error", {}).get("code") != "room_maintenance_failed"
            or "room_maintenance_repository_dirty" not in dirty_body.get("error", {}).get("message", "")
        ):
            fail("Room HTTP maintenance.submit dirty rejection", f"Agent dirty-repository rejection was not preserved: {dirty_body}")
        reports.append("PASS Agent-owned Room invalid-path and dirty-repository rejection stayed behind Hub HTTP projection")

        mcp_room_values: dict[str, dict[str, Any]] = {}
        for request_id, spec in enumerate(ROOM_OPERATIONS, start=50):
            payload = json.loads(json.dumps(spec["payload"]))
            mcp_room_values[spec["name"]] = room_mcp_call(
                hub_port,
                hub_key,
                full_session,
                request_id,
                document,
                schemas,
                spec["name"],
                payload,
                f"Hub Full MCP {spec['name']}",
            )
        if (
            "Remote parity daily" not in mcp_room_values["room.diary.read"].get("document", {}).get("content", "")
            or mcp_room_values["room.notebook.read"].get("content") != expected_room_files["Notebook/contract.md"]
            or not any(item.get("entity") == "parity" for item in mcp_room_values["room.state.list"].get("entities", []))
            or mcp_room_values["room.maintenance.submit"].get("state") != "applied"
            or mcp_room_values["room.maintenance.submit"].get("localApplied") is not True
            or not mcp_room_values["room.maintenance.submit"].get("revision")
        ):
            fail("Hub Full MCP current Room operations", f"MCP did not return real current Room values: {mcp_room_values}")
        if not (room_root / "Notebook/mcp.md").is_file() or (hub_root / "Notebook/mcp.md").exists():
            fail("Hub Full MCP maintenance ownership", "MCP submit did not mutate only the Agent-owned repository")
        reports.append("PASS Hub Full MCP dispatched all nine current Room operations and real local maintenance")

        room_process.stop()
        room_process = None
        wait_for_agent_offline(hub_port, hub_key, room_id, "Hub Room disconnect before lifecycle checks")
        inactive_response, inactive_body = room_http_call(
            hub_port,
            hub_key,
            document,
            schemas,
            "room.notebook.read",
            {"path": "Notebook/contract.md"},
            "Room HTTP no active Room",
            expected_status=404,
        )
        if inactive_response.status != 404 or inactive_body.get("error", {}).get("code") != "room_not_active":
            fail("Room HTTP no active Room", f"404 did not identify inactive Room (not a missing route): {inactive_body}")
        reports.append("PASS no active Room returned typed room_not_active without falling back to connected Normal Agent")

        reporting_id, reporting_secret = "parity-reporting-room", "parity-reporting-room-secret"
        register_hub_agent(
            hub_binary,
            hub_root / "hub.db",
            hub_config,
            hub_env,
            reporting_id,
            "Parity ReportingOnly Room",
            reporting_secret,
        )
        reporting_process, reporting_config, reporting_env = start_reporting_agent(
            agent_binary,
            root / "reporting-room",
            reporting_id,
            f"http://127.0.0.1:{hub_port}",
            reporting_secret,
            reports,
        )
        wait_for_agent(hub_port, hub_key, reporting_id, "Hub ReportingOnly Room connection", reporting_process)
        reporting_response, reporting_body = room_http_call(
            hub_port,
            hub_key,
            document,
            schemas,
            "room.notebook.read",
            {"path": "Notebook/contract.md"},
            "Room HTTP ReportingOnly no fallback",
            expected_status=404,
        )
        if reporting_response.status != 404 or reporting_body.get("error", {}).get("code") != "room_not_active":
            fail(
                "Room HTTP ReportingOnly no fallback",
                f"ReportingOnly Room became a route target instead of room_not_active: {reporting_body}",
            )
        reports.append("PASS online ReportingOnly Room did not activate or become an HTTP fallback target")
        reporting_process.stop()
        reporting_process = None
        wait_for_agent_offline(hub_port, hub_key, reporting_id, "Hub ReportingOnly Room disconnect")

        room_process = ManagedProcess(
            [str(agent_binary), "run", "--config", str(room_config)],
            room_env,
            "Hub Agent room reconnect",
        )
        reports.append(f"START Hub Agent room reconnect ({room_id})")
        wait_for_agent(hub_port, hub_key, room_id, "Hub Room reconnect", room_process)
        _, reconnected_read = room_http_call(
            hub_port,
            hub_key,
            document,
            schemas,
            "room.notebook.read",
            {"path": "Notebook/contract.md"},
            "Room HTTP Room reconnect retained content",
        )
        if reconnected_read.get("content") != expected_room_files["Notebook/contract.md"]:
            fail("Room HTTP Room reconnect retained content", f"reconnected Room read changed repository content: {reconnected_read}")
        reports.append("PASS same real Room identity reconnected with a new lease and retained Agent repository content")

        room_process.stop()
        room_process = None
        wait_for_agent_offline(hub_port, hub_key, room_id, "Hub Room disconnect before workflow")
        origin = root / "room-origin.git"
        git_env = dict(room_env, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL="/dev/null")
        run_checked(["git", "init", "--bare", "-b", "main", str(origin)], git_env, "Room workflow bare origin init")
        run_checked(["git", "-C", str(room_root), "remote", "add", "origin", str(origin)], git_env, "Room workflow origin add")
        run_checked(["git", "-C", str(room_root), "push", "-u", "origin", "main"], git_env, "Room workflow origin seed")
        room_config_data = json.loads(room_config.read_text())
        room_config_data.setdefault("room", {}).setdefault("maintenance", {})["mode"] = "workflow"
        room_config.write_text(json.dumps(room_config_data, indent=2) + "\n")
        room_process = ManagedProcess(
            [str(agent_binary), "run", "--config", str(room_config)],
            room_env,
            "Hub Agent room workflow reconnect",
        )
        reports.append(f"START Hub Agent room workflow reconnect ({room_id})")
        wait_for_agent(hub_port, hub_key, room_id, "Hub Room workflow reconnect", room_process)
        workflow_payload = {
            "items": [
                {
                    "slot": "notebook",
                    "payload": {
                        "path": "Notebook/workflow.md",
                        "title": "Workflow parity",
                        "body": "bounded wait",
                    },
                }
            ],
            "mode": "workflow",
            "waitSeconds": 1,
        }
        _, workflow_body = room_http_call(
            hub_port,
            hub_key,
            document,
            schemas,
            "room.maintenance.submit",
            workflow_payload,
            "Room HTTP maintenance.submit workflow bounded wait",
        )
        workflow_request = room_root / "maintenance/notebook/maintenance.json"
        origin_request = run_checked(
            [
                "git",
                "--git-dir",
                str(origin),
                "ls-tree",
                "-r",
                "--name-only",
                "main",
                "--",
                "maintenance/notebook/maintenance.json",
            ],
            git_env,
            "Room workflow origin request",
        ).stdout.strip()
        if origin_request != "maintenance/notebook/maintenance.json":
            fail("Room HTTP maintenance.submit workflow bounded wait", f"bare origin did not retain the submitted request: {origin_request!r}")
        if (
            workflow_body.get("mode") != "workflow"
            or workflow_body.get("state") != "submitted"
            or workflow_body.get("localApplied") is not False
            or workflow_body.get("sync") != "pending"
            or workflow_body.get("revision") is not None
            or not workflow_request.is_file()
            or (room_root / "Notebook/workflow.md").exists()
        ):
            fail("Room HTTP maintenance.submit workflow bounded wait", f"workflow wait was not submitted/not-cancelled: {workflow_body}")
        reports.append("PASS Room workflow submit returned submitted/pending after bounded wait and left request for worker (not cancelled)")
        room_process.stop()
        room_process = None
        wait_for_agent_offline(hub_port, hub_key, room_id, "Hub Room disconnect after workflow")


        normal_process.stop()
        normal_process = None
        wait_until(
            lambda: not any(
                item.get("agentId") == normal_id and item.get("online")
                for item in hub_json(hub_port, hub_key, "GET", "/v1/agents", None, "Hub normal disconnect")[1].get("agents", [])
            ),
            "Hub normal disconnect",
            "normal Agent disconnect",
        )
        cached_response, cached_body = hub_json(
            hub_port, hub_key, "GET",
            f"/v1/process/{completed_id}?" + urlencode({"agentId": normal_id}),
            None, "Hub cached process.status",
        )
        if cached_response.status != 200:
            fail("Hub cached process.status", f"HTTP {cached_response.status}: {cached_body}")
        validate_operation_response(document, "/v1/process/{processId}", "get", 200, cached_body, "Hub cached process.status")
        if (
            cached_body.get("processId") != completed_id
            or cached_body.get("freshness") not in {"cached", "stale"}
            or any(key in cached_body for key in ("stdout", "stderr", "result"))
        ):
            fail("Hub cached process.status", f"cached status exposed output or lost process identity: {cached_body}")
        cached_mcp_status = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 8, "tools/call",
                {"name": "hub.process.status", "arguments": {"agentId": normal_id, "processId": completed_id}},
                "Hub Full cached process status",
            ),
            "Hub Full cached process status",
        )
        if (
            cached_mcp_status.get("freshness") not in {"cached", "stale"}
            or completed_id not in json.dumps(cached_mcp_status)
            or any(key in cached_mcp_status for key in ("stdout", "stderr", "result"))
        ):
            fail("Hub Full cached process status", f"cache-only MCP status exposed body or lost process identity: {cached_mcp_status}")
        cached_mcp_list = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 9, "tools/call",
                {"name": "hub.process.list", "arguments": {"agentId": normal_id, "group": "parity-page", "limit": 1}},
                "Hub Full cached process list",
            ),
            "Hub Full cached process list",
        )
        if completed_id not in json.dumps(cached_mcp_list) or any(
            key in json.dumps(cached_mcp_list) for key in ('"stdout"', '"stderr"', '"result"')
        ):
            fail("Hub Full cached process list", f"cache-only MCP list did not return status metadata: {cached_mcp_list}")

        output_offline_response, output_offline_body = hub_json(
            hub_port, hub_key, "GET",
            f"/v1/process/{completed_id}/output?" + urlencode({"agentId": normal_id}),
            None, "Hub offline process.output",
        )
        if (
            output_offline_response.status != 503
            or output_offline_body.get("error", {}).get("code") != "process_output_unavailable"
            or output_offline_body.get("cached", {}).get("processId") != completed_id
            or output_offline_body.get("freshness") not in {"cached", "stale"}
            or output_offline_body.get("result") is not None
            or any(key in output_offline_body for key in ("stdout", "stderr"))
        ):
            fail("Hub offline process.output", f"expected typed body-free unavailable response: {output_offline_response.status}, {output_offline_body}")
        validate_operation_response(document, "/v1/process/{processId}/output", "get", 503, output_offline_body, "Hub offline process.output")

        result_offline_response, result_offline_body = hub_json(
            hub_port, hub_key, "GET",
            f"/v1/process/{completed_id}/result?" + urlencode({"agentId": normal_id}),
            None, "Hub offline process.result",
        )
        if (
            result_offline_response.status != 503
            or result_offline_body.get("error", {}).get("code") != "process_result_unavailable"
            or result_offline_body.get("cached", {}).get("processId") != completed_id
            or result_offline_body.get("freshness") not in {"cached", "stale"}
            or result_offline_body.get("result") is not None
            or any(key in result_offline_body for key in ("stdout", "stderr"))
        ):
            fail("Hub offline process.result", f"expected typed body-free unavailable response: {result_offline_response.status}, {result_offline_body}")
        validate_operation_response(document, "/v1/process/{processId}/result", "get", 503, result_offline_body, "Hub offline process.result")
        offline_mcp_result = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 10, "tools/call",
                {"name": "process.result", "arguments": {"agentId": normal_id, "processId": completed_id, "maxBytes": 8192}},
                "Hub Full offline process.result",
            ),
            "Hub Full offline process.result",
        )
        if offline_mcp_result.get("status") != "unavailable" or offline_mcp_result.get("result") is not None:
            fail("Hub Full offline process.result", f"MCP did not report unavailable result without body: {offline_mcp_result}")

        offline_cancel_response, offline_cancel_body = hub_json(
            hub_port, hub_key, "POST",
            f"/v1/process/{completed_id}/cancel?" + urlencode({"agentId": normal_id}),
            {}, "Hub offline process.cancel",
        )
        if (
            offline_cancel_response.status != 502
            or offline_cancel_body.get("error", {}).get("code") != "process_cancel_unavailable"
            or "cached" in offline_cancel_body
            or "observedAt" in offline_cancel_body
        ):
            fail("Hub offline process.cancel", f"expected typed unavailable response without cached success: {offline_cancel_response.status}, {offline_cancel_body}")
        validate_operation_response(document, "/v1/process/{processId}/cancel", "post", 502, offline_cancel_body, "Hub offline process.cancel")
        reports.append("PASS Hub cache-only process.status and offline output/result/cancel typed unavailable responses")
    finally:
        for process in (
            coordinator_process,
            normal_process,
            reporting_process,
            room_process,
            hub_process,
            http_process,
            local_process,
        ):
            if process is not None:
                process.stop()
        if confirmation is not None:
            confirmation.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--agent-bin", type=Path, default=Path("target/debug/agentic-gpt"))
    parser.add_argument("--hub-bin", type=Path, default=Path("target/debug/agentic-gpt-hub"))
    args = parser.parse_args()
    print("contract parity gate: START", flush=True)
    reports: list[str] = []
    root_path = Path(tempfile.mkdtemp(prefix="cg-", dir="/tmp"))
    try:
        if not args.agent_bin.exists():
            fail("startup", f"Agent binary does not exist: {args.agent_bin}")
        if not args.hub_bin.exists():
            fail("startup", f"Hub binary does not exist: {args.hub_bin}")
        document, schemas = load_openapi(Path("openapi/hub.yaml"))
        require_schema_contract(document, schemas)
        reports.append("PASS OpenAPI YAML loaded, local refs resolved, Draft202012 schemas checked with format validation")
        run_runtime_gate(root_path, args.agent_bin.resolve(), args.hub_bin.resolve(), document, schemas, reports)
        print("contract parity gate: PASS")
        for report in reports:
            print(report)
        print("LIMITATION standalone HTTP MCP and local standalone ReportingOnly WebSocket fixtures are exercised; production tunnel executable transport is intentionally not claimed (Hub generation tests cover replacement races)")
        return 0
    except GateError as error:
        print(f"contract parity gate: FAIL {error}", file=sys.stderr)
        for report in reports:
            print(report, file=sys.stderr)
        return 1
    finally:
        for process in ACTIVE_PROCESSES:
            process.stop()
        import shutil
        shutil.rmtree(root_path, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
