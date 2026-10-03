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
import base64
import random
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import http.client
import json
import fcntl
import sqlite3
import os
from pathlib import Path
import shlex
import queue
import select
import signal
import socket
import struct
import zlib
import subprocess
import sys
import tempfile
import threading
import socketserver
import time
from typing import Any, Iterable
from urllib.parse import parse_qs, quote, unquote, urlencode, urlsplit

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


def run_checked(
    args: list[str],
    env: dict[str, str],
    scenario: str,
    input_text: str | None = None,
) -> subprocess.CompletedProcess[str]:
    try:
        result = subprocess.run(
            args,
            input=input_text,
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
    env.pop("TMUX", None)
    env["TMUX_TMPDIR"] = str(runtime_dir)
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


class WebSocketRelay:
    """Raw relay that delays one real Agent response and can hold its receipt ACK."""

    MAX_FRAME_BYTES = 16 * 1024 * 1024

    def __init__(self, upstream_port: int) -> None:
        self.upstream_port = upstream_port
        self.response_group: str | None = None
        self.response_command_type = "process.exec"
        self.response_armed = False
        self.response_release = threading.Event()
        self.response_seen = threading.Event()
        self.late_response: dict[str, Any] | None = None
        self.settle_count = 0
        self.settle_seen = threading.Event()
        self.settle_response_messages: list[dict[str, Any]] = []
        self.receipt_hold_armed = False
        self.receipt_release = threading.Event()
        self.receipt_seen = threading.Event()
        self.settle_reply_hold_armed = False
        self.settle_reply_seen = threading.Event()
        self.held_receipt_identities: list[tuple[str, str, str, str]] = []
        self.forwarded_receipts = 0
        self.forwarded_receipt_identities: list[tuple[str, str, str, str]] = []
        self.commands: list[dict[str, Any]] = []
        self.source_reports: list[dict[str, Any]] = []
        self.source_report_seen = threading.Event()
        self.fail_next_settle_response = False
        self.settle_failure_seen = threading.Event()
        self._failed_settle_response_baseline: int | None = None
        self._settle_response_ids: set[tuple[str, str]] = set()
        self._expected_response: tuple[str, str] | None = None
        self._expected_receipt: tuple[str, str, str, str] | None = None
        self._condition = threading.Condition()
        self._next_connection_id = 0
        self._active_connections: set[int] = set()
        self._failed_settle_connection_id: int | None = None
        self.server = self._make_server()
        self.port = self.server.server_address[1]
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def _make_server(self) -> socketserver.ThreadingTCPServer:
        relay = self

        class Handler(socketserver.BaseRequestHandler):
            def handle(self) -> None:
                connection_id: int | None = None
                client = self.request
                upstream: socket.socket | None = None
                try:
                    handshake = relay._read_http(client)
                    if not handshake:
                        return
                    upstream = socket.create_connection(("127.0.0.1", relay.upstream_port), timeout=8)
                    upstream.sendall(handshake)
                    response = relay._read_http(upstream)
                    client.sendall(response)
                    if b" 101 " not in response:
                        return
                    client.settimeout(None)
                    upstream.settimeout(None)
                    connection_id = relay._connection_started()
                    relay._proxy(client, upstream, connection_id)
                except (OSError, ValueError, GateError):
                    pass
                finally:
                    with contextlib.suppress(OSError):
                        client.close()
                    if upstream is not None:
                        with contextlib.suppress(OSError):
                            upstream.close()
                    if connection_id is not None:
                        relay._connection_stopped(connection_id)

        server = socketserver.ThreadingTCPServer(("127.0.0.1", 0), Handler)
        server.daemon_threads = True
        server.allow_reuse_address = True
        return server

    @staticmethod
    def _read_http(sock: socket.socket) -> bytes:
        data = bytearray()
        while b"\r\n\r\n" not in data:
            part = sock.recv(1)
            if not part:
                return b""
            data.extend(part)
            if len(data) > 64 * 1024:
                raise GateError("WebSocket relay handshake exceeded limit")
        return bytes(data)

    @classmethod
    def _recv_exact(cls, sock: socket.socket, size: int) -> bytes:
        data = bytearray()
        while len(data) < size:
            part = sock.recv(size - len(data))
            if not part:
                raise EOFError
            data.extend(part)
        return bytes(data)

    @classmethod
    def _frame(cls, sock: socket.socket) -> tuple[bytes, int, bytes] | None:
        try:
            header = cls._recv_exact(sock, 2)
        except EOFError:
            return None
        first, second = header
        marker = second & 0x7F
        extension = b""
        length = marker
        if marker == 126:
            extension = cls._recv_exact(sock, 2)
            length = int.from_bytes(extension, "big")
        elif marker == 127:
            extension = cls._recv_exact(sock, 8)
            length = int.from_bytes(extension, "big")
        if length > cls.MAX_FRAME_BYTES:
            raise GateError(f"WebSocket relay frame exceeded {cls.MAX_FRAME_BYTES} bytes")
        mask = cls._recv_exact(sock, 4) if second & 0x80 else b""
        wire_payload = cls._recv_exact(sock, length)
        payload = wire_payload
        if mask:
            payload = bytes(value ^ mask[index % 4] for index, value in enumerate(wire_payload))
        return header + extension + mask + wire_payload, first & 0x0F, payload

    @staticmethod
    def _json_message(opcode: int, payload: bytes) -> dict[str, Any]:
        if opcode != 1:
            return {}
        try:
            value = json.loads(payload.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            return {}
        return value if isinstance(value, dict) else {}

    def arm_late_response(self, group: str, command_type: str = "process.exec") -> None:
        with self._condition:
            self.response_group = group
            self.response_command_type = command_type
            self.response_armed = True
            self.response_release.clear()
            self.response_seen.clear()
            self.late_response = None

    def release_late_response(self) -> None:
        self.response_release.set()

    def arm_receipt_hold(self) -> None:
        with self._condition:
            self._arm_receipt_hold_locked()

    def _arm_receipt_hold_locked(self) -> None:
        self.receipt_hold_armed = True
        self.receipt_release.clear()
        self.receipt_seen.clear()
        self.settle_reply_hold_armed = True
        self.settle_reply_seen.clear()

    def disarm_receipt_hold(self) -> None:
        with self._condition:
            self.receipt_hold_armed = False
            self.receipt_release.set()
            self.settle_reply_hold_armed = False
            self._condition.notify_all()

    def _connection_started(self) -> int:
        with self._condition:
            self._next_connection_id += 1
            connection_id = self._next_connection_id
            self._active_connections.add(connection_id)
            self._condition.notify_all()
            return connection_id

    def _connection_stopped(self, connection_id: int) -> None:
        with self._condition:
            self._active_connections.discard(connection_id)
            self._condition.notify_all()

    def wait_disconnected(self, timeout: float, scenario: str) -> None:
        deadline = time.monotonic() + timeout
        with self._condition:
            while self._active_connections:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    fail(scenario, "Agent WebSocket relay connection did not close")
                self._condition.wait(remaining)

    def wait_failed_settle_disconnected(self, timeout: float, scenario: str) -> None:
        deadline = time.monotonic() + timeout
        with self._condition:
            while True:
                connection_id = self._failed_settle_connection_id
                if connection_id is not None and connection_id not in self._active_connections:
                    return
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    fail(
                        scenario,
                        "Agent WebSocket relay connection carrying failed EventSettle response did not close",
                    )
                self._condition.wait(remaining)

    def wait_for_settle(self, count: int, timeout: float, scenario: str) -> None:
        deadline = time.monotonic() + timeout
        with self._condition:
            while self.settle_count < count:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    fail(scenario, f"observed {self.settle_count} EventSettle commands; expected at least {count}")
                self._condition.wait(remaining)
    def wait_settle_reply_held(self, timeout: float, scenario: str) -> None:
        if not self.settle_reply_seen.wait(timeout):
            fail(scenario, "relay did not hold the actual settled Agent Response for EventSettle")

    def wait_for_source_report(
        self, run_id: str, request_id: str, command_hash: str, timeout: float, scenario: str
    ) -> dict[str, Any]:
        deadline = time.monotonic() + timeout
        with self._condition:
            while True:
                for report in self.source_reports:
                    origin = report.get("origin", {})
                    if (
                        origin.get("runId") == run_id
                        and origin.get("requestId") == request_id
                        and origin.get("commandHash") == command_hash
                    ):
                        return report
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    fail(scenario, "Agent did not report the persisted bound EventSources on reconnect")
                self._condition.wait(remaining)

    def arm_settle_response_failure(self) -> None:
        with self._condition:
            self.fail_next_settle_response = True
            self._failed_settle_response_baseline = None
            self.settle_failure_seen.clear()

    def wait_settle_failure(self, timeout: float, scenario: str) -> int:
        if not self.settle_failure_seen.wait(timeout):
            fail(scenario, "relay did not drop an actual successful EventSettle response")
        with self._condition:
            baseline = self._failed_settle_response_baseline
        if baseline is None:
            fail(scenario, "relay dropped the EventSettle response without capturing its replay boundary")
        return baseline

    def command_types(self) -> list[str]:
        with self._condition:
            return [
                str(message.get("command", {}).get("type", ""))
                for message in self.commands
            ]

    def settle_payloads(self) -> list[dict[str, Any]]:
        with self._condition:
            payloads = []
            for message in self.commands:
                command = message.get("command")
                if (
                    isinstance(command, dict)
                    and command.get("type") == "event.settle"
                    and isinstance(command.get("payload"), dict)
                ):
                    payloads.append(command["payload"])
            return payloads

    def settle_envelopes(self) -> list[dict[str, Any]]:
        with self._condition:
            return [
                message
                for message in self.commands
                if message.get("command", {}).get("type") == "event.settle"
            ]


    def wait_for_process_command(
        self, group: str, timeout: float, scenario: str, command_type: str = "process.exec"
    ) -> dict[str, Any]:
        deadline = time.monotonic() + timeout
        with self._condition:
            while True:
                for envelope in self.commands:
                    command = envelope.get("command", {})
                    payload = command.get("payload", {}) if isinstance(command, dict) else {}
                    if command.get("type") == command_type and payload.get("group") == group:
                        return envelope
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    fail(scenario, f"relay did not observe the real {command_type} group {group!r}")
                self._condition.wait(remaining)


    def wait_receipt_held(self, timeout: float, scenario: str) -> None:
        if not self.receipt_seen.wait(timeout):
            fail(scenario, "relay did not hold the real EventSettle TransportAck")

    def wait_forwarded_receipt(self, previous_count: int, timeout: float, scenario: str) -> None:
        deadline = time.monotonic() + timeout
        with self._condition:
            while self.forwarded_receipts <= previous_count:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    fail(scenario, "relay did not forward the EventSettle receipt ACK")
                self._condition.wait(remaining)

    def settle_response_count(self) -> int:
        with self._condition:
            return len(self.settle_response_messages)

    def wait_for_settle_response(
        self,
        run_id: str,
        request_id: str,
        previous_count: int,
        timeout: float,
        scenario: str,
    ) -> dict[str, Any]:
        deadline = time.monotonic() + timeout
        with self._condition:
            while True:
                for message in self.settle_response_messages[previous_count:]:
                    if message.get("runId") == run_id and message.get("requestId") == request_id:
                        return message
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    fail(
                        scenario,
                        f"Agent did not replay settled response for run {run_id!r}, request {request_id!r}",
                    )
                self._condition.wait(remaining)

    def receipt_holds(self) -> list[tuple[str, str, str, str]]:
        with self._condition:
            return list(self.held_receipt_identities)

    def forwarded_receipt_ids(self) -> list[tuple[str, str, str, str]]:
        with self._condition:
            return list(self.forwarded_receipt_identities)


    def _observe_hub_command(self, message: dict[str, Any]) -> None:
        command = message.get("command")
        if not isinstance(command, dict):
            return
        with self._condition:
            self.commands.append(message)
            payload = command.get("payload")
            if (
                command.get("type") == self.response_command_type
                and isinstance(payload, dict)
                and payload.get("group") == self.response_group
                and self.response_armed
            ):
                self._expected_response = (
                    str(message.get("runId", "")),
                    str(message.get("requestId", command.get("requestId", ""))),
                )
            if command.get("type") == "event.settle":
                self.settle_count += 1
                self.settle_seen.set()
                self._expected_receipt = tuple(
                    str(message.get(key, "")) for key in ("eventId", "runId", "requestId", "commandHash")
                )
                self._settle_response_ids.add(
                    (str(message.get("runId", "")), str(message.get("requestId", command.get("requestId", ""))))
                )
            self._condition.notify_all()

    def _observe_agent_message(self, message: dict[str, Any], connection_id: int) -> bool:
        with self._condition:
            if message.get("type") == "event.sources":
                self.source_reports.append(message)
                self.source_report_seen.set()
                self._condition.notify_all()
            if message.get("type") == "response":
                identity = (str(message.get("runId", "")), str(message.get("requestId", "")))
                data = message.get("data")
                if (
                    identity in self._settle_response_ids
                    and isinstance(data, dict)
                    and data.get("status") == "settled"
                ):
                    self.settle_response_messages.append(message)
                    if self.fail_next_settle_response:
                        self.fail_next_settle_response = False
                        self._failed_settle_connection_id = connection_id
                        self._failed_settle_response_baseline = len(self.settle_response_messages)
                        self._arm_receipt_hold_locked()
                        self.settle_failure_seen.set()
                        self._condition.notify_all()
                        return True
                    self._condition.notify_all()
            return False

    def _should_hold_response(self, message: dict[str, Any]) -> bool:
        if message.get("type") != "response" or not self.response_armed:
            return False
        with self._condition:
            identity = (str(message.get("runId", "")), str(message.get("requestId", "")))
            if self._expected_response is None or identity != self._expected_response:
                return False
            self.late_response = message
            self.response_armed = False
            self.response_seen.set()
            self._condition.notify_all()
            return True

    def _should_hold_receipt(self, message: dict[str, Any]) -> bool:
        if message.get("type") != "transport_ack":
            return False
        with self._condition:
            identity = tuple(
                str(message.get(key, "")) for key in ("eventId", "runId", "requestId", "commandHash")
            )
            if not self.receipt_hold_armed or self._expected_receipt != identity:
                return False
            self.held_receipt_identities.append(identity)
            self.receipt_seen.set()
            self._condition.notify_all()
            return True

    def _should_hold_settle_reply(self, message: dict[str, Any]) -> bool:
        if message.get("type") != "response":
            return False
        identity = (str(message.get("runId", "")), str(message.get("requestId", "")))
        data = message.get("data")
        with self._condition:
            if (
                not self.settle_reply_hold_armed
                or identity not in self._settle_response_ids
                or not isinstance(data, dict)
                or data.get("status") != "settled"
            ):
                return False
            self.settle_reply_seen.set()
            self._condition.notify_all()
            return True


    def _forward(
        self, source: socket.socket, target: socket.socket, agent_to_hub: bool, connection_id: int
    ) -> None:
        blocked: list[bytes] | None = None
        release: threading.Event | None = None
        while True:
            if blocked is not None and release is not None and release.is_set():
                for frame in blocked:
                    target.sendall(frame)
                blocked = None
                release = None
                continue
            readable, _, _ = select.select([source], [], [], 0.1)
            if not readable:
                continue
            item = self._frame(source)
            if item is None:
                return
            raw, opcode, payload = item
            message = self._json_message(opcode, payload)
            if not agent_to_hub:
                self._observe_hub_command(message)
                if blocked is not None:
                    blocked.append(raw)
                else:
                    target.sendall(raw)
                continue
            if self._observe_agent_message(message, connection_id):
                return
            if blocked is not None:
                if self._should_hold_settle_reply(message) or self._should_hold_receipt(message):
                    release = self.receipt_release
                blocked.append(raw)
                continue
            if self._should_hold_response(message):
                blocked = [raw]
                release = self.response_release
                continue
            if self._should_hold_settle_reply(message):
                blocked = [raw]
                release = self.receipt_release
                continue
            if self._should_hold_receipt(message):
                blocked = [raw]
                release = self.receipt_release
                continue
            if message.get("type") == "transport_ack":
                with self._condition:
                    identity = tuple(
                        str(message.get(key, "")) for key in ("eventId", "runId", "requestId", "commandHash")
                    )
                    self.forwarded_receipt_identities.append(identity)
                    if identity == self._expected_receipt:
                        self.forwarded_receipts += 1
                    self._condition.notify_all()
            target.sendall(raw)

    def _proxy(self, client: socket.socket, upstream: socket.socket, connection_id: int) -> None:
        def forward(source: socket.socket, target: socket.socket, agent_to_hub: bool) -> None:
            try:
                self._forward(source, target, agent_to_hub, connection_id)
            except (OSError, ValueError, GateError):
                pass
            finally:
                for connection in (client, upstream):
                    with contextlib.suppress(OSError):
                        connection.shutdown(socket.SHUT_RDWR)

        directions = (
            threading.Thread(target=forward, args=(client, upstream, True), daemon=True),
            threading.Thread(target=forward, args=(upstream, client, False), daemon=True),
        )
        for direction in directions:
            direction.start()
        for direction in directions:
            direction.join()

    def close(self) -> None:
        self.response_release.set()
        self.receipt_release.set()
        self.server.shutdown()
        self.server.server_close()


class ManagedProcess:
    def __init__(
        self,
        args: list[str],
        env: dict[str, str],
        scenario: str,
        stdin: int | None = subprocess.DEVNULL,
        stdout_pipe: bool = False,
    ) -> None:
        self.args = args
        self.scenario = scenario
        self.stdout_capture = None if stdout_pipe else tempfile.TemporaryFile(mode="w+b")
        self.stderr_capture = tempfile.TemporaryFile(mode="w+b")
        self.closed = False
        try:
            self.process = subprocess.Popen(
                args,
                env=env,
                stdin=stdin,
                stdout=subprocess.PIPE if stdout_pipe else self.stdout_capture,
                stderr=self.stderr_capture,
                start_new_session=True,
            )
        except OSError as error:
            if self.stdout_capture is not None:
                self.stdout_capture.close()
            self.stderr_capture.close()
            fail(scenario, f"could not start process: {error}")
        ACTIVE_PROCESSES.append(self)
    def alive(self) -> bool:
        return self.process.poll() is None

    def diagnostics(self) -> str:
        def tail(capture: Any) -> str:
            if capture is None:
                return "<MCP stdio pipe>"
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
        for stream in (
            self.stdout_capture,
            self.stderr_capture,
            self.process.stdin,
            self.process.stdout,
        ):
            if stream is not None:
                with contextlib.suppress(OSError, ValueError):
                    stream.close()
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
                 headers: dict[str, str] | None = None, scenario: str = "HTTP",
                 timeout: float | None = None) -> HttpResponse:
    request_headers = {"Host": f"127.0.0.1:{port}", "Connection": "close"}
    if headers:
        request_headers.update(headers)
    payload = None
    if body is not None:
        payload = json.dumps(body, separators=(",", ":")).encode()
        request_headers.setdefault("Content-Type", "application/json")
        request_headers["Content-Length"] = str(len(payload))
    try:
        connection = http.client.HTTPConnection("127.0.0.1", port, timeout=TIMEOUT if timeout is None else timeout)
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


def mcp_exchange(
    port: int,
    token: str,
    request: dict[str, Any],
    session: str | None,
    scenario: str,
    timeout: float | None = None,
) -> tuple[HttpResponse, dict[str, Any]]:
    headers = {
        "Authorization": f"Bearer {token}",
        "Accept": "application/json, text/event-stream",
        "MCP-Protocol-Version": PROTOCOL_VERSION,
    }
    if session:
        headers["Mcp-Session-Id"] = session
    response = http_request(port, "POST", "/mcp", request, headers, scenario, timeout=timeout)
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


def mcp_call(
    port: int,
    token: str,
    session: str,
    request_id: int,
    method: str,
    params: dict[str, Any],
    label: str,
    timeout: float | None = None,
) -> dict[str, Any]:
    _, message = mcp_exchange(
        port,
        token,
        {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params},
        session,
        label,
        timeout=timeout,
    )
    return message

def stdio_exchange(process: ManagedProcess, request: dict[str, Any], scenario: str) -> dict[str, Any]:
    stdin = process.process.stdin
    stdout = process.process.stdout
    if stdin is None or stdout is None:
        fail(scenario, "Agent process was not started with MCP stdio pipes")
    try:
        stdin.write((json.dumps(request, separators=(",", ":")) + "\n").encode("utf-8"))
        stdin.flush()
        readable, _, _ = select.select([stdout], [], [], TIMEOUT)
        if not readable:
            fail(scenario, f"timed out waiting for MCP stdio response; {process.diagnostics()}")
        line = stdout.readline()
    except (OSError, ValueError) as error:
        fail(scenario, f"MCP stdio exchange failed: {error}; {process.diagnostics()}")
    if not line:
        fail(scenario, f"MCP stdio closed before responding; {process.diagnostics()}")
    try:
        value = json.loads(line.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(scenario, f"MCP stdio returned invalid JSON: {error}: {line[:500]!r}")
    if not isinstance(value, dict) or value.get("id") != request.get("id"):
        fail(scenario, f"MCP stdio response did not match request: {value!r}")
    return value


def stdio_notify(process: ManagedProcess, method: str, params: dict[str, Any], scenario: str) -> None:
    stdin = process.process.stdin
    if stdin is None:
        fail(scenario, "Agent process was not started with MCP stdio input")
    try:
        message = {"jsonrpc": "2.0", "method": method, "params": params}
        stdin.write((json.dumps(message, separators=(",", ":")) + "\n").encode("utf-8"))
        stdin.flush()
    except (OSError, ValueError) as error:
        fail(scenario, f"MCP stdio notification failed: {error}")


def open_stdio_session(process: ManagedProcess, label: str) -> list[dict[str, Any]]:
    initialized = stdio_exchange(
        process,
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "contract-parity-stdio", "version": "1"},
            },
        },
        f"{label} initialize",
    )
    if "error" in initialized or not initialized.get("result", {}).get("protocolVersion"):
        fail(f"{label} initialize", f"unexpected result: {initialized}")
    stdio_notify(process, "notifications/initialized", {}, f"{label} initialized")
    listed = stdio_exchange(
        process,
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
        f"{label} tools/list",
    )
    tools = listed.get("result", {}).get("tools")
    if "error" in listed or not isinstance(tools, list):
        fail(f"{label} tools/list", f"missing tools: {listed}")
    return tools


def stdio_tool_call(
    process: ManagedProcess,
    request_id: int,
    name: str,
    arguments: dict[str, Any],
    label: str,
) -> Any:
    return json_result(
        stdio_exchange(
            process,
            {
                "jsonrpc": "2.0",
                "id": request_id,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments},
            },
            label,
        ),
        label,
    )


def structured_tool_result(value: Any, scenario: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail(scenario, f"tool result is not an object: {value!r}")
    if value.get("isError") is True or "error" in value:
        fail(scenario, f"tool returned an error: {value}")
    structured = value.get("structuredContent")
    if isinstance(structured, dict):
        return structured
    content = value.get("content")
    if isinstance(content, list):
        for item in content:
            if isinstance(item, dict) and item.get("type") == "text":
                try:
                    decoded = json.loads(item.get("text", ""))
                except json.JSONDecodeError:
                    continue
                if isinstance(decoded, dict):
                    return decoded
    return value


def local_event_call(
    binary: Path,
    config: Path,
    env: dict[str, str],
    name: str,
    arguments: dict[str, Any],
    scenario: str,
) -> dict[str, Any]:
    return structured_tool_result(local_tool(binary, config, name, arguments, env, scenario), scenario)


def local_external_event(
    binary: Path,
    config: Path,
    env: dict[str, str],
    payload: dict[str, Any],
    scenario: str,
) -> dict[str, Any]:
    result = run_checked(
        [
            str(binary),
            "local",
            "--config",
            str(config),
            "call",
            "privateevent.inject",
            "--arguments-file",
            "-",
        ],
        env,
        scenario,
        input_text=json.dumps(payload, ensure_ascii=False),
    )
    return structured_tool_result(json_load_stdout(result, scenario), scenario)


def event_panel(value: dict[str, Any], scenario: str) -> dict[str, Any]:
    panel = value.get("events")
    if not isinstance(panel, dict) or not isinstance(panel.get("current"), str):
        fail(scenario, f"response omitted the event panel: {value}")
    entries = panel.get("new")
    if not isinstance(entries, list):
        fail(scenario, f"event panel new field is not a list: {panel}")
    if len(entries) > 5:
        fail(scenario, f"event panel exceeded the five-item cap: {panel}")
    return panel


def assert_event_counts(
    value: dict[str, Any],
    expected: tuple[int, int, int],
    scenario: str,
) -> dict[str, Any]:
    panel = event_panel(value, scenario)
    actual = panel["current"]
    low, medium, high = expected
    wanted = f"low: {low} | medium: {medium} | high: {high}"
    if actual != wanted:
        fail(scenario, f"pending event counts differ: expected {wanted!r}, got {actual!r}")
    return panel


def event_panel_ids(panel: dict[str, Any], scenario: str) -> list[str]:
    identifiers: list[str] = []
    for entry in panel["new"]:
        if not isinstance(entry, dict) or len(entry) != 1:
            fail(scenario, f"event panel item is not a single summary map: {entry!r}")
        key = next(iter(entry))
        event_id, separator, _summary = key.partition(" | ")
        if not separator or not event_id:
            fail(scenario, f"event panel key omitted its event id and summary: {key!r}")
        identifiers.append(event_id)
    return identifiers


def event_items(value: dict[str, Any], scenario: str) -> list[dict[str, Any]]:
    items = value.get("items")
    if not isinstance(items, list) or any(not isinstance(item, dict) for item in items):
        fail(scenario, f"event list omitted an item array: {value}")
    return items


def assert_event_tool_semantics(
    tools: list[dict[str, Any]],
    label: str,
    hub: bool = False,
    coordinator: bool = False,
) -> None:
    descriptors = descriptor_map(tools)
    event_names = {"event.list", "event.get", "event.mark"}
    if coordinator:
        leaked = event_names.intersection(descriptors)
        if leaked:
            fail(label, f"targeted event tools leaked into the no-target profile: {sorted(leaked)}")
    else:
        missing = event_names.difference(descriptors)
        if missing:
            fail(label, f"missing current event tools: {sorted(missing)}")
        for name, base_required in (
            ("event.list", set()),
            ("event.get", {"eventId"}),
            ("event.mark", {"eventIds"}),
        ):
            schema = descriptors[name].get("inputSchema", {})
            required = set(schema.get("required", []))
            expected = base_required | ({"agentId"} if hub else set())
            if required != expected:
                fail(label, f"{name} required fields changed: {sorted(required)}")
            if hub and "agentId" not in schema.get("properties", {}):
                fail(label, f"{name} does not expose its required target Agent")
    if "privateevent.inject" in descriptors:
        fail(label, "private external injection was advertised as a model-callable tool")


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
        "process.exec", "process.batch", "process.read", "process.list",
        "process.cancel",
    }
    if hub:
        required |= {"hub.process.status", "hub.process.list"}
    if coordinator:
        required = {"hub.process.status", "hub.process.list"}
    else:
        required |= {"event.list", "event.get", "event.mark"}
    for name in required:
        if name not in descriptors:
            fail(label, f"missing required descriptor {name}")
    assert_event_tool_semantics(tools, label, hub=hub, coordinator=coordinator)
    for name in ("process.read", "process.cancel", "hub.process.status"):
        if name not in descriptors:
            continue
        schema = descriptors[name].get("inputSchema", {})
        if "processId" not in schema.get("required", []):
            fail(label, f"{name} processId is not required")
    read_schema = descriptors.get("process.read", {}).get("inputSchema", {})
    read_props = read_schema.get("properties", {})
    wait_seconds = read_props.get("waitSeconds", {})
    if "process.read" in descriptors and (
        wait_seconds.get("default") != 5
        or wait_seconds.get("minimum") != 0
        or wait_seconds.get("maximum") != 30
    ):
        fail(label, "process.read waitSeconds is not default5/clamped0..30")
    view = read_props.get("view", {})
    if "process.read" in descriptors:
        if view.get("default") != "auto":
            fail(label, "process.read view does not advertise default auto")
        read_validator = Draft202012Validator(read_schema)
        read_base = {"processId": "schema-process"}
        if hub:
            read_base["agentId"] = "schema-agent"
        if not read_validator.is_valid(read_base):
            fail(label, "process.read rejects omission of optional read controls")
        for field, accepted, rejected in (
            ("view", ("auto", "status"), ("output", "AUTO", 1, {})),
            ("cursor", ("opaque-cursor",), (1, {}, [])),
            ("maxBytes", (4096, 8192, 1048576), (4095, 1048577, 4096.5, True)),
            ("waitSeconds", (0, 5, 30), (-1, 31, 0.5, True)),
        ):
            for value in accepted:
                if not read_validator.is_valid({**read_base, field: value}):
                    fail(label, f"process.read schema rejects valid {field}={value!r}")
            for value in rejected:
                if read_validator.is_valid({**read_base, field: value}):
                    fail(label, f"process.read schema accepts invalid {field}={value!r}")
    response_bytes = read_props.get("maxBytes", {})
    if "process.read" in descriptors and (
        response_bytes.get("default") is not None
        or response_bytes.get("minimum") != 4096
        or response_bytes.get("maximum") != 1048576
    ):
        fail(label, "process.read maxBytes must use configured defaults and enforce 4096..1048576")
    if "process.read" in descriptors and (
        "cursor" not in read_props or "cursor" in read_schema.get("required", [])
    ):
        fail(label, "process.read cursor must be optional string for output pagination")
    if hub and "process.read" in descriptors:
        if "agentId" not in read_schema.get("required", []):
            fail(label, "Hub process.read agentId is not required")
    retired_process_tools = {"process.status", "process.output", "process.result"}
    if retired_process_tools.intersection(descriptors):
        fail(label, f"retired live process tools are still advertised: {sorted(retired_process_tools.intersection(descriptors))}")
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
PROCESS_BINARY_OUTPUT = b"\xff" * 7000
PROCESS_BINARY_FORMAT = r"\377" * len(PROCESS_BINARY_OUTPUT)
PROCESS_ESCAPE_OUTPUT = ('"\\\n\t' * 700)
PROCESS_AUTO_MARKER = "contract-parity-auto-before-exit"
PROCESS_AUTO_COMMAND = f"printf '%s' '{PROCESS_AUTO_MARKER}'; sleep 2"
PROCESS_TAIL_MARKER = "contract-parity-post-exit-tail"
PROCESS_MCP_PAGINATION_COMMAND = 'printf "%s" "$1"; sleep 1'
PROCESS_TAIL_COMMAND = f"(sleep 2; printf '%s' '{PROCESS_TAIL_MARKER}') &"


