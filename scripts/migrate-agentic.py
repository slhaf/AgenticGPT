#!/usr/bin/env python3
"""迁移 Agentic 配置与私有状态；自动读取 ~/.agentic_gpt/config.json，--write 才写入。"""
from __future__ import annotations

import argparse
import contextlib
import copy
from contextlib import contextmanager
from datetime import datetime, timezone
import errno
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import signal
import socket
import sqlite3
import stat
import sys
import tempfile
from typing import Iterator
import uuid

ACTIONS = ("allow", "confirm", "deny")
OLD_CONFIRM = {
    "normal": ["sudo", "rm", "mv", "chmod", "chown", "mount", "systemctl", "service",
               "docker", "scp", "curl", "wget", "bash", "sh", "zsh", "fish", "perl", "ruby"],
    "room": ["sudo", "mount", "systemctl", "service", "scp"],
}
OLD_DENY = ["su", "mkfs", "dd", "ssh"]
OLD_READ = ["~/.cache", "~/.local/share", "/etc/os-release", "/proc/meminfo", "/proc/cpuinfo",
            "/proc/loadavg", "/proc/uptime", "/sys/class/power_supply", "/sys/class/thermal"]
OLD_WRITE = ["~/Documents", "~/Downloads", "/tmp"]
OLD_PATH_DENY = ["~/.ssh", "~/.gnupg", "~/.local/share/keyrings", "~/.password-store", "~/.mozilla",
                 "~/.config/google-chrome", "~/.config/chromium", "~/.config/Code/User/globalStorage",
                 "~/.npmrc", "~/.pypirc", "~/.cargo/credentials", "~/.docker/config.json", "~/.kube",
                 "~/.aws", "~/.config/gcloud", "~/.config/gh", "~/.config/hub", "~/.config/clash",
                 "~/.config/clash-verge"]


class MigrationError(ValueError):
    pass


def unique(values: list[str]) -> list[str]:
    return list(dict.fromkeys(values))


def object_value(value: object, label: str) -> dict:
    if not isinstance(value, dict):
        raise MigrationError(f"{label} 必须是对象")
    return value


def strings(value: object, label: str) -> list[str]:
    if not isinstance(value, list) or any(not isinstance(item, str) or "\0" in item for item in value):
        raise MigrationError(f"{label} 必须是无 NUL 的字符串数组")
    return value


def concrete_path(value: object, home: Path, label: str) -> str:
    if not isinstance(value, str) or not value or "\0" in value:
        raise MigrationError(f"{label} 必须是非空路径字符串")
    if "$" in value:
        raise MigrationError(f"{label} 使用环境变量；请先显式填写具体绝对路径")
    if value == "~" or value.startswith("~/"):
        value = str(home / value[2:]) if value != "~" else str(home)
    elif value.startswith("~"):
        raise MigrationError(f"{label} 不支持 ~user；请填写具体绝对路径")
    path = Path(value)
    if not path.is_absolute():
        raise MigrationError(f"{label} 必须是绝对路径")
    try:
        return str(path.resolve(strict=False))
    except (OSError, RuntimeError) as error:
        raise MigrationError(f"{label} 无法规范化路径") from error


def subtree(root: str) -> str:
    escaped = root.replace("\\", "\\\\").replace("*", "\\*")
    return escaped.rstrip("/") + "/**"


def argv_pattern(program: str, args: list[str]) -> str:
    return shlex.join([program, *args]) + " **"


def new_policy(value: object, default: str, label: str) -> dict:
    value = object_value(value, label)
    if set(value) - {"defaultAction", *ACTIONS}:
        raise MigrationError(f"{label} 包含未知策略字段")
    action = value.get("defaultAction", default)
    if action not in ACTIONS:
        raise MigrationError(f"{label}.defaultAction 必须是 allow、confirm 或 deny")
    return {"defaultAction": action, **{key: strings(value.get(key, []), f"{label}.{key}") for key in ACTIONS}}


