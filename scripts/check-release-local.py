#!/usr/bin/env python3
"""Validate a native candidate on this machine without installing or publishing it."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile


ROOT = Path(__file__).resolve().parents[1]


def run(*command, **kwargs):
    return subprocess.run(command, cwd=ROOT, check=True, **kwargs)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_digest():
    paths = subprocess.check_output(['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'], cwd=ROOT).split(b'\0')
    result = hashlib.sha256()
    for name in sorted(set(paths) - {b''}):
        path = ROOT / os.fsdecode(name)
        result.update(name + b'\0')
        if path.is_symlink():
            result.update(b'link\0' + os.fsencode(os.readlink(path)))
        elif path.is_file():
            result.update(str(path.stat().st_mode & 0o777).encode() + b'\0' + path.read_bytes())
        else:
            result.update(b'deleted')
        result.update(b'\0')
    return result.hexdigest()


def inspect_package(package):
    archive = subprocess.check_output(['dpkg-deb', '--fsys-tarfile', str(package)])
    with tarfile.open(fileobj=io.BytesIO(archive)) as payload:
        members = payload.getmembers()
        for member in members:
            if member.uid != 0 or member.gid != 0 or member.mode & 0o7022:
                raise ValueError('Unsafe archive ownership or mode: ' + member.name)
            if not (member.isdir() or member.isfile()):
                raise ValueError('Unexpected archive entry: ' + member.name)
        names = {member.name.removeprefix('./'): member for member in members}
        mapping = {
            'usr/libexec/maintainer-apt-backend.py': 'scripts/maintainer-apt-backend.py',
            'usr/share/dbus-1/system-services/io.github.maintainerapp.PackageOwner.service': 'data/dbus/io.github.maintainerapp.PackageOwner.service',
            'usr/share/dbus-1/system.d/io.github.maintainerapp.PackageOwner.conf': 'data/dbus/io.github.maintainerapp.PackageOwner.conf',
            'lib/systemd/system/maintainer-package-owner.service': 'data/systemd/maintainer-package-owner.service',
            'usr/bin/maintainer': 'target/release/maintainer-app',
            'usr/libexec/maintainer-helper': 'target/release/maintainer-helper',
            'usr/share/dbus-1/system-services/io.github.maintainerapp.Helper.service': 'data/dbus/io.github.maintainerapp.Helper.service',
            'usr/share/dbus-1/system.d/io.github.maintainerapp.Helper.conf': 'data/dbus/io.github.maintainerapp.Helper.conf',
            'usr/share/polkit-1/actions/io.github.maintainerapp.policy': 'data/polkit/io.github.maintainerapp.policy',
            'lib/systemd/system/maintainer-helper.service': 'data/systemd/maintainer-helper.service',
        }
        hashes = {}
        for installed, source in mapping.items():
            member = names[installed]
            if not member.isfile():
                raise ValueError('Missing package integration file: ' + installed)
            content = payload.extractfile(member).read()
            if content != (ROOT / source).read_bytes():
                raise ValueError('Package differs from built source: ' + installed)
            if member.mode != (0o755 if installed in ('usr/bin/maintainer', 'usr/libexec/maintainer-helper', 'usr/libexec/maintainer-apt-backend.py') else 0o644):
                raise ValueError('Wrong integration file mode: ' + installed)
            hashes[installed] = hashlib.sha256(content).hexdigest()
    control = subprocess.check_output(['dpkg-deb', '--ctrl-tarfile', str(package)])
    with tarfile.open(fileobj=io.BytesIO(control)) as archive:
        for name in ('preinst', 'postinst', 'postrm'):
            member = archive.getmember('./' + name)
            if member.mode != 0o755 or member.uid != 0 or member.gid != 0:
                raise ValueError('Unsafe maintainer script: ' + name)
            if archive.extractfile(member).read() != (ROOT / 'packaging/debian' / name).read_bytes():
                raise ValueError('Stale maintainer script: ' + name)
    return hashes


def verify(directory):
    manifest = json.loads((directory / 'LOCAL-CANDIDATE.json').read_text())
    package_name = manifest['package']
    if Path(package_name).name != package_name:
        raise ValueError('Invalid candidate package path')
    package = directory / package_name
    if manifest['schema_version'] != 1 or manifest['scope'] != 'native-machine-static-package':
        raise ValueError('Unknown proof scope')
    if manifest['source_sha256'] != source_digest() or manifest['lock_sha256'] != digest(ROOT / 'Cargo.lock'):
        raise ValueError('Source changed since validation')
    if manifest['package_sha256'] != digest(package):
        raise ValueError('Candidate bytes changed')
    if manifest['payload_sha256'] != inspect_package(package):
        raise ValueError('Candidate payload changed')
    if manifest['checks'] != checks():
        raise ValueError('Incomplete local checks')
    return manifest


def checks():
    return ['format', 'clippy', 'native-and-python-tests-three-passes', 'debug-build',
            'release-build', 'desktop', 'appstream', 'schemas', 'static-package-integration']


def build(directory):
    if os.geteuid() == 0:
        raise ValueError('Run local validation as a normal user; native fixtures must not be skipped')
    if directory.exists():
        raise ValueError('Use a new candidate directory')
    if directory == ROOT or ROOT in directory.parents:
        raise ValueError('Candidate output must be outside the source tree')
    before = source_digest()
    run('cargo', 'fmt', '--all', '--', '--check')
    run('cargo', 'clippy', '--locked', '--workspace', '--all-targets', '--all-features', '--', '-D', 'warnings')
    for index in range(3):
        print(f'Strict local test pass {index + 1}/3', flush=True)
        run('bash', 'scripts/test-native.sh')
        for suite in ('performance', 'release', 'package_transaction'):
            interpreter = '/usr/bin/python3' if suite == 'package_transaction' else 'python3'
            run(interpreter, '-W', 'error::ResourceWarning', '-m', 'unittest', 'discover', '-s', 'tests/' + suite, '-p', 'test_*.py')
    run('cargo', 'build', '--locked', '--workspace')
    run('python3', 'scripts/build-release.py')
    run('desktop-file-validate', 'data/io.github.maintainerapp.Maintainer.desktop')
    run('appstreamcli', 'validate', '--no-net', 'data/io.github.maintainerapp.Maintainer.metainfo.xml')
    run('glib-compile-schemas', '--strict', '--dry-run', 'data/')
    architecture = subprocess.check_output(['dpkg', '--print-architecture'], text=True).strip()
    run('bash', 'scripts/build-package.sh', env=dict(os.environ, EXPECTED_DEB_ARCH=architecture))
    version = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=ROOT))
    version = next(package['version'] for package in version['packages'] if package['name'] == 'maintainer-app')
    package = ROOT / f'maintainer_{version}-1_{architecture}.deb'
    payload = inspect_package(package)
    if before != source_digest():
        raise ValueError('Source changed during validation; rerun')
    manifest = dict(schema_version=1, scope='native-machine-static-package', architecture=architecture,
                    source_sha256=before, lock_sha256=digest(ROOT / 'Cargo.lock'), package=package.name,
                    package_sha256=digest(package), payload_sha256=payload, checks=checks())
    directory.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=directory.parent) as temporary:
        stage = Path(temporary) / 'candidate'
        stage.mkdir()
        shutil.copyfile(package, stage / package.name)
        (stage / 'LOCAL-CANDIDATE.json').write_text(json.dumps(manifest, indent=2) + '\n')
        stage.rename(directory)
    verify(directory)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('build', 'verify'))
    parser.add_argument('directory', type=Path)
    args = parser.parse_args()
    try:
        (build if args.mode == 'build' else verify)(args.directory.resolve())
    except (OSError, ValueError, KeyError, TypeError, StopIteration, subprocess.SubprocessError, tarfile.TarError) as error:
        parser.exit(1, 'Local candidate blocked: ' + str(error) + '\n')
    print('Local candidate verified: native machine only; static package inspection, no installed/upgrade or other-system acceptance.')


if __name__ == '__main__':
    main()
