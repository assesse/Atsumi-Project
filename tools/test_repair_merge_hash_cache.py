import hashlib
import json
import sqlite3
import tempfile
import unittest
from contextlib import closing
from pathlib import Path

from repair_merge_hash_cache import COLUMNS, apply, plan


class CacheRepairTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.database = self.root / "live.sqlite3"
        self.backup = self.root / "old.sqlite3"
        self.bytes = {1: b"unchanged", 2: b"donor", 3: b"already cached"}
        self.sha = {n: hashlib.sha256(data).hexdigest() for n, data in self.bytes.items()}
        for n, data in self.bytes.items():
            (self.root / f"{n}.webp").write_bytes(data)
        with closing(sqlite3.connect(self.database)) as db, db:
            db.executescript("""
                CREATE TABLE overlap_page_merges(merge_id TEXT,state TEXT,journal_json TEXT);
                CREATE TABLE download_artifacts(entry_id TEXT,gallery_id INTEGER,root_snapshot TEXT);
                CREATE TABLE download_pages(entry_id TEXT,gallery_id INTEGER,source_page_number INTEGER,sha256 TEXT,state TEXT,excluded INTEGER,relative_path TEXT,byte_length INTEGER);
            """)
            self.create_hash_table(db)
            journal = {"target_entry_id": "target", "source_entry_id": "source", "target_gallery_id": 10, "source_gallery_id": 20, "replacements": [{"target_page": 2, "source_page": 1, "source_sha256": self.sha[2]}]}
            db.execute("INSERT INTO overlap_page_merges VALUES('merge','applied',?)", (json.dumps(journal),))
            db.execute("INSERT INTO download_artifacts VALUES('target',10,?)", (str(self.root),))
            for n in self.bytes:
                db.execute("INSERT INTO download_pages VALUES('target',10,?,?,'present',0,?,?)", (n, self.sha[n], f"{n}.webp", len(self.bytes[n])))
            self.insert_hash(db, "source", 20, 1, self.sha[2])
            self.insert_hash(db, "target", 10, 3, self.sha[3])
        with closing(sqlite3.connect(self.backup)) as db, db:
            self.create_hash_table(db)
            for n in self.bytes:
                self.insert_hash(db, "target", 10, n, self.sha[n] if n != 2 else "0" * 64)

    def tearDown(self):
        self.temp.cleanup()

    def create_hash_table(self, db):
        db.execute("CREATE TABLE duplicate_page_hashes (" + ",".join(COLUMNS) + ",UNIQUE(entry_id,source_page_number,profile_version))")

    def insert_hash(self, db, entry, gallery, page, sha):
        db.execute("INSERT INTO duplicate_page_hashes VALUES(" + ",".join("?" for _ in COLUMNS) + ")", (entry, gallery, page, 1, sha, "0" * 16, "1" * 256, "0" * 16, 100, 40, .8, .5, 2, 2, 0, "original"))

    def test_restores_only_matching_missing_hashes_and_is_idempotent(self):
        rows = plan(self.database, self.backup, "merge")
        self.assertEqual([r["source_page_number"] for r in rows], [1, 2])
        count, backup = apply(self.database, "merge", rows)
        self.assertEqual(count, 2)
        with closing(sqlite3.connect(backup)) as db, db:
            self.assertEqual(db.execute("SELECT COUNT(*) FROM duplicate_page_hashes").fetchone()[0], 2)
        self.assertEqual(plan(self.database, self.backup, "merge"), [])
        for n, data in self.bytes.items():
            self.assertEqual((self.root / f"{n}.webp").read_bytes(), data)

    def test_refuses_changed_file(self):
        (self.root / "1.webp").write_bytes(b"external modification")
        with self.assertRaises(ValueError):
            plan(self.database, self.backup, "merge")

    def test_checkpoint_change_rolls_back_all_rows(self):
        rows = plan(self.database, self.backup, "merge")
        with closing(sqlite3.connect(self.database)) as db, db:
            db.execute("UPDATE download_pages SET sha256=? WHERE source_page_number=2", ("f" * 64,))
        with self.assertRaises(ValueError):
            apply(self.database, "merge", rows)
        with closing(sqlite3.connect(self.database)) as db, db:
            self.assertEqual(db.execute("SELECT COUNT(*) FROM duplicate_page_hashes").fetchone()[0], 2)

    def test_never_overwrites_current_cache(self):
        with closing(sqlite3.connect(self.database)) as db, db:
            self.insert_hash(db, "target", 10, 1, "f" * 64)
        rows = plan(self.database, self.backup, "merge")
        self.assertEqual([r["source_page_number"] for r in rows], [2])


if __name__ == "__main__":
    unittest.main()
