#!/usr/bin/env python3
"""Copy one stopped Agent's legacy private state into the flat state layout."""

from __future__ import annotations

import argparse
import contextlib
from datetime import datetime
import errno
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import sqlite3
import stat
import sys
import tempfile
from typing import Iterator


STATE_OBJECTS = ("events.sqlite3", "process.sqlite3", "active-skills.json", "skill-installs")
SQLITE_SUFFIXES = ("-wal", "-shm", "-journal")
EVENT_COLUMNS = {
    "event_id", "message", "severity", "created_at", "status", "source_kind",
    "source_ref", "shown_count", "expires_at", "terminal_at",
}
SOURCE_COLUMNS = {
    "source_kind", "source_ref", "response_state", "settled_at", "policy_json",
    "completion_seen", "completion_event_type", "completion_message", "completed_at", "notified",
}
PROCESS_COLUMNS = {
    "process_id", "agent_id", "group_name", "kind", "state", "created_at", "started_at",
    "updated_at", "finished_at", "info_json", "detail_json", "stdout_bytes",
    "stdout_start_offset", "stdout_end_offset", "stderr_bytes", "stderr_start_offset",
    "stderr_end_offset", "capture_status", "capture_error",
}


class MigrationError(Exception):
    pass


def checked_path(path: Path) -> Path:
    """Reject symbolic links in every existing path component, including parents."""
    path = Path(os.path.abspath(path))
    for part in (path, *path.parents):
        try:
            mode = part.lstat().st_mode
        except FileNotFoundError:
            continue
        if stat.S_ISLNK(mode):
            raise MigrationError(f"symbolic link is not allowed: {part}")
    return path


def exists(path: Path) -> bool:
    return os.path.lexists(path)


def new_state_objects(state: Path) -> list[str]:
    names = list(STATE_OBJECTS)
    for name in STATE_OBJECTS[:2]:
        names.extend(name + suffix for suffix in SQLITE_SUFFIXES)
    return [name for name in names if exists(state / name)]


def private_dir(path: Path) -> None:
    checked_path(path)
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    if not path.is_dir():
        raise MigrationError(f"directory required: {path}")
    path.chmod(0o700)