def configure_process_fixture(
    config_path: Path,
    http_port: int | None = None,
    http_token: str | None = None,
    process_response_bytes: int | None = None,
) -> None:
    try:
        data = json.loads(config_path.read_text())
        data["policy"] = {
            "allow": [
                {"program": "/usr/bin/printf", "argsPrefix": [marker]}
                for marker in PROCESS_MARKERS
            ]
            + [
                {"program": "/usr/bin/printf", "argsPrefix": [PROCESS_OVERFLOW_MARKER]},
                {"program": "/usr/bin/printf", "argsPrefix": ["%s", PROCESS_ESCAPE_OUTPUT]},
                {"program": "/usr/bin/printf", "argsPrefix": [PROCESS_BINARY_FORMAT]},
            ]
            + [
                {"program": "/bin/sh", "argsPrefix": ["-c", command]}
                for command in (
                    PROCESS_AUTO_COMMAND, PROCESS_TAIL_COMMAND, PROCESS_MCP_PAGINATION_COMMAND
                )
            ]
            + [
                {"program": "/usr/bin/true", "argsPrefix": []},
                {"program": "/usr/bin/sleep", "argsPrefix": ["30"]},
                {"program": "/usr/bin/sleep", "argsPrefix": ["1"]},
                {"program": "/usr/bin/printf", "argsPrefix": ["feedback-child-A"]},
                {"program": "/usr/bin/printf", "argsPrefix": ["feedback-child-B"]},
                {"program": "/usr/bin/pwd", "argsPrefix": []},
            ],
            "confirm": [],
            "deny": [{"program": "/usr/bin/echo", "argsPrefix": []}],
        }
        if process_response_bytes is not None:
            limits = data.setdefault("limits", {})
            if not isinstance(limits, dict):
                fail("process fixture", "generated limits config is not an object")
            limits["processResponseBytes"] = process_response_bytes
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

def write_large_mcp_result_fixture(
    workspace: Path, *, size: int = 512, filename: str = "large-mcp-result.png"
) -> None:
    """Create a valid incompressible image for MCP retention boundary fixtures."""
    width = height = size
    row_bytes = width * 4
    pixels = random.Random(0).randbytes(row_bytes * height)
    scanlines = b"".join(
        b"\x00" + pixels[offset : offset + row_bytes]
        for offset in range(0, len(pixels), row_bytes)
    )

    def chunk(kind: bytes, value: bytes) -> bytes:
        return (
            struct.pack(">I", len(value))
            + kind
            + value
            + struct.pack(">I", zlib.crc32(kind + value) & 0xFFFFFFFF)
        )

    image = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(scanlines, level=1))
        + chunk(b"IEND", b"")
    )
    try:
        (workspace / filename).write_bytes(image)
    except OSError as error:
        fail("MCP retention fixture", f"could not create the large image fixture: {error}")


def configure_event_fixture(
    config_path: Path,
    low_ttl_seconds: int | None = None,
    internal_overrides: dict[str, str] | None = None,
) -> None:
    try:
        data = json.loads(config_path.read_text())
        events = data.setdefault("events", {})
        if not isinstance(events, dict):
            fail("event fixture", "generated events config is not an object")
        if low_ttl_seconds is not None:
            events["lowTtlSeconds"] = low_ttl_seconds
        if internal_overrides is not None:
            events["internalOverrides"] = internal_overrides
        config_path.write_text(json.dumps(data, indent=2) + "\n")
    except (OSError, TypeError, json.JSONDecodeError) as error:
        fail("event fixture", f"could not configure private event fixture: {error}")


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



def validate_operation_request(
    document: dict[str, Any],
    schema_path: str,
    method: str,
    request_path: str,
    body: Any | None,
    scenario: str,
) -> None:
    operation = document.get("paths", {}).get(schema_path, {}).get(method.lower(), {})
    actual_url = urlsplit(request_path)
    expected_segments = schema_path.split("/")
    actual_segments = actual_url.path.split("/")
    if len(expected_segments) != len(actual_segments):
        fail(scenario, f"request path does not match {schema_path}: {request_path}")
    parameters = [
        resolve_local_ref(document, value)
        for value in operation.get("parameters", [])
    ]
    for expected, actual in zip(expected_segments, actual_segments):
        if expected.startswith("{") and expected.endswith("}"):
            name = expected[1:-1]
            parameter = next(
                (value for value in parameters if value.get("in") == "path" and value.get("name") == name),
                None,
            )
            if parameter is None:
                fail(scenario, f"OpenAPI operation omits path parameter {name}")
            value = unquote(actual)
            if parameter.get("required") is True and not value:
                fail(scenario, f"required path parameter {name} is empty")
            schema = resolve_local_ref(document, parameter.get("schema", {}))
            errors = list(schema_validator(document, schema).iter_errors(value))
            if errors:
                fail(scenario, f"path parameter {name} violates OpenAPI: {errors[0].message}")
        elif expected != actual:
            fail(scenario, f"request path does not match {schema_path}: {request_path}")

    query = parse_qs(actual_url.query, keep_blank_values=True)
    for parameter in parameters:
        if parameter.get("in") != "query":
            continue
        name = parameter.get("name")
        if not isinstance(name, str):
            fail(scenario, f"OpenAPI query parameter has no name: {parameter}")
        values = query.get(name)
        if not values:
            if parameter.get("required") is True:
                fail(scenario, f"request omitted required query parameter {name}")
            continue
        raw_value = values[-1]
        schema = resolve_local_ref(document, parameter.get("schema", {}))
        kind = schema.get("type")
        try:
            if kind == "integer":
                parsed_value: Any = int(raw_value)
            elif kind == "number":
                parsed_value = float(raw_value)
            elif kind == "boolean":
                if raw_value.lower() not in {"true", "false"}:
                    fail(scenario, f"query parameter {name} is not a valid boolean: {raw_value!r}")
                parsed_value = raw_value.lower() == "true"
            else:
                parsed_value = raw_value
        except ValueError:
            fail(scenario, f"query parameter {name} is not a valid {kind}: {raw_value!r}")
        errors = list(schema_validator(document, schema).iter_errors(parsed_value))
        if errors:
            fail(scenario, f"query parameter {name} violates OpenAPI: {errors[0].message}")

    request_body = operation.get("requestBody")
    if body is None:
        if isinstance(request_body, dict) and request_body.get("required") is True:
            fail(scenario, "request omitted its required JSON body")
        return
    schema = (
        request_body.get("content", {})
        .get("application/json", {})
        .get("schema")
        if isinstance(request_body, dict)
        else None
    )
    if schema is None:
        fail(scenario, "request body has no declared JSON schema")
    schema = resolve_local_ref(document, schema)
    errors = sorted(schema_validator(document, schema).iter_errors(body), key=str)
    if errors:
        error = errors[0]
        fail(scenario, f"request body violates OpenAPI at {list(error.path)}: {error.message}")


def require_schema_contract(document: dict[str, Any], schemas: dict[str, Any]) -> None:
    paths = document.get("paths", {})
    if any(path.startswith("/v1/jobs") for path in paths):
        fail("schema/contract", "retired /v1/jobs HTTP routes are still advertised")
    process_paths = {
        "/v1/process/exec", "/v1/process/batch", "/v1/process",
        "/v1/process/{processId}/read", "/v1/process/{processId}/cancel",
    }
    missing = process_paths - set(paths)
    if missing:
        fail("schema/contract", f"required process HTTP routes are missing: {sorted(missing)}")
    retired_read_paths = {
        "/v1/process/{processId}",
        "/v1/process/{processId}/output",
        "/v1/process/{processId}/result",
    }
    if retired_read_paths.intersection(paths):
        fail("schema/contract", f"retired live process HTTP routes are still advertised: {sorted(retired_read_paths.intersection(paths))}")
    list_parameters = paths["/v1/process"]["get"].get("parameters", [])
    list_by_name = {item["name"]: item for item in list_parameters}
    list_limit = list_by_name["limit"]["schema"]
    if list_limit.get("default") != 50 or list_limit.get("minimum") != 1 or list_limit.get("maximum") != 100:
        fail("schema/contract", "HTTP process list limit is not default50/clamped1..100")
    read_parameters = [
        resolve_local_ref(document, parameter)
        for parameter in paths["/v1/process/{processId}/read"]["get"].get("parameters", [])
    ]
    read_by_name = {item["name"]: item for item in read_parameters}
    agent_id = read_by_name.get("agentId", {})
    if agent_id.get("in") != "query" or agent_id.get("required") is not True:
        fail("schema/contract", "HTTP process.read must require agentId")
    wait_schema = read_by_name.get("waitSeconds", {}).get("schema", {})
    if wait_schema.get("default") != 5 or wait_schema.get("minimum") != 0 or wait_schema.get("maximum") != 30:
        fail("schema/contract", "HTTP process.read waitSeconds is not default5/clamped0..30")
    view_schema = resolve_local_ref(document, read_by_name.get("view", {}).get("schema", {}))
    if view_schema.get("default") != "auto" or view_schema.get("enum") != ["auto", "status"]:
        fail("schema/contract", "HTTP process.read view is not default auto with auto/status choices")
    if "cursor" not in read_by_name:
        fail("schema/contract", "HTTP process.read does not declare an output cursor")
    response_bytes = read_by_name.get("maxBytes", {}).get("schema", {})
    if (
        "default" in response_bytes
        or response_bytes.get("minimum") != 4096
        or response_bytes.get("maximum") != 1048576
    ):
        fail("schema/contract", "HTTP process.read maxBytes must use configured defaults and enforce 4096..1048576")
    for name in (
        "ProcessExecRequest", "ProcessBatchExecRequest", "ProcessResponse", "ProcessReadResponse", "ProcessBatchResponse",
        "ProcessListResponse", "ProcessOutputSegment", "ProcessOutputPage", "ProcessMcpResult",
        "ProcessReadView", "ProcessCancelResponse", "ProcessUnavailableResponse",
    ):
        if not isinstance(schemas.get(name), dict):
            fail("schema/contract", f"OpenAPI process schema {name} is missing")
    process_example = {
        "agentId": "schema-agent", "processId": "schema-process",
        "kind": "command", "state": "completed", "captureStatus": "complete",
    }
    validate_instance(document, schemas, "ProcessResponse", process_example, "schema/contract")
    for field in process_example:
        incomplete = {key: value for key, value in process_example.items() if key != field}
        assert_rejected(document, schemas, "ProcessResponse", incomplete, f"schema/contract required {field}")
    retired_response_fields = {
        "status": "completed", "completedInline": True, "pollAfterMs": 1000,
        "inlineOutput": {
            "stdout": {"data": "", "encoding": "utf8"},
            "stderr": {"data": "", "encoding": "utf8"},
        },
        "outputPreview": {"stdout": "", "stderr": ""}, "resultAvailable": False,
    }
    for field, value in retired_response_fields.items():
        assert_rejected(
            document, schemas, "ProcessResponse", {**process_example, field: value},
            f"schema/contract retired {field}",
        )
    mcp_status = resolve_local_ref(
        document, schemas["ProcessMcpResult"].get("properties", {}).get("status", {})
    )
    if set(mcp_status.get("enum", [])) != {
        "pending", "included", "deferred", "unavailable", "not_retained",
    }:
        fail("schema/contract", "ProcessMcpResult status choices changed")
    read_operation = paths["/v1/process/{processId}/read"]["get"]
    for status, schema_name in ((200, "ProcessReadResponse"), (503, "ProcessUnavailableResponse")):
        response = resolve_local_ref(document, read_operation.get("responses", {}).get(str(status)))
        response_schema = (
            response.get("content", {}).get("application/json", {}).get("schema")
            if isinstance(response, dict)
            else None
        )
        if not isinstance(response_schema, dict) or response_schema.get("$ref") != (
            f"#/components/schemas/{schema_name}"
        ):
            fail("schema/contract", f"HTTP process.read {status} response does not use {schema_name}")

    event_operations = {
        "/v1/events": ("get", "listEvents", "EventListResponse"),
        "/v1/events/{eventId}": ("get", "getEvent", "EventGetResponse"),
        "/v1/events/mark": ("post", "markEvents", "EventMarkResponse"),
    }
    missing_events = set(event_operations) - set(paths)
    if missing_events:
        fail("schema/contract", f"required event HTTP routes are missing: {sorted(missing_events)}")
    event_route_operations: dict[str, dict[str, Any]] = {}
    for path, (method, operation_id, response_name) in event_operations.items():
        operation = paths[path].get(method)
        if not isinstance(operation, dict) or operation.get("operationId") != operation_id:
            fail("schema/contract", f"{path} is missing the {operation_id} operation")
        event_route_operations[path] = operation
        response = resolve_local_ref(document, operation.get("responses", {}).get("200"))
        response_schema = (
            response.get("content", {}).get("application/json", {}).get("schema")
            if isinstance(response, dict)
            else None
        )
        if not isinstance(response_schema, dict) or response_schema.get("$ref") != (
            f"#/components/schemas/{response_name}"
        ):
            fail("schema/contract", f"{path} does not return the flat {response_name} schema")

    def event_parameter(path: str, name: str, location: str) -> dict[str, Any]:
        operation = event_route_operations[path]
        parameter = next(
            (
                resolve_local_ref(document, value)
                for value in operation.get("parameters", [])
                if resolve_local_ref(document, value).get("name") == name
                and resolve_local_ref(document, value).get("in") == location
            ),
            None,
        )
        if (
            parameter is None
            or parameter.get("required") is not True
            or resolve_local_ref(document, parameter.get("schema", {})).get("type") != "string"
        ):
            fail(
                "schema/contract",
                f"{path} does not require string {location} parameter {name}",
            )
        return parameter

    event_parameter("/v1/events", "agentId", "query")
    event_parameter("/v1/events/{eventId}", "agentId", "query")
    event_parameter("/v1/events/{eventId}", "eventId", "path")
    mark_operation = event_route_operations["/v1/events/mark"]
    mark_body = mark_operation.get("requestBody")
    mark_body_schema = (
        resolve_local_ref(
            document,
            mark_body.get("content", {}).get("application/json", {}).get("schema"),
        )
        if isinstance(mark_body, dict)
        else None
    )
    if (
        not isinstance(mark_body, dict)
        or mark_body.get("required") is not True
        or not isinstance(mark_body_schema, dict)
        or set(mark_body_schema.get("required", [])) != {"agentId", "eventIds"}
        or mark_body_schema.get("properties", {}).get("agentId", {}).get("type") != "string"
        or mark_body_schema.get("properties", {}).get("eventIds", {}).get("type") != "array"
        or mark_body_schema.get("properties", {}).get("eventIds", {}).get("maxItems") != 512
        or mark_body_schema.get("properties", {}).get("eventIds", {}).get("items", {}).get("type")
        != "string"
    ):
        fail("schema/contract", "EventMarkRequest does not require agentId and a bounded string-ID array")

    event_list_item = schemas.get("EventListItem", {})
    event_record = schemas.get("EventRecord", {})
    event_source = schemas.get("EventSource", {})
    if (
        not isinstance(event_list_item, dict)
        or not {"eventId", "summary", "severity", "createdAt", "status"}.issubset(
            set(event_list_item.get("required", []))
        )
        or event_list_item.get("additionalProperties") is not False
        or event_list_item.get("properties", {}).get("summary", {}).get("maxLength") != 32
        or not isinstance(event_record, dict)
        or not {"eventId", "message", "severity", "createdAt", "status", "source", "shownCount", "expiresAt"}.issubset(
            set(event_record.get("required", []))
        )
        or event_record.get("properties", {}).get("source", {}).get("$ref")
        != "#/components/schemas/EventSource"
        or not isinstance(event_source, dict)
        or not {"kind", "ref"}.issubset(set(event_source.get("required", [])))
        or event_source.get("additionalProperties") is not False
        or event_source.get("properties", {}).get("kind", {}).get("$ref")
        != "#/components/schemas/EventSourceKind"
        or event_source.get("properties", {}).get("ref", {}).get("type") != "string"
    ):
        fail("schema/contract", "event record schemas omitted required compact or provenance fields")
    for name, expected_values in (
        ("EventSeverity", ["low", "medium", "high"]),
        ("EventStatus", ["pending", "handled", "expired"]),
        ("EventSourceKind", ["process", "skill_install", "external"]),
    ):
        schema = schemas.get(name, {})
        if not isinstance(schema, dict) or schema.get("enum") != expected_values:
            fail("schema/contract", f"{name} enum does not match the event domain contract")

    event_list_response = schemas.get("EventListResponse", {})
    event_get = schemas.get("EventGetResponse", {})
    event_mark_response = schemas.get("EventMarkResponse", {})
    if (
        not isinstance(event_list_response, dict)
        or "items" not in event_list_response.get("required", [])
        or event_list_response.get("additionalProperties") is not False
        or event_list_response.get("properties", {}).get("items", {}).get("type") != "array"
        or event_list_response.get("properties", {}).get("items", {}).get("items", {}).get("$ref")
        != "#/components/schemas/EventListItem"
        or not isinstance(event_get, dict)
        or not event_get.get("allOf")
        or event_get.get("allOf", [{}])[0].get("$ref") != "#/components/schemas/EventRecord"
        or event_get.get("unevaluatedProperties") is not False
        or not isinstance(event_mark_response, dict)
        or not {"handledIds", "notFoundIds"}.issubset(
            set(event_mark_response.get("required", []))
        )
        or event_mark_response.get("additionalProperties") is not False
        or event_mark_response.get("properties", {}).get("handledIds", {}).get("items", {}).get("type")
        != "string"
        or event_mark_response.get("properties", {}).get("notFoundIds", {}).get("items", {}).get("type")
        != "string"
    ):
        fail("schema/contract", "event success schemas are not flat list, detail, and mark DTOs")

    optional_panel_schemas = {
        "EventListResponse": event_list_response.get("properties", {}),
        "EventMarkResponse": event_mark_response.get("properties", {}),
    }
    for branch in event_get.get("allOf", []):
        resolved_branch = resolve_local_ref(document, branch)
        if isinstance(resolved_branch, dict):
            optional_panel_schemas.setdefault("EventGetResponse", {}).update(
                resolved_branch.get("properties", {})
            )
    for name, properties in optional_panel_schemas.items():
        schema = schemas.get(name, {})
        required = set(schema.get("required", []))
        if name == "EventGetResponse":
            for branch in event_get.get("allOf", []):
                resolved_branch = resolve_local_ref(document, branch)
                if isinstance(resolved_branch, dict):
                    required.update(resolved_branch.get("required", []))
        if (
            not isinstance(properties, dict)
            or properties.get("events", {}).get("$ref") != "#/components/schemas/EventPanel"
            or "events" in required
        ):
            fail("schema/contract", f"{name} does not keep its compact event panel optional")
    panel_schema = schemas.get("EventPanel", {})
    if (
        not isinstance(panel_schema, dict)
        or panel_schema.get("properties", {}).get("new", {}).get("maxItems") != 5
    ):
        fail("schema/contract", "EventPanel does not enforce the five-entry exposure cap")
    error_schema = schemas.get("AgentErrorResponse", {})
    if (
        not isinstance(error_schema, dict)
        or error_schema.get("properties", {}).get("events", {}).get("$ref")
        != "#/components/schemas/EventPanel"
        or "events" in error_schema.get("required", [])
    ):
        fail("schema/contract", "targeted Agent error responses do not model an optional event panel")
    gateway_schema = schemas.get("ErrorResponse", {})
    if not isinstance(gateway_schema, dict) or "events" in gateway_schema.get("properties", {}):
        fail("schema/contract", "offline/gateway errors advertise an event panel")

    for path, operation in event_route_operations.items():
        for status in ("400", "404", "500"):
            response = resolve_local_ref(
                document, operation.get("responses", {}).get(status)
            )
            schema = (
                response.get("content", {}).get("application/json", {}).get("schema")
                if isinstance(response, dict)
                else None
            )
            if not isinstance(schema, dict) or schema.get("$ref") != (
                "#/components/schemas/AgentErrorResponse"
            ):
                fail("schema/contract", f"{path} HTTP {status} does not use AgentErrorResponse")
        for status in ("401", "504"):
            response = resolve_local_ref(
                document, operation.get("responses", {}).get(status)
            )
            schema = (
                response.get("content", {}).get("application/json", {}).get("schema")
                if isinstance(response, dict)
                else None
            )
            if not isinstance(schema, dict) or schema.get("$ref") != (
                "#/components/schemas/ErrorResponse"
            ):
                fail("schema/contract", f"{path} HTTP {status} must omit the event panel")
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
        {"command": shlex.join(["/usr/bin/printf", PROCESS_MARKERS[0]]), "waitSeconds": 5},
        env,
        "Agent local process.exec",
    )
    local_exec = local_exec.get("structuredContent", local_exec)
    local_process_id = local_exec.get("processId")
    local_output = local_exec.get("output", {})
    local_stdout = local_output.get("stdout", {})
    if (
        not local_process_id
        or local_exec.get("state") != "completed"
        or local_exec.get("agentId") != "parity-local"
        or local_stdout.get("data") != PROCESS_MARKERS[0]
        or local_stdout.get("encoding") != "utf8"
        or process_response_size(local_exec) > 8192
    ):
        fail("Agent local process.exec", f"printf did not complete with full read response: {local_exec}")
    local_status = local_tool(
        binary, config, "process.read",
        {"processId": local_process_id, "view": "status", "waitSeconds": 0}, env,
        "Agent local process.read status view",
    )
    local_status = local_status.get("structuredContent", local_status)
    if (
        local_status.get("state") != "completed"
        or process_read_has_artifacts(local_status)
    ):
        fail("Agent local process.read status view", f"status view exposed process artifacts: {local_status}")
    local_read = local_tool(
        binary, config, "process.read",
        {"processId": local_process_id, "maxBytes": 4096}, env,
        "Agent local process.read auto view",
    )
    local_read = local_read.get("structuredContent", local_read)
    local_page = local_read.get("output", {})
    if (
        local_read.get("processId") != local_process_id
        or local_page.get("stdout", {}).get("data") != PROCESS_MARKERS[0]
        or str(local_page.get("stdout", {}).get("startOffset")) != "0"
        or local_page.get("hasMore") is not False
        or "mcpResult" in local_read
    ):
        fail("Agent local process.read auto view", f"unified read omitted retained output or result applicability: {local_read}")
    reports.append("PASS Agent local Unix MCP process.read status/auto views and command result applicability")
    overflow_response = local_tool(
        binary,
        config,
        "process.exec",
        {"command": shlex.join(["/usr/bin/printf", PROCESS_OVERFLOW_MARKER]), "waitSeconds": 5},
        env,
        "Agent local process.exec overflow",
    )
    overflow_response = overflow_response.get("structuredContent", overflow_response)
    overflow_output = overflow_response.get("output", {})
    overflow_process_id = overflow_response.get("processId")
    if (
        overflow_response.get("state") != "completed"
        or overflow_process_id is None
        or (overflow_response.get("captureStatus") == "complete" and overflow_output.get("hasMore") is not True)
        or process_response_size(overflow_response) > 8192
    ):
        fail("Agent local process.exec overflow", f"creation response did not preserve identity and bounded read page: {overflow_response}")
    process_read_status = local_tool(
        binary, config, "process.read",
        {"processId": overflow_process_id, "view": "status", "waitSeconds": 0}, env,
        "Agent local process.read status",
    )
    process_read_status = process_read_status.get("structuredContent", process_read_status)
    if process_read_status.get("state") != "completed" or "output" in process_read_status:
        fail("Agent local process.read status", f"status view did not omit output: {process_read_status}")
    local_tool(binary, config, "skills.setActive", {"id": "demo", "active": True}, env, "Agent local Skill activate")
    skill_value = local_tool(binary, config, "skills.run", {"id": "demo", "path": "scripts/check.sh", "waitSeconds": 0}, env, "Agent local Skill run")
    skill_value = skill_value.get("structuredContent", skill_value)
    skill_id = skill_value.get("processId")
    if skill_value.get("state") not in {"starting", "running", "completed"} or not skill_id:
        fail("Agent local Skill run", f"invalid real Skill process envelope: {skill_value}")
    skill_done = local_tool(
        binary, config, "process.read",
        {"processId": skill_id, "view": "status", "waitSeconds": 5}, env,
        "Agent local Skill completion status view",
    )
    skill_done = skill_done.get("structuredContent", skill_done)
    if skill_done.get("state") != "completed" or "output" in skill_done:
        fail("Agent local Skill completion status view", f"status view returned output: {skill_done}")
    skill_read = local_tool(
        binary, config, "process.read", {"processId": skill_id, "maxBytes": 8192}, env,
        "Agent local Skill read",
    )
    skill_read = skill_read.get("structuredContent", skill_read)
    if "skill-parity" not in skill_read.get("output", {}).get("stdout", {}).get("data", ""):
        fail("Agent local Skill read", f"Skill output was not retrievable: {skill_read}")
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
    installed_done = local_tool(
        binary, config, "process.read",
        {"processId": installed_id, "view": "status", "waitSeconds": 5}, env,
        "Agent local installed Skill status view",
    )
    installed_done = installed_done.get("structuredContent", installed_done)
    if installed_done.get("state") != "completed" or "output" in installed_done:
        fail("Agent local installed Skill status view", f"status view returned output: {installed_done}")
    installed_read = local_tool(
        binary, config, "process.read", {"processId": installed_id, "maxBytes": 8192}, env,
        "Agent local installed Skill read",
    )
    installed_read = installed_read.get("structuredContent", installed_read)
    if "inline-skill" not in installed_read.get("output", {}).get("stdout", {}).get("data", ""):
        fail("Agent local installed Skill read", f"installed Skill output was not retrievable: {installed_read}")
    reports.append("PASS Agent local Unix MCP: tools/list, agent.info, Skill run/completion, inline install/get(0,5), and installed run/completion")
    return process, config, env, tools


