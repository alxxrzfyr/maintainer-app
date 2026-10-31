#!/usr/bin/env python3
"""Bind triple-tested, installed-validated CI artifacts to their source; never rebuild."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys


def require(condition):
    if not condition:
        raise ValueError('candidate source or artifact binding failed')


def load(path):
    return json.loads(path.read_text())


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source(metadata, repository, run_id, tag_name=None):
    workflow, run, tag = (load(metadata / name) for name in ('workflow.json', 'run.json', 'tag.json'))
    require(run['id'] == int(run_id) and run['repository']['full_name'] == repository)
    require(workflow['path'] == '.github/workflows/ci.yml' and run['workflow_id'] == workflow['id'])
    require(run['path'] == '.github/workflows/ci.yml')
    if tag_name is None:
        require(run['status'] == 'completed' and run['conclusion'] == 'success')
        require(run['head_branch'] == 'main')
    else:
        require(re.fullmatch(r'v[0-9]+\.[0-9]+\.[0-9]+', tag_name))
        require(run['head_branch'] == tag_name)
        require((run['status'], run['conclusion']) in
                (('in_progress', None), ('completed', 'success')))
    require(run['event'] == 'push')
    sha = run['head_sha']
    require(type(sha) is str and re.fullmatch('[a-f0-9]{40}', sha))
    require(tag['object']['type'] == 'commit' and tag['object']['sha'] == sha)
    jobs = [job for page in load(metadata / 'jobs.json') for job in page['jobs']]
    publishers = [job for job in jobs if job['name'] == 'publish']
    require(len(publishers) <= 1)
    for job in publishers:
        require(job['run_id'] == run['id'] and job['head_sha'] == sha)
        allowed = (('completed', 'skipped'),) if tag_name is None else (
            ('queued', None), ('in_progress', None), ('completed', 'success'))
        require((job['status'], job['conclusion']) in allowed)
    jobs = [job for job in jobs if job['name'] != 'publish']
    expected = {'check', 'package-amd64', 'package-arm64', 'installed-amd64', 'installed-arm64'}
    require({job['name'] for job in jobs} == expected and len(jobs) == len(expected))
    check = next(job for job in jobs if job['name'] == 'check')
    require(any(step.get('name') == 'Strict full tests (three passes)'
                and step['conclusion'] == 'success' for step in check['steps']))
    for job in jobs:
        require(job['run_id'] == run['id'] and job['head_sha'] == sha)
        require(job['status'] == 'completed' and job['conclusion'] == 'success')
        require(all(step['conclusion'] == 'success' for step in job['steps']))
    return sha


def artifact(directory, architecture, sha, run_id, run):
    manifest = load(directory / f'SOURCE-{architecture}.json')
    require(manifest['schema_version'] == 1 and type(manifest['schema_version']) is int)
    require(manifest['commit'] == sha and manifest['run_id'] == str(run_id))
    require(manifest['run_attempt'] == str(run['run_attempt']))
    require(manifest['architecture'] == architecture)
    require(manifest['lock_sha256'] == digest(directory / 'Cargo.lock'))
    packages = list(directory.glob('*.deb'))
    require(len(packages) == 1 and packages[0].name == manifest['package'])
    require(manifest['package_sha256'] == digest(packages[0]))
    require(manifest['repository'] == run['repository']['full_name'])


def main():
    try:
        mode, *args = sys.argv[1:]
        if mode == 'manifest':
            directory, architecture = Path(args[0]), args[1]
            packages = list(directory.glob(f'maintainer_*_{architecture}.deb'))
            require(len(packages) == 1)
            data = {'schema_version': 1, 'commit': os.environ['GITHUB_SHA'],
                    'repository': os.environ['GITHUB_REPOSITORY'],
                    'run_id': os.environ['GITHUB_RUN_ID'], 'run_attempt': os.environ['GITHUB_RUN_ATTEMPT'],
                    'architecture': architecture, 'lock_sha256': digest(directory / 'Cargo.lock'),
                    'package': packages[0].name, 'package_sha256': digest(packages[0])}
            (directory / f'SOURCE-{architecture}.json').write_text(json.dumps(data, indent=2) + '\n')
        elif mode == 'source':
            print('source_sha=' + source(Path(args[0]), args[1], args[2]))
        elif mode == 'tag':
            repository, tag, sha = args
            require(re.fullmatch(r'v[0-9]+\.[0-9]+\.[0-9]+', tag))
            response = json.loads(subprocess.check_output(['gh', 'api', f'repos/{repository}/git/ref/tags/{tag}'], text=True))
            if response['object']['type'] == 'tag':
                object_sha = response['object']['sha']
                require(re.fullmatch('[a-f0-9]{40}', object_sha))
                response = json.loads(subprocess.check_output(['gh', 'api', f'repos/{repository}/git/tags/{object_sha}'], text=True))
            require(response['object']['type'] == 'commit' and response['object']['sha'] == sha)
        elif mode == 'artifact':
            artifact(Path(args[0]), args[1], args[2], args[3], load(Path(args[4])))
            version = subprocess.check_output(['dpkg-deb', '-f', str(next(Path(args[0]).glob('*.deb'))), 'Version'], text=True).strip()
            require(version == args[5].removeprefix('v') + '-1')
        else:
            raise ValueError()
    except (OSError, ValueError, KeyError, TypeError, IndexError, subprocess.SubprocessError):
        sys.exit('Release blocked: candidate source or artifact binding failed.')


if __name__ == '__main__':
    main()
