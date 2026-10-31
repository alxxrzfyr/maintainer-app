"""The resolved SQLite feature graph must preserve the Ubuntu 22.04 ABI."""
from pathlib import Path
import subprocess
import unittest


class SqliteFloor(unittest.TestCase):
    def test_system_sqlite_does_not_enable_post_jammy_error_api(self):
        root = Path(__file__).resolve().parents[2]
        features = subprocess.check_output([
            'cargo', 'tree', '--locked', '--all-features', '--edges', 'features',
            '-p', 'maintainer-core'], cwd=root, text=True)
        # rusqlite's modern_sqlite calls sqlite3_error_offset from SQLite 3.38;
        # Jammy ships 3.37. Serialization itself predates that ABI requirement.
        self.assertNotIn('rusqlite feature "modern_sqlite"', features)