def migrate_config(config: dict, home: Path) -> tuple[dict, dict]:
    result = copy.deepcopy(config)
    notes: list[str] = []
    conflicts: list[str] = []
    profile = config.get("profile", "normal")
    if not isinstance(profile, str) or profile not in OLD_CONFIRM:
        raise MigrationError("profile 必须是 normal 或 room")
    if "profile" not in config:
        notes.append("缺少 profile：按历史初始化默认 normal 迁移内置预设。")
    workspace_raw = config.get("workspaceRoot")
    if workspace_raw is None:
        workspace_raw = str(home / ".agentic_gpt" / "workspace")
        notes.append("缺少 workspaceRoot：按当前用户 HOME 下历史初始化目录 .agentic_gpt/workspace 补齐。")
    workspace = concrete_path(workspace_raw, home, "workspaceRoot")
    if config.get("workspaceRoot") is None:
        result["workspaceRoot"] = workspace
        notes.append("显式写入 workspaceRoot；请核对工作区位置。")

    policy = object_value(config.get("policy", {}), "policy")
    if "defaultAction" in policy:
        result["policy"] = new_policy(policy, "confirm", "policy")
    else:
        if set(policy) - set(ACTIONS):
            raise MigrationError("旧 policy 包含未知字段或混合形状")
        if "policy" not in config:
            notes.append("缺少 policy：按历史空配置规则恢复，并迁入原 profile 的内置预设。")
        converted = {"defaultAction": "confirm", **{action: [] for action in ACTIONS}}
        old_rules: dict[str, list[tuple[str, list[str]]]] = {action: [] for action in ACTIONS}
        for action in ACTIONS:
            rules = policy.get(action, [])
            if not isinstance(rules, list):
                raise MigrationError(f"policy.{action} 必须是旧规则数组")
            for index, rule in enumerate(rules):
                label = f"policy.{action}[{index}]"
                rule = object_value(rule, label)
                if set(rule) != {"program", "argsPrefix"}:
                    raise MigrationError(f"{label} 必须仅含 program 和 argsPrefix；拒绝未知／混合规则形状")
                program = rule["program"]
                if not isinstance(program, str) or not program or "\0" in program:
                    raise MigrationError(f"{label}.program 必须是非空且无 NUL 的字符串")
                args = strings(rule["argsPrefix"], f"{label}.argsPrefix")
                old_rules[action].append((program, args))
                converted[action].append(argv_pattern(program, args))
        builtins = {"confirm": [(program, []) for program in OLD_CONFIRM[profile]],
                    "deny": [(program, []) for program in OLD_DENY]}
        if profile == "normal":
            builtins["confirm"] += [("python", ["-c"]), ("node", ["-e"])]
        for action, rules in builtins.items():
            converted[action] += [argv_pattern(program, args) for program, args in rules]
        for previous_action in ("allow", "confirm"):
            for index, (program, args) in enumerate(old_rules[previous_action]):
                for builtin_action, rules in builtins.items():
                    if ACTIONS.index(builtin_action) <= ACTIONS.index(previous_action):
                        continue
                    for builtin_program, prefix in rules:
                        overlap = args[:min(len(args), len(prefix))] == prefix[:min(len(args), len(prefix))]
                        if program == builtin_program and overlap:
                            conflicts.append(f"原配置 policy.{previous_action}[{index}] 可覆盖内置 {builtin_action}:{builtin_program}；新优先级下 {builtin_action} 优先，需人工审查。")
        result["policy"] = {key: unique(value) if isinstance(value, list) else value for key, value in converted.items()}
        notes.append(f"已将 {profile} 历史内置 confirm/deny 预设写入显式规则；运行时不再保留内置覆盖逻辑。未匹配命令默认 confirm。")

    paths = object_value(config.get("pathPolicy", {}), "pathPolicy")
    legacy_paths = not ({"read", "write"} & set(paths))
    baseline_read: list[str] = []
    baseline_write: list[str] = []
    if not legacy_paths:
        if set(paths) - {"read", "write"} or not {"read", "write"} <= set(paths):
            raise MigrationError("pathPolicy 新旧形状混合或缺少 read/write")
        result["pathPolicy"] = {key: new_policy(paths[key], "deny", f"pathPolicy.{key}") for key in ("read", "write")}
    else:
        if set(paths) - {"readOnlyRoots", "writeRoots", "denyRoots"}:
            raise MigrationError("旧 pathPolicy 包含未知字段")
        defaults = {"readOnlyRoots": OLD_READ, "writeRoots": [workspace, *OLD_WRITE], "denyRoots": OLD_PATH_DENY}
        roots: dict[str, list[str]] = {}
        for key, fallback in defaults.items():
            if key not in paths:
                notes.append(f"缺少 pathPolicy.{key}：按历史初始化预设补齐；其中 ~ 使用当前用户 HOME。")
            values = strings(paths.get(key, fallback), f"pathPolicy.{key}")
            roots[key] = unique([concrete_path(value, home, f"pathPolicy.{key}") for value in values])
        baseline_write = unique([workspace, *roots["writeRoots"]])
        baseline_read = unique([*baseline_write, *roots["readOnlyRoots"]])
        denied = [subtree(path) for path in roots["denyRoots"]]
        result["pathPolicy"] = {
            "read": {"defaultAction": "deny", "allow": [subtree(path) for path in baseline_read], "confirm": [], "deny": denied},
            "write": {"defaultAction": "deny", "allow": [subtree(path) for path in baseline_write], "confirm": [], "deny": denied},
        }
        notes.append("旧 workspace 自动可写改为显式 read/write allow；旧路径根转换为规范路径子树模式，原根字段已删除。")

    sandbox = object_value(config.get("sandbox", {}), "sandbox")
    migrated_sandbox = copy.deepcopy(sandbox)
    for key, baseline in (("readPaths", baseline_read), ("writePaths", baseline_write)):
        if key in sandbox:
            migrated_sandbox[key] = unique([concrete_path(value, home, f"sandbox.{key}") for value in strings(sandbox[key], f"sandbox.{key}")])
        else:
            migrated_sandbox[key] = baseline
            notes.append(f"sandbox.{key} 已从旧具体根目录生成；不从全局通配策略推导挂载。" if legacy_paths else f"新策略配置缺少 sandbox.{key}：保留新默认空挂载列表，不从策略推导。")
    result["sandbox"] = migrated_sandbox
    notes.append("策略优先级固定 deny > confirm > allow > defaultAction；主动确认可单次覆盖拒绝。迁移不会模拟旧运行时优先级。")
    before = authorization_view(config)
    after = authorization_view(result)
    raw_before = authorization_fields(config)
    raw_after = authorization_fields(result)
    changes = {key: {"before": before.get(key), "after": after.get(key)} for key in after if raw_before.get(key) != raw_after[key]}
    return result, {"changes": changes, "assumptionsAndChanges": notes, "conflicts": unique(conflicts)}