def start_http_agent(binary: Path, root: Path, reports: list[str]) -> tuple[ManagedProcess, Path, dict[str, str], int, str, list[dict[str, Any]]]:
    port = free_port()
    config, _, env = init_agent(binary, root, "standalone", "normal", "parity-http")
    prepare_skill_fixture(config)
    write_large_mcp_result_fixture(Path(json.loads(config.read_text())["workspaceRoot"]))
    write_large_mcp_result_fixture(
        Path(json.loads(config.read_text())["workspaceRoot"]),
        size=288, filename="retained-mcp-result.png",
    )
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
    configure_event_fixture(config, internal_overrides={"process.completed": "off"})
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
    skill_done = json_result(
        mcp_call(
            port, token, session, 5, "tools/call",
            {"name": "process.read", "arguments": {"processId": skill_id, "view": "status", "waitSeconds": 5}},
            "Agent HTTP Skill completion status view",
        ),
        "Agent HTTP Skill completion status view",
    )
    if skill_done.get("state") != "completed" or "output" in skill_done:
        fail("Agent HTTP Skill completion status view", f"status view returned output: {skill_done}")
    skill_output = json_result(
        mcp_call(
            port, token, session, 6, "tools/call",
            {"name": "process.read", "arguments": {"processId": skill_id, "maxBytes": 8192}},
            "Agent HTTP Skill read",
        ),
        "Agent HTTP Skill read",
    )
    if "skill-parity" not in skill_output.get("output", {}).get("stdout", {}).get("data", ""):
        fail("Agent HTTP Skill read", f"Skill output was not retrievable: {skill_output}")
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
    installed_done = json_result(
        mcp_call(
            port, token, session, 11, "tools/call",
            {"name": "process.read", "arguments": {"processId": installed_id, "view": "status", "waitSeconds": 5}},
            "Agent HTTP installed Skill status view",
        ),
        "Agent HTTP installed Skill status view",
    )
    if installed_done.get("state") != "completed" or "output" in installed_done:
        fail("Agent HTTP installed Skill status view", f"status view returned output: {installed_done}")
    installed_output = json_result(
        mcp_call(
            port, token, session, 12, "tools/call",
            {"name": "process.read", "arguments": {"processId": installed_id, "maxBytes": 8192}},
            "Agent HTTP installed Skill read",
        ),
        "Agent HTTP installed Skill read",
    )
    if "inline-skill" not in installed_output.get("output", {}).get("stdout", {}).get("data", ""):
        fail("Agent HTTP installed Skill read", f"installed Skill output was not retrievable: {installed_output}")
    reports.append("PASS Agent standalone streamable HTTP MCP: tools/list, Skill run/read status/auto, inline install/get(0,5), and installed run/read")
    return process, config, env, port, token, tools

def start_event_worker(
    binary: Path,
    config: Path,
    env: dict[str, str],
    scenario: str,
) -> ManagedProcess:
    return ManagedProcess(
        [
            str(binary),
            "stdio-worker",
            "--config",
            str(config),
            "--profile",
            "normal",
            "--supervisor-token",
            env["AGENTIC_GPT_SUPERVISOR_TOKEN"],
        ],
        env,
        scenario,
        stdin=subprocess.PIPE,
        stdout_pipe=True,
    )


def start_event_agent(
    binary: Path,
    root: Path,
) -> tuple[ManagedProcess, Path, dict[str, str], int, str]:
    port = free_port()
    config, _, env = init_agent(binary, root, "standalone", "normal", "parity-events")
    token = "contract-parity-events-token"
    data = json.loads(config.read_text())
    data["httpMcp"] = {
        "enabled": True,
        "host": "127.0.0.1",
        "port": port,
        "publicUrl": None,
        "bearerToken": "env:CONTRACT_PARITY_EVENTS_TOKEN",
        "allowHosts": ["localhost", "127.0.0.1", "::1"],
    }
    data["tunnel"] = {
        "tunnelId": "contract-parity-events",
        "apiKey": "env:CONTRACT_PARITY_EVENTS_TUNNEL_KEY",
        "client": {"autoDownload": False},
        "hubReporting": {"enabled": False, "detail": "metadata"},
    }
    config.write_text(json.dumps(data, indent=2) + "\n")
    configure_process_fixture(config)
    fixture_runtime_root = Path(tempfile.mkdtemp(prefix="e-", dir=root.parent))
    fixture_home = fixture_runtime_root / "h"
    fixture_runtime = fixture_runtime_root / "r"
    fixture_home.mkdir()
    fixture_runtime.mkdir()
    fixture_runtime.chmod(0o700)
    env = dict(
        env,
        HOME=str(fixture_home),
        XDG_RUNTIME_DIR=str(fixture_runtime),
        CONTRACT_PARITY_EVENTS_TOKEN=token,
        CONTRACT_PARITY_EVENTS_TUNNEL_KEY="unused-test-events-key",
        AGENTIC_GPT_SUPERVISOR_TOKEN="contract-parity-events-supervisor",
    )
    process = start_event_worker(binary, config, env, "Agent event stdio/Unix/HTTP MCP")
    socket_path = Path(env["HOME"]) / ".agentic_gpt" / "runtime" / "agent" / "parity-events" / "mcp.sock"
    wait_until(
        lambda: process.alive() and socket_path.exists() and tcp_ready(port),
        "Agent event multi-transport MCP",
        "stdio, private Unix socket, and HTTP MCP listeners",
        process=process,
    )
    return process, config, env, port, token


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
def restart_hub_process(
    binary: Path,
    root: Path,
    port: int,
    config: Path,
    env: dict[str, str],
    api_key: str,
    scenario: str,
) -> ManagedProcess:
    process = ManagedProcess(
        [
            str(binary),
            "--db",
            str(root / "hub.db"),
            "--config",
            str(config),
            "serve",
            "--bind",
            f"127.0.0.1:{port}",
            "--api-key",
            api_key,
            "--mcp-profile",
            "full",
        ],
        env,
        scenario,
    )
    wait_until(
        lambda: tcp_ready(port) and process.alive(),
        scenario,
        "restarted Hub HTTP listener with existing database/configuration",
        process=process,
    )
    return process




def register_hub_agent(hub_binary: Path, db: Path, config: Path, env: dict[str, str], agent_id: str, display: str, secret: str) -> None:
    run_checked(
        [str(hub_binary), "--db", str(db), "--config", str(config), "agent", "add", "--agent-id", agent_id, "--display-name", display, "--secret", secret],
        env,
        f"Hub register {agent_id}",
    )


def start_hub_agent(
    binary: Path,
    root: Path,
    profile: str,
    agent_id: str,
    hub_url: str,
    secret: str,
    reports: list[str],
    downstream: tuple[int, str] | None = None,
    process_response_bytes: int | None = None,
) -> tuple[ManagedProcess, Path, dict[str, str]]:
    config, _, env = init_agent(binary, root, "hub", profile, agent_id, hub_url, secret)
    if downstream is not None or process_response_bytes is not None:
        configure_process_fixture(
            config,
            *(downstream or (None, None)),
            process_response_bytes=process_response_bytes,
        )
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





def hub_json(port: int, api_key: str, method: str, path: str, body: Any | None,
             scenario: str, timeout: float | None = None) -> tuple[HttpResponse, Any]:
    response = http_request(
        port,
        method,
        path,
        body,
        {"Authorization": f"Bearer {api_key}"},
        scenario,
        timeout,
    )
    value = response.json(scenario) if response.body else None
    return response, value




def hub_event_http_call(
    port: int,
    api_key: str,
    document: dict[str, Any],
    method: str,
    path: str,
    body: Any | None,
    schema_path: str,
    schema_method: str,
    scenario: str,
    expected_status: int = 200,
) -> tuple[HttpResponse, dict[str, Any]]:
    if method.lower() != schema_method.lower():
        fail(scenario, f"request method {method} does not match OpenAPI {schema_method}")
    validate_operation_request(
        document, schema_path, schema_method, path, body, f"{scenario} request"
    )
    response, value = hub_json(port, api_key, method, path, body, scenario)
    if response.status != expected_status or not isinstance(value, dict):
        fail(scenario, f"HTTP {response.status}: {value}")
    validate_operation_response(
        document, schema_path, schema_method, expected_status, value, scenario
    )
    if expected_status == 200:
        event_panel(value, scenario)
    elif "events" in value:
        fail(scenario, f"error response unexpectedly included an event panel: {value}")
    return response, value


def hub_event_mcp_call(
    port: int,
    api_key: str,
    session: str,
    request_id: int,
    name: str,
    arguments: dict[str, Any],
    scenario: str,
) -> dict[str, Any]:
    return json_result(
        mcp_call(
            port,
            api_key,
            session,
            request_id,
            "tools/call",
            {"name": name, "arguments": arguments},
            scenario,
        ),
        scenario,
    )


def run_hub_event_gate(
    binary: Path,
    hub_port: int,
    hub_key: str,
    full_session: str,
    normal_id: str,
    normal_config: Path,
    normal_env: dict[str, str],
    room_id: str,
    document: dict[str, Any],
    reports: list[str],
) -> str:
    agents_response, agents_value = hub_json(
        hub_port, hub_key, "GET", "/v1/agents", None, "Hub no-target HTTP agents"
    )
    if agents_response.status != 200 or not isinstance(agents_value, dict) or "events" in agents_value:
        fail("Hub no-target HTTP agents", f"Agent listing was decorated with events: {agents_response.status}, {agents_value}")

    data = json.loads(normal_config.read_text())
    workspace = Path(data["workspaceRoot"])
    fixture_dir = workspace / "event-fixtures"
    fixture_dir.mkdir(parents=True, exist_ok=True)
    programs = {}
    for name, delay, exit_code in (("shared", 1, 0), ("medium", 1, 7), ("low", 10, 0)):
        program = fixture_dir / f"{name}.sh"
        program.write_text(f"#!/bin/sh\n/usr/bin/sleep {delay}\nexit {exit_code}\n")
        program.chmod(0o700)
        programs[name] = program
        data.setdefault("policy", {}).setdefault("allow", []).append(
            {"program": str(program), "argsPrefix": []}
        )
    data.setdefault("events", {}).setdefault("internalOverrides", {}).update(
        {"process.completed": "low", "process.failed": "medium"}
    )
    normal_process = next(
        process for process in ACTIVE_PROCESSES
        if str(normal_config) in process.args and process.alive()
    )
    reload_count = normal_process.diagnostics().count("live config reloaded;")
    normal_config.write_text(json.dumps(data, indent=2) + "\n")
    wait_until(
        lambda: normal_process.diagnostics().count("live config reloaded;") > reload_count,
        "Hub event producer policy", "the Agent to apply fixture policy", process=normal_process,
    )
    event_db = Path(normal_env["HOME"]) / ".agentic_gpt" / "state" / "agent" / normal_id / "events.sqlite3"

    def start_event_process(name: str) -> str:
        response, started = hub_json(
            hub_port, hub_key, "POST", "/v1/process/exec",
            {"agentId": normal_id, "command": shlex.join([str(programs[name])]),
             "needConfirm": False, "waitSeconds": 0},
            f"Hub real {name} event producer",
        )
        if response.status != 200 or started.get("state") not in ("starting", "running"):
            fail("Hub event producer admission", f"expected an actual nonterminal initial response: {started}")
        return started["processId"]

    def event_evidence(process_id: str) -> dict[str, Any] | None:
        with sqlite3.connect(f"{event_db.as_uri()}?mode=ro", uri=True, timeout=0.1) as connection:
            row = connection.execute(
                "SELECT event_id,message,severity,status,shown_count FROM events "
                "WHERE source_kind='process' AND source_ref=?", (process_id,),
            ).fetchone()
        if row is None:
            return None
        return {"eventId": row[0], "message": row[1], "severity": row[2],
                "status": row[3], "shownCount": row[4],
                "source": {"kind": "process", "ref": process_id}}

    def await_event(process_id: str) -> dict[str, Any]:
        wait_until(
            lambda: event_evidence(process_id) is not None,
            "Hub real event completion", "a process-produced inbox row",
            timeout=20, process=normal_process,
        )
        return event_evidence(process_id)

    shared_process_id = start_event_process("shared")
    inserted = await_event(shared_process_id)
    event_id = inserted["eventId"]
    message = inserted["message"]
    if inserted["severity"] != "low" or inserted["shownCount"] != 0:
        fail("Hub producer event before exposure", f"unexpected policy or premature exposure: {inserted}")

    mcp_list = hub_event_mcp_call(
        hub_port,
        hub_key,
        full_session,
        81,
        "event.list",
        {"agentId": normal_id, "status": "pending"},
        "Hub MCP event.list",
    )
    if event_id not in {item.get("eventId") for item in event_items(mcp_list, "Hub MCP event.list")}:
        fail("Hub MCP event.list", f"Agent inbox was not visible through Hub MCP: {mcp_list}")
    mcp_panel = assert_event_counts(mcp_list, (1, 0, 0), "Hub MCP shared inbox counts")
    if event_id not in event_panel_ids(mcp_panel, "Hub MCP shared inbox counts"):
        fail("Hub external injection exposure", f"the first public Hub MCP event call omitted the private-injected low event: {mcp_list}")

    http_list_response, http_list = hub_event_http_call(
        hub_port,
        hub_key,
        document,
        "GET",
        "/v1/events?" + urlencode({"agentId": normal_id, "status": "pending"}),
        None,
        "/v1/events",
        "get",
        "Hub HTTP event.list",
    )
    if event_id not in {item.get("eventId") for item in event_items(http_list, "Hub HTTP event.list")}:
        fail("Hub HTTP event.list", f"HTTP inbox did not share the Agent's event: {http_list}")
    if http_list.get("items") != mcp_list.get("items"):
        fail("Hub MCP/HTTP event.list parity", f"same target returned different event items: {mcp_list}, {http_list}")
    http_panel = assert_event_counts(http_list, (1, 0, 0), "Hub HTTP shared inbox counts")
    if event_id in event_panel_ids(http_panel, "Hub HTTP shared inbox counts"):
        fail("Hub external injection exposure", f"low event was exposed more than once: {http_list}")

    get_path = f"/v1/events/{event_id}?" + urlencode({"agentId": normal_id})
    _, http_get = hub_event_http_call(
        hub_port,
        hub_key,
        document,
        "GET",
        get_path,
        None,
        "/v1/events/{eventId}",
        "get",
        "Hub HTTP event.get",
    )
    mcp_get = hub_event_mcp_call(
        hub_port,
        hub_key,
        full_session,
        82,
        "event.get",
        {"agentId": normal_id, "eventId": event_id},
        "Hub MCP event.get",
    )
    for value, label in ((http_get, "Hub HTTP event.get"), (mcp_get, "Hub MCP event.get")):
        if value.get("message") != message or value.get("status") != "pending" or value.get("shownCount") != 1:
            fail(label, f"event.get lost the full pending record or one-exposure count: {value}")
        assert_event_source(value, "process", shared_process_id, label)
        panel = assert_event_counts(value, (1, 0, 0), label)
        if event_id in event_panel_ids(panel, label):
            fail(label, f"public event.get exposed a low event more than once: {value}")

    room_list = hub_event_mcp_call(
        hub_port,
        hub_key,
        full_session,
        83,
        "event.list",
        {"agentId": room_id, "status": "pending"},
        "Hub MCP isolated Agent event.list",
    )
    if event_items(room_list, "Hub MCP isolated Agent event.list"):
        fail("Hub Agent inbox isolation", f"another Agent's event leaked into the Room inbox: {room_list}")
    assert_event_counts(room_list, (0, 0, 0), "Hub Agent inbox isolation")

    mark_path = "/v1/events/mark"
    _, marked = hub_event_http_call(
        hub_port,
        hub_key,
        document,
        "POST",
        mark_path,
        {"agentId": normal_id, "eventIds": [event_id, "missing-parity-hub-event"]},
        mark_path,
        "post",
        "Hub HTTP event.mark",
    )
    if event_id not in marked.get("handledIds", []) or "missing-parity-hub-event" not in marked.get("notFoundIds", []):
        fail("Hub HTTP event.mark", f"handled and unknown ids were not separated: {marked}")
    repeated = hub_event_mcp_call(
        hub_port,
        hub_key,
        full_session,
        84,
        "event.mark",
        {"agentId": normal_id, "eventIds": [event_id]},
        "Hub MCP event.mark idempotent retry",
    )
    if "error" in repeated:
        fail("Hub MCP event.mark idempotence", f"marking an already handled event failed: {repeated}")
    _, handled = hub_event_http_call(
        hub_port,
        hub_key,
        document,
        "GET",
        "/v1/events?" + urlencode({"agentId": normal_id, "status": "handled"}),
        None,
        "/v1/events",
        "get",
        "Hub HTTP handled event.list",
    )
    if event_id not in {item.get("eventId") for item in event_items(handled, "Hub HTTP handled event.list")}:
        fail("Hub HTTP handled event.list", f"handled event history was lost: {handled}")
    handled_detail = hub_event_mcp_call(
        hub_port,
        hub_key,
        full_session,
        85,
        "event.get",
        {"agentId": normal_id, "eventId": event_id},
        "Hub MCP handled event.get",
    )
    if handled_detail.get("status") != "handled" or handled_detail.get("message") != message:
        fail("Hub shared mark state", f"handled state or original message differed across API: {handled_detail}")
    # Admit the slow low producer while the inbox is empty, before the medium
    # producer completes. No public call should consume its first exposure.
    low_process_id = start_event_process("low")
    medium_process_id = start_event_process("medium")
    medium_reminder = await_event(medium_process_id)
    medium_reminder_id = medium_reminder["eventId"]
    medium_message = medium_reminder["message"]
    if medium_reminder["severity"] != "medium" or medium_reminder["shownCount"] != 0:
        fail("Hub aggregate medium producer", f"unexpected policy or premature exposure: {medium_reminder}")

    for exposure, request_id in enumerate((86, 87), start=1):
        label = f"Hub medium reminder exposure {exposure} before aggregate"
        medium_exposure = hub_event_mcp_call(
            hub_port,
            hub_key,
            full_session,
            request_id,
            "event.list",
            {"agentId": normal_id, "status": "pending"},
            label,
        )
        medium_panel = assert_event_counts(medium_exposure, (0, 1, 0), label)
        if (
            event_panel_ids(medium_panel, label) != [medium_reminder_id]
            or event_panel_levels(medium_panel, label) != ["medium"]
        ):
            fail(label, f"medium reminder did not use both intended exposures: {medium_exposure}")

    low_reminder = await_event(low_process_id)
    low_reminder_id = low_reminder["eventId"]
    low_message = low_reminder["message"]
    if low_reminder["severity"] != "low" or low_reminder["shownCount"] != 0:
        fail("Hub aggregate low producer", f"unexpected policy or premature exposure: {low_reminder}")

    aggregate = hub_event_mcp_call(
        hub_port,
        hub_key,
        full_session,
        88,
        "mcp.listServers",
        {},
        "Hub no-target MCP aggregate",
    )
    aggregate_agents = aggregate.get("agents")
    if (
        "events" in aggregate
        or not isinstance(aggregate_agents, list)
        or any(isinstance(agent, dict) and "events" in agent for agent in aggregate_agents)
        or sum(
            isinstance(agent, dict) and agent.get("agentId") == normal_id
            for agent in aggregate_agents
        )
        != 1
    ):
        fail(
            "Hub no-target MCP aggregate",
            f"aggregate did not include the target Agent without event panels: {aggregate}",
        )

    targeted_discovery = hub_event_mcp_call(
        hub_port,
        hub_key,
        full_session,
        89,
        "mcp.listServers",
        {"agentId": normal_id},
        "Hub targeted MCP discovery after aggregate",
    )
    discovery_panel = assert_event_counts(
        targeted_discovery,
        (1, 1, 0),
        "Hub targeted MCP discovery after aggregate",
    )
    discovered_ids = event_panel_ids(
        discovery_panel, "Hub targeted MCP discovery after aggregate"
    )
    discovered_levels = event_panel_levels(
        discovery_panel, "Hub targeted MCP discovery after aggregate"
    )
    discovered_summaries = {}
    for entry in discovery_panel["new"]:
        key = next(iter(entry))
        discovered_summaries[key.split(" | ", 1)[0]] = key.split(" | ", 1)[1]
    if (
        discovered_ids != [medium_reminder_id, low_reminder_id]
        or discovered_levels != ["medium", "low"]
        or discovered_summaries
        != {
            medium_reminder_id: medium_message if len(medium_message) <= 32 else medium_message[:31] + "…",
            low_reminder_id: low_message if len(low_message) <= 32 else low_message[:31] + "…",
        }
    ):
        fail(
            "Hub targeted MCP discovery after aggregate",
            f"targeted discovery did not expose the same pending low/medium reminders: {targeted_discovery}",
        )
    seed_event_ids = [medium_reminder_id, low_reminder_id]
    seed_sources = {
        medium_reminder_id: (medium_process_id, "medium", medium_message),
        low_reminder_id: (low_process_id, "low", low_message),
    }
    for seed_event_id, (process_id, severity, expected_message) in seed_sources.items():
        source_evidence = event_evidence(process_id)
        if (
            source_evidence is None
            or source_evidence.get("eventId") != seed_event_id
            or source_evidence.get("severity") != severity
            or source_evidence.get("message") != expected_message
        ):
            fail("Hub aggregate seed identity", f"seed ID/type/source did not match its process producer: {source_evidence}")
        assert_event_source(source_evidence, "process", process_id, "Hub aggregate seed provenance")

    marked_seeds = hub_event_mcp_call(
        hub_port,
        hub_key,
        full_session,
        90,
        "event.mark",
        {"agentId": normal_id, "eventIds": seed_event_ids},
        "Hub aggregate seed cleanup",
    )
    if (
        set(marked_seeds.get("handledIds", [])) != set(seed_event_ids)
        or marked_seeds.get("notFoundIds") not in ([], None)
    ):
        fail("Hub aggregate seed cleanup", f"public event.mark did not handle exactly the known seed IDs: {marked_seeds}")
    for seed_event_id, (process_id, severity, expected_message) in seed_sources.items():
        seed_after_mark = event_evidence(process_id)
        if (
            seed_after_mark is None
            or seed_after_mark.get("eventId") != seed_event_id
            or seed_after_mark.get("severity") != severity
            or seed_after_mark.get("message") != expected_message
            or seed_after_mark.get("status") != "handled"
        ):
            fail("Hub aggregate seed cleanup", f"public mark did not settle the matching producer seed: {seed_after_mark}")
        assert_event_source(seed_after_mark, "process", process_id, "Hub aggregate seed cleanup provenance")
    seed_pending = hub_event_mcp_call(
        hub_port,
        hub_key,
        full_session,
        91,
        "event.list",
        {"agentId": normal_id, "status": "pending"},
        "Hub aggregate seeds cleared before inline",
    )
    if event_items(seed_pending, "Hub aggregate seeds cleared before inline"):
        fail("Hub aggregate seeds cleared before inline", f"pending inbox still contains events after exact seed marks: {seed_pending}")
    assert_event_counts(seed_pending, (0, 0, 0), "Hub aggregate seeds cleared before inline")
    reports.append("PASS targetless Hub mcp.listServers omits events without consuming low/medium exposures; targeted discovery exposes both reminders")
    reports.append("PASS Hub MCP/HTTP event list/get/mark share the target Agent inbox; no-target and cross-Agent isolation")
    reports.append("PASS Hub aggregate low/medium process seeds are publicly marked by exact IDs and cleared before inline")
    return event_id


def agent_http_tool_call(
    port: int,
    token: str,
    session: str,
    request_id: int,
    name: str,
    arguments: dict[str, Any],
    scenario: str,
    timeout: float | None = None,
) -> dict[str, Any]:
    return json_result(
        mcp_call(
            port,
            token,
            session,
            request_id,
            "tools/call",
            {"name": name, "arguments": arguments},
            scenario,
            timeout=timeout,
        ),
        scenario,
    )


