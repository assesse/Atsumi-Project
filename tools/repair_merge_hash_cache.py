"""Restore ONLY missing, byte-verified page hashes for one committed merge.

Dry-run by default. Both databases are read-only during planning. --apply
requires Atsumi to be stopped, creates a fresh non-overwriting SQLite backup,
then rechecks page checkpoints in one transaction. Never restores album bytes,
reviews, download states or old decisions from the backup.
"""
import argparse
import hashlib
import json
import os
import sqlite3
import subprocess
import uuid
from pathlib import Path

COLUMNS = ("entry_id", "gallery_id", "source_page_number", "profile_version",
           "artifact_sha256", "coarse_d_hash_hex", "detail_d_hash_hex", "p_hash_hex",
           "mean_luma", "std_dev", "non_uniform_ratio", "edge_density", "width",
           "height", "low_information", "computed_at")


def readonly(path):
    connection = sqlite3.connect(Path(path).resolve().as_uri() + "?mode=ro", uri=True, timeout=3)
    connection.row_factory = sqlite3.Row
    connection.execute("PRAGMA query_only=ON")
    return connection


def plan(database, backup, merge_id):
    live, old = readonly(database), readonly(backup)
    try:
        merge = live.execute("SELECT * FROM overlap_page_merges WHERE merge_id=? AND state='applied'", (merge_id,)).fetchone()
        if merge is None:
            raise ValueError("Only a committed merge can be repaired")
        journal = json.loads(merge["journal_json"])
        target, source = journal["target_entry_id"], journal["source_entry_id"]
        artifact = live.execute("SELECT * FROM download_artifacts WHERE entry_id=?", (target,)).fetchone()
        if artifact is None or artifact["gallery_id"] != journal["target_gallery_id"]:
            raise ValueError("Target identity changed")
        root = Path(artifact["root_snapshot"]).resolve(strict=True)
        replacement = {p["target_page"]: p for p in journal["replacements"]}
        rows = []
        pages = live.execute("SELECT * FROM download_pages WHERE entry_id=? AND state='present' AND excluded=0", (target,)).fetchall()
        for page in pages:
            number, sha = page["source_page_number"], page["sha256"]
            candidates = list(old.execute("SELECT * FROM duplicate_page_hashes WHERE entry_id=? AND gallery_id=? AND source_page_number=? AND artifact_sha256=?", (target, artifact["gallery_id"], number, sha)))
            donor = replacement.get(number)
            if donor and donor["source_sha256"] == sha:
                candidates += list(live.execute("SELECT * FROM duplicate_page_hashes WHERE entry_id=? AND gallery_id=? AND source_page_number=? AND artifact_sha256=?", (source, journal["source_gallery_id"], donor["source_page"], sha)))
            additions = {}
            for cached in candidates:
                if live.execute("SELECT 1 FROM duplicate_page_hashes WHERE entry_id=? AND source_page_number=? AND profile_version=?", (target, number, cached["profile_version"])).fetchone():
                    continue  # never overwrite even a conflicting live cache row
                values = dict(cached)
                values.update(entry_id=target, gallery_id=artifact["gallery_id"], source_page_number=number)
                additions[cached["profile_version"]] = values
            if additions:
                path = (root / page["relative_path"]).resolve(strict=True)
                if not path.is_relative_to(root):
                    raise ValueError("Page escapes the managed root")
                with path.open("rb") as stream:
                    actual = hashlib.file_digest(stream, "sha256").hexdigest()
                if path.stat().st_size != page["byte_length"] or actual != sha:
                    raise ValueError(f"Current page {number} does not match its verified checkpoint")
                rows.extend(additions.values())
        return rows
    finally:
        live.close()
        old.close()


def apply(database, merge_id, rows):
    database = Path(database).resolve(strict=True)
    snapshot = database.with_name(database.name + ".before-merge-cache-repair-" + str(uuid.uuid4()) + ".bak")
    # Exclusive reservation protects every existing backup.
    with snapshot.open("xb"):
        pass
    connection = sqlite3.connect(str(database), timeout=3)
    destination = sqlite3.connect(str(snapshot))
    try:
        connection.backup(destination)
    finally:
        destination.close()
    try:
        connection.execute("BEGIN IMMEDIATE")
        if not connection.execute("SELECT 1 FROM overlap_page_merges WHERE merge_id=? AND state='applied'", (merge_id,)).fetchone():
            raise ValueError("Merge state changed")
        inserted = 0
        for row in rows:
            valid = connection.execute("SELECT 1 FROM download_pages WHERE entry_id=? AND gallery_id=? AND source_page_number=? AND sha256=? AND state='present' AND excluded=0", (row["entry_id"], row["gallery_id"], row["source_page_number"], row["artifact_sha256"])).fetchone()
            if not valid:
                raise ValueError("Page checkpoint changed; no cache rows committed")
            cursor = connection.execute(f"INSERT OR IGNORE INTO duplicate_page_hashes ({','.join(COLUMNS)}) VALUES ({','.join('?' for _ in COLUMNS)})", [row[key] for key in COLUMNS])
            inserted += cursor.rowcount
        connection.commit()
        return inserted, str(snapshot)
    finally:
        connection.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", required=True)
    parser.add_argument("--backup", required=True)
    parser.add_argument("--merge-id", required=True)
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()
    def ensure_stopped():
        if os.name == "nt":
            processes = subprocess.run(["tasklist.exe", "/FI", "IMAGENAME eq atsumi.exe", "/FO", "CSV", "/NH"], capture_output=True, check=True, creationflags=subprocess.CREATE_NO_WINDOW)
            if b'"atsumi.exe"' in processes.stdout.lower():
                raise RuntimeError("Close Atsumi completely before repairing its cache")
    if args.apply:
        ensure_stopped()
    rows = plan(args.database, args.backup, args.merge_id)
    result = {"verifiedMissingHashes": len(rows), "applied": False}
    if args.apply and rows:
        ensure_stopped()
        count, snapshot = apply(args.database, args.merge_id, rows)
        result.update(applied=True, inserted=count, backup=snapshot)
    print(json.dumps(result))
