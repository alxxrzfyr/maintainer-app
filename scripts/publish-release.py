#!/usr/bin/env python3
"""Publish four exact installed-tested assets after validating source and tag."""
import importlib.util
import json
import os
from pathlib import Path
import posixpath
import re
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('candidate_gate', ROOT / 'scripts/validate-candidate.py')
GATE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GATE)


def command(*arguments):
    return subprocess.check_output(arguments, text=True, cwd=ROOT)


def api(repository, resource):
    return json.loads(command('gh', 'api', f'repos/{repository}/{resource}'))


def release_for_tag(repository, tag):
    try:
        return api(repository, f'releases/tags/{tag}')
    except subprocess.CalledProcessError as error:
        try:
            response = json.loads(error.output)
        except (TypeError, ValueError):
            raise error from None
        if response.get('status') == '404' and response.get('message') == 'Not Found':
            return None
        raise


def tag_commit(repository, tag):
    reference = api(repository, f'git/ref/tags/{tag}')
    if reference['object']['type'] == 'tag':
        reference = api(repository, f"git/tags/{reference['object']['sha']}")
    GATE.require(reference['object']['type'] == 'commit')
    GATE.require(re.fullmatch('[a-f0-9]{40}', reference['object']['sha']))
    return reference


def previous_release(repository, candidate_version):
    match = re.fullmatch(r'([0-9]+)\.([0-9]+)\.([0-9]+)-[0-9]+', candidate_version)
    GATE.require(match is not None)
    candidate = tuple(map(int, match.groups()))
    pages = json.loads(command('gh', 'api', '--paginate', '--slurp',
                              f'repos/{repository}/releases?per_page=100'))
    previous = []
    for release in (release for page in pages for release in page):
        match = re.fullmatch(r'v([0-9]+)\.([0-9]+)\.([0-9]+)', release['tag_name'])
        if match and not release['draft'] and not release['prerelease']:
            version = tuple(map(int, match.groups()))
            if version < candidate:
                previous.append((version, release['tag_name']))
    GATE.require(previous)
    return max(previous)[1]


def validate_source(directory, repository, tag, run_id, environment):
    GATE.require(re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repository))
    GATE.require(re.fullmatch(r'v[0-9]+\.[0-9]+\.[0-9]+', tag))
    GATE.require(re.fullmatch('[1-9][0-9]*', run_id))
    run = api(repository, f'actions/runs/{run_id}')
    reference = tag_commit(repository, tag)
    workflow = api(repository, f"actions/workflows/{run['workflow_id']}")
    jobs = json.loads(command('gh', 'api', '--paginate', '--slurp',
                             f'repos/{repository}/actions/runs/{run_id}/jobs?per_page=100'))
    for name, value in (('run', run), ('tag', reference), ('workflow', workflow), ('jobs', jobs)):
        (directory / f'{name}.json').write_text(json.dumps(value), encoding='utf-8')
    event = environment['GITHUB_EVENT_NAME']
    if event == 'push':
        GATE.require(environment['GITHUB_REF'] == 'refs/tags/' + tag)
        GATE.require(run_id == environment['GITHUB_RUN_ID'])
        GATE.require(run['head_sha'] == environment['GITHUB_SHA'])
        GATE.require(str(run['run_attempt']) == environment['GITHUB_RUN_ATTEMPT'])
        sha = GATE.source(directory, repository, run_id, tag_name=tag)
    else:
        GATE.require(event == 'workflow_dispatch')
        GATE.require(environment['GITHUB_REF'] == 'refs/heads/main')
        sha = GATE.source(directory, repository, run_id)
    subprocess.run(['git', 'merge-base', '--is-ancestor', sha, 'origin/main'],
                   cwd=ROOT, check=True)
    return sha, run