def event_panel_levels(panel: dict[str, Any], scenario: str) -> list[str]:
    levels: list[str] = []
    for entry in panel["new"]:
        value = next(iter(entry.values()))
        level, separator, _timestamp = value.partition(" | ")
        if not separator:
            fail(scenario, f"event panel value omitted its level and date: {value!r}")
        levels.append(level)
    return levels


def assert_event_source(
    record: dict[str, Any],
    kind: str,
    reference: str,
    scenario: str,
) -> None:
    source = record.get("source")
    if not isinstance(source, dict) or source.get("kind") != kind or source.get("ref") != reference:
        fail(scenario, f"event provenance changed: {record}")


def event_record_for_source(
    items: list[dict[str, Any]],
    source_kind: str,
    reference: str,
    get_record: Any,
    scenario: str,
) -> dict[str, Any] | None:
    for item in items:
        event_id = item.get("eventId")
        if not isinstance(event_id, str):
            fail(scenario, f"event list item omitted eventId: {item}")
        record = get_record(event_id)
        source = record.get("source", {})
        if source.get("kind") == source_kind and source.get("ref") == reference:
            return record
    return None


def read_transport_ledger_record(path: Path, run_id: str) -> dict[str, Any] | None:
    latest = None
    try:
        with path.open("r", encoding="utf-8") as ledger:
            for line in ledger:
                try:
                    record = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if isinstance(record, dict) and record.get("runId") == run_id:
                    latest = record
    except OSError:
        return None
    return latest


def read_internal_source_row(
    path: Path, run_id: str, request_id: str, command_hash: str
) -> dict[str, Any] | None:
    connection = None
    try:
        connection = sqlite3.connect(f"{path.as_uri()}?mode=ro", uri=True, timeout=0.05)
        row = connection.execute(
            """
            SELECT source_kind,source_ref,response_state,completion_seen,
                   completion_event_type,completion_message,completed_at,
                   origin_run_id,origin_request_id,origin_command_hash
            FROM internal_sources
            WHERE origin_run_id=?1 AND origin_request_id=?2 AND origin_command_hash=?3
            """,
            (run_id, request_id, command_hash),
        ).fetchone()
    except (OSError, sqlite3.Error):
        return None
    finally:
        if connection is not None:
            connection.close()
    if row is None:
        return None
    return {
        "sourceKind": row[0],
        "sourceRef": row[1],
        "responseState": row[2],
        "completionSeen": row[3],
        "completionEventType": row[4],
        "completionMessage": row[5],
        "completedAt": row[6],
        "originRunId": row[7],
        "originRequestId": row[8],
        "originCommandHash": row[9],
    }


def read_internal_source_rows(
    path: Path, run_id: str, request_id: str, command_hash: str
) -> list[dict[str, Any]]:
    connection = None
    try:
        connection = sqlite3.connect(f"{path.as_uri()}?mode=ro", uri=True, timeout=0.05)
        rows = connection.execute(
            """
            SELECT source_kind,source_ref,response_state,completion_seen,
                   completion_event_type,completion_message,completed_at,
                   origin_run_id,origin_request_id,origin_command_hash
            FROM internal_sources
            WHERE origin_run_id=?1 AND origin_request_id=?2 AND origin_command_hash=?3
            ORDER BY source_kind,source_ref
            """,
            (run_id, request_id, command_hash),
        ).fetchall()
    except (OSError, sqlite3.Error):
        return []
    finally:
        if connection is not None:
            connection.close()
    return [
        {
            "sourceKind": row[0],
            "sourceRef": row[1],
            "responseState": row[2],
            "completionSeen": row[3],
            "completionEventType": row[4],
            "completionMessage": row[5],
            "completedAt": row[6],
            "originRunId": row[7],
            "originRequestId": row[8],
            "originCommandHash": row[9],
        }
        for row in rows
    ]


def read_hub_feedback_deltas(path: Path, run_id: str) -> list[dict[str, Any]]:
    connection = None
    try:
        connection = sqlite3.connect(f"{path.as_uri()}?mode=ro", uri=True, timeout=0.05)
        rows = connection.execute(
            """
            SELECT source_kind,source_ref,request_id,payload_json,acked_at
            FROM event_response_feedback_delta WHERE run_id=?1
            ORDER BY source_kind,source_ref
            """,
            (run_id,),
        ).fetchall()
    except (OSError, sqlite3.Error):
        return []
    finally:
        if connection is not None:
            connection.close()
    return [
        {
            "sourceKind": row[0],
            "sourceRef": row[1],
            "requestId": row[2],
            "payload": row[3],
            "ackedAt": row[4],
        }
        for row in rows
    ]

def read_hub_feedback_row(path: Path, run_id: str) -> dict[str, Any] | None:
    connection = None
    try:
        connection = sqlite3.connect(f"{path.as_uri()}?mode=ro", uri=True, timeout=0.05)
        row = connection.execute(
            """
            SELECT decision,event_sources_json,sources_json,feedback_request_id,
                   feedback_payload_json,acked_at
            FROM event_response_feedback WHERE run_id=?1
            """,
            (run_id,),
        ).fetchone()
    except (OSError, sqlite3.Error):
        return None
    finally:
        if connection is not None:
            connection.close()
    if row is None:
        return None
    return {
        "decision": row[0],
        "eventSources": row[1],
        "sources": row[2],
        "feedbackRequestId": row[3],
        "feedbackPayload": row[4],
        "ackedAt": row[5],
    }

def read_hub_run_record(path: Path, run_id: str) -> dict[str, Any] | None:
    connection = None
    try:
        connection = sqlite3.connect(f"{path.as_uri()}?mode=ro", uri=True, timeout=0.05)
        row = connection.execute(
            """
            SELECT request_id,agent_id,command_type,command_hash,status,acked_at,result_json
            FROM agent_runs WHERE run_id=?1
            """,
            (run_id,),
        ).fetchone()
    except (OSError, sqlite3.Error):
        return None
    finally:
        if connection is not None:
            connection.close()
    if row is None:
        return None
    try:
        result = json.loads(row[6]) if row[6] is not None else None
    except json.JSONDecodeError:
        return None
    return {
        "requestId": row[0],
        "agentId": row[1],
        "commandType": row[2],
        "commandHash": row[3],
        "status": row[4],
        "ackedAt": row[5],
        "result": result,
    }


def run_hub_late_response_gate(
    agent_binary: Path,
    hub_binary: Path,
    hub_process: ManagedProcess,
    hub_port: int,
    hub_key: str,
    hub_root: Path,
    hub_config: Path,
    hub_env: dict[str, str],
    normal_id: str,
    normal_process: ManagedProcess,
    room_id: str,
    room_process: ManagedProcess,
    document: dict[str, Any],
    reports: list[str],
) -> tuple[ManagedProcess, ManagedProcess, WebSocketRelay]:
    agent_id = "parity-timeout"
    secret = "parity-timeout-secret"
    register_hub_agent(
        hub_binary, hub_root / "hub.db", hub_config, hub_env, agent_id, "Parity timeout", secret
    )
    relay = WebSocketRelay(hub_port)
    agent_root = hub_root.parent / "hub-timeout"
    agent, config, env = start_hub_agent(
        agent_binary,
        agent_root,
        "normal",
        agent_id,
        f"http://127.0.0.1:{relay.port}",
        secret,
        reports,
    )
    configure_process_fixture(config)
    agent_workspace = Path(json.loads(config.read_text())["workspaceRoot"])
    wait_for_agent(hub_port, hub_key, agent_id, "Hub delayed-response Agent connection", agent)
    relay.arm_late_response("parity-late-event")
    relay.arm_receipt_hold()
    response, value = hub_json(
        hub_port,
        hub_key,
        "POST",
        "/v1/process/exec",
        {
            "agentId": agent_id,
            "command": shlex.join(["/usr/bin/sleep", "1"]),
            "cwd": str(agent_workspace),
            "group": "parity-late-event",
            "needConfirm": False,
            "waitSeconds": 30,
        },
        "Hub real delayed process.exec",
        timeout=45,
    )
    validate_operation_response(
        document, "/v1/process/exec", "post", 504, value, "Hub real delayed process.exec"
    )
    if response.status != 504 or not isinstance(value, dict) or value.get("error", {}).get("code") != "process_exec_timeout":
        fail("Hub real delayed process.exec", f"expected the actual 35-second Hub timeout: {response.status}, {value}")
    if not relay.response_seen.is_set() or not isinstance(relay.late_response, dict):
        fail("Hub real delayed response", "relay did not intercept the matching actual Agent Response frame")
    late_response = relay.late_response
    process_envelope = relay.wait_for_process_command(
        "parity-late-event", 5, "Hub delayed-response process.exec envelope"
    )
    wire_command = process_envelope.get("command", {})
    wire_payload = wire_command.get("payload", {}) if isinstance(wire_command, dict) else {}
    expected_command = shlex.join(["/usr/bin/sleep", "1"])
    if (
        wire_payload.get("command") != expected_command
        or wire_payload.get("cwd") != str(agent_workspace)
        or "program" in wire_payload
        or "args" in wire_payload
    ):
        fail("Hub HTTP-to-wire process contract", f"Agent wire request did not preserve command/cwd: {wire_payload}")
    process_origin = {
        "runId": process_envelope.get("runId"),
        "requestId": process_envelope.get("requestId"),
        "commandHash": process_envelope.get("commandHash"),
    }
    if (
        not all(isinstance(value, str) and value for value in process_origin.values())
        or late_response.get("runId") != process_origin["runId"]
        or late_response.get("requestId") != process_origin["requestId"]
    ):
        fail(
            "Hub delayed-response origin",
            f"late Agent response did not match its actual process.exec envelope: "
            f"{process_origin}, {late_response}",
        )
    result = late_response.get("data")
    dispositions = late_response.get("eventSources")
    if not isinstance(result, dict) or result.get("state") != "completed":
        fail("Hub real delayed terminal response", f"Agent response was not a completed process response: {late_response}")
    process_id = result.get("processId")
    if (
        not isinstance(process_id, str)
        or not isinstance(dispositions, list)
        or len(dispositions) != 1
        or dispositions[0].get("source", {}).get("kind") != "process"
        or dispositions[0].get("source", {}).get("ref") != process_id
        or dispositions[0].get("includesTerminal") is not True
    ):
        fail("Hub late terminal event metadata", f"Agent response omitted its bound terminal source: {late_response}")
    relay.release_late_response()
    relay.wait_for_settle(1, 10, "Hub late terminal EventSettle")
    relay.wait_receipt_held(10, "Hub late terminal EventSettle")
    relay.wait_settle_reply_held(10, "Hub late terminal EventSettle")

    hub_process.stop()
    relay.wait_disconnected(5, "Hub restart with held EventSettle receipt")
    relay.arm_receipt_hold()
    hub_process = restart_hub_process(
        hub_binary,
        hub_root,
        hub_port,
        hub_config,
        hub_env,
        hub_key,
        "Hub late-response durable restart",
    )
    wait_for_agent(hub_port, hub_key, normal_id, "Hub normal Agent after restart", normal_process)
    wait_for_agent(hub_port, hub_key, room_id, "Hub Room Agent after restart", room_process)
    wait_for_agent(hub_port, hub_key, agent_id, "Hub delayed-response Agent after restart", agent)
    relay.wait_for_settle(2, 10, "Hub late terminal EventSettle replay")
    relay.wait_receipt_held(10, "Hub late terminal EventSettle replay")
    relay.wait_settle_reply_held(10, "Hub late terminal EventSettle replay")
    settle_envelopes = relay.settle_envelopes()
    if len(settle_envelopes) != 2:
        fail(
            "Hub delayed-response held receipt checkpoint",
            f"expected initial and Hub-restart EventSettle attempts before Agent restart: "
            f"{len(settle_envelopes)}",
        )
    ledger_path = Path(env["HOME"]) / ".agentic_gpt" / "transport-runs.jsonl"
    hub_db_path = hub_root / "hub.db"
    settle_identities: list[tuple[str, str, str]] = []
    for index, envelope in enumerate(settle_envelopes, start=1):
        command = envelope.get("command")
        payload = command.get("payload") if isinstance(command, dict) else None
        run_id = envelope.get("runId")
        request_id = envelope.get("requestId")
        command_hash = envelope.get("commandHash")
        if (
            not isinstance(command, dict)
            or command.get("type") != "event.settle"
            or command.get("requestId") != request_id
            or not isinstance(payload, dict)
            or not all(isinstance(value, str) and value for value in (run_id, request_id, command_hash))
        ):
            fail(
                "Hub delayed-response held receipt checkpoint",
                f"EventSettle attempt {index} omitted its durable command identity: {envelope}",
            )
        ledger_record = read_transport_ledger_record(ledger_path, run_id)
        hub_record = read_hub_run_record(hub_db_path, run_id)
        if (
            ledger_record is None
            or ledger_record.get("runId") != run_id
            or ledger_record.get("agentId") != agent_id
            or ledger_record.get("requestId") != request_id
            or ledger_record.get("commandHash") != command_hash
            or ledger_record.get("status") != "completed"
            or ledger_record.get("result") != {"status": "settled"}
            or hub_record is None
            or hub_record.get("requestId") != request_id
            or hub_record.get("agentId") != agent_id
            or hub_record.get("commandType") != "event.settle"
            or hub_record.get("commandHash") != command_hash
            or hub_record.get("result") is not None
            or hub_record.get("ackedAt") is not None
        ):
            fail(
                "Hub delayed-response held receipt checkpoint",
                f"EventSettle attempt {index} was not durably complete at Agent while its "
                f"Hub result/TransportAck remained outstanding: Agent={ledger_record}, Hub={hub_record}",
            )
        settle_identities.append((run_id, request_id, command_hash))

    second_settle_envelope = settle_envelopes[1]
    second_settle_command = second_settle_envelope["command"]
    second_settle_payload = second_settle_command.get("payload")
    second_settle_receipt = tuple(
        str(second_settle_envelope.get(key, ""))
        for key in ("eventId", "runId", "requestId", "commandHash")
    )
    feedback_before_restart = read_hub_feedback_row(hub_db_path, process_origin["runId"])
    if feedback_before_restart is None or not isinstance(feedback_before_restart.get("feedbackPayload"), str):
        fail(
            "Hub delayed-response feedback checkpoint",
            f"the timed-out process origin had no durable EventSettle outbox row: {feedback_before_restart}",
        )
    try:
        feedback_payload_before_restart = json.loads(feedback_before_restart["feedbackPayload"])
    except json.JSONDecodeError as error:
        fail(
            "Hub delayed-response feedback checkpoint",
            f"persisted EventSettle payload was invalid JSON: {error}",
        )
    if (
        feedback_before_restart.get("decision") != "no_terminal"
        or feedback_before_restart.get("feedbackRequestId") != second_settle_command.get("requestId")
        or feedback_before_restart.get("ackedAt") is not None
        or feedback_payload_before_restart != second_settle_payload
        or feedback_payload_before_restart.get("origin") != process_origin
        or feedback_payload_before_restart.get("dispositions")
        != [{"source": {"kind": "process", "ref": process_id}, "includesTerminal": False}]
        or not all(second_settle_receipt)
        or len(relay.receipt_holds()) != 2
        or relay.receipt_holds()[-1] != second_settle_receipt
    ):
        fail(
            "Hub delayed-response feedback checkpoint",
            f"held EventSettle receipt did not preserve the exact unacked origin/payload: "
            f"feedback={feedback_before_restart}, receipt={relay.receipt_holds()}",
        )
    settle_response_baseline = relay.settle_response_count()

    agent.stop()
    relay.wait_disconnected(5, "Agent restart with held EventSettle receipt")
    relay.disarm_receipt_hold()
    agent = ManagedProcess(
        [str(agent_binary), "run", "--config", str(config)],
        env,
        "Hub delayed-response Agent restart",
    )
    wait_for_agent(
        hub_port,
        hub_key,
        agent_id,
        "Hub delayed-response Agent after restart",
        agent,
    )
    first_settle_run_id, first_settle_request_id, _ = settle_identities[0]
    second_settle_run_id, second_settle_request_id, _ = settle_identities[1]
    # Agent reconnect replays completed ledger responses before any Hub resend is guaranteed.
    replayed_first_settle = relay.wait_for_settle_response(
        first_settle_run_id,
        first_settle_request_id,
        settle_response_baseline,
        10,
        "Hub delayed-response Agent ledger replay",
    )
    replayed_second_settle = relay.wait_for_settle_response(
        second_settle_run_id,
        second_settle_request_id,
        settle_response_baseline,
        10,
        "Hub delayed-response Agent ledger replay",
    )
    if (
        replayed_first_settle.get("data", {}).get("status") != "settled"
        or replayed_second_settle.get("data", {}).get("status") != "settled"
    ):
        fail(
            "Hub delayed-response Agent ledger replay",
            f"Agent restart did not replay both completed EventSettle results: "
            f"{replayed_first_settle}, {replayed_second_settle}",
        )

    def settle_results_persisted() -> bool:
        for run_id, request_id, command_hash in settle_identities:
            record = read_hub_run_record(hub_db_path, run_id)
            if (
                record is None
                or record.get("requestId") != request_id
                or record.get("agentId") != agent_id
                or record.get("commandType") != "event.settle"
                or record.get("commandHash") != command_hash
                or record.get("status") != "completed"
                or record.get("result") != {"status": "settled"}
            ):
                return False
        feedback = read_hub_feedback_row(hub_db_path, process_origin["runId"])
        return (
            feedback is not None
            and feedback.get("decision") == "no_terminal"
            and feedback.get("feedbackRequestId") == second_settle_command.get("requestId")
            and feedback.get("feedbackPayload") == feedback_before_restart.get("feedbackPayload")
            and feedback.get("ackedAt") is not None
        )

    wait_until(
        settle_results_persisted,
        "Hub delayed-response Agent ledger replay",
        "Agent restart response replay to persist both Hub run results and acknowledge the exact feedback outbox",
        timeout=10,
        process=agent,
    )
    feedback_after_restart = read_hub_feedback_row(hub_db_path, process_origin["runId"])
    if (
        feedback_after_restart is None
        or feedback_after_restart.get("feedbackPayload") != feedback_before_restart.get("feedbackPayload")
        or feedback_after_restart.get("ackedAt") is None
        or read_hub_feedback_deltas(hub_db_path, process_origin["runId"])
    ):
        fail(
            "Hub delayed-response feedback replay",
            f"Agent ledger replay changed or duplicated the acknowledged EventSettle outbox: "
            f"{feedback_after_restart}",
        )

    session = open_mcp_session(hub_port, hub_key, "Hub delayed-response event APIs")
    list_path = "/v1/events?" + urlencode({"agentId": agent_id, "status": "pending"})
    list_response, first_list = hub_event_http_call(
        hub_port,
        hub_key,
        document,
        "GET",
        list_path,
        None,
        "/v1/events",
        "get",
        "Hub late response first event.list",
    )
    if list_response.status != 200:
        fail("Hub late response first event.list", f"HTTP {list_response.status}: {first_list}")
    first_items = event_items(first_list, "Hub late response first event.list")
    if len(first_items) != 1:
        fail("Hub late response single event", f"expected exactly one pending completion event: {first_list}")
    event_id = first_items[0].get("eventId")
    first_panel = assert_event_counts(first_list, (1, 0, 0), "Hub late response first event.list")
    if (
        not isinstance(event_id, str)
        or event_panel_ids(first_panel, "Hub late response first event.list") != [event_id]
        or event_panel_levels(first_panel, "Hub late response first event.list") != ["low"]
    ):
        fail("Hub EventSettle exposure isolation", f"the first public panel did not expose the single event once: {first_list}")
    detail = hub_event_mcp_call(
        hub_port,
        hub_key,
        session,
        2,
        "event.get",
        {"agentId": agent_id, "eventId": event_id},
        "Hub late response event.get",
    )
    if (
        detail.get("status") != "pending"
        or detail.get("severity") != "low"
        or detail.get("shownCount") != 1
        or detail.get("message") is None
        or process_id not in detail.get("message", "")
        or "completed" not in detail.get("message", "")
    ):
        fail("Hub late response event detail", f"EventSettle did not create one pending process completion event: {detail}")
    assert_event_source(detail, "process", process_id, "Hub late response event provenance")
    detail_panel = assert_event_counts(detail, (1, 0, 0), "Hub late response event.get")
    if event_id in event_panel_ids(detail_panel, "Hub late response event.get"):
        fail("Hub EventSettle exposure isolation", f"event.get exposed the low event a second time: {detail}")
    second_list = hub_event_mcp_call(
        hub_port,
        hub_key,
        session,
        3,
        "event.list",
        {"agentId": agent_id, "status": "pending"},
        "Hub late response second event.list",
    )
    if [item.get("eventId") for item in event_items(second_list, "Hub late response second event.list")] != [event_id]:
        fail("Hub late response single event", f"repeated query duplicated or lost the event: {second_list}")
    second_panel = assert_event_counts(second_list, (1, 0, 0), "Hub late response second event.list")
    if event_id in event_panel_ids(second_panel, "Hub late response second event.list"):
        fail("Hub EventSettle exposure isolation", f"low event remained visible after its one panel exposure: {second_list}")

    terminal = process_request(
        hub_port,
        hub_key,
        agent_id,
        "/usr/bin/true",
        "parity-terminal-inline",
        5,
        "Hub normal terminal response suppression",
    )
    terminal_id = terminal.get("processId")
    if terminal.get("state") != "completed" or not terminal_id:
        fail("Hub normal terminal response suppression", f"real terminal process did not finish: {terminal}")
    terminal_panel = assert_event_counts(terminal, (1, 0, 0), "Hub normal terminal response suppression")
    if event_id in event_panel_ids(terminal_panel, "Hub normal terminal response suppression"):
        fail("Hub normal terminal response suppression", f"terminal operation changed pending-event visibility: {terminal}")
    final_list_response, final_list = hub_event_http_call(
        hub_port,
        hub_key,
        document,
        "GET",
        list_path,
        None,
        "/v1/events",
        "get",
        "Hub terminal suppression pending query",
    )
    final_items = event_items(final_list, "Hub terminal suppression pending query")
    if final_list_response.status != 200 or [item.get("eventId") for item in final_items] != [event_id]:
        fail("Hub normal terminal response suppression", f"terminal response added or removed a pending event: {final_list}")
    terminal_event = event_record_for_source(
        final_items,
        "process",
        str(terminal_id),
        lambda lookup_id: hub_event_mcp_call(
            hub_port,
            hub_key,
            session,
            4,
            "event.get",
            {"agentId": agent_id, "eventId": lookup_id},
            "Hub terminal suppression event lookup",
        ),
        "Hub terminal suppression pending query",
    )
    if terminal_event is not None:
        fail("Hub normal terminal response suppression", f"original terminal response created a completion event: {terminal_event}")
    reports.append("PASS Hub late terminal response, Hub/Agent ledger replay, EventSettle panel isolation, and normal terminal suppression")
    return hub_process, agent, relay


