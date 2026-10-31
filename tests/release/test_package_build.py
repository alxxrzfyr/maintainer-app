"""Build synthetic payloads only; never install packages or rebuild app binaries."""
import io
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tarfile
import unittest


class FreshPackageStaging(unittest.TestCase):
    def test_stale_payload_cannot_enter_package_and_stage_is_removed(self):
        root = Path(__file__).resolve().parents[2]
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            for directory in ('data', 'packaging', 'scripts'):
                shutil.copytree(root / directory, work / directory)
            binaries = work / 'target/release'
            binaries.mkdir(parents=True)
            for name in ('maintainer-app', 'maintainer-helper'):
                (binaries / name).write_bytes(b'synthetic executable fixture')
            tools = work / 'tools'
            tools.mkdir()
            cargo = tools / 'cargo'
            cargo.write_text('#!/bin/sh\nprintf \'%s\\n\' \'{"packages":[{"name":"maintainer-app","version":"0.0.1"}]}\'\n')
            cargo.chmod(0o755)
            architecture = subprocess.check_output(['dpkg', '--print-architecture'], text=True).strip()
            stale = work / f'maintainer_0.0.1-1_{architecture}'
            stale.mkdir()
            (stale / 'stale-payload').write_text('must never ship')
            environment = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ['PATH'],
                               EXPECTED_DEB_ARCH=architecture, TMPDIR=str(work))
            subprocess.run(['bash', str(root / 'scripts/build-package.sh')], cwd=work,
                           env=environment, check=True, capture_output=True, text=True, umask=0o002)
            package = work / f'maintainer_0.0.1-1_{architecture}.deb'
            listing = subprocess.check_output(['dpkg-deb', '--contents', str(package)], text=True)
            spec = importlib.util.spec_from_file_location('local_gate', root / 'scripts/check-release-local.py')
            local_gate = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(local_gate)
            local_gate.ROOT = work
            hashes = local_gate.inspect_package(package)
            self.assertIn('usr/bin/maintainer', hashes)
            (binaries / 'maintainer-app').write_bytes(b'different runtime')
            with self.assertRaises(ValueError):
                local_gate.inspect_package(package)
            self.assertNotIn('stale-payload', listing)
            self.assertIn('./usr/bin/maintainer', listing)
            archive = subprocess.check_output(['dpkg-deb', '--fsys-tarfile', str(package)])
            with tarfile.open(fileobj=io.BytesIO(archive), mode='r:') as payload:
                for member in payload.getmembers():
                    if member.isdir():
                        self.assertEqual(member.mode, 0o755, member.name)
                    elif member.isfile():
                        self.assertEqual(member.mode & 0o022, 0, member.name)
            self.assertTrue((stale / 'stale-payload').is_file())
            self.assertEqual(list(work.glob('maintainer-package.*')), [])
            extracted = work / 'extracted'
            subprocess.run(['dpkg-deb', '--extract', str(package), str(extracted)], check=True)
            for relative in ('usr/share/maintainer/styles/light.css',
                             'usr/share/maintainer/styles/dark.css',
                             'usr/share/dbus-1/system.d/io.github.maintainerapp.Helper.conf',
                             'lib/systemd/system/maintainer-helper.service',
                             'usr/share/icons/hicolor/symbolic/apps/io.github.maintainerapp.Maintainer-symbolic.svg'):
                self.assertTrue((extracted / relative).is_file(), relative)
            # Exercise the installed harness's byte comparator against the real
            # extracted synthetic package, without touching installed host files.
            harness = (root / 'scripts/test-package.sh').read_text()
            body = harness.split('verify_payload_file() {', 1)[1].split('\n}\n', 1)[0]
            comparator = 'fail() { exit 1; }\nverify_payload_file() {' + body + '\n}\nverify_payload_file "$1" "$2"\n'
            expected = extracted / 'usr/share/maintainer/styles/light.css'
            installed = work / 'installed.css'
            shutil.copyfile(expected, installed)
            command = ['bash', '-c', comparator, 'fixture', str(expected), str(installed)]
            self.assertEqual(subprocess.run(command, capture_output=True).returncode, 0)
            installed.write_text('stale source checkout style')
            self.assertNotEqual(subprocess.run(command, capture_output=True).returncode, 0)


if __name__ == '__main__':
    unittest.main()
