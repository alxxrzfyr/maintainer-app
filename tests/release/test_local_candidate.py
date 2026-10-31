"""Local proof rejects changed bytes without installing anything on the host."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('local_candidate', Path(__file__).resolve().parents[2] / 'scripts/check-release-local.py')
GATE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GATE)


class LocalCandidate(unittest.TestCase):
    def test_exact_candidate_and_changes(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            package = directory / 'maintainer_1.0.0-1_amd64.deb'
            package.write_bytes(b'fixture package')
            manifest = dict(schema_version=1, scope='native-machine-static-package',
                            source_sha256='source', lock_sha256='lock', package=package.name,
                            package_sha256=GATE.digest(package), payload_sha256={'runtime': 'hash'},
                            checks=GATE.checks())
            proof = directory / 'LOCAL-CANDIDATE.json'
            proof.write_text(json.dumps(manifest))
            real_digest = GATE.digest
            with patch.object(GATE, 'source_digest', return_value='source'), patch.object(
                    GATE, 'digest', side_effect=lambda path: 'lock' if path.name == 'Cargo.lock' else real_digest(path)), patch.object(
                    GATE, 'inspect_package', return_value={'runtime': 'hash'}):
                self.assertEqual(GATE.verify(directory), manifest)
                for field, replacement in (('source_sha256', 'changed'), ('lock_sha256', 'changed'),
                                           ('package_sha256', 'changed'), ('payload_sha256', {}),
                                           ('checks', []), ('package', '../escape.deb'),
                                           ('scope', 'installed-tested')):
                    with self.subTest(field=field):
                        proof.write_text(json.dumps(dict(manifest, **{field: replacement})))
                        with self.assertRaises(ValueError):
                            GATE.verify(directory)
                proof.write_text(json.dumps(manifest))
                package.write_bytes(b'tampered')
                with self.assertRaises(ValueError):
                    GATE.verify(directory)

    def test_build_refuses_source_tree_and_existing_outputs(self):
        with self.assertRaises(ValueError):
            GATE.build(GATE.ROOT / 'candidate-output')
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaises(ValueError):
                GATE.build(Path(temporary))


if __name__ == '__main__':
    unittest.main()