def run_hub_feedback_delta_recovery_gate(
    agent_binary: Path,
    hub_binary: Path,
    hub_process: ManagedProcess,
    hub_port: int,
    hub_key: str,
    hub_root: Path,
    hub_config: Path,
    hub_env: dict[str, str],
    document: dict[str, Any],
    reports: list[str],
) -> tuple[ManagedProcess, ManagedProcess, WebSocketRelay]:
    scenario = "Hub supplemental batch feedback recovery"
    agent_id = "parity-feedback-delta"
    secret = "parity-feedback-delta-secret"
    group = "parity-feedback-delta-batch"
    register_hub_agent(
        hub_binary, hub_root / "hub.db", hub_config, hub_env, agent_id, "Parity feedback delta", secret
    )
    relay = WebSocketRelay(hub_port)
    agent_root = hub_root.parent / "hub-feedback-delta"
    agent, config, env = start_hub_agent(
        agent_binary, agent_root, "normal", agent_id,
        f"http://127.0.0.1:{relay.port}", secret, reports,
    )
    configure_process_fixture(config)
    wait_for_agent(hub_port, hub_key, agent_id, f"{scenario} Agent connection", agent)
    relay.arm_late_response(group, command_type="process.batch")
    request_outcome: dict[str, Any] = {}
    request_started = threading.Event()

    def issue_batch() -> None:
        request_started.set()
        try:
            request_outcome["result"] = hub_json(
                hub_port, hub_key, "POST", "/v1/process/batch",
                {
                    "agentId": agent_id,
                    "group": group,
                    "elements": [
                        {"command": shlex.join(["/usr/bin/printf", "feedback-child-A"])},
                        {"command": shlex.join(["/usr/bin/printf", "feedback-child-B"])},
                    ],
                    "needConfirm": False,
                    "waitSeconds": 30,
                },
                f"{scenario} delayed actual process.batch",
                timeout=45,
            )
        except BaseException as error:
            request_outcome["error"] = error

    request_thread = threading.Thread(target=issue_batch, name="parity-feedback-delta-batch", daemon=True)
    request_thread.start()
    if not request_started.wait(5):
        fail(scenario, "actual process.batch request did not start")
    envelope = relay.wait_for_process_command(group, 15, scenario, command_type="process.batch")
    origin = {
        "runId": envelope.get("runId"),
        "requestId": envelope.get("requestId"),
        "commandHash": envelope.get("commandHash"),
    }
    if not all(isinstance(value, str) and value for value in origin.values()):
        fail(scenario, f"actual process.batch envelope omitted its durable origin: {envelope}")
    event_db_path = Path(env["HOME"]) / ".agentic_gpt" / "state" / "agent" / agent_id / "events.sqlite3"

    def terminal_sources_awaiting() -> bool:
        rows = read_internal_source_rows(event_db_path, origin["runId"], origin["requestId"], origin["commandHash"])
        return len(rows) == 2 and all(
            row.get("sourceKind") == "process"
            and row.get("responseState") == "awaiting_response"
            and row.get("completionSeen") == 1
            and row.get("completionEventType") == "process.completed"
            and isinstance(row.get("completionMessage"), str)
            and row.get("completedAt")
            and row.get("originRunId") == origin["runId"]
            and row.get("originRequestId") == origin["requestId"]
            and row.get("originCommandHash") == origin["commandHash"]
            for row in rows
        )

    wait_until(
        terminal_sources_awaiting, scenario,
        "two actual process.batch completion sources to remain awaiting their original response",
        timeout=20, process=agent,
    )
    wait_until(
        lambda: relay.response_seen.is_set(), scenario,
        "relay to intercept the real completed process.batch Agent response",
        timeout=10, process=agent,
    )
    late_response = relay.late_response
    actual_rows = read_internal_source_rows(event_db_path, origin["runId"], origin["requestId"], origin["commandHash"])
    batch_result = late_response.get("data") if isinstance(late_response, dict) else None
    reported_processes = batch_result.get("processes") if isinstance(batch_result, dict) else None
    actual_dispositions = late_response.get("eventSources") if isinstance(late_response, dict) else None
    expected_refs = {row["sourceRef"] for row in actual_rows}
    returned_refs = {
        item.get("source", {}).get("ref")
        for item in actual_dispositions
        if isinstance(item, dict) and item.get("source", {}).get("kind") == "process"
    } if isinstance(actual_dispositions, list) else set()
    reported_process_ids = {
        item.get("processId") for item in reported_processes if isinstance(item, dict)
    } if isinstance(reported_processes, list) else set()
    if (
        not isinstance(batch_result, dict)
        or batch_result.get("status") != "completed"
        or not isinstance(reported_processes, list)
        or len(reported_processes) != 2
        or any(item.get("state") != "completed" for item in reported_processes if isinstance(item, dict))
        or reported_process_ids != expected_refs
        or len(expected_refs) != 2
        or not isinstance(actual_dispositions, list)
        or len(actual_dispositions) != 2
        or returned_refs != expected_refs
        or any(item.get("includesTerminal") is not True for item in actual_dispositions)
        or not all(row["sourceRef"] in row["completionMessage"] and "completed" in row["completionMessage"].lower()
                   for row in actual_rows)
    ):
        fail(scenario, f"actual Agent SQLite terminal children and delayed response differed: {actual_rows}, {late_response}")
    request_thread.join(45)
    if request_thread.is_alive():
        fail(scenario, "the original process.batch Hub call did not reach its timeout")
    if "error" in request_outcome:
        fail(scenario, f"original process.batch HTTP call failed: {request_outcome['error']}")
    result = request_outcome.get("result")
    if (not isinstance(result, tuple) or len(result) != 2 or result[0].status != 504
            or result[1].get("error", {}).get("code") != "process_batch_timeout"):
        fail(scenario, f"expected original Hub process.batch NoTerminal timeout: {result}")
    if not terminal_sources_awaiting():
        fail(scenario, "timed-out Agent child sources were not both still awaiting settlement")

    # Temporary recovery checkpoint, not a live partial-admission race reproduction.
    hub_process.stop()
    relay.wait_disconnected(5, f"{scenario} Hub checkpoint")
    sources = sorted(
        [{"kind": "process", "ref": row["sourceRef"]} for row in actual_rows],
        key=lambda source: source["ref"],
    )
    source_a, source_b = sources
    actual_by_ref = {row["sourceRef"]: row for row in actual_rows}
    primary_payload = {
        "origin": origin,
        "dispositions": [{"source": source_a, "includesTerminal": False}],
    }
    primary_payload_json = json.dumps(primary_payload, separators=(",", ":"))
    primary_request_id = "fixture-primary-" + origin["runId"]
    now = datetime.now(timezone.utc).isoformat()
    with sqlite3.connect(hub_root / "hub.db", timeout=5) as connection:
        updated = connection.execute(
            """UPDATE event_response_feedback SET
                event_sources_json=?,sources_json=?,feedback_request_id=?,
                feedback_payload_json=?,acked_at=NULL,updated_at=?
            WHERE run_id=? AND request_id=? AND agent_id=? AND command_hash=?
              AND command_type='process.batch' AND decision='no_terminal'""",
            (
                json.dumps(actual_dispositions, separators=(",", ":")),
                json.dumps(sources, separators=(",", ":")),
                primary_request_id,
                primary_payload_json,
                now,
                origin["runId"],
                origin["requestId"],
                agent_id,
                origin["commandHash"],
            ),
        )
        if updated.rowcount != 1:
            fail(scenario, f"actual timed-out Hub NoTerminal row was not available for the recovery fixture: {updated.rowcount}")
    seeded = read_hub_feedback_row(hub_root / "hub.db", origin["runId"])
    if (seeded is None or seeded.get("feedbackRequestId") != primary_request_id
            or seeded.get("feedbackPayload") != primary_payload_json or seeded.get("ackedAt") is not None):
        fail(scenario, f"temporary recovery checkpoint changed the primary-A request/payload: {seeded}")

    hub_process = restart_hub_process(
        hub_binary, hub_root, hub_port, hub_config, hub_env, hub_key, f"{scenario} Hub restart"
    )
    wait_for_agent(hub_port, hub_key, agent_id, f"{scenario} Agent reconnect", agent)
    session = open_mcp_session(hub_port, hub_key, f"{scenario} public event APIs")
    pending_path = "/v1/events?" + urlencode({"agentId": agent_id, "status": "pending"})
    list_response, listed = hub_event_http_call(
        hub_port, hub_key, document, "GET", pending_path, None,
        "/v1/events", "get", f"{scenario} public event.list",
    )
    items = event_items(listed, f"{scenario} public event.list")
    if list_response.status != 200 or len(items) != 2:
        fail(scenario, f"public event.list did not settle both actual child sources: {listed}")
    panel = assert_event_counts(listed, (2, 0, 0), f"{scenario} first panel")
    event_ids = [item.get("eventId") for item in items]
    panel_ids = event_panel_ids(panel, f"{scenario} first panel")
    if (any(not isinstance(event_id, str) for event_id in event_ids)
            or len(set(event_ids)) != 2 or set(panel_ids) != set(event_ids) or len(panel_ids) != 2):
        fail(scenario, f"first panel did not expose each source once: {listed}")
    item_by_ref: dict[str, dict[str, Any]] = {}
    for index, event_id in enumerate(event_ids, start=1):
        detail = hub_event_mcp_call(
            hub_port, hub_key, session, index, "event.get",
            {"agentId": agent_id, "eventId": event_id}, f"{scenario} event.get {event_id}",
        )
        source = detail.get("source", {})
        source_ref = source.get("ref")
        if (source.get("kind") != "process" or source_ref not in actual_by_ref
                or source_ref in item_by_ref):
            fail(scenario, f"event.get had missing/duplicate child provenance: {detail}")
        if (detail.get("status") != "pending" or detail.get("severity") != "low"
                or detail.get("shownCount") != 1
                or detail.get("message") != actual_by_ref[source_ref]["completionMessage"]):
            fail(scenario, f"event.get did not preserve one actual completed source: {detail}")
        assert_event_source(detail, "process", source_ref, f"{scenario} event provenance")
        detail_panel = assert_event_counts(detail, (2, 0, 0), f"{scenario} detail panel")
        if event_id in event_panel_ids(detail_panel, f"{scenario} detail panel"):
            fail(scenario, f"event.get exposed its event again: {detail}")
        item_by_ref[source_ref] = detail
    if set(item_by_ref) != set(actual_by_ref):
        fail(scenario, f"event.get omitted a real batch child: {item_by_ref}")

    primary_after = read_hub_feedback_row(hub_root / "hub.db", origin["runId"])
    deltas = read_hub_feedback_deltas(hub_root / "hub.db", origin["runId"])
    if (primary_after is None or primary_after.get("feedbackRequestId") != primary_request_id
            or primary_after.get("feedbackPayload") != primary_payload_json
            or primary_after.get("ackedAt") is None or len(deltas) != 1
            or deltas[0].get("sourceRef") != source_b["ref"] or deltas[0].get("ackedAt") is None):
        fail(scenario, f"primary/delta were not separately acknowledged: {primary_after}, {deltas}")
    delta_payload = json.loads(deltas[0]["payload"])
    if (delta_payload.get("origin") != origin
            or delta_payload.get("dispositions") != [{"source": source_b, "includesTerminal": False}]
            or not deltas[0].get("requestId")):
        fail(scenario, f"supplemental B feedback had the wrong actual identity: {deltas}")
    delta_snapshot = deltas[0].copy()
    settle_count = relay.settle_count
    replay_response, replayed = hub_event_http_call(
        hub_port, hub_key, document, "GET", pending_path, None,
        "/v1/events", "get", f"{scenario} acknowledged replay",
    )
    replay_items = event_items(replayed, f"{scenario} acknowledged replay")
    replay_panel_ids = event_panel_ids(event_panel(replayed, f"{scenario} replay panel"), f"{scenario} replay panel")
    feedback_replay = read_hub_feedback_row(hub_root / "hub.db", origin["runId"])
    if (replay_response.status != 200 or len(replay_items) != 2
            or {item.get("eventId") for item in replay_items} != set(event_ids)
            or replay_panel_ids or relay.settle_count != settle_count
            or feedback_replay is None
            or feedback_replay.get("feedbackRequestId") != primary_request_id
            or feedback_replay.get("feedbackPayload") != primary_payload_json):
        fail(scenario, f"acked replay changed feedback or duplicated public events: {replayed}")
    command_types = relay.command_types()
    event_list_index = next((index for index, kind in enumerate(command_types) if kind == "event.list"), None)
    settle_indexes = [index for index, kind in enumerate(command_types) if kind == "event.settle"]
    if event_list_index is None or len(settle_indexes) < 2 or settle_indexes[1] >= event_list_index:
        fail(scenario, f"public event.list crossed supplemental B settlement: {command_types}")
    reports.append(
        "PASS actual process.batch supplemental B recovery fixture, primary/delta acknowledgements, "
        "and exactly-once public child events"
    )
    reports.append(
        "LIMITATION supplemental recovery uses a temporary Hub DB checkpoint seeded with real "
        "run/origin/source/message evidence; live partial-A admission followed by reconnect and source-B "
        "discovery remains [INFERENCE], not a reproduced interleaving"
    )
    return hub_process, agent, relay


def run_hub_agent_crash_recovery_gate(
    agent_binary: Path,
    hub_binary: Path,
    hub_port: int,
    hub_key: str,
    hub_root: Path,
    hub_config: Path,
    hub_env: dict[str, str],
    document: dict[str, Any],
    reports: list[str],
) -> tuple[ManagedProcess, WebSocketRelay]:
    agent_id = "parity-crash"
    secret = "parity-crash-secret"
    register_hub_agent(
        hub_binary, hub_root / "hub.db", hub_config, hub_env, agent_id, "Parity crash", secret
    )
    relay = WebSocketRelay(hub_port)
    agent_root = hub_root.parent / "hub-crash"
    agent, config, env = start_hub_agent(
        agent_binary,
        agent_root,
        "normal",
        agent_id,
        f"http://127.0.0.1:{relay.port}",
        secret,
        reports,
    )
    configure_process_fixture(config)
    wait_for_agent(hub_port, hub_key, agent_id, "Hub crash-recovery Agent connection", agent)

    group = "parity-bound-source-crash"
    request_outcome: dict[str, Any] = {}
    request_started = threading.Event()

    def issue_process_request() -> None:
        request_started.set()
        try:
            request_outcome["response"], request_outcome["value"] = hub_json(
                hub_port,
                hub_key,
                "POST",
                "/v1/process/exec",
                {
                    "agentId": agent_id,
                    "command": shlex.join(["/usr/bin/sleep", "1"]),
                    "group": group,
                    "needConfirm": False,
                    "waitSeconds": 30,
                },
                "Hub Agent crash after persisted process completion",
                timeout=45,
            )
        except BaseException as error:
            request_outcome["error"] = error

    request_thread = threading.Thread(
        target=issue_process_request, name="parity-hub-agent-crash-request", daemon=True
    )
    request_thread.start()
    if not request_started.wait(5):
        fail("Hub Agent crash request", "HTTP process.exec request did not start")
    envelope = relay.wait_for_process_command(
        group, 15, "Hub Agent crash bound process command"
    )
    run_id = envelope.get("runId")
    request_id = envelope.get("requestId")
    command_hash = envelope.get("commandHash")
    if not all(isinstance(value, str) and value for value in (run_id, request_id, command_hash)):
        fail("Hub Agent crash origin", f"process.exec envelope omitted its durable origin: {envelope}")

    ledger_path = Path(env["HOME"]) / ".agentic_gpt" / "transport-runs.jsonl"
    event_db_path = (
        Path(env["HOME"])
        / ".agentic_gpt"
        / "state"
        / "agent"
        / agent_id
        / "events.sqlite3"
    )

    def ledger_started() -> bool:
        record = read_transport_ledger_record(ledger_path, str(run_id))
        return record is not None and record.get("status") == "started"

    wait_until(
        ledger_started,
        "Hub Agent crash transport ledger",
        "the original transport run to be durably started",
        timeout=10,
        process=agent,
    )
    lock_file = Path(f"{ledger_path}.lock").open("a+b")
    fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX)
    try:
        def completion_persisted() -> bool:
            row = read_internal_source_row(
                event_db_path, str(run_id), str(request_id), str(command_hash)
            )
            return (
                row is not None
                and row.get("responseState") == "awaiting_response"
                and row.get("completionSeen") == 1
                and row.get("completionEventType") == "process.completed"
                and bool(row.get("completionMessage"))
                and bool(row.get("completedAt"))
            )

        wait_until(
            completion_persisted,
            "Hub Agent crash source completion",
            "a bound process completion to persist before transport response commit",
            timeout=15,
            process=agent,
        )
        source_before_crash = read_internal_source_row(
            event_db_path, str(run_id), str(request_id), str(command_hash)
        )
        ledger_before_crash = read_transport_ledger_record(ledger_path, str(run_id))
        if (
            source_before_crash is None
            or source_before_crash.get("sourceKind") != "process"
            or source_before_crash.get("responseState") != "awaiting_response"
            or source_before_crash.get("completionSeen") != 1
            or source_before_crash.get("completionEventType") != "process.completed"
            or not isinstance(source_before_crash.get("completionMessage"), str)
            or "completed" not in source_before_crash["completionMessage"].lower()
            or str(source_before_crash.get("sourceRef", "")) not in source_before_crash["completionMessage"]
            or ledger_before_crash is None
            or ledger_before_crash.get("status") != "started"
        ):
            fail(
                "Hub Agent crash before response commit",
                f"Agent did not persist a bound completion while the transport run remained started: "
                f"source={source_before_crash}, ledger={ledger_before_crash}",
            )
        agent.stop()
    finally:
        fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)
        lock_file.close()

    request_thread.join(45)
    if request_thread.is_alive():
        fail("Hub Agent crash request", "HTTP process.exec did not finish after the Agent crash")
    if "error" in request_outcome:
        fail("Hub Agent crash request", f"Hub HTTP process.exec failed unexpectedly: {request_outcome['error']}")
    response = request_outcome.get("response")
    value = request_outcome.get("value")
    if not isinstance(response, HttpResponse) or not isinstance(value, dict):
        fail("Hub Agent crash request", f"HTTP process.exec returned no error envelope: {request_outcome}")
    validate_operation_response(
        document,
        "/v1/process/exec",
        "post",
        504,
        value,
        "Hub Agent crash process.exec error",
    )
    if (
        response.status != 504
        or value.get("error", {}).get("code") != "process_exec_timeout"
        or "events" in value
    ):
        fail(
            "Hub Agent crash process.exec error",
            f"expected a terminal timeout without an event panel: {response.status}, {value}",
        )
    ledger_after_crash = read_transport_ledger_record(ledger_path, str(run_id))
    if ledger_after_crash is None or ledger_after_crash.get("status") != "started":
        fail(
            "Hub Agent crash transport ledger",
            f"transport response was committed despite the locked Agent crash: {ledger_after_crash}",
        )

    hub_db_path = hub_root / "hub.db"

    def no_terminal_recorded() -> bool:
        row = read_hub_feedback_row(hub_db_path, str(run_id))
        return row is not None and row.get("decision") == "no_terminal"

    wait_until(
        no_terminal_recorded,
        "Hub Agent crash feedback",
        "the Hub to durably record NoTerminal for the timed-out response",
    )
    feedback_before_recovery = read_hub_feedback_row(hub_db_path, str(run_id))
    if feedback_before_recovery is None or feedback_before_recovery.get("decision") != "no_terminal":
        fail(
            "Hub Agent crash feedback",
            f"Hub did not persist the NoTerminal decision: {feedback_before_recovery}",
        )

    previous_receipts = relay.forwarded_receipts
    relay.arm_settle_response_failure()
    agent = ManagedProcess(
        [str(agent_binary), "run", "--config", str(config)],
        env,
        "Hub crash-recovery Agent restart",
    )
    wait_for_agent(hub_port, hub_key, agent_id, "Hub crash-recovery Agent reconnect", agent)
    report = relay.wait_for_source_report(
        str(run_id),
        str(request_id),
        str(command_hash),
        15,
        "Hub crash-recovery bound EventSources",
    )
    reported_sources = report.get("sources")
    source_ref = source_before_crash["sourceRef"]
    if (
        not isinstance(reported_sources, list)
        or len(reported_sources) != 1
        or reported_sources[0].get("kind") != "process"
        or reported_sources[0].get("ref") != source_ref
    ):
        fail(
            "Hub crash-recovery bound EventSources",
            f"Agent did not recover the exact durably bound process source: {report}",
        )

    def recovered_sources_persisted() -> bool:
        row = read_hub_feedback_row(hub_db_path, str(run_id))
        return row is not None and isinstance(row.get("sources"), str)

    wait_until(
        recovered_sources_persisted,
        "Hub crash-recovery source persistence",
        "the Hub to persist Agent-reported source identities",
        process=agent,
    )
    feedback_with_sources = read_hub_feedback_row(hub_db_path, str(run_id))
    if feedback_with_sources is None or not isinstance(feedback_with_sources.get("sources"), str):
        fail(
            "Hub crash-recovery source persistence",
            f"Hub did not persist Agent-reported source identities: {feedback_with_sources}",
        )
    try:
        stored_sources = json.loads(feedback_with_sources["sources"])
    except json.JSONDecodeError as error:
        fail("Hub crash-recovery source persistence", f"invalid persisted source identities: {error}")
    if stored_sources != reported_sources:
        fail(
            "Hub crash-recovery source persistence",
            f"Hub changed the Agent's recovered source identities: {stored_sources}",
        )

    relay.wait_for_settle(1, 15, "Hub crash-recovery initial EventSettle")
    settle_response_baseline = relay.wait_settle_failure(
        15, "Hub crash-recovery transient EventSettle response failure"
    )
    relay.wait_failed_settle_disconnected(5, "Hub crash-recovery transient disconnect")
    wait_for_agent(hub_port, hub_key, agent_id, "Hub crash-recovery Agent after transient failure", agent)

    session_path = "/v1/events?" + urlencode({"agentId": agent_id, "status": "pending"})
    list_outcome: dict[str, Any] = {}
    list_started = threading.Event()

    def issue_first_event_list() -> None:
        list_started.set()
        try:
            list_outcome["result"] = hub_event_http_call(
                hub_port,
                hub_key,
                document,
                "GET",
                session_path,
                None,
                "/v1/events",
                "get",
                "Hub crash-recovery first public event.list",
            )
        except BaseException as error:
            list_outcome["error"] = error

    list_thread = threading.Thread(
        target=issue_first_event_list, name="parity-hub-crash-event-list", daemon=True
    )
    list_thread.start()
    if not list_started.wait(5):
        fail("Hub crash-recovery event.list", "targeted public event.list request did not start")
    settle_envelopes = relay.settle_envelopes()
    if not settle_envelopes:
        fail("Hub crash-recovery EventSettle replay", "missing original settlement command")
    original_settle = settle_envelopes[0]
    replayed_settle = relay.wait_for_settle_response(
        str(original_settle["runId"]),
        str(original_settle["requestId"]),
        settle_response_baseline,
        15,
        "Hub crash-recovery completed EventSettle response replay",
    )
    relay.wait_settle_reply_held(
        15, "Hub crash-recovery completed EventSettle response replay barrier"
    )
    settle_envelopes = relay.settle_envelopes()
    if len(settle_envelopes) != 1:
        fail(
            "Hub crash-recovery EventSettle replay",
            f"reconnect did not recover through the original pending settle request: {settle_envelopes}",
        )
    settle_envelope = settle_envelopes[0]
    settle_command = settle_envelope.get("command")
    first_payload = settle_command.get("payload") if isinstance(settle_command, dict) else None
    settle_run_id = settle_envelope.get("runId")
    settle_request_id = settle_envelope.get("requestId")
    settle_hash = settle_envelope.get("commandHash")
    settle_receipt = tuple(
        str(settle_envelope.get(key, ""))
        for key in ("eventId", "runId", "requestId", "commandHash")
    )
    if (
        not isinstance(settle_command, dict)
        or settle_command.get("type") != "event.settle"
        or settle_command.get("requestId") != settle_request_id
        or not isinstance(first_payload, dict)
        or not all(isinstance(value, str) and value for value in (settle_run_id, settle_request_id, settle_hash))
        or not all(settle_receipt)
        or replayed_settle.get("data", {}).get("status") != "settled"
    ):
        fail(
            "Hub crash-recovery EventSettle replay",
            f"Agent did not replay the matching successful settlement response: "
            f"envelope={settle_envelope}, response={replayed_settle}",
        )
    ledger_record = read_transport_ledger_record(ledger_path, str(settle_run_id))
    hub_settle_record = read_hub_run_record(hub_db_path, str(settle_run_id))
    feedback_before_release = read_hub_feedback_row(hub_db_path, str(run_id))
    if feedback_before_release is None or not isinstance(feedback_before_release.get("feedbackPayload"), str):
        fail(
            "Hub crash-recovery outstanding checkpoint",
            f"durable NoTerminal outbox payload was unavailable while its response was held: "
            f"{feedback_before_release}",
        )
    try:
        first_payload = json.loads(feedback_before_release["feedbackPayload"])
    except json.JSONDecodeError as error:
        fail(
            "Hub crash-recovery outstanding checkpoint",
            f"durable NoTerminal outbox payload was invalid JSON: {error}",
        )
    if (
        ledger_record is None
        or ledger_record.get("runId") != settle_run_id
        or ledger_record.get("agentId") != agent_id
        or ledger_record.get("requestId") != settle_request_id
        or ledger_record.get("commandHash") != settle_hash
        or ledger_record.get("status") != "completed"
        or ledger_record.get("result") != {"status": "settled"}
        or hub_settle_record is None
        or hub_settle_record.get("requestId") != settle_request_id
        or hub_settle_record.get("agentId") != agent_id
        or hub_settle_record.get("commandType") != "event.settle"
        or hub_settle_record.get("commandHash") != settle_hash
        or hub_settle_record.get("status") != "acked"
        or not hub_settle_record.get("ackedAt")
        or hub_settle_record.get("result") is not None
        or feedback_before_release.get("decision") != "no_terminal"
        or feedback_before_release.get("feedbackRequestId") != settle_request_id
        or feedback_before_release.get("ackedAt") is not None
        or first_payload != settle_command.get("payload")
        or first_payload.get("origin")
        != {"runId": run_id, "requestId": request_id, "commandHash": command_hash}
        or first_payload.get("dispositions")
        != [{"source": {"kind": "process", "ref": source_ref}, "includesTerminal": False}]
        or replayed_settle.get("runId") != settle_run_id
        or replayed_settle.get("requestId") != settle_request_id
        or relay.forwarded_receipt_ids().count(settle_receipt) != 1
        or relay.receipt_holds()
    ):
        fail(
            "Hub crash-recovery outstanding checkpoint",
            f"held replay did not match the completed Agent ledger, exact forwarded command receipt, "
            f"and unacked Hub outbox: Agent={ledger_record}, Hub={hub_settle_record}, "
            f"feedback={feedback_before_release}, heldReceipts={relay.receipt_holds()}",
        )
    dispositions = first_payload.get("dispositions")
    if (
        first_payload.get("origin", {}).get("runId") != run_id
        or first_payload.get("origin", {}).get("requestId") != request_id
        or first_payload.get("origin", {}).get("commandHash") != command_hash
        or not isinstance(dispositions, list)
        or len(dispositions) != 1
        or dispositions[0].get("source", {}).get("kind") != "process"
        or dispositions[0].get("source", {}).get("ref") != source_ref
        or dispositions[0].get("includesTerminal") is not False
    ):
        fail(
            "Hub crash-recovery EventSettle disposition",
            f"NoTerminal did not settle the recovered process source without a terminal response: "
            f"{first_payload}",
        )
    time.sleep(0.1)
    command_types = relay.command_types()
    list_thread_alive = list_thread.is_alive()
    if not list_thread_alive or "event.list" in command_types:
        fail(
            "Hub crash-recovery EventSettle barrier",
            f"public event.list did not remain blocked behind the held completed-settle response: "
            f"commands={command_types}, list_thread_alive={list_thread_alive}, list_outcome={list_outcome}",
        )
    relay.disarm_receipt_hold()

    def replay_checkpoint_persisted() -> bool:
        run_record = read_hub_run_record(hub_db_path, str(settle_run_id))
        feedback = read_hub_feedback_row(hub_db_path, str(run_id))
        return (
            run_record is not None
            and run_record.get("requestId") == settle_request_id
            and run_record.get("commandHash") == settle_hash
            and run_record.get("status") == "completed"
            and run_record.get("result") == {"status": "settled"}
            and feedback is not None
            and feedback.get("decision") == "no_terminal"
            and feedback.get("feedbackRequestId") == settle_request_id
            and feedback.get("feedbackPayload") == feedback_before_release.get("feedbackPayload")
            and feedback.get("ackedAt") is not None
        )

    wait_until(
        replay_checkpoint_persisted,
        "Hub crash-recovery EventSettle response replay",
        "the held Agent response replay to complete the Hub run and acknowledge the durable feedback",
        timeout=10,
        process=agent,
    )
    if relay.forwarded_receipt_ids().count(settle_receipt) != 1:
        fail(
            "Hub crash-recovery TransportAck identity",
            f"Agent restart duplicated or changed the original EventSettle receipt: "
            f"{relay.forwarded_receipt_ids()}",
        )
    list_thread.join(30)
    if list_thread.is_alive():
        fail("Hub crash-recovery event.list", "public event.list did not resume after EventSettle")
    if "error" in list_outcome:
        fail("Hub crash-recovery event.list", f"public event.list failed: {list_outcome['error']}")
    list_result = list_outcome.get("result")
    if not isinstance(list_result, tuple) or len(list_result) != 2:
        fail("Hub crash-recovery event.list", f"public event.list returned no result: {list_outcome}")
    list_response, list_value = list_result
    items = event_items(list_value, "Hub crash-recovery first public event.list")
    if (
        list_response.status != 200
        or len(items) != 1
        or items[0].get("severity") != "low"
        or items[0].get("status") != "pending"
    ):
        fail(
            "Hub crash-recovery completion event",
            f"first public event query did not return one pending low completion: {list_value}",
        )
    event_id = items[0].get("eventId")
    if not isinstance(event_id, str):
        fail("Hub crash-recovery completion event", f"completion event omitted its ID: {items[0]}")
    first_panel = assert_event_counts(
        list_value, (1, 0, 0), "Hub crash-recovery first public event.list"
    )
    if (
        event_panel_ids(first_panel, "Hub crash-recovery first public event.list") != [event_id]
        or event_panel_levels(first_panel, "Hub crash-recovery first public event.list") != ["low"]
    ):
        fail(
            "Hub crash-recovery single exposure",
            f"first public panel did not expose exactly the recovered low event: {list_value}",
        )

    detail_path = "/v1/events/" + quote(event_id, safe="") + "?" + urlencode({"agentId": agent_id})
    _, detail = hub_event_http_call(
        hub_port,
        hub_key,
        document,
        "GET",
        detail_path,
        None,
        "/v1/events/{eventId}",
        "get",
        "Hub crash-recovery event.get",
    )
    if (
        detail.get("status") != "pending"
        or detail.get("severity") != "low"
        or detail.get("shownCount") != 1
        or not isinstance(detail.get("message"), str)
        or str(source_ref) not in detail["message"]
        or "completed" not in detail["message"].lower()
    ):
        fail(
            "Hub crash-recovery completion event body",
            f"recovered event did not preserve the actual completed process body: {detail}",
        )
    assert_event_source(detail, "process", str(source_ref), "Hub crash-recovery event detail provenance")
    detail_panel = assert_event_counts(detail, (1, 0, 0), "Hub crash-recovery event.get")
    if event_id in event_panel_ids(detail_panel, "Hub crash-recovery event.get"):
        fail("Hub crash-recovery single exposure", f"event.get exposed the low event a second time: {detail}")

    settled_source = read_internal_source_row(
        event_db_path, str(run_id), str(request_id), str(command_hash)
    )
    if (
        settled_source is None
        or settled_source.get("responseState") != "async_eligible"
        or settled_source.get("completionSeen") != 1
        or detail.get("message") != source_before_crash.get("completionMessage")
        or detail.get("createdAt") != source_before_crash.get("completedAt")
    ):
        fail(
            "Hub crash-recovery source settlement",
            f"EventSettle did not settle the persisted bound completion: {settled_source}",
        )
    feedback_after_replay = read_hub_feedback_row(hub_db_path, str(run_id))
    if (
        feedback_after_replay is None
        or feedback_after_replay.get("decision") != "no_terminal"
        or not feedback_after_replay.get("ackedAt")
        or not isinstance(feedback_after_replay.get("feedbackPayload"), str)
    ):
        fail(
            "Hub crash-recovery feedback acknowledgement",
            f"replayed EventSettle response was not durably acknowledged: {feedback_after_replay}",
        )
    try:
        persisted_payload = json.loads(feedback_after_replay["feedbackPayload"])
    except json.JSONDecodeError as error:
        fail(
            "Hub crash-recovery feedback acknowledgement",
            f"persisted EventSettle payload was invalid JSON: {error}",
        )
    if persisted_payload != first_payload:
        fail(
            "Hub crash-recovery feedback acknowledgement",
            f"durable EventSettle payload differed from the held response's request: {persisted_payload}",
        )
    types_after_replay = relay.command_types()
    settle_indexes = [index for index, kind in enumerate(types_after_replay) if kind == "event.settle"]
    event_list_index = next(
        (index for index, kind in enumerate(types_after_replay) if kind == "event.list"),
        None,
    )
    if len(settle_indexes) != 1 or event_list_index is None or settle_indexes[0] >= event_list_index:
        fail(
            "Hub crash-recovery EventSettle ordering",
            f"public event.list did not follow the single recovered EventSettle response: {types_after_replay}",
        )
    agent.stop()
    relay.wait_disconnected(5, "Hub crash-recovery Agent offline event.list")
    offline_path = "/v1/events?" + urlencode({"agentId": agent_id, "status": "pending"})
    hub_event_http_call(
        hub_port,
        hub_key,
        document,
        "GET",
        offline_path,
        None,
        "/v1/events",
        "get",
        "Hub offline event.list",
        expected_status=504,
    )
    reports.append("PASS Hub offline event.list error schema omits the optional event panel")
    reports.append(
        "PASS Hub Agent crash after durable process completion, NoTerminal source recovery, "
        "EventSettle retry barrier, completed-event provenance, and single exposure"
    )
    return agent, relay


