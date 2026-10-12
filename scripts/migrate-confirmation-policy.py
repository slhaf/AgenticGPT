#!/usr/bin/env python3
"""显式迁移旧命令／路径策略；默认仅预览，不读取默认 HOME 或调用运行时。"""
from __future__ import annotations

import argparse
import copy
from contextlib import contextmanager
from datetime import datetime, timezone
import fcntl
import json
import os
from pathlib import Path
import shlex
import stat
import sys
import tempfile
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


def concrete_path(value: object, home: Path | None, label: str) -> str:
    if not isinstance(value, str) or not value or "\0" in value:
        raise MigrationError(f"{label} 必须是非空路径字符串")
    if "$" in value:
        raise MigrationError(f"{label} 使用环境变量；请先显式填写具体绝对路径")
    if value == "~" or value.startswith("~/"):
        if home is None:
            raise MigrationError(f"{label} 需要 --home；脚本不会查询当前 HOME")
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


def migrate(config: dict, home: Path | None, workspace_override: str | None) -> tuple[dict, dict]:
    result = copy.deepcopy(config)
    notes: list[str] = []
    conflicts: list[str] = []
    profile = config.get("profile", "normal")
    if not isinstance(profile, str) or profile not in OLD_CONFIRM:
        raise MigrationError("profile 必须是 normal 或 room")
    if "profile" not in config:
        notes.append("缺少 profile：按历史初始化默认 normal 迁移内置预设。")
    workspace_raw = workspace_override if workspace_override is not None else config.get("workspaceRoot")
    if workspace_raw is None:
        if home is None:
            raise MigrationError("缺少 workspaceRoot：必须提供 --workspace 或 --home")
        workspace_raw = str(home / ".agentic_gpt" / "workspace")
        notes.append("缺少 workspaceRoot：按显式 --home 下历史初始化目录 .agentic_gpt/workspace 补齐。")
    workspace = concrete_path(workspace_raw, home, "workspaceRoot")
    if workspace_override or "workspaceRoot" not in config:
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
                notes.append(f"缺少 pathPolicy.{key}：按历史初始化预设补齐；其中 ~ 仅使用显式 --home。")
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


def sync_directory(path: Path) -> None:
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


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
    try:
        fd, staged = tempfile.mkstemp(prefix=f".{destination.name}.migration.", dir=parent)
        with os.fdopen(fd, "wb") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        exists = destination.exists() or destination.is_symlink()
        if (expected is None and exists) or (expected is not None and (not exists or regular_file(destination) != expected)):
            raise MigrationError("目标在写入期间发生变化，拒绝替换；备份已保留")
        os.replace(staged, destination)
        staged = None
        sync_directory(parent)
    finally:
        if staged is not None:
            Path(staged).unlink(missing_ok=True)
    return backup


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path, help="明确指定的旧配置 JSON 路径")
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--output", type=Path, help="写入指定输出文件；已有目标先备份")
    group.add_argument("--write", action="store_true", help="备份后原子替换输入文件")
    parser.add_argument("--home", type=Path, help="明确指定旧配置中 ~ 所代表的绝对目录；不会自动查询 HOME")
    parser.add_argument("--workspace", help="明确指定工作区绝对路径；覆盖或补齐 workspaceRoot")
    args = parser.parse_args()
    try:
        if args.home is not None and not args.home.is_absolute():
            raise MigrationError("--home 必须是绝对路径")
        original = regular_file(args.input)
        config = object_value(json.loads(original, object_pairs_hook=json_object), "配置")
        result, report = migrate(config, args.home, args.workspace)
        destination = args.input if args.write else args.output
        if destination is not None:
            if args.output is not None and args.output.resolve(strict=False) == args.input.resolve(strict=False):
                raise MigrationError("原地修改必须使用 --write")
            if not (args.write and not report["changes"]):
                with configuration_mutation_lock(destination):
                    existed = destination.exists() or destination.is_symlink()
                    expected = regular_file(destination) if existed else None
                    backup_bytes = expected if expected is not None else original
                    if args.write and backup_bytes != original:
                        raise MigrationError("输入在迁移期间发生变化，拒绝覆盖")
                    backup = atomic_write(destination, (json.dumps(result, ensure_ascii=False, indent=2) + "\n").encode(), backup_bytes, expected)
                    report["written"] = str(destination)
                    report["backup"] = str(backup)
            else:
                report["written"] = None
                report["status"] = "授权配置已是目标形状，无需写入"
        else:
            report["status"] = "仅预览；使用 --output 或 --write 才会写入"
        print(json.dumps(report, ensure_ascii=False, indent=2))
        return 0
    except (MigrationError, OSError, UnicodeError, json.JSONDecodeError) as error:
        print(f"迁移失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