def authorization_fields(config: dict) -> dict:
    sandbox = config.get("sandbox")
    return {"workspaceRoot": config.get("workspaceRoot"), "policy": config.get("policy"),
            "pathPolicy": config.get("pathPolicy"),
            "sandboxPaths": {key: sandbox.get(key) for key in ("readPaths", "writePaths")} if isinstance(sandbox, dict) else None}


def command_policy_summary(policy: object) -> object:
    if not isinstance(policy, dict):
        return None
    summary = {"defaultAction": policy.get("defaultAction")}
    for action in ACTIONS:
        rules = policy.get(action)
        if rules is None:
            summary[action] = None
            continue
        summarized = []
        for index, rule in enumerate(rules):
            reference = f"policy.{action}[{index}]"
            if isinstance(rule, dict):
                summarized.append({"rule": reference, "program": rule.get("program"),
                                   "argsCount": len(rule.get("argsPrefix", []))})
            else:
                summarized.append({"rule": reference, "pattern": "[redacted]"})
        summary[action] = summarized
    return summary


def authorization_view(config: dict) -> dict:
    view = authorization_fields(config)
    view["policy"] = command_policy_summary(config.get("policy"))
    return view


def json_object(pairs: list[tuple[str, object]]) -> dict:
    result = {}
    for key, value in pairs:
        if key in result:
            raise MigrationError("配置存在重复 JSON 字段，拒绝迁移")
        result[key] = value
    return result