def run_agent_event_gate(binary: Path, root: Path, reports: list[str]) -> ManagedProcess:
    process, config, env, port, token = start_event_agent(binary, root)
    http_session = open_mcp_session(port, token, "Agent event HTTP MCP")
    http_tools_message = mcp_call(
        port, token, http_session, 2, "tools/list", {}, "Agent event HTTP tools/list"
    )
    http_tools = http_tools_message.get("result", {}).get("tools")
    if not isinstance(http_tools, list):
        fail("Agent event HTTP tools/list", f"missing tools: {http_tools_message}")
    stdio_tools = open_stdio_session(process, "Agent event stdio MCP")
    unix_tools = local_surface(binary, config, env, "Agent event Unix tools/list")
    for tools, label in (
        (http_tools, "Agent event HTTP descriptors"),
        (stdio_tools, "Agent event stdio descriptors"),
        (unix_tools, "Agent event Unix descriptors"),
    ):
        assert_tool_semantics(tools, label)
    for name in ("event.list", "event.get", "event.mark"):
        schemas = [
            descriptor_map(tools)[name].get("inputSchema")
            for tools in (unix_tools, stdio_tools, http_tools)
        ]
        if schemas[0] != schemas[1] or schemas[1] != schemas[2]:
            fail("Agent event transport parity", f"{name} input schemas differ by ingress")

    http_ids = iter(range(3, 1000))

    def http_tool(name: str, arguments: dict[str, Any], scenario: str) -> dict[str, Any]:
        return agent_http_tool_call(
            port, token, http_session, next(http_ids), name, arguments, scenario
        )

    empty = local_event_call(binary, config, env, "event.list", {"status": "pending"}, "Agent event initial inbox")
    assert_event_counts(empty, (0, 0, 0), "Agent event initial inbox")
    if event_items(empty, "Agent event initial inbox"):
        fail("Agent event initial inbox", f"new Agent had unexpected pending events: {empty}")
    spoof_rejected = False
    try:
        local_external_event(
            binary,
            config,
            env,
            {
                "message": "forged source must be rejected",
                "ref": "parity-forged-source",
                "severity": "high",
                "source": {"kind": "process", "ref": "forged"},
            },
            "Agent external injection source rejection",
        )
    except GateError:
        spoof_rejected = True
    if not spoof_rejected:
        fail("Agent external injection source rejection", "caller-controlled source was accepted")
    after_spoof = local_event_call(
        binary, config, env, "event.list", {"status": "pending"}, "Agent inbox after source rejection"
    )
    assert_event_counts(after_spoof, (0, 0, 0), "Agent inbox after source rejection")
    if event_items(after_spoof, "Agent inbox after source rejection"):
        fail("Agent external injection source rejection", f"rejected source left an event: {after_spoof}")

    high_message = "high severity panel ordering"
    high = local_external_event(
        binary,
        config,
        env,
        {"message": high_message, "ref": "parity-panel-high", "severity": "high"},
        "Agent external high event",
    )
    medium = local_external_event(
        binary,
        config,
        env,
        {"message": "medium severity panel ordering", "ref": "parity-panel-medium", "severity": "medium"},
        "Agent external medium event",
    )
    unicode_message = "界" * 40
    low = local_external_event(
        binary,
        config,
        env,
        {"message": unicode_message, "ref": "parity-unicode-summary"},
        "Agent external default-low Unicode event",
    )
    high_id = high.get("eventId")
    medium_id = medium.get("eventId")
    low_id = low.get("eventId")
    if not all(isinstance(value, str) and value for value in (high_id, medium_id, low_id)):
        fail("Agent external event injection", f"an inserted event omitted its id: {high}, {medium}, {low}")
    assert_event_source(high, "external", "parity-panel-high", "Agent external high provenance")
    assert_event_source(medium, "external", "parity-panel-medium", "Agent external medium provenance")
    assert_event_source(low, "external", "parity-unicode-summary", "Agent external default-low provenance")
    if (
        high.get("severity") != "high"
        or medium.get("severity") != "medium"
        or low.get("severity") != "low"
        or high.get("expiresAt") is not None
        or medium.get("expiresAt") is not None
    ):
        fail("Agent event severity and TTL policy", f"severity defaults or medium/high TTL changed: {high}, {medium}, {low}")
    try:
        created_at = datetime.fromisoformat(low["createdAt"].replace("Z", "+00:00"))
        expires_at = datetime.fromisoformat(low["expiresAt"].replace("Z", "+00:00"))
    except (KeyError, TypeError, ValueError) as error:
        fail("Agent default low TTL", f"low event omitted valid timestamps: {low}; {error}")
    if abs((expires_at - created_at).total_seconds() - 86400) > 0.01:
        fail("Agent default low TTL", f"default low expiration was not 24 hours after creation: {low}")
    for record, scenario in (
        (high, "Agent private injection high acknowledgment"),
        (medium, "Agent private injection medium acknowledgment"),
        (low, "Agent private injection low acknowledgment"),
    ):
        if "events" in record or record.get("shownCount") != 0:
            fail(scenario, f"private injection acknowledgment consumed public exposure: {record}")
    first_public = http_tool(
        "event.list",
        {"status": "pending"},
        "Agent injected event first public MCP panel",
    )
    insertion_panel = assert_event_counts(
        first_public, (1, 1, 1), "Agent injected event first public MCP panel"
    )
    if (
        event_panel_ids(insertion_panel, "Agent severity ordering panel")
        != [high_id, medium_id, low_id]
        or event_panel_levels(insertion_panel, "Agent severity ordering panel")
        != ["high", "medium", "low"]
    ):
        fail("Agent severity ordering panel", f"panel did not sort high, medium, low: {insertion_panel}")
    first_low = event_record_for_source(
        event_items(first_public, "Agent injected event first public MCP panel"),
        "external",
        "parity-unicode-summary",
        lambda _event_id: low,
        "Agent injected event first public MCP panel",
    )
    if first_low is None or first_low.get("eventId") != low_id:
        fail("Agent injected event first public MCP panel", f"the first public MCP call omitted the CLI-injected low event: {first_public}")
    low_summary = unicode_message[:31] + "…"
    if len(low_summary) != 32:
        fail("Agent Unicode summary fixture", "test fixture did not contain the 32-character boundary")
    insertion_key = next(iter(insertion_panel["new"][2]))
    _inserted_id, separator, insertion_summary = insertion_key.partition(" | ")
    if not separator or _inserted_id != low_id or insertion_summary != low_summary:
        fail("Agent Unicode panel summary", f"summary was not exactly 31 Unicode characters plus ellipsis: {insertion_key!r}")

    hidden_low = local_event_call(
        binary, config, env, "event.list", {"status": "pending"}, "Agent hidden low event remains counted"
    )
    hidden_panel = assert_event_counts(hidden_low, (1, 1, 1), "Agent hidden low event remains counted")
    hidden_ids = event_panel_ids(hidden_panel, "Agent hidden low event remains counted")
    if low_id in hidden_ids or hidden_ids != [high_id, medium_id]:
        fail("Agent low exposure policy", f"low event was not hidden after its one exposure: {hidden_panel}")
    low_item = next((item for item in event_items(first_public, "Agent injected event first public MCP panel") if item.get("eventId") == low_id), None)
    if low_item is None or low_item.get("summary") != low_summary or len(low_item["summary"]) != 32:
        fail("Agent Unicode list summary", f"event.list lost the 32-character summary: {low_item}")

    low_detail = stdio_tool_call(
        process, 3, "event.get", {"eventId": low_id}, "Agent stdio event.get full message"
    )
    if low_detail.get("message") != unicode_message or low_detail.get("status") != "pending":
        fail("Agent stdio event.get full message", f"get did not return the full pending message: {low_detail}")
    low_detail_panel = assert_event_counts(low_detail, (1, 1, 1), "Agent stdio event.get full message")
    if low_id in event_panel_ids(low_detail_panel, "Agent stdio event.get full message") or low_detail.get("shownCount") != 1:
        fail("Agent private injection exposure", f"low event was not exposed exactly once after its injection acknowledgment: {low_detail}")
    assert_event_source(low_detail, "external", "parity-unicode-summary", "Agent stdio event.get provenance")

    medium_hidden = local_event_call(
        binary, config, env, "event.list", {"status": "pending"}, "Agent medium exposure limit"
    )
    medium_panel = assert_event_counts(medium_hidden, (1, 1, 1), "Agent medium exposure limit")
    if (
        event_panel_ids(medium_panel, "Agent medium exposure limit") != [high_id]
        or medium_id not in {item.get("eventId") for item in event_items(medium_hidden, "Agent medium exposure limit")}
    ):
        fail("Agent medium exposure limit", f"medium event did not hide after three public exposures while remaining pending: {medium_hidden}")

    marked = http_tool(
        "event.mark", {"eventIds": [low_id, "missing-parity-event"]}, "Agent HTTP event.mark"
    )
    if low_id not in marked.get("handledIds", []) or "missing-parity-event" not in marked.get("notFoundIds", []):
        fail("Agent event.mark outcomes", f"mark did not separate handled and unknown ids: {marked}")
    repeated_mark = stdio_tool_call(
        process, 4, "event.mark", {"eventIds": [low_id]}, "Agent stdio event.mark idempotent retry"
    )
    if "error" in repeated_mark:
        fail("Agent event.mark idempotence", f"marking an already handled event failed: {repeated_mark}")
    handled_detail = local_event_call(
        binary, config, env, "event.get", {"eventId": low_id}, "Agent Unix handled event.get"
    )
    if handled_detail.get("status") != "handled" or handled_detail.get("message") != unicode_message:
        fail("Agent event.mark idempotence", f"handled history or message was lost: {handled_detail}")
    handled_list = http_tool(
        "event.list", {"status": "handled"}, "Agent HTTP handled event.list"
    )
    if low_id not in {item.get("eventId") for item in event_items(handled_list, "Agent HTTP handled event.list")}:
        fail("Agent handled event list", f"handled event was not queryable: {handled_list}")
    pending_after_mark = local_event_call(
        binary, config, env, "event.list", {"status": "pending"}, "Agent pending inbox after mark"
    )
    assert_event_counts(pending_after_mark, (0, 1, 1), "Agent pending inbox after mark")
    if low_id in {item.get("eventId") for item in event_items(pending_after_mark, "Agent pending inbox after mark")}:
        fail("Agent event.mark state", f"handled event remained pending: {pending_after_mark}")

    inline = local_event_call(
        binary,
        config,
        env,
        "process.exec",
        {"command": shlex.join(["/usr/bin/printf", PROCESS_MARKERS[0]]), "waitSeconds": 5},
        "Agent Unix inline process.exec",
    )
    inline_id = inline.get("processId")
    if (
        not inline_id
        or inline.get("state") != "completed"
        or inline.get("output", {}).get("stdout", {}).get("data") != PROCESS_MARKERS[0]
    ):
        fail("Agent Unix inline process.exec", f"inline process response omitted terminal output: {inline}")
    assert_event_counts(inline, (0, 1, 1), "Agent inline result keeps original content")
    inline_list = local_event_call(
        binary, config, env, "event.list", {"status": "pending"}, "Agent inline suppression query"
    )
    inline_match = event_record_for_source(
        event_items(inline_list, "Agent inline suppression query"),
        "process",
        str(inline_id),
        lambda event_id: local_event_call(
            binary, config, env, "event.get", {"eventId": event_id}, "Agent inline event lookup"
        ),
        "Agent inline suppression query",
    )
    if inline_match is not None:
        fail("Agent inline completion suppression", f"inline process created a completion event: {inline_match}")

    asynchronous = stdio_tool_call(
        process,
        5,
        "process.exec",
        {"command": shlex.join(["/usr/bin/sleep", "1"]), "waitSeconds": 0},
        "Agent stdio asynchronous process.exec",
    )
    async_id = asynchronous.get("processId")
    if not async_id or asynchronous.get("state") in {"completed", "failed", "cancelled"}:
        fail("Agent stdio asynchronous process.exec", f"creation response was not asynchronous: {asynchronous}")
    assert_event_counts(asynchronous, (0, 1, 1), "Agent asynchronous creation response")
    completed = agent_http_tool_call(
        port,
        token,
        http_session,
        next(http_ids),
        "process.read",
        {"processId": async_id, "view": "status", "waitSeconds": 5},
        "Agent HTTP async process.read status view",
    )
    if completed.get("state") != "completed" or "output" in completed:
        fail("Agent HTTP async process.read status view", f"terminal status view returned output: {completed}")
    completion_panel = assert_event_counts(completed, (1, 1, 1), "Agent async completion notification")
    completion_ids = event_panel_ids(completion_panel, "Agent async completion notification")
    async_event_id = next((value for value in completion_ids if value != high_id), None)
    if async_event_id is None:
        fail("Agent async completion notification", f"async completion was not included in the next response panel: {completed}")
    completion_detail = http_tool(
        "event.get", {"eventId": async_event_id}, "Agent HTTP async event.get"
    )
    assert_event_source(completion_detail, "process", str(async_id), "Agent async completion provenance")
    if completion_detail.get("severity") != "low" or completion_detail.get("status") != "pending":
        fail("Agent default internal severity", f"unconfigured internal completion was not pending low: {completion_detail}")
    hidden_async = local_event_call(
        binary, config, env, "event.list", {"status": "pending"}, "Agent async hidden-count query"
    )
    hidden_async_panel = assert_event_counts(hidden_async, (1, 1, 1), "Agent async hidden-count query")
    if async_event_id in event_panel_ids(hidden_async_panel, "Agent async hidden-count query"):
        fail("Agent async hidden-count query", f"low event remained visible after one exposure: {hidden_async_panel}")
    if async_event_id not in {item.get("eventId") for item in event_items(hidden_async, "Agent async hidden-count query")}:
        fail("Agent async hidden-count query", f"hidden event disappeared from pending list: {hidden_async}")

    high_records = [high]
    for index in range(1, 7):
        high_records.append(
            local_external_event(
                binary,
                config,
                env,
                {
                    "message": f"high cap event {index}",
                    "severity": "high",
                    "ref": f"parity-panel-high-{index}",
                },
                f"Agent high cap event {index}",
            )
        )
    expected_cap_ids = [record["eventId"] for record in high_records[:5]]
    capped_response = local_event_call(
        binary,
        config,
        env,
        "event.list",
        {"status": "pending"},
        "Agent five-item cap and severity order",
    )
    capped = assert_event_counts(capped_response, (1, 1, 7), "Agent five-item cap and severity order")
    if (
        event_panel_ids(capped, "Agent five-item cap and severity order") != expected_cap_ids
        or event_panel_levels(capped, "Agent five-item cap and severity order") != ["high"] * 5
    ):
        fail("Agent five-item cap and severity order", f"panel cap or oldest-first same-severity order changed: {capped}")
    all_pending = local_event_call(
        binary, config, env, "event.list", {"status": "pending"}, "Agent full pending inbox"
    )
    if len(event_items(all_pending, "Agent full pending inbox")) != 9:
        fail("Agent hidden and capped event counting", f"pending items were lost or duplicated: {all_pending}")
    assert_event_counts(all_pending, (1, 1, 7), "Agent hidden and capped event counting")

    persistent = local_external_event(
        binary,
        config,
        env,
        {"message": "survives Agent reopen", "ref": "parity-persist", "severity": "high"},
        "Agent persistent event insertion",
    )
    persistent_id = persistent.get("eventId")
    if not isinstance(persistent_id, str):
        fail("Agent persistent event insertion", f"missing event id: {persistent}")
    process.stop()
    process = start_event_worker(binary, config, env, "Agent event restart")
    socket_path = Path(env["HOME"]) / ".agentic_gpt" / "runtime" / "agent" / "parity-events" / "mcp.sock"
    wait_until(
        lambda: process.alive() and socket_path.exists() and tcp_ready(port),
        "Agent event restart",
        "reopened stdio, Unix, and HTTP listeners",
        process=process,
    )
    persisted = local_event_call(
        binary, config, env, "event.get", {"eventId": persistent_id}, "Agent Unix persistent event.get"
    )
    if (
        persisted.get("status") != "pending"
        or persisted.get("message") != "survives Agent reopen"
        or persisted.get("source", {}).get("ref") != "parity-persist"
    ):
        fail("Agent persistent reopen", f"reopened Agent did not retain the event record: {persisted}")
    reopened_stdio = open_stdio_session(process, "Agent restarted stdio MCP")
    assert_event_tool_semantics(reopened_stdio, "Agent restarted stdio descriptors")
    stdio_persisted = stdio_tool_call(
        process, 3, "event.get", {"eventId": persistent_id}, "Agent stdio persistent event.get"
    )
    if stdio_persisted.get("message") != "survives Agent reopen":
        fail("Agent stdio persistent reopen", f"reopened stdio MCP lost the event: {stdio_persisted}")
    reports.append(
        "PASS Agent stdio/Unix/HTTP MCP shared inbox, fixed external source, Unicode summary, exposure/cap/order, get/mark, inline suppression, async low completion, and durable reopen"
    )
    return process


def run_http_internal_policy_off_gate(
    port: int, token: str, session: str, reports: list[str]
) -> None:
    process_start = agent_http_tool_call(
        port,
        token,
        session,
        13,
        "process.exec",
        {
            "command": shlex.join(["/usr/bin/sleep", "1"]),
            "waitSeconds": 0,
        },
        "Agent HTTP internal-off async process.exec",
    )
    process_id = process_start.get("processId")
    if not process_id or process_start.get("state") in {"completed", "failed", "cancelled"}:
        fail("Agent HTTP internal-off async process.exec", f"sleep did not start asynchronously: {process_start}")
    agent_http_tool_call(
        port,
        token,
        session,
        14,
        "process.read",
        {"processId": process_id, "view": "status", "waitSeconds": 5},
        "Agent HTTP internal-off process.read status view",
    )
    pending = agent_http_tool_call(
        port,
        token,
        session,
        15,
        "event.list",
        {"status": "pending"},
        "Agent HTTP internal-off event.list",
    )
    lookup_ids = iter(range(16, 1000))
    match = event_record_for_source(
        event_items(pending, "Agent HTTP internal-off event.list"),
        "process",
        str(process_id),
        lambda event_id: agent_http_tool_call(
            port,
            token,
            session,
            next(lookup_ids),
            "event.get",
            {"eventId": event_id},
            "Agent HTTP internal-off event.get",
        ),
        "Agent HTTP internal-off event.list",
    )
    if match is not None:
        fail("Agent HTTP internal policy off", f"process.completed=off created a completion event: {match}")
    reports.append("PASS Agent internal process-completion off policy")


def run_live_event_policy_reload_gate(
    binary: Path, root: Path, reports: list[str]
) -> None:
    process, config, env, port, token = start_event_agent(binary, root / "event-policy-reload")
    try:
        configure_event_fixture(config)
        session = open_mcp_session(port, token, "Agent live event-policy reload")
        old_start = agent_http_tool_call(
            port,
            token,
            session,
            3,
            "process.exec",
            {"command": shlex.join(["/usr/bin/sleep", "30"]), "waitSeconds": 0},
            "Agent pre-reload async process.exec",
        )
        old_id = old_start.get("processId")
        if not old_id or old_start.get("state") in {"completed", "failed", "cancelled"}:
            fail("Agent pre-reload async process.exec", f"old process was not admitted asynchronously: {old_start}")

        previous_reload_count = process.diagnostics().count("live config reloaded;")
        run_checked(
            [
                str(binary),
                "--language",
                "en",
                "config",
                "--config",
                str(config),
                "set",
                "events.internalOverrides.process.completed",
                "medium",
            ],
            env,
            "Agent live event policy config set",
        )
        wait_until(
            lambda: process.alive()
            and process.diagnostics().count("live config reloaded;") > previous_reload_count,
            "Agent live event policy reload",
            "the running daemon's successful live-config-reloaded log",
            timeout=8,
            process=process,
        )
        info = agent_http_tool_call(
            port,
            token,
            session,
            4,
            "agent.info",
            {},
            "Agent live event-policy reload runtime snapshot",
        )
        if info.get("config", {}).get("liveSubsetMatchesDisk") is not True:
            fail("Agent live event policy reload", f"runtime did not apply the CLI-written subset: {info}")

        new_start = agent_http_tool_call(
            port,
            token,
            session,
            5,
            "process.exec",
            {"command": shlex.join(["/usr/bin/sleep", "1"]), "waitSeconds": 0},
            "Agent post-reload async process.exec",
        )
        new_id = new_start.get("processId")
        if not new_id or new_start.get("state") in {"completed", "failed", "cancelled"}:
            fail("Agent post-reload async process.exec", f"new process was not admitted asynchronously: {new_start}")
        new_done = agent_http_tool_call(
            port,
            token,
            session,
            6,
            "process.read",
            {"processId": new_id, "view": "status", "waitSeconds": 5},
            "Agent post-reload process.read status view",
        )
        old_done = agent_http_tool_call(
            port,
            token,
            session,
            7,
            "process.read",
            {"processId": old_id, "view": "status", "waitSeconds": 30},
            "Agent pre-reload process.read status view",
            timeout=45,
        )
        if new_done.get("state") != "completed" or old_done.get("state") != "completed":
            fail("Agent live event policy process completion", f"both admitted processes must finish: {old_done}, {new_done}")
        pending = agent_http_tool_call(
            port,
            token,
            session,
            8,
            "event.list",
            {"status": "pending"},
            "Agent live event policy event.list",
        )
        items = event_items(pending, "Agent live event policy event.list")
        if {item.get("eventId") for item in items} == set() or len(items) != 2:
            fail("Agent live event policy event.list", f"expected exactly the two completion records: {pending}")
        expected = {str(old_id): "low", str(new_id): "medium"}
        if {item.get("severity") for item in items} != set(expected.values()):
            fail("Agent live event policy event.list", f"admission snapshots were not reflected in the list: {items}")
        lookup_ids = iter(range(9, 1000))
        by_source: dict[str, dict[str, Any]] = {}
        for item in items:
            event_id = item.get("eventId")
            detail = agent_http_tool_call(
                port,
                token,
                session,
                next(lookup_ids),
                "event.get",
                {"eventId": event_id},
                "Agent live event policy event.get",
            )
            source = detail.get("source", {})
            reference = source.get("ref")
            if source.get("kind") != "process" or reference not in expected:
                fail("Agent live event policy event.get provenance", f"completion source did not match an admitted process: {detail}")
            if (
                detail.get("status") != "pending"
                or detail.get("severity") != expected[reference]
                or not isinstance(detail.get("message"), str)
                or reference not in detail["message"]
                or "completed" not in detail["message"]
            ):
                fail("Agent live event policy event.get body", f"full completion record did not preserve its admission snapshot: {detail}")
            by_source[reference] = detail
        if set(by_source) != set(expected):
            fail("Agent live event policy source coverage", f"event.get omitted an admitted process source: {by_source}")
        reports.append("PASS live config CLI reload snapshots old low and new medium process event policies")
    finally:
        process.stop()


