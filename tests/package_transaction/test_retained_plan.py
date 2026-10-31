"""Run with /usr/bin/python3 -m unittest discover -s tests/package_transaction -v.

No root, network, host APT state, maintainer scripts, or host package changes.
The trusted local repository is deliberately a fixture, not signature evidence.
"""
import fcntl
import hashlib
import multiprocessing
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import apt
import apt_pkg
from retained_plan import RetainedPlan, backend


def lock_available(path, result):
    with open(path, 'a') as stream:
        try:
            fcntl.lockf(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
            result.put(True)
        except BlockingIOError:
            result.put(False)


class AptFixture(unittest.TestCase):
    def setUp(self):
        if os.geteuid() == 0:
            self.skipTest('fixture intentionally requires an unprivileged user')
        self.temporary = tempfile.TemporaryDirectory(prefix='maintainer-apt-proof-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for name in ('etc/apt/sources.list.d', 'var/lib/dpkg', 'var/lib/apt/lists/partial',
                     'var/cache/apt/archives/partial', 'var/lib/dpkg/updates', 'var/log/apt', 'repo'):
            (self.root / name).mkdir(parents=True)
        (self.root / '.maintainer-fixture').touch()
        (self.root / 'var/lib/dpkg/status').touch()
        self.entries = []
        self.package('maintainer-fixture-library', '1')
        self.package('maintainer-fixture-app', '1', 'maintainer-fixture-library (= 1)')
        self.publish()
        # Clear all inherited hooks/options. Explicit paths keep APT and dpkg
        # writes inside this fixture, including dpkg's otherwise absolute log.
        for key in list(apt_pkg.config.keys()):
            apt_pkg.config.clear(key)
        settings = {
            'APT::Architecture': 'amd64', 'APT::Architectures::': 'amd64',
            'Dir': str(self.root), 'Dir::State': 'var/lib/apt',
            'Dir::State::status': str(self.root / 'var/lib/dpkg/status'),
            'Dir::State::lists': 'lists', 'Dir::Cache': 'var/cache/apt',
            'Dir::Cache::archives': 'archives', 'Dir::Etc': 'etc/apt',
            'Dir::Etc::sourcelist': 'sources.list',
            'Dir::Etc::sourceparts': 'sources.list.d',
            'Dir::Bin::dpkg': '/usr/bin/dpkg', 'Dir::Bin::methods': '/usr/lib/apt/methods',
            'Dir::Log': str(self.root / 'var/log/apt'),
            'Acquire::IndexTargets::deb::Packages::MetaKey': '$(COMPONENT)/binary-$(ARCHITECTURE)/Packages',
            'Acquire::IndexTargets::deb::Packages::flatMetaKey': 'Packages',
            'Acquire::IndexTargets::deb::Packages::ShortDescription': 'Packages',
        }
        for key, value in settings.items():
            apt_pkg.config.set(key, value)
        for option in (f'--root={self.root}', '--force-not-root',
                       f'--log={self.root}/var/log/dpkg.log'):
            apt_pkg.config.set('DPkg::Options::', option)
        apt_pkg.init_system()
        cache = apt.Cache()
        cache.update()

    def package(self, name, version, depends='', architecture='all', multiarch=''):
        build = self.root / f'build-{name}-{version}-{architecture}'
        (build / 'DEBIAN').mkdir(parents=True)
        payload = build / 'usr/share' / name
        payload.mkdir(parents=True)
        (payload / 'version').write_text(version)
        control = (f'Package: {name}\nVersion: {version}\nArchitecture: {architecture}\n'
                   'Maintainer: Fixture <fixture@invalid>\nDescription: fixture only\n')
        if multiarch:
            control += f'Multi-Arch: {multiarch}\n'
        if depends:
            control += f'Depends: {depends}\n'
        (build / 'DEBIAN/control').write_text(control)
        filename = f'{name}-{version}' + ('' if architecture == 'all' else '-' + architecture) + '.deb'
        subprocess.run(['/usr/bin/dpkg-deb', '--build', str(build),
                        str(self.root / 'repo' / filename)], check=True,
                       stdout=subprocess.DEVNULL)
        data = (self.root / 'repo' / filename).read_bytes()
        self.entries.append(control + f'Filename: {filename}\nSize: {len(data)}\n'
                            f'SHA256: {hashlib.sha256(data).hexdigest()}\n\n')

    def publish(self):
        (self.root / 'repo/Packages').write_text(''.join(self.entries))
        (self.root / 'etc/apt/sources.list').write_text(
            f'deb [trusted=yes] file:{self.root}/repo ./\n')

    def transaction(self):
        transaction = RetainedPlan(self.root)
        self.addCleanup(transaction.close)
        return transaction

    def available(self, filename):
        context = multiprocessing.get_context('fork')
        result = context.Queue()
        worker = context.Process(target=lock_available,
                                 args=(str(self.root / 'var/lib/dpkg' / filename), result))
        worker.start()
        worker.join(5)
        self.assertFalse(worker.is_alive())
        self.assertEqual(worker.exitcode, 0)
        return result.get(timeout=1)


class RetainedPlanTests(AptFixture):
    def test_actual_commit_consumes_reviewed_dependencies_and_releases_locks(self):
        transaction = self.transaction()
        changes, identifier = transaction.resolve([('maintainer-fixture-app', '1')])
        self.assertEqual([c.name for c in changes],
                         ['maintainer-fixture-app', 'maintainer-fixture-library'])
        self.assertTrue(all(c.new_version == '1' and c.architecture == 'all' for c in changes))
        self.assertFalse(self.available('lock-frontend'))
        self.assertFalse(self.available('lock'))
        self.assertTrue(transaction.commit(identifier))
        self.assertEqual(transaction.phase, 'succeeded')
        actual = apt.Cache()
        installed = {(p.name, p.installed.version) for p in actual if p.is_installed}
        self.assertEqual(installed, {(c.name, c.new_version) for c in changes})
        self.assertTrue(self.available('lock-frontend'))
        self.assertTrue(self.available('lock'))

    def test_repository_changes_fail_closed_instead_of_re_resolving(self):
        transaction = self.transaction()
        changes, identifier = transaction.resolve([('maintainer-fixture-app', '1')])
        self.package('maintainer-fixture-library', '2')
        self.package('maintainer-fixture-app', '2', 'maintainer-fixture-library (= 2)')
        self.publish()
        # Another index refresh is allowed while the package-system lock is
        # held. It changes disk indexes, not this already retained depcache.
        # A root-TCB writer can replace files while bypassing advisory locks;
        # retaining old records must still fail safely, never re-resolve.
        transaction.list_lock.__exit__(None, None, None)
        transaction.list_lock = None
        apt.Cache().update()
        with self.assertRaises(apt_pkg.Error):
            transaction.commit(identifier)
        self.assertEqual(transaction.phase, 'failed')
        self.assertEqual((self.root / 'var/lib/dpkg/status').read_text(), '')
        self.assertTrue(all(change.new_version == '1' for change in changes))
        self.assertEqual(apt.Cache()['maintainer-fixture-app'].candidate.version, '2')

    def test_archive_tampering_fails_before_dpkg(self):
        transaction = self.transaction()
        _, identifier = transaction.resolve([('maintainer-fixture-app', '1')])
        archive = self.root / 'repo/maintainer-fixture-app-1.deb'
        archive.write_bytes(archive.read_bytes() + b'tampered')
        with self.assertRaises(apt.cache.FetchFailedException):
            transaction.commit(identifier)
        self.assertEqual(transaction.phase, 'failed')
        self.assertEqual((self.root / 'var/lib/dpkg/status').read_text(), '')
        self.assertTrue(self.available('lock-frontend'))

    def test_frontend_lock_retained_and_cancellation_denied_at_mutation(self):
        transaction = self.transaction()
        _, identifier = transaction.resolve([('maintainer-fixture-app', '1')])
        original = apt.progress.base.InstallProgress.run
        observed = []
        def run(progress, manager):
            self.assertEqual(transaction.phase, 'mutating')
            self.assertFalse(transaction.cancel())
            self.assertFalse(self.available('lock-frontend'))
            # APT releases the inner lock for its dpkg child while retaining
            # the frontend lock against other conforming APT frontends.
            self.assertTrue(self.available('lock'))
            competing = subprocess.run(
                ['/usr/bin/dpkg', f'--root={self.root}', '--force-not-root',
                 f'--log={self.root}/var/log/dpkg.log', '--unpack',
                 str(self.root / 'repo/maintainer-fixture-library-1.deb')],
                capture_output=True, text=True)
            self.assertNotEqual(competing.returncode, 0)
            self.assertIn('frontend lock', competing.stderr)
            observed.append(True)
            return original(progress, manager)
        with patch.object(apt.progress.base.InstallProgress, 'run', run):
            self.assertTrue(transaction.commit(identifier))
        self.assertEqual(observed, [True])

    def test_full_upgrade_then_dependency_removal_match_actual_sets(self):
        initial = self.transaction()
        _, identifier = initial.resolve([('maintainer-fixture-app', '1')])
        initial.commit(identifier)
        self.package('maintainer-fixture-library', '2')
        self.package('maintainer-fixture-app', '2', 'maintainer-fixture-library (= 2)')
        self.publish()
        apt.Cache().update()
        upgrade = self.transaction()
        changes, identifier = upgrade.resolve(full=True)
        self.assertEqual(len(changes), 2)
        self.assertTrue(all(c.old_version == '1' and c.new_version == '2'
                            and c.operation == 'upgrade' for c in changes))
        upgrade.commit(identifier)
        actual = apt.Cache()
        self.assertEqual({(p.name, p.installed.version) for p in actual if p.is_installed},
                         {(c.name, c.new_version) for c in changes})
        removal = self.transaction()
        changes, identifier = removal.resolve(remove=['maintainer-fixture-library'])
        self.assertEqual({c.name for c in changes},
                         {'maintainer-fixture-app', 'maintainer-fixture-library'})
        self.assertTrue(all(c.operation == 'remove' and c.old_version == '2'
                            and c.new_version == '' for c in changes))
        removal.commit(identifier)
        self.assertFalse(any(p.is_installed for p in apt.Cache()))

    def test_kernel_removal_preserves_running_and_fallback_images(self):
        kernels = [f'linux-image-1.0.0-{number}-fixture' for number in (1, 2, 3)]
        for name in kernels:
            self.package(name, '1')
        self.publish()
        apt.Cache().update()
        install = self.transaction()
        _, identifier = install.resolve([(name, '1') for name in kernels])
        install.commit(identifier)
        remove = self.transaction()
        _, identifier = remove.resolve_intent('kernel', (kernels[0],))
        remove.commit(identifier)
        fallback = self.transaction()
        with self.assertRaisesRegex(backend.Rejected, 'kernel_fallback_required'):
            fallback.resolve_intent('kernel', (kernels[1],))
        fallback.close()
        protected = self.transaction()
        with patch.object(backend, 'running_kernel_package', side_effect=lambda name: name == kernels[1]):
            with self.assertRaisesRegex(backend.Rejected, 'protected_package'):
                protected.resolve_intent('kernel', (kernels[1],))
        self.assertTrue(apt.Cache()[kernels[1]].is_installed)

    def test_architecture_aliases_cannot_request_same_package_twice(self):
        transaction = self.transaction()
        with self.assertRaisesRegex(backend.Rejected, 'duplicate_package_identity'):
            transaction.resolve([('maintainer-fixture-app', '1'),
                                 ('maintainer-fixture-app:amd64', '1')])

    def test_explicit_fetch_cancellation_never_enters_dpkg(self):
        transaction = self.transaction()
        _, identifier = transaction.resolve([('maintainer-fixture-app', '1')])
        transaction.cancelled = lambda: transaction.phase == 'fetching'
        with self.assertRaises(Exception):
            transaction.commit(identifier)
        self.assertEqual(transaction.phase, 'cancelled')
        self.assertEqual((self.root / 'var/lib/dpkg/status').read_text(), '')
        self.assertTrue(self.available('lock-frontend'))

    def test_maintainer_self_update_is_reserved_for_external_package_manager(self):
        self.package('maintainer', '1')
        self.publish()
        apt.Cache().update()
        transaction = self.transaction()
        with self.assertRaisesRegex(backend.Rejected, 'maintainer_requires_external_update'):
            transaction.resolve([('maintainer', '1')])
        self.assertEqual((self.root / 'var/lib/dpkg/status').read_text(), '')

    def test_kernel_metapackage_tracking_cannot_be_removed_by_the_resolver(self):
        kernels = [f'linux-image-5.4.0-{n}-generic' for n in (25, 26, 27)]
        for name in kernels:
            self.package(name, '1')
        self.package('linux-image-generic', '1', kernels[0] + ' (= 1)')
        self.publish()
        apt.Cache().update()
        transaction = self.transaction()
        _, identifier = transaction.resolve([(name, '1') for name in
                                              ['linux-image-generic', *kernels[1:]]])
        transaction.commit(identifier)
        protected = self.transaction()
        with self.assertRaisesRegex(backend.Rejected, 'protected_package'):
            protected.resolve_intent('kernel', (kernels[0],))
        self.assertTrue(apt.Cache()['linux-image-generic'].is_installed)
        self.assertTrue(apt.Cache()[kernels[0]].is_installed)

    def test_preexisting_interrupted_configuration_is_not_implicitly_executed(self):
        (self.root / 'var/lib/dpkg/updates/0001').write_text('interrupted fixture')
        with self.assertRaisesRegex(backend.Rejected, 'recovery_required'):
            self.transaction()
        self.assertEqual((self.root / 'var/lib/dpkg/status').read_text(), '')

    def test_selected_downgrade_is_explicit_and_matches_actual_version(self):
        self.package('maintainer-fixture-app', '2', 'maintainer-fixture-library (= 1)')
        self.publish()
        apt.Cache().update()
        initial = self.transaction()
        _, identifier = initial.resolve([('maintainer-fixture-app', '2')])
        initial.commit(identifier)
        downgrade = self.transaction()
        changes, identifier = downgrade.resolve([('maintainer-fixture-app', '1')])
        self.assertEqual([(c.operation, c.old_version, c.new_version) for c in changes],
                         [('downgrade', '2', '1')])
        downgrade.commit(identifier)
        self.assertEqual(apt.Cache()['maintainer-fixture-app'].installed.version, '1')

    def test_selected_held_package_is_not_changed(self):
        initial = self.transaction()
        _, identifier = initial.resolve([('maintainer-fixture-app', '1')])
        initial.commit(identifier)
        status = self.root / 'var/lib/dpkg/status'
        status.write_text(status.read_text().replace('Status: install ok installed',
                                                    'Status: hold ok installed'))
        self.package('maintainer-fixture-app', '2', 'maintainer-fixture-library (= 1)')
        self.publish()
        apt.Cache().update()
        held = self.transaction()
        with self.assertRaisesRegex(backend.Rejected, 'held_package'):
            held.resolve([('maintainer-fixture-app', '2')])
        self.assertEqual(apt.Cache()['maintainer-fixture-app'].installed.version, '1')

    def test_multiarch_identity_is_explicit_and_actual_install_matches(self):
        self.package('maintainer-fixture-multi', '1', architecture='amd64', multiarch='same')
        self.package('maintainer-fixture-multi', '1', architecture='i386', multiarch='same')
        self.publish()
        apt_pkg.config.set('APT::Architectures::', 'i386')
        apt_pkg.init_system()
        subprocess.run(['/usr/bin/dpkg', f'--root={self.root}', '--force-not-root',
                        '--add-architecture', 'i386'], check=True, capture_output=True)
        apt.Cache().update()
        transaction = self.transaction()
        changes, identifier = transaction.resolve([('maintainer-fixture-multi:amd64', '1'),
                                                   ('maintainer-fixture-multi:i386', '1')])
        self.assertEqual({(c.name, c.architecture) for c in changes},
                         {('maintainer-fixture-multi', 'amd64'),
                          ('maintainer-fixture-multi', 'i386')})
        transaction.commit(identifier)
        self.assertEqual(backend.installed(apt.Cache()),
                         {(c.name, c.architecture): c.new_version for c in changes})

    def test_installation_failure_requires_recovery_and_cannot_be_retried(self):
        transaction = self.transaction()
        _, identifier = transaction.resolve([('maintainer-fixture-app', '1')])
        with patch.object(apt.progress.base.InstallProgress, 'run',
                          side_effect=RuntimeError('injected dpkg-stage failure')):
            with self.assertRaisesRegex(RuntimeError, 'dpkg-stage'):
                transaction.commit(identifier)
        self.assertEqual(transaction.phase, 'recovery_required')
        self.assertFalse(transaction.cancel())
        with self.assertRaises(ValueError):
            transaction.commit(identifier)
        self.assertTrue(self.available('lock-frontend'))

    def test_wrong_approval_and_changed_depcache_never_mutate(self):
        transaction = self.transaction()
        _, identifier = transaction.resolve([('maintainer-fixture-app', '1')])
        with self.assertRaisesRegex(ValueError, 'binding'):
            transaction.commit('unreviewed')
        transaction.cache['maintainer-fixture-library'].mark_keep()
        with self.assertRaisesRegex(ValueError, 'binding'):
            transaction.commit(identifier)
        self.assertEqual((self.root / 'var/lib/dpkg/status').read_text(), '')

    def test_cancel_before_mutation_releases_native_locks(self):
        transaction = self.transaction()
        transaction.resolve([('maintainer-fixture-app', '1')])
        self.assertTrue(transaction.cancel())
        self.assertFalse(transaction.cancel())
        self.assertEqual(transaction.phase, 'cancelled')
        self.assertTrue(self.available('lock-frontend'))
        with self.assertRaises(ValueError):
            transaction.commit('anything')

    def test_native_lock_contention_refuses_second_owner(self):
        transaction = self.transaction()
        context = multiprocessing.get_context('fork')
        result = context.Queue()
        def contender():
            # Child inherited libapt's lock counter, so use a fresh interpreter.
            code = ('import fcntl,sys; f=open(sys.argv[1],"a"); '
                    'fcntl.lockf(f,fcntl.LOCK_EX|fcntl.LOCK_NB)')
            attempt = subprocess.run(['/usr/bin/python3', '-c', code,
                                      str(self.root / 'var/lib/dpkg/lock-frontend')],
                                     capture_output=True)
            result.put(attempt.returncode)
        worker = context.Process(target=contender)
        worker.start()
        worker.join(5)
        self.assertNotEqual(result.get(timeout=1), 0)
        transaction.close()

    def test_unsigned_repository_is_not_implicitly_authorized(self):
        (self.root / 'etc/apt/sources.list').write_text(
            f'deb file:{self.root}/repo ./\n')
        with self.assertRaises(apt.cache.FetchFailedException):
            apt.Cache().update()


if __name__ == '__main__':
    unittest.main()
