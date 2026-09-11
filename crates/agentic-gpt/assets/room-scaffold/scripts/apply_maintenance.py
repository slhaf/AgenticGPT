#!/usr/bin/env python3
"""Apply bounded Room maintenance requests in this checkout.

This file is intentionally repository-owned: local Agentic execution and the workflow
invoke this same semantic executor. Requests are JSON files named maintenance.json in
one of the five slot directories described by manual/.
"""
from __future__ import annotations

import json
import os
import re
import sys
from pathlib import Path
from typing import Any

MAX_REQUEST_BYTES = 64 * 1024
MAX_TEXT_CHARS = 64 * 1024
MAX_SUMMARY_CHARS = 8 * 1024
MAX_ENTRIES = 128
MAX_TAGS = 8
MAX_TAG_CHARS = 64
MAX_PATH_CHARS = 240
SLOT_PATHS = {
    "diary.daily": ("maintenance", "diary", "daily"),
    "diary.weekly": ("maintenance", "diary", "weekly"),
    "diary.monthly": ("maintenance", "diary", "monthly"),
    "notebook": ("maintenance", "notebook"),
    "entity": ("maintenance", "entity"),
}


def fail(message: str) -> None:
    raise ValueError(message)


def bounded_text(value: Any, label: str, limit: int = MAX_TEXT_CHARS) -> str:
    if not isinstance(value, str) or len(value) > limit or "\x00" in value:
        fail(f"{label} must be a bounded string")
    return value


def object_only(value: Any, allowed: set[str]) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) - allowed:
        fail("request must be an object with known fields")
    return value


def safe_relative(root: Path, value: str, prefix: str) -> Path:
    if (
        not value.startswith(prefix + "/")
        or "\\" in value
        or len(value) > MAX_PATH_CHARS
    ):
        fail("path outside semantic root")
    relative_parts = value[len(prefix) + 1 :].split("/")
    prefix_parts = prefix.split("/")
    parts = prefix_parts + relative_parts
    if (
        not relative_parts
        or any(not part or part in {".", ".."} for part in parts)
    ):
        fail("invalid relative path")
    current = root
    for part in parts:
        current = current / part
        if current.is_symlink():
            fail("symlink path is not allowed")
    return current


def render_diary(slot: str, payload: dict[str, Any]) -> tuple[Path, str]:
    summary = payload.get("summary", "")
    if summary is None:
        summary = ""
    summary = bounded_text(summary, "summary", MAX_SUMMARY_CHARS)
    entries = payload.get("entries", [])
    if not isinstance(entries, list) or len(entries) > MAX_ENTRIES:
        fail("entries must be a bounded array")
    rendered: list[str] = []
    if summary:
        rendered.append(summary)
    for entry in entries:
        item = object_only(entry, {"text", "tags"})
        text = bounded_text(item.get("text"), "entry.text", MAX_SUMMARY_CHARS)
        tags = item.get("tags", [])
        if not isinstance(tags, list) or len(tags) > MAX_TAGS:
            fail("entry.tags must be a bounded array")
        clean_tags = [bounded_text(tag, "entry.tag", MAX_TAG_CHARS) for tag in tags]
        suffix = "" if not clean_tags else " (" + ", ".join(clean_tags) + ")"
        rendered.append(f"- {text}{suffix}")
    heading = {"diary.daily": "Daily", "diary.weekly": "Weekly", "diary.monthly": "Monthly"}[slot]
    body = "\n".join(rendered)
    content = f"# {heading}\n\n## Summary\n\n{summary}\n\n## Entries\n\n{body}\n"
    return Path("Diary") / heading, content


def apply_diary(root: Path, slot: str, payload: dict[str, Any]) -> None:
    relative = {
        "diary.daily": "Diary/Daily/current.md",
        "diary.weekly": "Diary/Weekly/current.md",
        "diary.monthly": "Diary/Monthly/current.md",
    }[slot]
    target = safe_relative(root, relative, "Diary")
    _, content = render_diary(slot, payload)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(content, encoding="utf-8")


def apply_notebook(root: Path, payload: dict[str, Any]) -> None:
    path_value = bounded_text(payload.get("path"), "path", MAX_PATH_CHARS)
    title = payload.get("title", "")
    title = bounded_text(title, "title", 512)
    body = bounded_text(payload.get("body"), "body")
    target = safe_relative(root, path_value, "Notebook")
    if target.suffix != ".md":
        fail("notebook path must end in .md")
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text((f"# {title}\n\n" if title else "") + body.rstrip() + "\n", encoding="utf-8")


def apply_entity(root: Path, payload: dict[str, Any]) -> None:
    entity = bounded_text(payload.get("entity"), "entity", 160)
    if not entity or "/" in entity or "\\" in entity or entity in {".", ".."}:
        fail("invalid entity")
    content = bounded_text(payload.get("content"), "content")
    target = safe_relative(root, f"State/entities/{entity}.md", "State/entities")
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(content.rstrip() + "\n", encoding="utf-8")


def load_request(path: Path) -> dict[str, Any]:
    if path.is_symlink() or not path.is_file() or path.stat().st_size > MAX_REQUEST_BYTES:
        fail(f"invalid request file: {path}")
    value = json.loads(path.read_text(encoding="utf-8"))
    return value if isinstance(value, dict) else fail("request must be an object")


def main() -> int:
    script = Path(__file__).absolute()
    if script.is_symlink() or script.parent.is_symlink() or script.parents[1].is_symlink():
        fail("executor path must not contain a symlink")
    root = script.parents[1]
    requests: list[tuple[str, Path]] = []
    for slot, parts in SLOT_PATHS.items():
        directory = safe_relative(root, "/".join(parts), "maintenance")
        if not directory.is_dir() or directory.is_symlink():
            fail(f"missing maintenance slot: {slot}")
        files = sorted(directory.glob("maintenance.json"))
        if len(files) > 1:
            fail(f"multiple requests in slot: {slot}")
        if files:
            requests.append((slot, files[0]))
    if not requests:
        return 0
    for slot, path in requests:
        payload = load_request(path)
        if slot.startswith("diary."):
            object_only(payload, {"summary", "entries"})
            apply_diary(root, slot, payload)
        elif slot == "notebook":
            object_only(payload, {"path", "title", "body"})
            apply_notebook(root, payload)
        else:
            object_only(payload, {"entity", "content"})
            apply_entity(root, payload)
        path.unlink()
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"room maintenance rejected: {error}", file=sys.stderr)
        raise SystemExit(2)
