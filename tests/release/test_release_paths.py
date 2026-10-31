"""Release builds retain compiler settings while removing local path prefixes."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location('release_build', Path(__file__).resolve().parents[2] / 'scripts/build-release.py')
BUILD = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILD)


class ReleasePaths(unittest.TestCase):
    def test_existing_encoded_flags_and_specific_prefix_order(self):
        environment = {'HOME': '/home/example', 'CARGO_HOME': '/opt/cargo cache',
                       'CARGO_ENCODED_RUSTFLAGS': '-C\x1fopt-level=3', 'RUSTFLAGS': '-D warnings'}
        result = BUILD.release_environment(environment, Path('/home/example/project'))
        self.assertEqual(result['CARGO_ENCODED_RUSTFLAGS'].split('\x1f'), [
            '-C', 'opt-level=3', '--remap-path-prefix=/home/example=/build/user',
            '--remap-path-prefix=/opt/cargo cache=/build/cargo',
            '--remap-path-prefix=/home/example/project=/build/maintainer'])
        self.assertEqual(environment['CARGO_ENCODED_RUSTFLAGS'], '-C\x1fopt-level=3')

    def test_plain_flags_are_preserved_without_shell_expansion(self):
        result = BUILD.release_environment({'HOME': '/home/example', 'RUSTFLAGS': '-D warnings'})
        self.assertEqual(result['CARGO_ENCODED_RUSTFLAGS'].split('\x1f')[:2], ['-D', 'warnings'])

    def test_main_builds_the_locked_workspace_with_private_paths_remapped(self):
        with patch.object(BUILD.subprocess, 'run') as run, patch.object(BUILD, 'verify_release_paths') as verify:
            BUILD.main()
        arguments, options = run.call_args
        self.assertEqual(arguments[0], ['cargo', 'build', '--locked', '--workspace', '--release'])
        self.assertTrue(options['check'])
        self.assertIn('--remap-path-prefix=', options['env']['CARGO_ENCODED_RUSTFLAGS'])
        verify.assert_called_once()

    def test_binary_verification_rejects_embedded_local_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = root / 'target/release'
            directory.mkdir(parents=True)
            for name in ('maintainer-app', 'maintainer-helper'):
                (directory / name).write_bytes(b'/build/maintainer/file.rs')
            environment = {'HOME': '/home/example'}
            BUILD.verify_release_paths(root, environment)
            for sensitive in (str(root).encode(), b'/home/example/.cargo/registry'):
                (directory / 'maintainer-helper').write_bytes(sensitive)
                with self.assertRaises(ValueError):
                    BUILD.verify_release_paths(root, environment)

    def test_rustc_artifact_contains_remapped_source_location(self):
        # Exercise rustc itself: a mocked environment cannot prove path removal.
        with tempfile.TemporaryDirectory(prefix='maintainer-private-path-') as temporary:
            root = Path(temporary)
            source = root / 'private.rs'
            binary = root / 'probe'
            source.write_text('fn main() { println!("{}", file!()); }')
            BUILD.subprocess.run(['rustc', str(source), '--remap-path-prefix=' + str(root) + '=/build/maintainer', '-o', str(binary)], check=True)
            output = BUILD.subprocess.check_output([str(binary)], text=True)
            self.assertEqual(output.strip(), '/build/maintainer/private.rs')
            self.assertNotIn(str(root).encode(), binary.read_bytes())