def verified_assets(directory, repository, tag, sha, run_id, run):
    version = tag.removeprefix('v')
    lock = subprocess.check_output(['git', 'show', f'{sha}:Cargo.lock'], cwd=ROOT)
    assets = []
    for architecture in ('amd64', 'arm64'):
        target = directory / architecture
        target.mkdir()
        command('gh', 'run', 'download', run_id, '--repo', repository,
                '--name', f'maintainer-validated-{architecture}', '--dir', str(target))
        GATE.artifact(target, architecture, sha, run_id, run)
        GATE.require((target / 'Cargo.lock').read_bytes() == lock)
        package = target / f'maintainer_{version}-1_{architecture}.deb'
        checksum = target / f'SHA256SUMS-{architecture}'
        GATE.require(set(path.name for path in target.iterdir()) == {
            package.name, checksum.name, 'Cargo.lock', f'SOURCE-{architecture}.json'})
        GATE.require(command('dpkg-deb', '-f', str(package), 'Version').strip() == version + '-1')
        GATE.require(command('dpkg-deb', '-f', str(package), 'Architecture').strip() == architecture)
        GATE.require(checksum.read_text(encoding='utf-8') ==
                     GATE.digest(package) + '  ' + package.name + '\n')
        assets.extend((package, checksum))
    return assets


def release_notes(tag, sha, repository):
    path = f'docs/releases/{tag.removeprefix("v")}.md'
    notes = command('git', 'show', f'{sha}:{path}')
    notes = notes.replace('Draft release notes. This version has not been published. ', '', 1)

    def link(match):
        destination = match.group(2)
        if destination.startswith(('https://', 'http://', '#')):
            return match.group(0)
        target = posixpath.normpath(posixpath.join(posixpath.dirname(path), destination))
        GATE.require(not target.startswith(('../', '/')))
        return f'[{match.group(1)}](https://github.com/{repository}/blob/{sha}/{target})'

    return re.sub(r'\[([^\]]+)\]\(([^)]+)\)', link, notes)


def publish(repository, tag, assets, notes_path):
    expected = {path.name: 'sha256:' + GATE.digest(path) for path in assets}
    GATE.require(len(expected) == 4)
    release = release_for_tag(repository, tag)
    if release is None:
        command('gh', 'release', 'create', tag, '--repo', repository, '--verify-tag',
                '--draft', '--title', f'Maintainer {tag.removeprefix("v")}',
                '--notes-file', str(notes_path))
        release = release_for_tag(repository, tag)
    GATE.require(release is not None and release['tag_name'] == tag)
    existing = {asset['name']: asset for asset in release['assets']}
    GATE.require(len(existing) == len(release['assets']) and set(existing) <= set(expected))
    for name, asset in existing.items():
        GATE.require(asset.get('digest') == expected[name])
    missing = [str(path) for path in assets if path.name not in existing]
    if missing:
        command('gh', 'release', 'upload', tag, *missing, '--repo', repository)
    release = release_for_tag(repository, tag)
    GATE.require(release is not None and len(release['assets']) == 4)
    GATE.require({asset['name']: asset.get('digest') for asset in release['assets']} == expected)
    if release['draft'] or release.get('prerelease', False):
        command('gh', 'release', 'edit', tag, '--repo', repository,
                '--draft=false', '--prerelease=false', '--latest', '--notes-file', str(notes_path))
    print(f'https://github.com/{repository}/releases/tag/{tag}')


def main():
    repository = os.environ['GITHUB_REPOSITORY']
    tag = os.environ['RELEASE_TAG']
    run_id = os.environ['CANDIDATE_RUN_ID']
    with tempfile.TemporaryDirectory(prefix='maintainer-publish-', dir=os.environ['RUNNER_TEMP']) as temporary:
        directory = Path(temporary)
        sha, run = validate_source(directory, repository, tag, run_id, os.environ)
        assets = verified_assets(directory, repository, tag, sha, run_id, run)
        notes_path = directory / 'release-notes.md'
        notes_path.write_text(release_notes(tag, sha, repository), encoding='utf-8')
        # Recheck the remote tag immediately before any release mutation.
        GATE.require(tag_commit(repository, tag)['object']['sha'] == sha)
        publish(repository, tag, assets, notes_path)


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == 'previous':
        print(previous_release(os.environ['GITHUB_REPOSITORY'], sys.argv[2]))
    else:
        main()