def regular_file(path: Path) -> bytes:
    if not stat.S_ISREG(path.lstat().st_mode):
        raise MigrationError("输入／目标必须是普通文件，拒绝符号链接")
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as handle:
        if not stat.S_ISREG(os.fstat(handle.fileno()).st_mode):
            raise MigrationError("输入／目标必须是普通文件，拒绝符号链接")
        return handle.read()


@contextmanager
def configuration_mutation_lock(destination: Path):
    if destination.is_symlink():
        raise MigrationError("迁移目标必须是普通文件，拒绝符号链接")
    if destination.exists():
        normalized = destination.resolve(strict=True)
    else:
        normalized = destination.parent.resolve(strict=True) / destination.name
    lock_path = Path(f"{normalized}.config.lock")
    fd = os.open(lock_path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    locked = False
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise MigrationError("配置锁必须是普通文件，拒绝符号链接")
        fcntl.flock(fd, fcntl.LOCK_EX)
        locked = True
        os.fchmod(fd, 0o600)
        yield
    finally:
        # 保留锁文件，避免其他写者持有旧 inode 时创建第二把锁。
        try:
            if locked:
                fcntl.flock(fd, fcntl.LOCK_UN)
        finally:
            os.close(fd)


def atomic_write(destination: Path, content: bytes, backup_bytes: bytes, expected: bytes | None) -> Path:
    parent = destination.parent
    backup = parent / f"{destination.name}.backup.{datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S')}.{uuid.uuid4().hex}.json"
    backup_fd = os.open(backup, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(backup_fd, "wb") as handle:
            handle.write(backup_bytes)
            handle.flush()
            os.fsync(handle.fileno())
        sync_directory(parent)
    except OSError:
        backup.unlink(missing_ok=True)
        raise
    staged: str | None = None
    replacement_attempted = False
    try:
        fd, staged = tempfile.mkstemp(prefix=f".{destination.name}.migration.", dir=parent)
        with os.fdopen(fd, "wb") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        exists = destination.exists() or destination.is_symlink()
        if (expected is None and exists) or (expected is not None and (not exists or regular_file(destination) != expected)):
            raise MigrationError("目标在写入期间发生变化，拒绝替换；备份已保留")
        replacement_attempted = True
        os.replace(staged, destination)
        staged = None
        sync_directory(parent)
    except BaseException:
        if replacement_attempted:
            previous_mask = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGINT, signal.SIGTERM})
            try:
                current = regular_file(destination)
                if current == content:
                    restore_configuration(destination, backup_bytes)
                elif current != backup_bytes:
                    raise MigrationError("配置在回滚前被其他写者修改；请使用保留备份恢复")
            finally:
                signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
        raise
    finally:
        if staged is not None:
            Path(staged).unlink(missing_ok=True)
    return backup


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


def restore_configuration(destination: Path, original: bytes) -> None:
    fd, temporary = tempfile.mkstemp(prefix=".config.rollback.", dir=destination.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(original)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, destination)
        sync_directory(destination.parent)
    finally:
        Path(temporary).unlink(missing_ok=True)


def rollback_state(staging: Path, state: Path, names: list[str]) -> None:
    previous_mask = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGINT, signal.SIGTERM})
    try:
        for name in reversed(names):
            if not exists(staging / name) and exists(state / name):
                os.rename(state / name, staging / name)
        sync_directory(state)
    finally:
        signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)