@contextlib.contextmanager
def exclusive_lock(path: Path, label: str) -> Iterator[None]:
    checked_path(path)
    fd = os.open(path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        os.fchmod(fd, 0o600)
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise MigrationError(f"regular lock file required: {path}")
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise MigrationError(f"{label}; stop the Agent before migration") from error
        yield
    finally:
        os.close(fd)


def read_json(path: Path) -> object:
    checked_path(path)
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(fd, "r", encoding="utf-8") as stream:
        if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
            raise MigrationError(f"regular JSON file required: {path}")
        return json.load(stream)


def legacy_key(agent_id: str) -> str:
    if not agent_id.strip():
        raise MigrationError("config agentId must be a nonempty string")
    if len(agent_id.encode("utf-8")) <= 128 and re.fullmatch(r"[A-Za-z0-9._-]+", agent_id):
        if agent_id in (".", ".."):
            raise MigrationError("agentId does not identify a distinct legacy state directory")
        return agent_id
    return "id-" + hashlib.sha256(agent_id.encode("utf-8")).hexdigest()


def check_socket(agentic: Path, agent_id: str) -> None:
    # Old local control only accepts this ASCII alphabet. Other IDs have no socket.
    if not re.fullmatch(r"[A-Za-z0-9._-]+", agent_id) or agent_id in (".", ".."):
        return
    path = checked_path(agentic / "runtime" / "agent" / agent_id / "mcp.sock")
    if not exists(path):
        return
    if not stat.S_ISSOCK(path.lstat().st_mode):
        raise MigrationError(f"unexpected local socket object: {path}")
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as probe:
        probe.settimeout(0.5)
        try:
            probe.connect(str(path))
        except OSError as error:
            if error.errno not in (errno.ENOENT, errno.ECONNREFUSED):
                raise MigrationError(f"cannot verify that the Agent socket is inactive: {path}") from error
        else:
            raise MigrationError(f"Agent socket is active: {path}; stop the Agent before migration")


def copy_private(source: Path, target: Path) -> None:
    checked_path(source)
    mode = source.lstat().st_mode
    if stat.S_ISDIR(mode):
        target.mkdir(mode=0o700)
        for child in sorted(source.iterdir()):
            copy_private(child, target / child.name)
    elif stat.S_ISREG(mode):
        source_fd = os.open(source, os.O_RDONLY | os.O_NOFOLLOW)
        with os.fdopen(source_fd, "rb") as reader:
            if not stat.S_ISREG(os.fstat(reader.fileno()).st_mode):
                raise MigrationError(f"source changed while copying: {source}")
            target_fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            with os.fdopen(target_fd, "wb") as writer:
                shutil.copyfileobj(reader, writer)
                writer.flush()
                os.fsync(writer.fileno())
    else:
        raise MigrationError(f"only regular files and directories can be migrated: {source}")


def database_snapshot(source: Path, target: Path, scratch: Path) -> None:
    """Recover a copied WAL in staging; never open or checkpoint the original DB."""
    copy = scratch / source.name
    copy_private(source, copy)
    for suffix in SQLITE_SUFFIXES:
        sidecar = source.with_name(source.name + suffix)
        if exists(sidecar):
            copy_private(sidecar, copy.with_name(copy.name + suffix))
    with contextlib.closing(sqlite3.connect(copy)) as original:
        with contextlib.closing(sqlite3.connect(target)) as snapshot:
            original.backup(snapshot)
    target.chmod(0o600)


def validate_database(connection: sqlite3.Connection) -> None:
    result = connection.execute("PRAGMA integrity_check").fetchall()
    if result != [("ok",)]:
        raise MigrationError("SQLite integrity check failed")
    if connection.execute("PRAGMA foreign_key_check").fetchone() is not None:
        raise MigrationError("SQLite foreign key check failed")


def columns(connection: sqlite3.Connection, table: str) -> set[str]:
    return {row[1] for row in connection.execute(f'PRAGMA table_info("{table}")')}


def migrate_events(path: Path) -> None:
    with contextlib.closing(sqlite3.connect(path)) as connection:
        validate_database(connection)
        version = connection.execute("PRAGMA user_version").fetchone()[0]
        if version not in (1, 2, 3):
            raise MigrationError(f"unsupported legacy event schema: {version}")
        if not SOURCE_COLUMNS <= columns(connection, "internal_sources") or not EVENT_COLUMNS <= columns(connection, "events"):
            raise MigrationError("legacy event schema is incomplete")
        with connection:
            if version == 1:
                for column in ("origin_run_id", "origin_request_id", "origin_command_hash"):
                    connection.execute(f"ALTER TABLE internal_sources ADD COLUMN {column} TEXT")
            if not {"origin_run_id", "origin_request_id", "origin_command_hash"} <= columns(connection, "internal_sources"):
                raise MigrationError("legacy event origin schema is incomplete")
            connection.execute("DROP TABLE IF EXISTS event_store_metadata")
            connection.execute("PRAGMA user_version=4")
        validate_database(connection)


def process_json(raw: str) -> object:
    def reject_constant(value: str) -> None:
        raise MigrationError(f"non-JSON number in process history: {value}")

    return json.loads(raw, parse_constant=reject_constant)


def timestamp(value: object, field: str) -> None:
    if not isinstance(value, str):
        raise MigrationError(f"process {field} must be an RFC3339 timestamp")
    match = re.fullmatch(
        r"([0-9]{4}-[0-9]{2}-[0-9]{2})[Tt ]([0-9]{2}:[0-9]{2}:)([0-9]{2})"
        r"(\.[0-9]+)?([Zz]|[+-][0-9]{2}:[0-9]{2})", value,
    )
    if match is None:
        raise MigrationError(f"process {field} must be an RFC3339 timestamp")
    zone = match[5]
    if zone not in ("Z", "z") and (int(zone[1:3]) > 23 or int(zone[4:6]) > 59):
        raise MigrationError(f"process {field} has an invalid UTC offset")
    # Chrono accepts RFC3339 leap seconds; datetime is used only to check the date,
    # time and offset after substituting :59 for the representable leap second.
    second = "59" if match[3] == "60" else match[3]
    fraction = (match[4] or "")[:7]
    normalized_zone = "+00:00" if zone in ("Z", "z") else zone
    normalized = match[1] + "T" + match[2] + second + fraction + normalized_zone
    try:
        datetime.fromisoformat(normalized)
    except ValueError as error:
        raise MigrationError(f"process {field} has an invalid timestamp") from error


def optional_strings(value: dict, fields: tuple[str, ...]) -> None:
    for field in fields:
        if value.get(field) is not None and not isinstance(value[field], str):
            raise MigrationError(f"process {field} must be a string or null")


def optional_integer(value: dict, field: str, minimum: int, maximum: int) -> None:
    number = value.get(field)
    if number is not None and (type(number) is not int or not minimum <= number <= maximum):
        raise MigrationError(f"process {field} is outside its integer range")


def validate_info(value: object, process_id: str) -> None:
    if not isinstance(value, dict):
        raise MigrationError("process info_json must contain an object")
    if not isinstance(value.get("processId"), str) or value["processId"] != process_id:
        raise MigrationError("process JSON processId does not match SQL process_id")
    for field, allowed in (
        ("kind", {"command", "skill", "mcp"}),
        ("state", {"queued", "waiting_confirmation", "starting", "running", "completed",
                   "failed", "rejected", "cancel_requested", "cancelled", "timed_out",
                   "detached", "unknown_after_restart", "skipped"}),
        ("captureStatus", {"not_started", "capturing", "complete", "incomplete", "not_applicable"}),
    ):
        if not isinstance(value.get(field), str) or value[field] not in allowed:
            raise MigrationError(f"process {field} is missing or invalid")
    for field in ("createdAt", "updatedAt"):
        timestamp(value.get(field), field)
    for field in ("startedAt", "finishedAt"):
        if value.get(field) is not None:
            timestamp(value[field], field)
    optional_strings(value, (
        "group", "batchId", "batchCallId", "program", "workingDirectory", "commandPreview",
        "rejectReason", "skillId", "skillPath", "installedDigest", "mcpServerId", "mcpToolName",
        "cancelOutcome", "terminationEvidence", "captureError",
    ))
    optional_integer(value, "batchIndex", 0, sys.maxsize * 2 + 1)
    optional_integer(value, "exitCode", -(2 ** 31), 2 ** 31 - 1)
    if "args" in value and (not isinstance(value["args"], list) or not all(isinstance(arg, str) for arg in value["args"])):
        raise MigrationError("process args must be an array of strings")
    if "cancelRequested" in value and type(value["cancelRequested"]) is not bool:
        raise MigrationError("process cancelRequested must be a boolean")


def local_info(raw: str, process_id: str) -> str:
    value = process_json(raw)
    validate_info(value, process_id)
    value.pop("agentId", None)
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def local_detail(raw: str | None, process_id: str) -> str | None:
    if raw is None:
        return None
    value = process_json(raw)
    if not isinstance(value, dict) or not isinstance(value.get("process"), dict):
        raise MigrationError("process detail_json must contain a process object")
    validate_info(value["process"], process_id)
    if type(value.get("detailAvailable")) is not bool:
        raise MigrationError("process detailAvailable must be a boolean")
    if "resultAvailable" in value and type(value["resultAvailable"]) is not bool:
        raise MigrationError("process resultAvailable must be a boolean")
    optional_integer(value, "resultBytes", 0, sys.maxsize * 2 + 1)
    optional_strings(value, ("resultSha256", "resultPreview"))
    error = value.get("error")
    if error is not None and (
        not isinstance(error, dict)
        or not isinstance(error.get("code"), str)
        or not isinstance(error.get("message"), str)
    ):
        raise MigrationError("process error must contain string code and message")
    value["process"].pop("agentId", None)
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def migrate_processes(path: Path) -> None:
    if sqlite3.sqlite_version_info < (3, 35, 0):
        raise MigrationError("process migration requires Python linked to SQLite 3.35 or newer")
    with contextlib.closing(sqlite3.connect(path)) as connection:
        validate_database(connection)
        version = connection.execute("PRAGMA user_version").fetchone()[0]
        if version != 1:
            raise MigrationError(f"unsupported legacy process schema: {version}")
        if not PROCESS_COLUMNS <= columns(connection, "processes"):
            raise MigrationError("legacy process schema is incomplete")
        if not {"process_id", "event_type", "message", "completed_at"} <= columns(connection, "process_event_completions"):
            raise MigrationError("legacy process completion schema is incomplete")
        with connection:
            rows = connection.execute("SELECT process_id,info_json,detail_json FROM processes")
            while batch := rows.fetchmany(32):
                for process_id, info, detail in batch:
                    connection.execute(
                        "UPDATE processes SET info_json=?,detail_json=? WHERE process_id=?",
                        (local_info(info, process_id), local_detail(detail, process_id), process_id),
                    )
            connection.execute("ALTER TABLE processes DROP COLUMN agent_id")
            connection.execute("PRAGMA user_version=2")
        validate_database(connection)


def sync_directory(path: Path) -> None:
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def sync_tree(path: Path) -> None:
    for child in path.iterdir():
        if child.is_dir():
            sync_tree(child)
        else:
            fd = os.open(child, os.O_RDONLY | os.O_NOFOLLOW)
            try:
                os.fchmod(fd, 0o600)
                os.fsync(fd)
            finally:
                os.close(fd)
    path.chmod(0o700)
    sync_directory(path)


def publish(staging: Path, state: Path, names: list[str]) -> bool:
    if new_state_objects(state):
        return False
    published: list[str] = []
    try:
        for name in names:
            if exists(state / name):
                raise MigrationError(f"target appeared during migration: {name}")
            # Record intent before rename: a signal can be delivered immediately after
            # the OS moved the object, before Python executes the next instruction.
            published.append(name)
            os.rename(staging / name, state / name)
        sync_directory(state)
    except BaseException:
        # Restore the complete staged set on ordinary errors and interruptions.
        previous_mask = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGINT, signal.SIGTERM})
        try:
            for name in reversed(published):
                if not exists(staging / name) and exists(state / name):
                    os.rename(state / name, staging / name)
            sync_directory(state)
        finally:
            signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
        raise
    return True


