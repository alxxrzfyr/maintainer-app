"""Publication consumes verified assets and never replaces different bytes."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location('publisher', Path(__file__).resolve().parents[2] / 'scripts/publish-release.py')
PUBLISH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PUBLISH)


class ReleasePublishing(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.files = []
        for architecture in ('amd64', 'arm64'):
            package = self.root / f'maintainer_1.3.0-1_{architecture}.deb'
            package.write_bytes(architecture.encode())
            checksum = self.root / f'SHA256SUMS-{architecture}'
            checksum.write_text(PUBLISH.GATE.digest(package) + '  ' + package.name + '\n')
            self.files.extend((package, checksum))
        self.notes = self.root / 'notes.md'
        self.notes.write_text('Reviewed release notes')

    def release(self, draft, paths=None):
        return {'tag_name': 'v1.3.0', 'draft': draft, 'assets': [
            {'name': path.name, 'digest': 'sha256:' + PUBLISH.GATE.digest(path)}
            for path in (self.files if paths is None else paths)]}

    def test_new_release_is_draft_until_all_four_assets_are_verified(self):
        responses = [None, self.release(True, []), self.release(True)]
        with patch.object(PUBLISH, 'release_for_tag', side_effect=responses), patch.object(
                PUBLISH, 'command') as command:
            PUBLISH.publish('owner/repo', 'v1.3.0', self.files, self.notes)
        commands = [call.args for call in command.call_args_list]
        self.assertIn('--draft', commands[0])
        self.assertIn('--verify-tag', commands[0])
        self.assertEqual(commands[1][:4], ('gh', 'release', 'upload', 'v1.3.0'))
        self.assertTrue(all(str(path) in commands[1] for path in self.files))
        self.assertIn('--draft=false', commands[2])
        self.assertNotIn('--clobber', repr(commands))

    def test_identical_published_release_is_idempotent(self):
        with patch.object(PUBLISH, 'release_for_tag', return_value=self.release(False)), patch.object(
                PUBLISH, 'command') as command:
            PUBLISH.publish('owner/repo', 'v1.3.0', self.files, self.notes)
        command.assert_not_called()

    def test_matching_prerelease_is_promoted_for_a_stable_tag(self):
        release = dict(self.release(False), prerelease=True)
        with patch.object(PUBLISH, 'release_for_tag', return_value=release), patch.object(
                PUBLISH, 'command') as command:
            PUBLISH.publish('owner/repo', 'v1.3.0', self.files, self.notes)
        self.assertIn('--prerelease=false', command.call_args.args)
        self.assertIn('--latest', command.call_args.args)

    def test_partial_release_only_uploads_missing_assets(self):
        with patch.object(PUBLISH, 'release_for_tag', side_effect=[
                self.release(True, self.files[:2]), self.release(True)]), patch.object(
                PUBLISH, 'command') as command:
            PUBLISH.publish('owner/repo', 'v1.3.0', self.files, self.notes)
        upload = command.call_args_list[0].args
        self.assertTrue(all(str(path) not in upload for path in self.files[:2]))
        self.assertTrue(all(str(path) in upload for path in self.files[2:]))

    def test_changed_or_unexpected_existing_assets_block_every_write(self):
        for change in ('digest', 'name'):
            release = self.release(True)
            release['assets'][0][change] = 'unexpected'
            with self.subTest(change=change), patch.object(
                    PUBLISH, 'release_for_tag', return_value=release), patch.object(
                    PUBLISH, 'command') as command:
                with self.assertRaises(ValueError):
                    PUBLISH.publish('owner/repo', 'v1.3.0', self.files, self.notes)
                command.assert_not_called()

    def test_incomplete_or_corrupt_upload_never_publishes(self):
        for release in (self.release(True, self.files[:2]), self.release(True)):
            if len(release['assets']) == 4:
                release['assets'][0]['digest'] = 'sha256:wrong'
            with patch.object(PUBLISH, 'release_for_tag', side_effect=[
                    self.release(True, []), release]), patch.object(PUBLISH, 'command') as command:
                with self.assertRaises(ValueError):
                    PUBLISH.publish('owner/repo', 'v1.3.0', self.files, self.notes)
                self.assertNotIn('--draft=false', repr(command.call_args_list))

    def test_only_a_real_404_is_treated_as_a_missing_release(self):
        for status in ('404', '403'):
            error = subprocess.CalledProcessError(1, ['gh'], output=json.dumps(
                {'status': status, 'message': 'Not Found'}))
            with patch.object(PUBLISH, 'api', side_effect=error):
                if status == '404':
                    self.assertIsNone(PUBLISH.release_for_tag('owner/repo', 'v1.3.0'))
                else:
                    with self.assertRaises(subprocess.CalledProcessError):
                        PUBLISH.release_for_tag('owner/repo', 'v1.3.0')

    def test_existing_tags_support_annotated_and_lightweight_refs(self):
        commit = {'object': {'type': 'commit', 'sha': 'a' * 40}}
        with patch.object(PUBLISH, 'api', return_value=commit):
            self.assertEqual(PUBLISH.tag_commit('owner/repo', 'v1.3.0'), commit)
        with patch.object(PUBLISH, 'api', side_effect=[
                {'object': {'type': 'tag', 'sha': 'b' * 40}}, commit]):
            self.assertEqual(PUBLISH.tag_commit('owner/repo', 'v1.3.0'), commit)

    def test_previous_release_excludes_current_newer_drafts_and_prereleases(self):
        releases = [{'tag_name': tag, 'draft': draft, 'prerelease': prerelease}
                    for tag, draft, prerelease in [
                        ('v1.3.0', False, False), ('v2.0.0', False, False),
                        ('v1.2.0', False, False), ('v1.1.0', False, False),
                        ('v1.2.9', True, False), ('v1.2.8', False, True),
                        ('v1.3.0-rc.1', False, True)]]
        with patch.object(PUBLISH, 'command', return_value=json.dumps([releases])):
            self.assertEqual(PUBLISH.previous_release('owner/repo', '1.3.0-1'), 'v1.2.0')
        with patch.object(PUBLISH, 'command', return_value=json.dumps([releases[:2]])):
            with self.assertRaises(ValueError):
                PUBLISH.previous_release('owner/repo', '1.3.0-1')

    def test_release_notes_use_immutable_links_and_keep_acceptance_limits(self):
        notes = ('Draft release notes. This version has not been published. '
                 'Manual checks remain open. See [status](../release-status.md).')
        with patch.object(PUBLISH, 'command', return_value=notes):
            result = PUBLISH.release_notes('v1.3.0', 'a' * 40, 'owner/repo')
        self.assertNotIn('Draft release notes', result)
        self.assertIn('Manual checks remain open', result)
        self.assertIn('/blob/' + 'a' * 40 + '/docs/release-status.md', result)

    def test_untrusted_tag_and_run_inputs_are_rejected_before_network_access(self):
        for tag, run_id in [('v1.3.0;echo bad', '123'), ('v1.3.0', '../123'),
                            ('v1.3.0-rc.1', '123')]:
            with self.subTest(tag=tag, run_id=run_id), patch.object(PUBLISH, 'api') as api:
                with self.assertRaises(ValueError):
                    PUBLISH.validate_source(self.root, 'owner/repo', tag, run_id, {})
                api.assert_not_called()

    def test_active_tag_and_manual_recovery_are_bound_to_their_source(self):
        sha = 'a' * 40
        run = {'id': 123, 'head_sha': sha, 'head_branch': 'v1.3.0', 'event': 'push',
               'status': 'in_progress', 'conclusion': None, 'run_attempt': 1,
               'repository': {'full_name': 'owner/repo'}, 'workflow_id': 10,
               'path': '.github/workflows/ci.yml'}
        jobs = [{'name': name, 'run_id': 123, 'head_sha': sha, 'status': 'completed',
                 'conclusion': 'success', 'steps': [{'name': 'Strict full tests (three passes)',
                                                  'conclusion': 'success'}]}
                for name in ('check', 'package-amd64', 'package-arm64',
                             'installed-amd64', 'installed-arm64')]
        environment = {'GITHUB_EVENT_NAME': 'push', 'GITHUB_REF': 'refs/tags/v1.3.0',
                       'GITHUB_RUN_ID': '123', 'GITHUB_SHA': sha, 'GITHUB_RUN_ATTEMPT': '1'}

        def api(_, resource):
            if resource.startswith('actions/runs/'):
                return run
            if resource.startswith('actions/workflows/'):
                return {'id': 10, 'path': '.github/workflows/ci.yml'}
            return {'object': {'type': 'commit', 'sha': sha}}

        with patch.object(PUBLISH, 'api', side_effect=api), patch.object(
                PUBLISH, 'command', return_value=json.dumps([{'jobs': jobs}])), patch.object(
                PUBLISH.subprocess, 'run'):
            self.assertEqual(PUBLISH.validate_source(
                self.root, 'owner/repo', 'v1.3.0', '123', environment)[0], sha)
            for key, value in [('GITHUB_REF', 'refs/heads/main'), ('GITHUB_RUN_ID', '124'),
                               ('GITHUB_SHA', 'b' * 40), ('GITHUB_RUN_ATTEMPT', '2')]:
                with self.subTest(key=key), self.assertRaises(ValueError):
                    PUBLISH.validate_source(self.root, 'owner/repo', 'v1.3.0', '123',
                                            dict(environment, **{key: value}))
            run.update(status='completed', conclusion='success', head_branch='main')
            manual = {'GITHUB_EVENT_NAME': 'workflow_dispatch', 'GITHUB_REF': 'refs/heads/main'}
            self.assertEqual(PUBLISH.validate_source(
                self.root, 'owner/repo', 'v1.3.0', '123', manual)[0], sha)
            with self.assertRaises(ValueError):
                PUBLISH.validate_source(self.root, 'owner/repo', 'v1.3.0', '123',
                                        dict(manual, GITHUB_REF='refs/heads/feature'))

    def test_downloaded_packages_require_exact_manifests_checksums_and_metadata(self):
        sha = 'a' * 40
        run = {'run_attempt': 1, 'repository': {'full_name': 'owner/repo'}}
        lock = b'fixture lockfile'

        def verify(case):
            def download(*arguments):
                if arguments[0] == 'dpkg-deb':
                    return '1.3.0-1' if arguments[-1] == 'Version' else Path(arguments[2]).name.split('_')[-1][:-4]
                target = Path(arguments[arguments.index('--dir') + 1])
                architecture = arguments[arguments.index('--name') + 1].removeprefix('maintainer-validated-')
                package = target / f'maintainer_1.3.0-1_{architecture}.deb'
                package.write_bytes(b'fixture package')
                (target / 'Cargo.lock').write_bytes(lock)
                checksum = target / f'SHA256SUMS-{architecture}'
                checksum.write_bytes((PUBLISH.GATE.digest(package) + '  ' + package.name + '\n').encode())
                manifest = {'schema_version': 1, 'commit': sha, 'run_id': '123', 'run_attempt': '1',
                            'repository': 'owner/repo', 'architecture': architecture,
                            'lock_sha256': PUBLISH.GATE.digest(target / 'Cargo.lock'),
                            'package': package.name, 'package_sha256': PUBLISH.GATE.digest(package)}
                if case == 'source':
                    manifest['commit'] = 'b' * 40
                if case == 'attempt':
                    manifest['run_attempt'] = '2'
                (target / f'SOURCE-{architecture}.json').write_text(json.dumps(manifest))
                if case == 'checksum':
                    checksum.write_text('wrong checksum')
                if case == 'package':
                    package.write_bytes(b'changed after testing')
                if case == 'lock':
                    (target / 'Cargo.lock').write_bytes(b'foreign lockfile')
                if case == 'extra':
                    (target / 'unexpected.deb').write_bytes(b'extra')
                return ''

            with tempfile.TemporaryDirectory() as temporary, patch.object(
                    PUBLISH, 'command', side_effect=download), patch.object(
                    PUBLISH.subprocess, 'check_output', return_value=lock):
                return PUBLISH.verified_assets(Path(temporary), 'owner/repo', 'v1.3.0', sha, '123', run)

        self.assertEqual(len(verify('valid')), 4)
        for case in ('source', 'attempt', 'checksum', 'package', 'lock', 'extra'):
            with self.subTest(case=case), self.assertRaises(ValueError):
                verify(case)


if __name__ == '__main__':
    unittest.main()