@contextmanager
def prepare_state(config: dict, agentic: Path):
    state = checked_path(agentic / "state")
    report: dict = {"objects": [], "notImported": []}
    present = new_state_objects(state)
    if present:
        report.update(status="跳过：新布局已有状态，不覆盖或合并", existing=present)
        yield None, report
        return
    legacy = checked_path(state / "agent")
    if not legacy.is_dir():
        report["status"] = "跳过：没有旧状态目录"
        yield None, report
        return
    agent_id = config.get("agentId")
    if not isinstance(agent_id, str):
        raise MigrationError("配置必须包含字符串 agentId，以选择旧状态目录")
    source = checked_path(legacy / legacy_key(agent_id))
    report["source"] = str(source)
    check_socket(agentic, agent_id)
    if not source.is_dir():
        report["status"] = "跳过：当前 agentId 没有旧状态目录"
        yield None, report
        return
    names = [name for name in STATE_OBJECTS if exists(source / name)]
    for name in names:
        checked_path(source / name)
        if (source / name).is_dir() != (name == "skill-installs"):
            raise MigrationError(f"unexpected state object type: {source / name}")
    jobs = [name for name in ("jobs.sqlite3", *("jobs.sqlite3" + suffix for suffix in SQLITE_SUFFIXES)) if exists(source / name)]
    known = set(STATE_OBJECTS) | set(jobs)
    known.update(name + suffix for name in STATE_OBJECTS[:2] for suffix in SQLITE_SUFFIXES)
    unknown = sorted(entry.name for entry in source.iterdir() if entry.name not in known)
    report["notImported"] = jobs + unknown
    if not names:
        report["status"] = "跳过：旧目录没有受支持的状态对象"
        yield None, report
        return
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
        report.update(objects=names, status="已复制、转换并验证；尚未发布")
        yield staging, report


def run_migration(home: Path, write: bool) -> dict:
    agentic = checked_path(home / ".agentic_gpt")
    destination = checked_path(agentic / "config.json")
    # Require an existing config; never initialize a new installation.
    regular_file(destination)
    private_dir(agentic)
    with exclusive_lock(agentic / ".agent-state-migration.lock", "another migration is running"), \
            exclusive_lock(Path(str(destination) + ".run.lock"), "Agent run lock is held"), \
            configuration_mutation_lock(destination):
        original = regular_file(destination)
        config = object_value(json.loads(original, object_pairs_hook=json_object), "配置")
        result, report = migrate_config(config, home)
        agent_id = config.get("agentId")
        if isinstance(agent_id, str):
            check_socket(agentic, agent_id)
        with prepare_state(config, agentic) as (staging, state_report):
            report["stateMigration"] = state_report
            if not write:
                report["status"] = "仅预览；使用 --write 才会写入配置和发布状态"
                return report
            if regular_file(destination) != original:
                raise MigrationError("输入在迁移期间发生变化，拒绝覆盖")
            if isinstance(agent_id, str):
                check_socket(agentic, agent_id)
            state = agentic / "state"
            published = False
            publication_attempted = False
            configuration_attempted = False
            content = (json.dumps(result, ensure_ascii=False, indent=2) + "\n").encode()
            try:
                if staging is not None:
                    private_dir(state)
                    publication_attempted = True
                    if not publish(staging, state, state_report["objects"]):
                        raise MigrationError("新状态在迁移期间出现，拒绝发布；配置未写入")
                    published = True
                if report["changes"]:
                    configuration_attempted = True
                    backup = atomic_write(destination, content, original, original)
                    report.update(written=str(destination), backup=str(backup))
                else:
                    report.update(written=None)
                if published:
                    state_report["status"] = "已迁移；旧目录保留用于回滚"
                report["status"] = "迁移完成" if report["changes"] or published else "无需迁移"
                return report
            except BaseException:
                previous_mask = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGINT, signal.SIGTERM})
                try:
                    try:
                        if configuration_attempted:
                            current = regular_file(destination)
                            if current == content:
                                restore_configuration(destination, original)
                            elif current != original:
                                raise MigrationError("配置在回滚前被其他写者修改；请使用保留备份恢复")
                    finally:
                        if publication_attempted:
                            rollback_state(staging, state, state_report["objects"])
                finally:
                    signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
                raise


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, add_help=False)
    parser.add_argument("--write", action="store_true", help="备份并迁移标准配置及私有状态")
    args = parser.parse_args()

    def interrupted(signum: int, _frame: object) -> None:
        raise MigrationError(f"migration interrupted by signal {signum}")

    signal.signal(signal.SIGTERM, interrupted)
    try:
        home = Path.home()
        if not home.is_absolute():
            raise MigrationError("当前用户 HOME 必须是绝对路径")
        report = run_migration(home, args.write)
        print(json.dumps(report, ensure_ascii=False, indent=2))
        return 0
    except (MigrationError, OSError, RuntimeError, UnicodeError, sqlite3.Error, ValueError, TypeError, KeyboardInterrupt) as error:
        print(f"迁移失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
