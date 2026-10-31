#!/usr/bin/env python3
"""Build locked release binaries without embedding personal build paths."""
import os
from pathlib import Path
import subprocess


ROOT = Path(__file__).resolve().parents[1]


def release_environment(environment, root=ROOT):
    result = dict(environment)
    encoded = result.get('CARGO_ENCODED_RUSTFLAGS')
    flags = encoded.split('\x1f') if encoded else result.get('RUSTFLAGS', '').split()
    home = Path(result['HOME']).resolve()
    cargo_home = Path(result.get('CARGO_HOME', home / '.cargo')).resolve()
    # More specific prefixes come last because rustc applies the last match.
    flags.extend([
        f'--remap-path-prefix={home}=/build/user',
        f'--remap-path-prefix={cargo_home}=/build/cargo',
        f'--remap-path-prefix={root}=/build/maintainer',
    ])
    result['CARGO_ENCODED_RUSTFLAGS'] = '\x1f'.join(flags)
    return result


def main():
    subprocess.run(['cargo', 'build', '--locked', '--workspace', '--release'],
                   cwd=ROOT, env=release_environment(os.environ), check=True)
    verify_release_paths(ROOT, os.environ)


def verify_release_paths(root, environment):
    home = Path(environment['HOME']).resolve()
    prefixes = (os.fsencode(root), os.fsencode(Path(environment.get('CARGO_HOME', home / '.cargo')).resolve()))
    target = Path(environment.get('CARGO_TARGET_DIR', root / 'target'))
    if not target.is_absolute():
        target = root / target
    for name in ('maintainer-app', 'maintainer-helper'):
        content = (target / 'release' / name).read_bytes()
        if any(prefix in content for prefix in prefixes):
            raise ValueError('Release binary contains a local build path: ' + name)


if __name__ == '__main__':
    main()
