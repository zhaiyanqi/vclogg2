"""Recreate the schema-only SQLite catalog inputs for AOCI without opening user databases."""

import json
import re
import sqlite3
from contextlib import closing
from pathlib import Path
from tempfile import TemporaryDirectory


ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / ".local" / "aoci-schema"


def sql_literal(source, pattern):
    matches = re.findall(pattern, source, re.DOTALL)
    if len(matches) != 1:
        raise ValueError(f"Expected one schema literal matching {pattern!r}, got {len(matches)}")
    return matches[0]


def desktop_schema():
    source = (ROOT / "crates/vclogg-data/src/state_repository.rs").read_text(encoding="utf-8")
    # Both statements are Rust string literals. Decode the escaped quotes and
    # backslashes in the default word-boundary setting without altering Unicode.
    batch = sql_literal(source, r'"(CREATE TABLE IF NOT EXISTS file_sessions .*?)",\s*\)')
    index = sql_literal(source, r'"(CREATE INDEX IF NOT EXISTS idx_file_sessions_pinned_recent[^"]+)"')
    return json.loads(f'"{batch}"', strict=False) + "\n" + index + ";"


def server_schema():
    source = (ROOT / "server/internal/store/store.go").read_text(encoding="utf-8")
    migrations = sql_literal(source, r"`(CREATE TABLE IF NOT EXISTS schema_migrations [^`]+)`")
    schema = sql_literal(source, r"const schemaSQL = `([^`]+)`")
    return migrations + ";\n" + schema


def main():
    OUTPUT.mkdir(parents=True, exist_ok=True)
    with TemporaryDirectory(dir=OUTPUT) as temporary:
        for name, sql in (("desktop", desktop_schema()), ("server", server_schema())):
            path = Path(temporary) / f"{name}.sqlite"
            with closing(sqlite3.connect(path)) as db:
                # Run the fresh-install SQL, then retain just the catalog. The
                # server seeds its standard permissions and role during setup.
                db.executescript(sql)
                tables = db.execute(
                    "SELECT name FROM sqlite_schema WHERE type='table' AND name NOT GLOB 'sqlite_*'"
                ).fetchall()
                for (table,) in tables:
                    quoted = '"' + table.replace('"', '""') + '"'
                    db.execute("DELETE FROM " + quoted)
                db.commit()
                if db.execute("PRAGMA integrity_check").fetchone() != ("ok",):
                    raise ValueError(f"Invalid SQLite catalog for {name}")
            path.replace(OUTPUT / path.name)
            print(f"{name}: {len(tables)} tables")


if __name__ == "__main__":
    main()
