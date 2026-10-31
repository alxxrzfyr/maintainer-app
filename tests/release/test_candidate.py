import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('candidate', Path(__file__).resolve().parents[2] / 'scripts/validate-candidate.py')
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)


class CandidateBinding(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.sha = 'a' * 40
        self.run = {'id': 123, 'repository': {'full_name': 'owner/repo'}, 'workflow_id': 10,
                    'path': '.github/workflows/ci.yml', 'status': 'completed', 'conclusion': 'success',
                    'event': 'push', 'head_branch': 'main', 'head_sha': self.sha, 'run_attempt': 2}
        self.jobs = [{'name': name, 'run_id': 123, 'head_sha': self.sha, 'status': 'completed',
                      'conclusion': 'success', 'steps': [{'name': 'Strict full tests (three passes)' if name == 'check' else 'Fixture step', 'conclusion': 'success'}]}
                     for name in ('check', 'package-amd64', 'package-arm64', 'installed-amd64', 'installed-arm64')]
        self.write('workflow.json', {'id': 10, 'path': '.github/workflows/ci.yml'})
        self.write('tag.json', {'object': {'type': 'commit', 'sha': self.sha}})

    def write(self, name, data):
        (self.base / name).write_text(json.dumps(data))

    def validate(self):
        self.write('run.json', self.run)
        self.write('jobs.json', [{'jobs': self.jobs}])
        return gate.source(self.base, 'owner/repo', '123')

    def test_candidate_and_promotion_workflow_gates(self):
        root = Path(__file__).resolve().parents[2]
        candidate = (root / '.github/workflows/ci.yml').read_text()
        promote = (root / '.github/workflows/promote.yml').read_text()
        self.assertNotIn('  publish:', candidate)
        self.assertNotIn('continue-on-error:', candidate + promote)
        self.assertIn('  installed-artifact:\n    name: installed-${{ matrix.deb_arch }}\n    needs: package', candidate)
        self.assertIn('name: maintainer-validated-${{ matrix.deb_arch }}', candidate)
        self.assertIn('workflow_dispatch:', promote)
        self.assertNotIn('  publish:', promote)
        self.assertNotIn('candidate_run_id', promote)
        self.assertIn('scripts/check-release-local.py build', promote)
        self.assertIn('for pass in 1 2 3; do', candidate)
        self.assertIn('Strict full tests (three passes)', candidate)
        self.assertNotIn('validate-release.py', promote)
        self.assertNotIn('cargo build', promote)

    def test_release_branches_and_system_apt_tests_are_covered(self):
        root = Path(__file__).resolve().parents[2]
        candidate = (root / '.github/workflows/ci.yml').read_text()
        local = (root / '.github/workflows/promote.yml').read_text()
        self.assertIn('"release/**"', candidate)
        for workflow in (candidate, local):
            self.assertIn('python3-apt', workflow)
        self.assertIn('/usr/bin/python3 -W error::ResourceWarning -m unittest discover -s tests/package_transaction', candidate)
        self.assertNotIn('actions/checkout@v4', candidate + local)
        self.assertNotIn('actions/upload-artifact@v4', candidate + local)
        self.assertNotIn('actions/download-artifact@v4', candidate + local)

    def test_promotion_can_bind_original_source_without_rebuilding(self):
        self.assertEqual(self.validate(), self.sha)
        # Promotion checkout SHA is intentionally not the original source SHA.
        package = self.base / 'candidate.deb'
        package.write_bytes(b'original bytes')
        lock = self.base / 'Cargo.lock'
        lock.write_bytes(b'original lock')
        manifest = {'schema_version': 1, 'commit': self.sha, 'run_id': '123', 'run_attempt': '2',
                    'repository': 'owner/repo', 'architecture': 'amd64', 'package': package.name,
                    'lock_sha256': gate.digest(lock), 'package_sha256': gate.digest(package)}
        self.write('SOURCE-amd64.json', manifest)
        gate.artifact(self.base, 'amd64', self.sha, '123', self.run)
        package.write_bytes(b'rebuilt bytes')
        with self.assertRaises(ValueError):
            gate.artifact(self.base, 'amd64', self.sha, '123', self.run)

    def test_wrong_source_repository_workflow_branch_and_state_block(self):
        original = copy.deepcopy(self.run)
        for field, value in (('workflow_id', 11), ('head_branch', 'feature'), ('event', 'pull_request'),
                             ('conclusion', 'failure'), ('head_sha', 'b' * 40), ('id', 124)):
            with self.subTest(field=field):
                self.run = dict(original, **{field: value})
                with self.assertRaises(ValueError):
                    self.validate()
        self.run = original
        self.run['repository']['full_name'] = 'attacker/repo'
        with self.assertRaises(ValueError):
            self.validate()

    def test_source_without_triple_test_protocol_blocks(self):
        self.jobs[0]['steps'][0]['name'] = 'Single historical test pass'
        with self.assertRaises(ValueError):
            self.validate()

    def test_skipped_missing_and_foreign_jobs_block(self):
        self.jobs[0]['steps'][0]['conclusion'] = 'skipped'
        with self.assertRaises(ValueError):
            self.validate()
        self.jobs[0]['steps'][0]['conclusion'] = 'success'
        self.jobs[0]['head_sha'] = 'b' * 40
        with self.assertRaises(ValueError):
            self.validate()
        self.jobs.pop(0)
        with self.assertRaises(ValueError):
            self.validate()


if __name__ == '__main__':
    unittest.main()
