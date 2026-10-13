"""统一迁移入口的数据保留、预览、并发准入及失败撤回集成测试。"""
import contextlib
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import stat
import subprocess
import tempfile
import unittest
from unittest import mock

SCRIPT = Path(__file__).resolve().parents[1] / "migrate-agentic.py"
spec = importlib.util.spec_from_file_location("migrate_agentic", SCRIPT)
migration = importlib.util.module_from_spec(spec)
spec.loader.exec_module(migration)


class MigrationIntegrationTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="am-")
        self.addCleanup(self.temporary.cleanup)
        self.home = Path(self.temporary.name)
        self.agentic = self.home / ".agentic_gpt"
        self.agentic.mkdir()
        self.config = self.agentic / "config.json"
        self.original = json.dumps({
            "agentId": "test", "profile": "normal", "keep": {"value": 42},
            "policy": {"allow": [{"program": "echo", "argsPrefix": ["secret-marker"]}], "confirm": [], "deny": []},
        }).encode()
        self.config.write_bytes(self.original)
        self.source = self.agentic / "state" / "agent" / "test"
        self.source.mkdir(parents=True)
        (self.source / "active-skills.json").write_text('["skill-one"]')
        (self.source / "skill-installs").mkdir()
        (self.source / "skill-installs" / "record.json").write_text('{"digest":"old"}')
        self.env = dict(os.environ, HOME=str(self.home), TMPDIR=str(self.home),
                        DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent",
                        DBUS_SYSTEM_BUS_ADDRESS="unix:path=/nonexistent")
        for key in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME", "XDG_RUNTIME_DIR"):
            self.env[key] = str(self.home / key.lower())

    def run_cli(self, *args):
        return subprocess.run(["python3", str(SCRIPT), *args], env=self.env,
                              capture_output=True, text=True, timeout=10)

    def assert_no_publication(self):
        self.assertEqual(self.config.read_bytes(), self.original)
        self.assertEqual(migration.new_state_objects(self.agentic / "state"), [])
        self.assertEqual(list(self.agentic.glob(".state-migration-*")), [])
        self.assertEqual(list(self.agentic.glob(".config*")), [])

    def test_preview_write_and_repeat_preserve_data_and_backup(self):
        preview = self.run_cli()
        self.assertEqual(preview.returncode, 0, preview.stderr)
        self.assertNotIn("secret-marker", preview.stdout)
        self.assert_no_publication()
        self.assertEqual(list(self.agentic.glob("config.json.backup.*")), [])
        written = self.run_cli("--write")
        self.assertEqual(written.returncode, 0, written.stderr)
        report = json.loads(written.stdout)
        self.assertEqual(Path(report["backup"]).read_bytes(), self.original)
        result = json.loads(self.config.read_bytes())
        self.assertEqual(result["keep"], {"value": 42})
        self.assertIn("echo secret-marker **", result["policy"]["allow"])
        self.assertEqual((self.agentic / "state" / "active-skills.json").read_bytes(),
                         (self.source / "active-skills.json").read_bytes())
        self.assertEqual((self.agentic / "state" / "skill-installs" / "record.json").read_bytes(),
                         (self.source / "skill-installs" / "record.json").read_bytes())
        for path in (self.config, Path(report["backup"]), self.agentic / "state" / "active-skills.json"):
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
        before = self.config.read_bytes()
        backups = list(self.agentic.glob("config.json.backup.*"))
        repeated = self.run_cli("--write")
        self.assertEqual(repeated.returncode, 0, repeated.stderr)
        self.assertEqual(json.loads(repeated.stdout)["changes"], {})
        self.assertEqual(self.config.read_bytes(), before)
        self.assertEqual(list(self.agentic.glob("config.json.backup.*")), backups)

    def test_legacy_databases_convert_without_changing_originals(self):
        event_path = self.source / "events.sqlite3"
        with contextlib.closing(sqlite3.connect(event_path)) as db:
            db.executescript('''
                CREATE TABLE events(event_id TEXT PRIMARY KEY,message TEXT,severity TEXT,created_at TEXT,
                  status TEXT,source_kind TEXT,source_ref TEXT,shown_count INTEGER,expires_at TEXT,terminal_at TEXT);
                CREATE TABLE internal_sources(source_kind TEXT,source_ref TEXT,response_state TEXT,
                  settled_at TEXT,policy_json TEXT,completion_seen INTEGER,completion_event_type TEXT,
                  completion_message TEXT,completed_at TEXT,notified INTEGER);
                CREATE TABLE event_store_metadata(agent_id TEXT);
                INSERT INTO events(event_id,message) VALUES('event-one','retained message');
                PRAGMA user_version=1;
            ''')
        process_path = self.source / "process.sqlite3"
        info = {"processId": "p-one", "agentId": "test", "kind": "command", "state": "completed",
                "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-01T00:00:01Z", "captureStatus": "complete"}
        detail = {"process": info, "detailAvailable": True, "result": {"agentId": "business-value"}}
        with contextlib.closing(sqlite3.connect(process_path)) as db:
            db.executescript('''
                CREATE TABLE processes(process_id TEXT PRIMARY KEY,agent_id TEXT,group_name TEXT,kind TEXT,
                  state TEXT,created_at TEXT,started_at TEXT,updated_at TEXT,finished_at TEXT,info_json TEXT,
                  detail_json TEXT,stdout_bytes BLOB,stdout_start_offset INTEGER,stdout_end_offset INTEGER,
                  stderr_bytes BLOB,stderr_start_offset INTEGER,stderr_end_offset INTEGER,capture_status TEXT,capture_error TEXT);
                CREATE TABLE process_event_completions(process_id TEXT,event_type TEXT,message TEXT,completed_at TEXT);
                PRAGMA user_version=1;
            ''')
            db.execute('INSERT INTO processes(process_id,agent_id,info_json,detail_json,stdout_bytes) VALUES(?,?,?,?,?)',
                       ('p-one', 'test', json.dumps(info), json.dumps(detail), b'output'))
            db.execute('INSERT INTO process_event_completions VALUES(?,?,?,?)', ('p-one', 'completed', 'done', '2026-01-01T00:00:01Z'))
            db.commit()
        originals = {path: path.read_bytes() for path in (event_path, process_path)}
        result = self.run_cli("--write")
        self.assertEqual(result.returncode, 0, result.stderr)
        for path, content in originals.items():
            self.assertEqual(path.read_bytes(), content)
        with contextlib.closing(sqlite3.connect(self.agentic / "state" / "events.sqlite3")) as db:
            self.assertEqual(db.execute('PRAGMA user_version').fetchone(), (4,))
            self.assertEqual(db.execute('SELECT message FROM events').fetchone(), ('retained message',))
            self.assertEqual(db.execute("SELECT name FROM sqlite_master WHERE name='event_store_metadata'").fetchall(), [])
        with contextlib.closing(sqlite3.connect(self.agentic / "state" / "process.sqlite3")) as db:
            self.assertEqual(db.execute('PRAGMA user_version').fetchone(), (2,))
            self.assertNotIn('agent_id', {row[1] for row in db.execute('PRAGMA table_info(processes)')})
            converted_info, converted_detail, output = db.execute('SELECT info_json,detail_json,stdout_bytes FROM processes').fetchone()
            self.assertNotIn('agentId', json.loads(converted_info))
            self.assertNotIn('agentId', json.loads(converted_detail)['process'])
            self.assertEqual(json.loads(converted_detail)['result']['agentId'], 'business-value')
            self.assertEqual(output, b'output')
            self.assertEqual(db.execute('SELECT message FROM process_event_completions').fetchone(), ('done',))

    def test_invalid_state_prevents_configuration_write(self):
        (self.source / "active-skills.json").write_text('invalid json')
        result = self.run_cli("--write")
        self.assertNotEqual(result.returncode, 0)
        self.assert_no_publication()
        self.assertEqual(list(self.agentic.glob("config.json.backup.*")), [])

    def test_existing_or_absent_state_does_not_block_config_migration(self):
        existing = self.agentic / "state" / "active-skills.json"
        existing.write_text('["new-state"]')
        result = self.run_cli("--write")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(existing.read_text(), '["new-state"]')
        self.assertEqual(json.loads(self.config.read_bytes())["policy"]["defaultAction"], 'confirm')
        existing.unlink()
        (self.source / "active-skills.json").unlink()
        (self.source / "skill-installs" / "record.json").unlink()
        (self.source / "skill-installs").rmdir()
        self.source.rmdir()
        self.config.write_bytes(self.original)
        result = self.run_cli("--write")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(migration.new_state_objects(self.agentic / "state"), [])

    def test_active_agent_and_symlink_are_rejected_before_write(self):
        with (self.agentic / 'config.json.run.lock').open('w') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            result = self.run_cli("--write")
            self.assertNotEqual(result.returncode, 0)
            self.assert_no_publication()
        (self.source / 'skill-installs' / 'link').symlink_to(self.config)
        result = self.run_cli("--write")
        self.assertNotEqual(result.returncode, 0)
        self.assert_no_publication()

    def test_config_sync_failure_rolls_back_both_migrations(self):
        original_sync = migration.sync_directory
        failed = False
        def fail_after_replace(path):
            nonlocal failed
            if path == self.agentic and self.config.read_bytes() != self.original and not failed:
                failed = True
                raise OSError('injected config fsync failure')
            original_sync(path)
        with mock.patch.object(migration, 'sync_directory', side_effect=fail_after_replace):
            with self.assertRaises(OSError):
                migration.run_migration(self.home, True)
        self.assertTrue(failed)
        self.assert_no_publication()
        backups = list(self.agentic.glob('config.json.backup.*'))
        self.assertEqual(len(backups), 1)
        self.assertEqual(backups[0].read_bytes(), self.original)
        result = self.run_cli('--write')
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_interrupt_after_config_commit_restores_both_migrations(self):
        original_write = migration.atomic_write
        def interrupt_after_commit(*args):
            original_write(*args)
            raise KeyboardInterrupt()
        with mock.patch.object(migration, 'atomic_write', side_effect=interrupt_after_commit):
            with self.assertRaises(KeyboardInterrupt):
                migration.run_migration(self.home, True)
        self.assert_no_publication()
        self.assertEqual(len(list(self.agentic.glob('config.json.backup.*'))), 1)

    def test_partial_state_publication_is_retracted(self):
        original_rename = migration.os.rename
        count = 0
        def fail_second_publication(source, target):
            nonlocal count
            if Path(source).parent.name == 'new':
                count += 1
                if count == 2:
                    raise OSError('injected state rename failure')
            return original_rename(source, target)
        with mock.patch.object(migration.os, 'rename', side_effect=fail_second_publication):
            with self.assertRaises(OSError):
                migration.run_migration(self.home, True)
        self.assert_no_publication()


if __name__ == '__main__':
    unittest.main()
