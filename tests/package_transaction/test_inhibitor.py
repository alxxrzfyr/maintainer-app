"""No live logind calls: real fixed fixture children and the shared native engine."""
import os
import subprocess
import unittest
from unittest.mock import patch

import apt.progress.base

from retained_plan import backend, fixture_inhibitor
import test_retained_plan as fixtures


class HolderTests(unittest.TestCase):
    def test_real_echo_handshake_and_eof_release(self):
        holder = fixture_inhibitor()
        process = holder.process
        self.assertIsNone(process.poll())
        self.assertEqual(os.getpgid(process.pid), process.pid)
        holder.close()
        self.assertEqual(process.returncode, 0)
        holder.close()

    def test_denial_bad_handshake_and_timeout_fail_closed(self):
        commands = [('/usr/bin/false',),
                    ('/usr/bin/python3', '-I', '-c', 'import os;os.write(1,b"wrong\\n")'),
                    ('/usr/bin/sleep', '10')]
        for command in commands:
            with self.subTest(command=command):
                with patch.object(backend, 'INHIBITOR_COMMAND', command):
                    with self.assertRaisesRegex(backend.Rejected, 'inhibition_unavailable'):
                        backend.ShutdownInhibitor()

    def test_unresponsive_holder_cleanup_only_reaps_its_own_group(self):
        holder = fixture_inhibitor()
        holder.close()
        process = subprocess.Popen(['/usr/bin/sleep', '10'], stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE, start_new_session=True)
        holder.process = process
        holder.close()
        self.assertEqual(process.returncode, -9)


class NativeInhibitionTests(fixtures.AptFixture):
    def test_inhibitor_spans_mutation_postconditions_receipt_and_lock_release(self):
        transaction = self.transaction()
        _, identifier = transaction.resolve([('maintainer-fixture-app', '1')])
        observed = []
        process = []
        def factory():
            holder = fixture_inhibitor()
            process.append(holder.process)
            close = holder.close
            def release():
                self.assertIsNone(transaction.lock)
                self.assertIsNone(transaction.list_lock)
                observed.append('release')
                close()
            holder.close = release
            return holder
        transaction.inhibitor_factory = factory
        def event(owner):
            if owner.phase in ('mutating', 'succeeded'):
                owner.inhibitor.ensure_alive()
                observed.append(owner.phase)
        transaction.events = event
        original = backend.installed
        def postconditions(cache):
            transaction.inhibitor.ensure_alive()
            observed.append('postconditions')
            return original(cache)
        with patch.object(backend, 'installed', postconditions):
            self.assertTrue(transaction.commit(identifier))
        self.assertEqual(observed, ['mutating', 'postconditions', 'succeeded', 'release'])
        self.assertEqual(process[0].returncode, 0)

    def test_inhibitor_denial_prevents_mutation(self):
        transaction = self.transaction()
        _, identifier = transaction.resolve([('maintainer-fixture-app', '1')])
        def denied():
            raise backend.Rejected('inhibition_unavailable')
        transaction.inhibitor_factory = denied
        with self.assertRaisesRegex(backend.Rejected, 'inhibition_unavailable'):
            transaction.commit(identifier)
        self.assertFalse(transaction.mutation_started)
        self.assertEqual(transaction.phase, 'failed')
        self.assertEqual(transaction.error, 'inhibition_unavailable')
        self.assertEqual((self.root / 'var/lib/dpkg/status').read_text(), '')

    def test_mutation_failure_retains_inhibitor_until_recovery_receipt(self):
        transaction = self.transaction()
        _, identifier = transaction.resolve([('maintainer-fixture-app', '1')])
        observed = []
        def event(owner):
            if owner.phase == 'recovery_required':
                owner.inhibitor.ensure_alive()
                observed.append(owner.phase)
        transaction.events = event
        with patch.object(apt.progress.base.InstallProgress, 'run', side_effect=RuntimeError('fixture failure')):
            with self.assertRaises(RuntimeError):
                transaction.commit(identifier)
        self.assertEqual(observed, ['recovery_required'])
        self.assertIsNone(transaction.inhibitor)


if __name__ == '__main__':
    unittest.main()
