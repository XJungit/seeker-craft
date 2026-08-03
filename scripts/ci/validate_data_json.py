#!/usr/bin/env python3
"""Validate JSON data files under data/ (tasks, profiles, blueprints, actions).

CI gate: catches hand-edited JSON corruption (unparseable, missing required
fields) before it silently breaks task loading / prompt building / blueprints.
Pure stdlib, runs on any platform without dependencies.

Usage: python scripts/ci/validate_data_json.py
Exit code 0 = all valid, 1 = at least one problem.
"""
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ERRORS = []


def check_dir(rel: str, required: dict[str, list[str]], key: str, recursive: bool = False) -> int:
    """Validate every *.json under ROOT/rel. required maps field -> must exist.

    Returns number of files checked.
    """
    d = ROOT / rel
    if not d.is_dir():
        ERRORS.append(f"[{rel}] 目录不存在")
        return 0
    count = 0
    files = list(d.glob("*.json")) if not recursive else list(d.rglob("*.json"))
    for p in sorted(files):
        count += 1
        try:
            data = json.loads(p.read_text(encoding="utf-8"))
        except UnicodeDecodeError as e:
            ERRORS.append(f"[{rel}] {p.relative_to(ROOT)} 不是 UTF-8: {e}")
            continue
        except json.JSONDecodeError as e:
            ERRORS.append(f"[{rel}] {p.relative_to(ROOT)} JSON 解析失败: {e}")
            continue
        if not isinstance(data, dict):
            ERRORS.append(f"[{rel}] {p.relative_to(ROOT)} 顶层必须是对象")
            continue
        for field in required.get(key, []):
            if field not in data:
                ERRORS.append(f"[{rel}] {p.relative_to(ROOT)} 缺少字段 '{field}'")
    return count


def check_blueprint_blocks() -> None:
    d = ROOT / "data" / "blueprints"
    for p in sorted(d.glob("*.json")):
        try:
            data = json.loads(p.read_text(encoding="utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            continue  # already reported by check_dir
        blocks = data.get("blocks") if isinstance(data, dict) else None
        if not isinstance(blocks, list) or not blocks:
            ERRORS.append(f"[blueprints] {p.name} blocks 必须是非空数组")
            continue
        for i, b in enumerate(blocks):
            if not isinstance(b, dict) or "block" not in b:
                ERRORS.append(f"[blueprints] {p.name} blocks[{i}] 缺少 'block'")


def check_action_script() -> None:
    d = ROOT / "data" / "actions"
    for p in sorted(d.glob("*.rhai.json")):
        try:
            data = json.loads(p.read_text(encoding="utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            continue
        if not isinstance(data, dict) or not isinstance(data.get("script"), str) or not data["script"].strip():
            ERRORS.append(f"[actions] {p.name} 缺少非空 'script'")


def main() -> int:
    n_tasks = check_dir("data/tasks", {"task": ["id", "name", "tier", "goal", "success"]}, "task")
    n_profiles = check_dir("data/profiles", {"profile": ["name", "system_prompt"]}, "profile")
    n_defaults = check_dir("data/profiles/defaults", {"default": ["name"]}, "default")
    n_blueprints = check_dir("data/blueprints", {"blueprint": ["name", "description", "blocks"]}, "blueprint")
    n_actions = check_dir("data/actions", {"action": ["name", "description", "script"]}, "action")
    check_blueprint_blocks()
    check_action_script()
    print(f"[validate] tasks={n_tasks} profiles={n_profiles} defaults={n_defaults} "
          f"blueprints={n_blueprints} actions={n_actions}")
    if ERRORS:
        print("[validate] FAILED:")
        for e in ERRORS:
            print(f"  - {e}")
        return 1
    print("[validate] OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