def migrate(config_path: Path, agentic: Path, dry_run: bool) -> int:
    checked_path(agentic)
    state = checked_path(agentic / "state")
    present = new_state_objects(state)
    if present:
        print("skip: new state already exists (" + ", ".join(present) + ")")
        return 0
    # The config directory already exists; do not initialize any Agent configuration.
    config_path = checked_path(config_path)
    if not config_path.is_file():
        raise MigrationError(f"config file does not exist: {config_path}")
    private_dir(agentic)
    with exclusive_lock(agentic / ".agent-state-migration.lock", "another migration is running"):
        if new_state_objects(state):
            print("skip: new state appeared while acquiring the migration lock")
            return 0
        with exclusive_lock(Path(str(config_path) + ".run.lock"), "Agent run lock is held"):
            config = read_json(config_path)
            if not isinstance(config, dict) or not isinstance(config.get("agentId"), str):
                raise MigrationError("config must contain string agentId")
            agent_id = config["agentId"]
            source = checked_path(state / "agent" / legacy_key(agent_id))
            check_socket(agentic, agent_id)
            if not source.is_dir():
                raise MigrationError(f"selected legacy state directory does not exist: {source}")
            names = [name for name in STATE_OBJECTS if exists(source / name)]
            for name in names:
                checked_path(source / name)
                expected_dir = name == "skill-installs"
                if (source / name).is_dir() != expected_dir:
                    raise MigrationError(f"unexpected state object type: {source / name}")
            jobs = [name for name in ("jobs.sqlite3", *("jobs.sqlite3" + suffix for suffix in SQLITE_SUFFIXES)) if exists(source / name)]
            if jobs:
                print("not imported: legacy jobs database (" + ", ".join(jobs) + "); original retained")
            known = set(STATE_OBJECTS) | set(jobs)
            known.update(name + suffix for name in STATE_OBJECTS[:2] for suffix in SQLITE_SUFFIXES)
            unknown = [entry.name for entry in source.iterdir() if entry.name not in known]
            if unknown:
                print("not imported: unknown legacy objects (" + ", ".join(sorted(unknown)) + "); originals retained")
            if not names:
                print("skip: selected legacy directory has no supported state objects")
                return 0
            # Stage on the target filesystem so each publication rename is atomic.
            with tempfile.TemporaryDirectory(prefix=".state-migration-", dir=agentic) as temporary:
                scratch = Path(temporary)
                staging = scratch / "new"
                staging.mkdir(mode=0o700)
                for name in names:
                    if name.endswith(".sqlite3"):
                        database_snapshot(source / name, staging / name, scratch)
                    else:
                        copy_private(source / name, staging / name)
                if "events.sqlite3" in names:
                    migrate_events(staging / "events.sqlite3")
                if "process.sqlite3" in names:
                    migrate_processes(staging / "process.sqlite3")
                if "active-skills.json" in names:
                    read_json(staging / "active-skills.json")
                sync_tree(staging)
                if dry_run:
                    print("dry-run: copied, converted and validated " + ", ".join(names) + "; no new state published")
                    return 0
                check_socket(agentic, agent_id)
                private_dir(state)
                if not publish(staging, state, names):
                    print("skip: new state appeared before publication")
                    return 0
                print("migrated: " + ", ".join(names))
                print(f"original retained for rollback: {source}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, help="legacy config (default: $HOME/.agentic_gpt/config.json)")
    parser.add_argument("--dry-run", action="store_true", help="copy, convert and validate without publishing new state")
    args = parser.parse_args()
    home = os.environ.get("HOME")
    if not home:
        print("error: HOME is required", file=sys.stderr)
        return 1
    agentic = Path(home) / ".agentic_gpt"
    def interrupted(signum: int, _frame: object) -> None:
        raise MigrationError(f"migration interrupted by signal {signum}")

    signal.signal(signal.SIGTERM, interrupted)
    try:
        return migrate(args.config or agentic / "config.json", agentic, args.dry_run)
    except (MigrationError, OSError, sqlite3.Error, ValueError, TypeError, KeyboardInterrupt) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