def run_event_ttl_gate(binary: Path, root: Path, reports: list[str]) -> None:
    config, _, env = init_agent(binary, root, "local", "normal", "parity-event-ttl")
    configure_event_fixture(config, low_ttl_seconds=2)
    configure_process_fixture(config)
    process = ManagedProcess(
        [str(binary), "run", "--config", str(config)], env, "Agent event TTL"
    )
    socket_path = Path(env["HOME"]) / ".agentic_gpt" / "runtime" / "agent" / "parity-event-ttl" / "mcp.sock"
    wait_until(
        lambda: socket_path.exists() and process.alive(),
        "Agent event TTL",
        "private Unix MCP socket",
        process=process,
    )
    try:
        record = local_external_event(
            binary,
            config,
            env,
            {"message": "short lived low event", "ref": "parity-ttl"},
            "Agent low TTL injection",
        )
        event_id = record.get("eventId")
        try:
            created_at = datetime.fromisoformat(record["createdAt"].replace("Z", "+00:00"))
            expires_at = datetime.fromisoformat(record["expiresAt"].replace("Z", "+00:00"))
        except (KeyError, TypeError, ValueError) as error:
            fail("Agent low TTL expiration", f"event omitted valid creation/expiry timestamps: {record}; {error}")
        if not isinstance(event_id, str) or abs((expires_at - created_at).total_seconds() - 2) > 0.01:
            fail("Agent low TTL expiration", f"configured two-second TTL was not captured at creation: {record}")
        expired_record: dict[str, Any] = {}

        def expired() -> bool:
            current = local_event_call(
                binary, config, env, "event.get", {"eventId": event_id}, "Agent TTL boundary get"
            )
            if current.get("status") == "expired":
                expired_record.update(current)
                return True
            return False

        wait_until(expired, "Agent low TTL boundary", "event.get to observe expiration", timeout=TIMEOUT)
        pending = local_event_call(
            binary, config, env, "event.list", {"status": "pending"}, "Agent pending list after TTL"
        )
        assert_event_counts(pending, (0, 0, 0), "Agent pending list after TTL")
        if event_id in {item.get("eventId") for item in event_items(pending, "Agent pending list after TTL")}:
            fail("Agent low TTL boundary", f"expired low event remained pending: {pending}")
        history = local_event_call(
            binary, config, env, "event.list", {"status": "expired"}, "Agent expired history"
        )
        if event_id not in {item.get("eventId") for item in event_items(history, "Agent expired history")}:
            fail("Agent expired history", f"expired record was not retained for history: {history}")
        assert_event_source(expired_record, "external", "parity-ttl", "Agent TTL provenance")
        reports.append("PASS Agent low TTL expiry boundary and retained expired history through Unix MCP")
    finally:
        process.stop()


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


def process_request(port: int, api_key: str, agent_id: str, command: str, group: str,
                    wait_seconds: int | None = 0, scenario: str = "Hub process.exec",
                    cwd: str | None = None) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "agentId": agent_id,
        "command": command,
        "group": group,
        "needConfirm": False,
    }
    if cwd is not None:
        payload["cwd"] = cwd
    if wait_seconds is not None:
        payload["waitSeconds"] = wait_seconds
    response, value = hub_json(port, api_key, "POST", "/v1/process/exec", payload, scenario)
    if response.status != 200:
        fail(scenario, f"HTTP {response.status}: {value}")
    if not isinstance(value, dict):
        fail(scenario, f"response is not an object: {value}")
    return value


def decode_output_segment(segment: dict[str, Any], scenario: str) -> bytes:
    data = segment.get("data")
    if not isinstance(data, str):
        fail(scenario, f"output segment has no string data: {segment}")
    if segment.get("encoding") == "utf8":
        decoded = data.encode("utf-8")
    elif segment.get("encoding") == "base64":
        try:
            decoded = base64.b64decode(data, validate=True)
        except (ValueError, base64.binascii.Error) as error:
            fail(scenario, f"output segment is not valid base64: {error}")
    else:
        fail(scenario, f"unsupported output encoding: {segment.get('encoding')!r}")
    try:
        start = int(segment["startOffset"])
        end = int(segment["endOffset"])
    except (KeyError, TypeError, ValueError):
        fail(scenario, f"output segment offsets are not decimal strings: {segment}")
    if end - start != len(decoded):
        fail(scenario, f"output byte offsets do not match the returned data: {segment}")
    return decoded


def process_response_size(value: dict[str, Any]) -> int:
    body = {key: item for key, item in value.items() if key != "events"}
    return len(json.dumps(body, separators=(",", ":"), ensure_ascii=False).encode("utf-8"))


def process_read_has_artifacts(response: dict[str, Any]) -> bool:
    return "output" in response or "mcpResult" in response


def hub_process_read(
    port: int,
    api_key: str,
    agent_id: str,
    process_id: str,
    document: dict[str, Any],
    schemas: dict[str, Any],
    scenario: str,
    *,
    wait_seconds: int | None = None,
    view: str | None = None,
    cursor: str | None = None,
    max_bytes: int | None = None,
    expected_status: int = 200,
) -> tuple[HttpResponse, dict[str, Any]]:
    query: dict[str, Any] = {"agentId": agent_id}
    for name, value in (
        ("waitSeconds", wait_seconds),
        ("view", view),
        ("cursor", cursor),
        ("maxBytes", max_bytes),
    ):
        if value is not None:
            query[name] = value
    response, result = hub_json(
        port,
        api_key,
        "GET",
        f"/v1/process/{quote(process_id, safe='')}/read?" + urlencode(query),
        None,
        scenario,
    )
    if response.status != expected_status or not isinstance(result, dict):
        fail(scenario, f"HTTP {response.status}: {result}")
    validate_operation_response(
        document,
        "/v1/process/{processId}/read",
        "get",
        expected_status,
        result,
        scenario,
    )
    if expected_status == 200:
        validate_instance(document, schemas, "ProcessReadResponse", result, f"{scenario} response")
    return response, result


def collect_http_process_stdout(
    port: int,
    api_key: str,
    agent_id: str,
    process_id: str,
    document: dict[str, Any],
    schemas: dict[str, Any],
    scenario: str,
    expected: bytes,
    *,
    first_max_bytes: int | None,
    continuation_max_bytes: int,
) -> list[dict[str, Any]]:
    collected = bytearray()
    pages: list[dict[str, Any]] = []
    cursor: str | None = None
    for index in range(64):
        response_budget = (
            first_max_bytes if first_max_bytes is not None else 4096
        ) if index == 0 else continuation_max_bytes
        response, value = hub_process_read(
            port,
            api_key,
            agent_id,
            process_id,
            document,
            schemas,
            scenario,
            cursor=cursor,
            wait_seconds=1,
            max_bytes=first_max_bytes if index == 0 else continuation_max_bytes,
        )
        if process_response_size(value) > response_budget:
            fail(scenario, f"serialized ProcessResponse exceeded its {response_budget}-byte budget")
        output = value.get("output")
        if not isinstance(output, dict):
            fail(scenario, f"read response omitted its output page: {value}")
        stdout = output.get("stdout")
        if not isinstance(stdout, dict):
            fail(scenario, f"read output page omitted its stdout segment: {output}")
        if stdout.get("gap"):
            fail(scenario, f"unexpected output gap while assembling fixture: {stdout}")
        try:
            segment_start = int(stdout.get("startOffset", "-1"))
        except (TypeError, ValueError):
            fail(scenario, f"output start offset is not decimal: {stdout}")
        if segment_start != len(collected):
            fail(scenario, f"cursor page did not continue at the returned byte offset: {stdout}")
        collected.extend(decode_output_segment(stdout, scenario))
        pages.append(value)
        if output.get("eof") is True:
            if output.get("hasMore") is not False:
                fail(scenario, f"EOF still advertises unread retained output: {output}")
            break
        if not isinstance(output.get("hasMore"), bool) or not output.get("nextCursor"):
            fail(scenario, f"output continuation omitted its cursor: {output}")
        if value.get("captureStatus") == "incomplete":
            fail(scenario, f"fixture output capture failed before EOF: {value}")
        cursor = output["nextCursor"]
    else:
        fail(scenario, "output cursor did not reach EOF within the page bound")
    if bytes(collected) != expected:
        fail(
            scenario,
            f"cursor pages did not reproduce exact output ({len(collected)} bytes, expected {len(expected)})",
        )
    return pages


def collect_mcp_process_stdout(
    port: int,
    token: str,
    session: str,
    agent_id: str,
    process_id: str,
    expected: bytes,
    scenario: str,
    *,
    request_id_start: int,
    max_bytes: int,
) -> list[dict[str, Any]]:
    collected = bytearray()
    pages: list[dict[str, Any]] = []
    cursor: str | None = None
    seen_cursors: set[str] = set()
    for index in range(64):
        page_scenario = f"{scenario} page {index + 1}"
        arguments: dict[str, Any] = {
            "agentId": agent_id,
            "processId": process_id,
            "waitSeconds": 1,
            "maxBytes": max_bytes,
        }
        if cursor is not None:
            arguments["cursor"] = cursor
        value = json_result(
            mcp_call(
                port,
                token,
                session,
                request_id_start + index,
                "tools/call",
                {"name": "process.read", "arguments": arguments},
                page_scenario,
            ),
            page_scenario,
        )
        if not isinstance(value, dict):
            fail(page_scenario, f"process.read result is not an object: {value}")
        if (
            value.get("agentId") != agent_id
            or value.get("processId") != process_id
            or value.get("state") != "completed"
        ):
            fail(page_scenario, f"cursor response lost process identity or terminal state: {value}")
        if process_response_size(value) > max_bytes:
            fail(page_scenario, f"serialized ProcessResponse exceeded its {max_bytes}-byte budget")
        output = value.get("output")
        if not isinstance(output, dict):
            fail(page_scenario, f"process.read response omitted its output page: {value}")
        stdout = output.get("stdout")
        if not isinstance(stdout, dict) or stdout.get("gap"):
            fail(page_scenario, f"process.read page omitted contiguous stdout: {output}")
        try:
            segment_start = int(stdout.get("startOffset", "-1"))
        except (TypeError, ValueError):
            fail(page_scenario, f"output start offset is not decimal: {stdout}")
        if segment_start != len(collected):
            fail(page_scenario, f"MCP cursor page did not continue at the returned byte offset: {stdout}")
        collected.extend(decode_output_segment(stdout, page_scenario))
        pages.append(value)
        if output.get("eof") is True:
            if output.get("hasMore") is not False:
                fail(page_scenario, f"EOF still advertises unread retained output: {output}")
            break
        if not isinstance(output.get("hasMore"), bool):
            fail(page_scenario, f"output page has invalid continuation state: {output}")
        if value.get("captureStatus") == "incomplete":
            fail(page_scenario, f"fixture output capture failed before EOF: {value}")
        next_cursor = output.get("nextCursor")
        if not isinstance(next_cursor, str) or not next_cursor:
            fail(page_scenario, f"output cursor is absent: {output}")
        if output.get("hasMore") and next_cursor in seen_cursors:
            fail(page_scenario, f"backlogged output cursor did not advance: {output}")
        seen_cursors.add(next_cursor)
        cursor = next_cursor
    else:
        fail(scenario, "MCP output cursor did not reach EOF within the page bound")
    if bytes(collected) != expected:
        fail(
            scenario,
            f"MCP cursor pages did not reproduce exact output ({len(collected)} bytes, expected {len(expected)})",
        )
    return pages


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
    event_agent_process = None
    hub_process = None
    timeout_agent = None
    timeout_relay = None
    delta_agent = None
    delta_relay = None
    crash_agent = None
    crash_relay = None
    confirmation: ConfirmationReceiver | None = None
    try:
        local_process, local_config, local_env, local_tools = start_local_agent(agent_binary, root / "local", reports)
        http_process, http_config, http_env, http_port, http_token, http_tools = start_http_agent(
            agent_binary, root / "http", reports
        )
        event_agent_process = run_agent_event_gate(
            agent_binary, root / "events-agent", reports
        )
        run_live_event_policy_reload_gate(
            agent_binary, root / "events-policy-reload", reports
        )
        assert_tool_semantics(local_tools, "Agent local descriptor")
        assert_tool_semantics(http_tools, "Agent HTTP descriptor")
        assert_room_tool_semantics(local_tools, "Agent local current Room descriptors")
        assert_room_tool_semantics(http_tools, "Agent HTTP current Room descriptors")
        assert_skill_semantics(local_tools, "Agent local Skill descriptors")
        assert_skill_semantics(http_tools, "Agent HTTP Skill descriptors")
        if descriptor_map(local_tools).keys() != descriptor_map(http_tools).keys():
            fail("Agent local/HTTP parity", "tools/list names differ")
        for name in ("process.read", "process.list", "process.cancel", "skills.install.get", "skills.run"):
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
                        "command": shlex.join(["/usr/bin/printf", PROCESS_MARKERS[0]]),
                        "waitSeconds": 5,
                    },
                },
                "Agent HTTP process.exec",
            ),
            "Agent HTTP process.exec",
        )
        http_process_id = http_exec.get("processId")
        http_output = http_exec.get("output", {})
        if (
            not http_process_id
            or http_exec.get("agentId") != "parity-http"
            or http_exec.get("state") != "completed"
            or http_output.get("stdout", {}).get("data") != PROCESS_MARKERS[0]
            or http_output.get("stdout", {}).get("encoding") != "utf8"
        ):
            fail("Agent HTTP process.exec", f"printf did not complete with full process read response: {http_exec}")
        if process_response_size(http_exec) > 8192:
            fail("Agent HTTP process.exec", f"creation response exceeded the configured process response budget: {http_exec}")
        run_http_internal_policy_off_gate(http_port, http_token, http_session, reports)
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
            process_response_bytes=4096,
        )
        room_process, room_config, room_env = start_hub_agent(
            agent_binary, root / "hub-room", "room", room_id, f"http://127.0.0.1:{hub_port}", room_secret, reports
        )
        wait_for_agent(hub_port, hub_key, normal_id, "Hub normal Agent connection", normal_process)
        wait_for_agent(hub_port, hub_key, room_id, "Hub Room Agent connection", room_process)
        reports.append("PASS Hub Full with connected normal and active Room Agent")
        hub_process, timeout_agent, timeout_relay = run_hub_late_response_gate(
            agent_binary,
            hub_binary,
            hub_process,
            hub_port,
            hub_key,
            hub_root,
            hub_config,
            hub_env,
            normal_id,
            normal_process,
            room_id,
            room_process,
            document,
            reports,
        )
        hub_process, delta_agent, delta_relay = run_hub_feedback_delta_recovery_gate(
            agent_binary,
            hub_binary,
            hub_process,
            hub_port,
            hub_key,
            hub_root,
            hub_config,
            hub_env,
            document,
            reports,
        )
        crash_agent, crash_relay = run_hub_agent_crash_recovery_gate(
            agent_binary,
            hub_binary,
            hub_port,
            hub_key,
            hub_root,
            hub_config,
            hub_env,
            document,
            reports,
        )

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
        normal_config_before_event_gate = normal_config.read_bytes()
        run_hub_event_gate(
            agent_binary,
            hub_port,
            hub_key,
            full_session,
            normal_id,
            normal_config,
            normal_env,
            room_id,
            document,
            reports,
        )


        normal_workspace = Path(json.loads(normal_config.read_text())["workspaceRoot"])
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
                        "command": f"/usr/bin/pwd; {shlex.join(['/usr/bin/printf', PROCESS_MARKERS[1]])}",
                        "cwd": str(normal_workspace),
                        "needConfirm": False,
                        "waitSeconds": 5,
                    },
                },
                "Hub Full process.exec printf",
            ),
            "Hub Full process.exec printf",
        )
        full_id = full_exec.get("processId") if isinstance(full_exec, dict) else None
        full_output = full_exec.get("output", {}) if isinstance(full_exec, dict) else {}
        if (
            not full_id
            or full_exec.get("agentId") != normal_id
            or full_exec.get("state") != "completed"
            or full_output.get("stdout", {}).get("data") != f"{normal_workspace.resolve()}\n{PROCESS_MARKERS[1]}"
            or full_output.get("stdout", {}).get("encoding") != "utf8"
            or process_response_size(full_exec) > 4096
        ):
            fail("Hub Full process.exec printf", f"process response omitted identity or bounded output: {full_exec}")
        inline_panel = assert_event_counts(
            full_exec, (0, 0, 0), "Hub inline process.exec event suppression"
        )
        if inline_panel["new"]:
            fail("Hub inline process.exec event suppression", f"inline terminal response had completion events: {full_exec}")
        inline_pending = hub_event_mcp_call(
            hub_port,
            hub_key,
            full_session,
            92,
            "event.list",
            {"agentId": normal_id, "status": "pending"},
            "Hub inline completion event query",
        )
        if event_items(inline_pending, "Hub inline completion event query"):
            fail("Hub inline process.exec event suppression", f"terminal Agent result generated an event: {inline_pending}")
        assert_event_counts(inline_pending, (0, 0, 0), "Hub inline completion event query")
        reload_count = normal_process.diagnostics().count("live config reloaded;")
        normal_config.write_bytes(normal_config_before_event_gate)
        wait_until(
            lambda: normal_process.diagnostics().count("live config reloaded;") > reload_count,
            "Hub event producer policy restoration",
            "the Agent to restore its original process event policy after inline suppression",
            process=normal_process,
        )
        reports.append("PASS Hub event producer policy overrides are reloaded back to the original config after inline suppression")
        full_status = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 4, "tools/call",
                {
                    "name": "process.read",
                    "arguments": {
                        "agentId": normal_id,
                        "processId": full_id,
                        "waitSeconds": 1,
                        "view": "status",
                    },
                },
                "Hub Full process.read status view",
            ),
            "Hub Full process.read status view",
        )
        if (
            full_status.get("agentId") != normal_id
            or full_status.get("processId") != full_id
            or full_status.get("state") != "completed"
            or process_read_has_artifacts(full_status)
        ):
            fail("Hub Full process.read status view", f"status view returned artifacts or lost process identity: {full_status}")
        full_read = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 5, "tools/call",
                {
                    "name": "process.read",
                    "arguments": {"agentId": normal_id, "processId": full_id, "maxBytes": 4096},
                },
                "Hub Full process.read auto view",
            ),
            "Hub Full process.read auto view",
        )
        if (
            full_read.get("agentId") != normal_id
            or full_read.get("processId") != full_id
            or full_read.get("output", {}).get("stdout", {}).get("data") != f"{normal_workspace.resolve()}\n{PROCESS_MARKERS[1]}"
            or "mcpResult" in full_read
        ):
            fail("Hub Full process.read auto view", f"unified read omitted command output or MCP applicability: {full_read}")
        mcp_page_exec = json_result(
            mcp_call(
                hub_port,
                hub_key,
                full_session,
                200,
                "tools/call",
                {
                    "name": "process.exec",
                    "arguments": {
                        "agentId": normal_id,
                        "command": shlex.join([
                            "/bin/sh", "-c", PROCESS_MCP_PAGINATION_COMMAND,
                            "sh", PROCESS_OVERFLOW_MARKER,
                        ]),
                        "needConfirm": False,
                        "waitSeconds": 5,
                    },
                },
                "Hub MCP process.read cursor fixture",
            ),
            "Hub MCP process.read cursor fixture",
        )
        mcp_page_id = mcp_page_exec.get("processId") if isinstance(mcp_page_exec, dict) else None
        if (
            not mcp_page_id
            or mcp_page_exec.get("agentId") != normal_id
            or mcp_page_exec.get("state") != "completed"
        ):
            fail("Hub MCP process.read cursor fixture", f"large MCP output process did not complete: {mcp_page_exec}")
        mcp_pages = collect_mcp_process_stdout(
            hub_port,
            hub_key,
            full_session,
            normal_id,
            mcp_page_id,
            PROCESS_OVERFLOW_MARKER.encode(),
            "Hub MCP process.read cursor pagination",
            request_id_start=210,
            max_bytes=4096,
        )
        if len(mcp_pages) < 2 or mcp_pages[0].get("output", {}).get("hasMore") is not True:
            fail("Hub MCP process.read cursor pagination", "MCP process.read did not return a continuable first page")
        reports.append("PASS Hub MCP process.read cursor pages preserve contiguous output within each response budget")
        for request_id, (retired_tool, arguments) in enumerate(
            (
                ("process.status", {"agentId": normal_id, "processId": full_id, "waitSeconds": 0}),
                ("process.output", {"agentId": normal_id, "processId": full_id, "maxBytes": 8192}),
                ("process.result", {"agentId": normal_id, "processId": full_id, "maxBytes": 8192}),
            ),
            start=100,
        ):
            retired_call = mcp_call(
                hub_port,
                hub_key,
                full_session,
                request_id,
                "tools/call",
                {"name": retired_tool, "arguments": arguments},
                f"Retired process MCP tool {retired_tool}",
            )
            if "error" not in retired_call and retired_call.get("result", {}).get("isError") is not True:
                fail(
                    f"Retired process MCP tool {retired_tool}",
                    f"retired live process tool remained callable: {retired_call}",
                )

        page_exec = process_request(
            hub_port, hub_key, normal_id,
            shlex.join(["/usr/bin/printf", PROCESS_OVERFLOW_MARKER]),
            "parity-read-pages", 5, "Hub HTTP process.read page fixture",
        )
        validate_operation_response(
            document, "/v1/process/exec", "post", 200, page_exec, "Hub HTTP process.read page fixture",
        )
        page_process_id = page_exec.get("processId")
        if not page_process_id or page_exec.get("state") != "completed":
            fail("Hub HTTP process.read page fixture", f"large output process did not complete: {page_exec}")
        pages = collect_http_process_stdout(
            hub_port,
            hub_key,
            normal_id,
            page_process_id,
            document,
            schemas,
            "Hub HTTP process.read configured-default pagination",
            PROCESS_OVERFLOW_MARKER.encode(),
            first_max_bytes=None,
            continuation_max_bytes=4096,
        )
        if (
            len(pages) < 2
            or pages[0].get("output", {}).get("hasMore") is not True
            or not pages[0].get("output", {}).get("nextCursor")
        ):
            fail("Hub HTTP process.read configured-default pagination", "the configured 4096-byte response cap did not paginate real output")

        escaped_exec = process_request(
            hub_port, hub_key, normal_id,
            shlex.join(["/usr/bin/printf", "%s", PROCESS_ESCAPE_OUTPUT]),
            "parity-read-json-escape", 5,
            "Hub HTTP process.read JSON escaping fixture",
        )
        escaped_id = escaped_exec.get("processId")
        if not escaped_id:
            fail("Hub HTTP process.read JSON escaping fixture", f"process omitted its id: {escaped_exec}")
        escaped_pages = collect_http_process_stdout(
            hub_port,
            hub_key,
            normal_id,
            escaped_id,
            document,
            schemas,
            "Hub HTTP process.read JSON escaping budget",
            PROCESS_ESCAPE_OUTPUT.encode(),
            first_max_bytes=4096,
            continuation_max_bytes=4096,
        )
        if (
            len(escaped_pages) < 2
            or escaped_pages[0].get("output", {}).get("stdout", {}).get("encoding") != "utf8"
        ):
            fail("Hub HTTP process.read JSON escaping budget", "JSON-escaped output did not honor the response budget")

        binary_exec = process_request(
            hub_port, hub_key, normal_id,
            shlex.join(["/usr/bin/printf", PROCESS_BINARY_FORMAT]),
            "parity-read-base64", 5, "Hub HTTP process.read binary fixture",
        )
        binary_id = binary_exec.get("processId")
        if not binary_id:
            fail("Hub HTTP process.read binary fixture", f"process omitted its id: {binary_exec}")
        binary_pages = collect_http_process_stdout(
            hub_port,
            hub_key,
            normal_id,
            binary_id,
            document,
            schemas,
            "Hub HTTP process.read base64 budget",
            PROCESS_BINARY_OUTPUT,
            first_max_bytes=4096,
            continuation_max_bytes=4096,
        )
        if binary_pages[0].get("output", {}).get("stdout", {}).get("encoding") != "base64":
            fail("Hub HTTP process.read base64 budget", "invalid UTF-8 output was not preserved as base64")

        invalid_budget_response, invalid_budget = hub_process_read(
            hub_port,
            hub_key,
            normal_id,
            page_process_id,
            document,
            schemas,
            "Hub HTTP process.read rejects sub-minimum budget",
            max_bytes=4095,
            expected_status=400,
        )
        if invalid_budget_response.status != 400:
            fail("Hub HTTP process.read rejects sub-minimum budget", f"invalid maxBytes was accepted: {invalid_budget}")
        invalid_view_response, invalid_view = hub_process_read(
            hub_port,
            hub_key,
            normal_id,
            page_process_id,
            document,
            schemas,
            "Hub HTTP process.read rejects status cursor",
            wait_seconds=0,
            view="status",
            cursor="opaque-cursor",
            max_bytes=4096,
            expected_status=400,
        )
        if invalid_view_response.status != 400:
            fail("Hub HTTP process.read rejects status cursor", f"status and cursor were combined: {invalid_view}")

        for retired_path in (
            f"/v1/process/{full_id}",
            f"/v1/process/{full_id}/output",
            f"/v1/process/{full_id}/result",
        ):
            retired_response = http_request(
                hub_port,
                "GET",
                retired_path,
                headers={"Authorization": f"Bearer {hub_key}"},
                scenario=f"Retired process HTTP route {retired_path}",
            )
            if retired_response.status != 404:
                fail(
                    f"Retired process HTTP route {retired_path}",
                    f"retired live read route remained reachable: HTTP {retired_response.status}",
                )

        auto_start = process_request(
            hub_port, hub_key, normal_id,
            shlex.join(["/bin/sh", "-c", PROCESS_AUTO_COMMAND]),
            "parity-read-auto-wait", 0, "Hub process.read auto wait fixture",
        )
        auto_id = auto_start.get("processId")
        if not auto_id:
            fail("Hub process.read auto wait fixture", f"process omitted its id: {auto_start}")
        auto_started = time.monotonic()
        _, auto_read = hub_process_read(
            hub_port, hub_key, normal_id, auto_id, document, schemas,
            "Hub process.read auto returns backlog",
            wait_seconds=5,
        )
        auto_elapsed = time.monotonic() - auto_started
        if (
            auto_elapsed >= 1.5
            or auto_read.get("state") in {"completed", "failed", "cancelled"}
            or auto_read.get("output", {}).get("stdout", {}).get("data") != PROCESS_AUTO_MARKER
        ):
            fail("Hub process.read auto returns backlog", f"auto did not return available output before process exit: {auto_read}")
        status_started = time.monotonic()
        auto_status = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 74, "tools/call",
                {
                    "name": "process.read",
                    "arguments": {
                        "agentId": normal_id,
                        "processId": auto_id,
                        "waitSeconds": 5,
                        "view": "status",
                    },
                },
                "Hub process.read status waits for exit",
                timeout=8,
            ),
            "Hub process.read status waits for exit",
        )
        status_elapsed = time.monotonic() - status_started
        if (
            auto_status.get("state") != "completed"
            or "output" in auto_status
            or status_elapsed < 0.5
        ):
            fail("Hub process.read status waits for exit", f"status returned on output rather than process exit: {auto_status}")

        tail_start = process_request(
            hub_port, hub_key, normal_id,
            shlex.join(["/bin/sh", "-c", PROCESS_TAIL_COMMAND]),
            "parity-read-tail-after-exit", 0, "Hub process.read tail fixture",
        )
        tail_id = tail_start.get("processId")
        if not tail_id:
            fail("Hub process.read tail fixture", f"process omitted its id: {tail_start}")
        tail_status = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 75, "tools/call",
                {
                    "name": "process.read",
                    "arguments": {
                        "agentId": normal_id,
                        "processId": tail_id,
                        "waitSeconds": 5,
                        "view": "status",
                    },
                },
                "Hub process.read terminal while capture continues",
                timeout=8,
            ),
            "Hub process.read terminal while capture continues",
        )
        if tail_status.get("state") != "completed" or tail_status.get("captureStatus") != "capturing":
            fail("Hub process.read terminal while capture continues", f"process/capture terminal states were not separated: {tail_status}")
        tail_read = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 76, "tools/call",
                {
                    "name": "process.read",
                    "arguments": {"agentId": normal_id, "processId": tail_id, "waitSeconds": 5},
                },
                "Hub process.read captures tail after exit",
                timeout=8,
            ),
            "Hub process.read captures tail after exit",
        )
        if (
            tail_read.get("state") != "completed"
            or tail_read.get("output", {}).get("stdout", {}).get("data") != PROCESS_TAIL_MARKER
        ):
            fail("Hub process.read captures tail after exit", f"post-exit output tail was not retained: {tail_read}")
        tail_settled = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 78, "tools/call",
                {
                    "name": "process.read",
                    "arguments": {
                        "agentId": normal_id, "processId": tail_id, "waitSeconds": 5,
                        "cursor": tail_read["output"]["nextCursor"],
                    },
                },
                "Hub process.read capture EOF after tail",
            ),
            "Hub process.read capture EOF after tail",
        )
        if (
            tail_settled.get("output", {}).get("stdout", {}).get("data") != ""
            or tail_settled.get("output", {}).get("eof") is not True
        ):
            fail("Hub process.read capture EOF after tail", f"tail capture did not settle after its writer exited: {tail_settled}")

        # These waitSeconds=0 fixtures deliberately publish completion events.
        # Settle only their exact sources before testing inline suppression.
        wait_fixture_events: dict[str, str] = {}

        def find_wait_fixture_events() -> bool:
            pending = hub_event_mcp_call(
                hub_port, hub_key, full_session, 79, "event.list",
                {"agentId": normal_id, "status": "pending"},
                "Hub read wait fixture completion events",
            )
            for process_id in (auto_id, tail_id):
                record = event_record_for_source(
                    event_items(pending, "Hub read wait fixture completion events"),
                    "process", process_id,
                    lambda event_id: hub_event_mcp_call(
                        hub_port, hub_key, full_session, 80, "event.get",
                        {"agentId": normal_id, "eventId": event_id},
                        "Hub read wait fixture event provenance",
                    ),
                    "Hub read wait fixture event provenance",
                )
                if record is not None:
                    wait_fixture_events[process_id] = record["eventId"]
            return set(wait_fixture_events) == {auto_id, tail_id}

        wait_until(find_wait_fixture_events, "Hub read wait fixtures", "both completion events")
        marked_wait_events = hub_event_mcp_call(
            hub_port, hub_key, full_session, 81, "event.mark",
            {"agentId": normal_id, "eventIds": list(wait_fixture_events.values())},
            "Hub read wait fixture event cleanup",
        )
        if set(marked_wait_events.get("handledIds", [])) != set(wait_fixture_events.values()):
            fail("Hub read wait fixture event cleanup", f"fixture events were not handled: {marked_wait_events}")
        reports.append("PASS Hub read auto/status waits and post-exit capture; exact fixture completion events settled")

        denied = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 6, "tools/call",
                {"name": "process.exec", "arguments": {
                    "agentId": normal_id,
                    "command": shlex.join(["/usr/bin/echo", "must-be-denied"]),
                    "needConfirm": False, "waitSeconds": 5,
                }},
                "Hub Full policy denied process",
            ),
            "Hub Full policy denied process",
        )
        if (
            denied.get("state") != "rejected"
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
        if {"process.exec", "process.batch", "process.read", "process.cancel", "job.get"}.intersection(names) or leaked:
            fail("Hub Coordinator profile", f"execution or Room tool leaked into tools/list: {sorted(leaked)}")
        hidden = mcp_call(
            coordinator_port,
            coordinator_key,
            coordinator_session,
            3,
            "tools/call",
            {"name": "process.exec", "arguments": {"agentId": normal_id, "command": "/usr/bin/true"}},
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
            "command": f"/usr/bin/pwd; {shlex.join(['/usr/bin/printf', PROCESS_MARKERS[2]])}",
            "cwd": str(normal_workspace),
            "group": "parity-completed",
            "needConfirm": False,
            "waitSeconds": 5,
        }
        validate_instance(document, schemas, "ProcessExecRequest", completed_request, "Hub HTTP process.exec request")
        invalid_exec = dict(completed_request)
        invalid_exec.pop("needConfirm")
        assert_rejected(document, schemas, "ProcessExecRequest", invalid_exec, "Hub HTTP process.exec negative request")
        legacy_exec = {
            "agentId": normal_id,
            "program": "/usr/bin/printf",
            "args": [PROCESS_MARKERS[2]],
            "group": "parity-legacy-exec",
            "needConfirm": False,
            "waitSeconds": 5,
        }
        assert_rejected(
            document, schemas, "ProcessExecRequest", legacy_exec,
            "Hub HTTP process.exec rejects old program/args input",
        )
        legacy_http_response = http_request(
            hub_port,
            "POST",
            "/v1/process/exec",
            legacy_exec,
            {"Authorization": f"Bearer {hub_key}"},
            "Hub HTTP rejects old process.exec input",
        )
        legacy_http_error = legacy_http_response.body.decode("utf-8", errors="replace")
        if (
            legacy_http_response.status != 422
            or "unknown field" not in legacy_http_error
            or "program" not in legacy_http_error
        ):
            fail(
                "Hub HTTP rejects old process.exec input",
                f"expected HTTP 422 for the unknown legacy program field: "
                f"{legacy_http_response.status}, {legacy_http_error!r}",
            )
        legacy_exec_list_response, legacy_exec_list = hub_json(
            hub_port,
            hub_key,
            "GET",
            "/v1/process?" + urlencode({"agentId": normal_id, "group": "parity-legacy-exec"}),
            None,
            "Hub HTTP legacy process.exec has no startup result",
        )
        if legacy_exec_list_response.status != 200:
            fail(
                "Hub HTTP legacy process.exec has no startup result",
                f"HTTP {legacy_exec_list_response.status}: {legacy_exec_list}",
            )
        validate_operation_response(
            document,
            "/v1/process",
            "get",
            200,
            legacy_exec_list,
            "Hub HTTP legacy process.exec has no startup result",
        )
        if legacy_exec_list.get("processes") != []:
            fail(
                "Hub HTTP legacy process.exec has no startup result",
                f"rejected legacy request started a process: {legacy_exec_list}",
            )
        legacy_mcp = mcp_call(
            hub_port, hub_key, full_session, 11, "tools/call",
            {
                "name": "process.exec",
                "arguments": {
                    "agentId": normal_id,
                    "program": "/usr/bin/true",
                    "args": [],
                    "needConfirm": False,
                    "waitSeconds": 5,
                },
            },
            "Hub MCP rejects old process.exec input",
        )
        if "error" not in legacy_mcp and legacy_mcp.get("result", {}).get("isError") is not True:
            fail("Hub MCP rejects old process.exec input", f"legacy program/args call was not rejected: {legacy_mcp}")
        completed = process_request(
            hub_port,
            hub_key,
            normal_id,
            completed_request["command"],
            "parity-completed",
            5,
            "Hub HTTP completed pwd/printf chain",
            cwd=completed_request["cwd"],
        )
        validate_operation_response(document, "/v1/process/exec", "post", 200, completed, "Hub HTTP completed printf")
        completed_id = completed.get("processId")
        if (
            not completed_id
            or completed.get("state") != "completed"
            or completed.get("output", {}).get("stdout", {}).get("data") != f"{normal_workspace.resolve()}\n{PROCESS_MARKERS[2]}"
        ):
            fail("Hub HTTP completed printf", f"expected complete process observation: {completed}")
        if process_response_size(completed) > 4096:
            fail("Hub HTTP completed printf", f"creation response exceeded the configured 4096-byte budget: {completed}")
        assert_event_counts(completed, (0, 0, 0), "Hub HTTP inline completion suppression")
        completed_events_response, completed_events = hub_event_http_call(
            hub_port,
            hub_key,
            document,
            "GET",
            "/v1/events?" + urlencode({"agentId": normal_id, "status": "pending"}),
            None,
            "/v1/events",
            "get",
            "Hub HTTP inline completion event query",
        )
        if event_items(completed_events, "Hub HTTP inline completion event query"):
            fail("Hub HTTP inline completion suppression", f"terminal response created a completion event: {completed_events}")

        active = process_request(
            hub_port, hub_key, normal_id,
            shlex.join(["/usr/bin/sleep", "30"]), "parity-active", 0,
        )
        validate_operation_response(document, "/v1/process/exec", "post", 200, active, "Hub HTTP active process")
        active_id = active.get("processId")
        if not active_id or active.get("state") in {"completed", "failed", "cancelled", "rejected", "timed_out"}:
            fail("Hub HTTP active process", f"sleep30 was not active: {active}")

        status_response, status_body = hub_process_read(
            hub_port,
            hub_key,
            normal_id,
            active_id,
            document,
            schemas,
            "Hub HTTP process.read status view",
            wait_seconds=0,
            view="status",
        )
        if (
            status_body.get("processId") != active_id
            or status_body.get("state") in {"completed", "failed", "cancelled", "rejected", "timed_out"}
            or "output" in status_body
        ):
            fail("Hub HTTP process.read status view", f"status view was not active metadata: {status_body}")

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

        batch_subdir = normal_workspace / "batch-element"
        batch_subdir.mkdir(exist_ok=True)
        batch_payload = {
            "agentId": normal_id,
            "cwd": str(normal_workspace),
            "elements": [
                {
                    "command": f"/usr/bin/pwd; {shlex.join(['/usr/bin/printf', PROCESS_MARKERS[3]])}",
                    "cwd": str(batch_subdir),
                },
                {
                    "command": f"/usr/bin/pwd; {shlex.join(['/usr/bin/printf', PROCESS_MARKERS[4]])}",
                },
            ],
            "needConfirm": False,
            "waitSeconds": 5,
        }
        validate_instance(document, schemas, "ProcessBatchExecRequest", batch_payload, "Hub HTTP process.batch request")
        invalid_batch = dict(batch_payload)
        invalid_batch.pop("needConfirm")
        assert_rejected(document, schemas, "ProcessBatchExecRequest", invalid_batch, "Hub HTTP process.batch negative request")
        legacy_batch = {
            "agentId": normal_id,
            "group": "parity-legacy-batch",
            "cwd": str(normal_workspace),
            "elements": [
                {
                    "program": "/usr/bin/pwd",
                    "args": [],
                    "cwd": str(batch_subdir),
                },
                {"program": "/usr/bin/pwd", "args": []},
            ],
            "needConfirm": False,
            "waitSeconds": 5,
        }
        assert_rejected(
            document, schemas, "ProcessBatchExecRequest", legacy_batch,
            "Hub HTTP process.batch rejects old program/args input",
        )
        legacy_batch_response = http_request(
            hub_port,
            "POST",
            "/v1/process/batch",
            legacy_batch,
            {"Authorization": f"Bearer {hub_key}"},
            "Hub HTTP rejects old process.batch input",
        )
        legacy_batch_error = legacy_batch_response.body.decode("utf-8", errors="replace")
        if (
            legacy_batch_response.status != 422
            or "unknown field" not in legacy_batch_error
            or "program" not in legacy_batch_error
        ):
            fail(
                "Hub HTTP rejects old process.batch input",
                f"expected HTTP 422 for the unknown legacy element program field: "
                f"{legacy_batch_response.status}, {legacy_batch_error!r}",
            )
        legacy_batch_list_response, legacy_batch_list = hub_json(
            hub_port,
            hub_key,
            "GET",
            "/v1/process?" + urlencode({"agentId": normal_id, "group": "parity-legacy-batch"}),
            None,
            "Hub HTTP legacy process.batch has no startup result",
        )
        if legacy_batch_list_response.status != 200:
            fail(
                "Hub HTTP legacy process.batch has no startup result",
                f"HTTP {legacy_batch_list_response.status}: {legacy_batch_list}",
            )
        validate_operation_response(
            document,
            "/v1/process",
            "get",
            200,
            legacy_batch_list,
            "Hub HTTP legacy process.batch has no startup result",
        )
        if legacy_batch_list.get("processes") != []:
            fail(
                "Hub HTTP legacy process.batch has no startup result",
                f"rejected legacy request started processes: {legacy_batch_list}",
            )
        batch_response, batch_body = hub_json(
            hub_port, hub_key, "POST", "/v1/process/batch", batch_payload, "Hub HTTP process.batch"
        )
        if batch_response.status != 200:
            fail("Hub HTTP process.batch", f"HTTP {batch_response.status}: {batch_body}")
        validate_operation_response(document, "/v1/process/batch", "post", 200, batch_body, "Hub HTTP process.batch")
        batch_processes = batch_body.get("processes") if isinstance(batch_body, dict) else None
        if (
            not batch_body.get("batchId")
            or batch_body.get("status") != "completed"
            or not isinstance(batch_processes, list)
            or len(batch_processes) != 2
            or [process.get("state") for process in batch_processes] != ["completed", "completed"]
            or not all(process.get("processId") and process.get("agentId") == normal_id for process in batch_processes)
            or batch_processes[0].get("output", {}).get("stdout", {}).get("data") != f"{batch_subdir.resolve()}\n{PROCESS_MARKERS[3]}"
            or batch_processes[1].get("output", {}).get("stdout", {}).get("data") != f"{normal_workspace.resolve()}\n{PROCESS_MARKERS[4]}"
        ):
            fail("Hub HTTP process.batch", f"ordered complete process observations were not returned: {batch_body}")
        if process_response_size(batch_body) > 4096:
            fail("Hub HTTP process.batch", f"aggregate batch response exceeded the configured 4096-byte budget: {batch_body}")

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
        if (
            mcp_body.get("state") != "completed"
            or not mcp_process_id
            or mcp_body.get("mcpResult", {}).get("status") != "deferred"
        ):
            fail("Hub HTTP mcp.callTool", f"large retained MCP result was not deferred in the bounded response: {mcp_body}")
        bounded_read = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 7, "tools/call",
                {
                    "name": "process.read",
                    "arguments": {"agentId": normal_id, "processId": mcp_process_id, "maxBytes": 4096},
                },
                "Hub Full deferred MCP result",
            ),
            "Hub Full deferred MCP result",
        )
        deferred_result = bounded_read.get("mcpResult", {})
        if (
            deferred_result.get("status") != "deferred"
            or deferred_result.get("bytes", 0) <= 4096
            or "value" in deferred_result
        ):
            fail("Hub Full deferred MCP result", f"large retained result was not reported deferred: {bounded_read}")
        mcp_read = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 70, "tools/call",
                {
                    "name": "process.read",
                    "arguments": {"agentId": normal_id, "processId": mcp_process_id, "maxBytes": 524288},
                },
                "Hub Full explicit MCP result",
            ),
            "Hub Full explicit MCP result",
        )
        mcp_result = mcp_read.get("mcpResult", {})
        mcp_value = mcp_result.get("value", {})
        mcp_info = mcp_value.get("structuredContent", {}) if isinstance(mcp_value, dict) else {}
        mcp_identity = mcp_info.get("identity", {}) if isinstance(mcp_info, dict) else {}
        if (
            mcp_result.get("status") != "included"
            or mcp_identity.get("agentId") != "parity-http"
            or mcp_identity.get("profile") != "normal"
            or mcp_identity.get("transport") != "tunnel-stdio"
        ):
            fail("Hub Full explicit MCP result", f"deferred downstream result was not retrievable intact: {mcp_read}")

        non_log_cursor_call = mcp_call(
            hub_port,
            hub_key,
            full_session,
            80,
            "tools/call",
            {
                "name": "process.read",
                "arguments": {
                    "agentId": normal_id,
                    "processId": mcp_process_id,
                    "cursor": "not-a-log-cursor",
                    "maxBytes": 524288,
                },
            },
            "Hub process.read rejects output cursors for downstream results",
        )
        non_log_cursor_result = non_log_cursor_call.get("result")
        if (
            "error" not in non_log_cursor_call
            and (
                not isinstance(non_log_cursor_result, dict)
                or non_log_cursor_result.get("isError") is not True
            )
        ):
            fail(
                "Hub process.read rejects output cursors for downstream results",
                f"downstream MCP result accepted an output cursor: {non_log_cursor_call}",
            )

        large_mcp_payload = {
            "agentId": normal_id,
            "serverId": "standalone-http",
            "toolName": "file.read",
            "arguments": {"path": "large-mcp-result.png"},
            "waitSeconds": 5,
        }
        validate_instance(
            document, schemas, "McpCallToolRequest", large_mcp_payload,
            "Hub HTTP mcp.callTool non-retained fixture request",
        )
        large_mcp_response, large_mcp_body = confirmed_hub_json(
            hub_port,
            hub_key,
            "POST",
            "/v1/mcp/callTool",
            large_mcp_payload,
            confirmation,
            "Hub HTTP mcp.callTool non-retained fixture",
        )
        if large_mcp_response.status != 200:
            fail("Hub HTTP mcp.callTool non-retained fixture", f"HTTP {large_mcp_response.status}: {large_mcp_body}")
        validate_operation_response(
            document,
            "/v1/mcp/callTool",
            "post",
            200,
            large_mcp_body,
            "Hub HTTP mcp.callTool non-retained fixture",
        )
        large_mcp_id = large_mcp_body.get("processId")
        if not large_mcp_id:
            fail("Hub HTTP mcp.callTool non-retained fixture", f"process omitted its id: {large_mcp_body}")
        not_retained = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 77, "tools/call",
                {
                    "name": "process.read",
                    "arguments": {
                        "agentId": normal_id,
                        "processId": large_mcp_id,
                        "maxBytes": 524288,
                    },
                },
                "Hub Full not-retained MCP result",
                timeout=8,
            ),
            "Hub Full not-retained MCP result",
        )
        not_retained_result = not_retained.get("mcpResult", {})
        if not_retained_result.get("status") != "not_retained" or "value" in not_retained_result:
            fail("Hub Full not-retained MCP result", f"oversized MCP result remained available: {not_retained}")
        reports.append("PASS Hub process.read MCP result deferred, re-retrievable, and explicitly not-retained states")

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
                {
                    "name": "process.read",
                    "arguments": {"agentId": normal_id, "processId": batch_results[0]["processId"], "maxBytes": 524288},
                },
                "Hub Full first batch MCP result",
            ),
            "Hub Full first batch MCP result",
        )
        second_result = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 72, "tools/call",
                {
                    "name": "process.read",
                    "arguments": {"agentId": normal_id, "processId": batch_results[1]["processId"], "maxBytes": 524288},
                },
                "Hub Full second batch MCP result",
            ),
            "Hub Full second batch MCP result",
        )
        first_mcp_result = first_result.get("mcpResult", {})
        second_mcp_result = second_result.get("mcpResult", {})
        first_content = first_mcp_result.get("value", {}).get("structuredContent", {})
        second_content = second_mcp_result.get("value", {}).get("structuredContent", {})
        identity = first_content.get("identity", {})
        if (
            batch_results[0]["processId"] == batch_results[1]["processId"]
            or first_mcp_result.get("status") != "included"
            or second_mcp_result.get("status") != "included"
            or identity.get("agentId") != "parity-http"
            or identity.get("profile") != "normal"
            or identity.get("transport") != "tunnel-stdio"
            or not any(skill.get("id") == "demo" for skill in second_content.get("skills", []))
        ):
            fail("Hub HTTP mcp.batch", f"distinct downstream info/skills results were not in request order: {mcp_batch_body}")
        reports.append("PASS Hub HTTP process.batch and real downstream mcp.callTool/mcp.batch with distinct ordered results")

        retained_batch_response, retained_batch = confirmed_hub_json(
            hub_port, hub_key, "POST", "/v1/mcp/batch",
            {
                "agentId": normal_id,
                "calls": [
                    {
                        "id": f"image-{index}", "serverId": "standalone-http",
                        "toolName": "file.read",
                        "arguments": {"path": "retained-mcp-result.png"},
                    }
                    for index in range(6)
                ],
                "waitSeconds": 5,
            },
            confirmation, "Hub MCP batch aggregate retention boundary",
        )
        if retained_batch_response.status != 200:
            fail("Hub MCP batch aggregate retention boundary", f"HTTP {retained_batch_response.status}: {retained_batch}")
        validate_operation_response(
            document, "/v1/mcp/batch", "post", 200, retained_batch,
            "Hub MCP batch aggregate retention boundary",
        )
        retained_children = retained_batch.get("results", [])
        if (
            retained_batch.get("status") != "completed"
            or len(retained_children) != 6
            or any(child.get("mcpResult", {}).get("status") != "deferred" for child in retained_children)
            or sum(child.get("mcpResult", {}).get("bytes", 0) for child in retained_children) <= 2 * 1024 * 1024
            or process_response_size(retained_batch) > 4096
        ):
            fail("Hub MCP batch aggregate retention boundary", f"retained aggregate overflow was not deferred within budget: {retained_batch}")
        expected_image = (
            Path(json.loads(http_config.read_text())["workspaceRoot"]) / "retained-mcp-result.png"
        ).read_bytes()
        for child in retained_children:
            _, restored = hub_process_read(
                hub_port, hub_key, normal_id, child["processId"], document, schemas,
                "Hub MCP aggregate-omitted result recovery", max_bytes=1048576,
            )
            result = restored.get("mcpResult", {})
            images = [
                block for block in result.get("value", {}).get("content", [])
                if block.get("type") == "image"
            ]
            if (
                result.get("status") != "included" or len(images) != 1
                or base64.b64decode(images[0].get("data", ""), validate=True) != expected_image
                or process_response_size(restored) > 1048576
            ):
                fail("Hub MCP aggregate-omitted result recovery", f"retained image was not recovered intact for {child['processId']}")
        reports.append("PASS Hub MCP batch exceeding internal 2 MiB aggregate reports deferred and recovers all six retained images intact")

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
            or room_run_body.get("agentId") != room_id
            or room_run_body.get("state") != "completed"
            or "inline-skill" not in room_run_body.get("output", {}).get("stdout", {}).get("data", "")
        ):
            fail("Hub Room skills.run", f"active Room routing did not preserve process identity and output: {room_run_body}")
        room_process_id = room_run_body["processId"]
        room_agent_id = room_run_body["agentId"]
        room_status_response, room_status = hub_process_read(
            hub_port,
            hub_key,
            room_agent_id,
            room_process_id,
            document,
            schemas,
            "Hub Room process.read status view",
            wait_seconds=0,
            view="status",
        )
        if (
            room_status.get("agentId") != room_agent_id
            or room_status.get("processId") != room_process_id
            or "output" in room_status
        ):
            fail("Hub Room process.read status view", f"status view did not retain the skill's actual Agent identity: {room_status}")
        reports.append("PASS Hub Room HTTP inline skills install/get/run unified response and pinned active-room process identity")

        pagination_ids: list[str] = []
        for _ in range(101):
            value = process_request(
                hub_port, hub_key, normal_id, "/usr/bin/true", "parity-page", 0,
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
        reports.append("PASS Hub HTTP process.read/cancel and process.list typed group error, 50/100/1 pages, and cursor")
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
            or "events" in cached_mcp_status
            or any(key in cached_mcp_status for key in ("output", "mcpResult"))
        ):
            fail("Hub Full cached process status", f"cache-only MCP status exposed process artifacts or lost identity: {cached_mcp_status}")
        cached_mcp_list = json_result(
            mcp_call(
                hub_port, hub_key, full_session, 9, "tools/call",
                {"name": "hub.process.list", "arguments": {"agentId": normal_id, "group": "parity-page", "limit": 1}},
                "Hub Full cached process list",
            ),
            "Hub Full cached process list",
        )
        if (
            "events" in cached_mcp_list
            or completed_id not in json.dumps(cached_mcp_list)
            or any(key in json.dumps(cached_mcp_list) for key in ('"stdout"', '"stderr"', '"result"'))
        ):
            fail("Hub Full cached process list", f"cache-only MCP list did not return status metadata: {cached_mcp_list}")

        offline_read_response, offline_read_body = hub_process_read(
            hub_port,
            hub_key,
            normal_id,
            completed_id,
            document,
            schemas,
            "Hub offline process.read",
            expected_status=503,
        )
        offline_cached = offline_read_body.get("cached", {})
        if (
            offline_read_response.status != 503
            or offline_read_body.get("status") != "unavailable"
            or offline_read_body.get("error", {}).get("code") != "process_read_unavailable"
            or offline_cached.get("processId") != completed_id
            or offline_read_body.get("freshness") not in {"cached", "stale"}
            or any(key in offline_read_body for key in ("output", "mcpResult", "events"))
        ):
            fail("Hub offline process.read", f"expected typed body-free live-read unavailable response: {offline_read_response.status}, {offline_read_body}")

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
            or "events" in offline_cancel_body
        ):
            fail("Hub offline process.cancel", f"expected typed unavailable response without cached success: {offline_cancel_response.status}, {offline_cancel_body}")
        validate_operation_response(document, "/v1/process/{processId}/cancel", "post", 502, offline_cancel_body, "Hub offline process.cancel")
        reports.append("PASS cache-only hub.process.status, typed offline process.read, and unavailable cancel")
    finally:
        for process in (
            coordinator_process,
            normal_process,
            reporting_process,
            room_process,
            hub_process,
            timeout_agent,
            delta_agent,
            crash_agent,
            local_process,
            event_agent_process,
        ):
            if process is not None:
                process.stop()
        if confirmation is not None:
            confirmation.close()
        for relay in (timeout_relay, delta_relay, crash_relay):
            if relay is not None:
                relay.close()


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
