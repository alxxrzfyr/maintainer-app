"""Exercise the production owner's actual pipes, receipts and native engine."""
import multiprocessing
import io
import os
from pathlib import Path
import select
import subprocess
import unittest
from unittest.mock import patch

from retained_plan import backend, RetainedPlan
import test_retained_plan as fixtures


def serve(directory, identifier, request_fd, response_fd, close_fds):
    for descriptor in close_fds:
        os.close(descriptor)
    # Exactly as production, utilities have no terminal/protocol descriptors.
    null = os.open('/dev/null', os.O_RDWR)
    for descriptor in (0, 1, 2):
        os.dup2(null, descriptor)
    os.close(null)
    request = os.fdopen(request_fd, 'rb', buffering=0)
    output = os.fdopen(response_fd, 'w', encoding='ascii', buffering=1)
    with patch.object(backend, 'Backend', lambda **kwargs: RetainedPlan(Path(directory).parent, **kwargs)):
        result = backend.run_owner(os.getuid(), identifier, backend.Commands(request),
                                   output, Path(directory))
    os._exit(result)


class OwnerTests(fixtures.AptFixture):
    def start_owner(self, identifier='a' * 32):
        directory = self.root / 'receipts'
        directory.mkdir(mode=0o700, exist_ok=True)
        request_read, request_write = os.pipe()
        response_read, response_write = os.pipe()
        process = multiprocessing.get_context('fork').Process(
            target=serve, args=(str(directory), identifier, request_read,
                                response_write, (request_write, response_read)))
        process.start()
        os.close(request_read)
        os.close(response_write)
        sender = os.fdopen(request_write, 'w', encoding='ascii', buffering=1)
        receiver = os.fdopen(response_read, 'rb', buffering=0)
        self.addCleanup(sender.close)
        self.addCleanup(receiver.close)
        def cleanup():
            if process.is_alive():
                process.terminate()
                process.join(5)
        self.addCleanup(cleanup)
        return process, sender, receiver, directory / identifier

    def line(self, receiver):
        ready, _, _ = select.select([receiver], [], [], 10)
        self.assertTrue(ready, 'owner protocol deadline expired')
        return receiver.readline().decode('ascii').rstrip('\n')

    def review_owner(self, sender, receiver):
        sender.write('SELECTED\tmaintainer-fixture-app=1\n')
        lines = []
        while True:
            line = self.line(receiver)
            lines.append(line)
            if line == 'END':
                break
            self.assertTrue(line, lines)
        fingerprint = next(line.split('\t')[1] for line in lines if line.startswith('PLAN\t'))
        self.assertEqual(len(fingerprint), 64)
        return fingerprint, lines

    def finish(self, process, expected):
        process.join(15)
        self.assertFalse(process.is_alive(), 'native owner did not finish')
        self.assertEqual(process.exitcode, expected)

    def test_owner_approved_commit_survives_both_pipes_disconnecting(self):
        process, sender, receiver, receipt = self.start_owner()
        fingerprint, lines = self.review_owner(sender, receiver)
        self.assertEqual(len([line for line in lines if line.startswith('ITEM\t')]), 2)
        sender.write('APPROVE\t' + fingerprint + '\n')
        sender.close()
        receiver.close()
        self.finish(process, 0)
        data = receipt.read_text()
        self.assertIn('STATE\tsucceeded\n', data)
        self.assertIn('INTENT\tselected\nREQUEST\tmaintainer-fixture-app=1\n', data)
        self.assertEqual(receipt.stat().st_mode & 0o777, 0o600)
        self.assertEqual((self.root / 'usr/share/maintainer-fixture-app/version').read_text(), '1')

    def test_owner_preapproval_disconnect_cancels_without_mutation(self):
        process, sender, receiver, receipt = self.start_owner()
        self.review_owner(sender, receiver)
        sender.close()
        self.finish(process, 1)
        self.assertIn('STATE\tcancelled\n', receipt.read_text())
        self.assertEqual((self.root / 'var/lib/dpkg/status').read_text(), '')

    def test_owner_explicit_cancel_and_wrong_approval_do_not_mutate(self):
        for identifier, command in [('b' * 32, 'CANCEL'),
                                    ('c' * 32, 'APPROVE\t' + '0' * 64)]:
            process, sender, receiver, receipt = self.start_owner(identifier)
            self.review_owner(sender, receiver)
            sender.write(command + '\n')
            self.finish(process, 1)
            expected = 'cancelled' if command == 'CANCEL' else 'failed'
            self.assertIn('STATE\t' + expected + '\n', receipt.read_text())
            self.assertEqual((self.root / 'var/lib/dpkg/status').read_text(), '')

    def test_owner_does_not_overwrite_duplicate_receipt(self):
        process, sender, receiver, receipt = self.start_owner()
        self.review_owner(sender, receiver)
        sender.write('CANCEL\n')
        self.finish(process, 1)
        original = receipt.read_bytes()
        second, sender2, receiver2, _ = self.start_owner()
        sender2.write('FULL\n')
        self.finish(second, 1)
        self.assertEqual(receipt.read_bytes(), original)

    def test_global_owner_lock_refuses_another_transaction(self):
        first, sender, receiver, receipt = self.start_owner('d' * 32)
        self.review_owner(sender, receiver)
        second, sender2, receiver2, second_receipt = self.start_owner('f' * 32)
        sender2.write('FULL\n')
        self.finish(second, 1)
        self.assertFalse(second_receipt.exists())
        sender.write('CANCEL\n')
        self.finish(first, 1)
        self.assertIn('STATE\tcancelled\n', receipt.read_text())

    def test_owner_refuses_replay_after_interrupted_receipt(self):
        directory = self.root / 'receipts'
        directory.mkdir(mode=0o700)
        (directory / ('e' * 32)).write_text('VERSION\t1\nSTATE\tmutating\nEND\n')
        process, sender, receiver, receipt = self.start_owner()
        sender.write('FULL\n')
        self.finish(process, 1)
        self.assertFalse(receipt.exists())
        self.assertEqual((self.root / 'var/lib/dpkg/status').read_text(), '')

    def test_owner_receipt_admission_is_bounded(self):
        directory = self.root / 'receipts'
        directory.mkdir(mode=0o700)
        for index in range(128):
            (directory / f'{index:032x}').write_text('STATE\tcancelled\nEND\n')
        process, sender, receiver, receipt = self.start_owner()
        sender.write('FULL\n')
        self.finish(process, 1)
        self.assertFalse(receipt.exists())
        self.assertEqual(len([p for p in directory.iterdir() if backend.IDENTIFIER.fullmatch(p.name)]), 128)

    def test_production_cli_refuses_unprivileged_fixture_options(self):
        script = Path(__file__).resolve().parents[2] / 'scripts/maintainer-apt-backend.py'
        attempt = subprocess.run(['/usr/bin/python3', '-I', str(script),
                                  str(os.getuid()), 'a' * 32, '--root', str(self.root)],
                                 capture_output=True, timeout=5)
        self.assertEqual(attempt.returncode, 2)
        self.assertEqual(attempt.stdout, b'')

    def test_request_parser_rejects_options_controls_and_duplicates(self):
        for request in ('SELECTED\t--help=1', 'SELECTED\tbad/path=1',
                        'SELECTED\tapp=1\nSTATE\tsucceeded', 'FULL\tanything',
                        'KERNEL\tlinux-image-generic', 'SELECTED\tapp=1\tapp=2',
                        'SELECTED\tapp=1\t' + '\t'.join(f'app{i}=1' for i in range(256))):
            with self.assertRaises(backend.Rejected, msg=request):
                backend.parse_request(request)

    def completed_review(self, approve):
        process, sender, receiver, receipt = self.start_owner()
        self.review_owner(sender, receiver)
        if approve:
            # The review fingerprint is retained in the authoritative receipt.
            fields = dict(line.split('\t', 1) for line in receipt.read_text().splitlines()
                          if line.startswith('PLAN\t'))
            sender.write('APPROVE\t' + fields['PLAN'].split('\t')[0] + '\n')
        else:
            sender.write('CANCEL\n')
        self.finish(process, 0 if approve else 1)
        text = receipt.read_text().replace('STATE\tsucceeded\n', 'STATE\trecovery_required\n')
        text = text.replace('STATE\tcancelled\n', 'STATE\trecovery_required\n')
        receipt.write_text(text)
        return receipt

    def test_reconcile_only_confirms_observed_configured_postconditions(self):
        receipt = self.completed_review(True)
        self.assertEqual(backend.reconcile(os.getuid(), receipt.name, io.StringIO(), receipt.parent), 0)
        self.assertIn('STATE\tsucceeded\n', receipt.read_text())
        self.assertNotIn('ERROR\t', receipt.read_text())

    def test_repaired_but_unfulfilled_plan_is_failed_without_replay(self):
        receipt = self.completed_review(False)
        self.assertEqual(backend.reconcile(os.getuid(), receipt.name, io.StringIO(), receipt.parent), 0)
        self.assertIn('STATE\tfailed\n', receipt.read_text())
        self.assertIn('ERROR\trepaired_outcome_unknown\n', receipt.read_text())
        self.assertEqual((self.root / 'var/lib/dpkg/status').read_text(), '')

    def test_dirty_package_state_cannot_be_reconciled_as_success(self):
        receipt = self.completed_review(True)
        status = self.root / 'var/lib/dpkg/status'
        status.write_text(status.read_text().replace('Status: install ok installed',
                                                     'Status: install ok half-configured'))
        self.assertEqual(backend.reconcile(os.getuid(), receipt.name, io.StringIO(), receipt.parent), 1)
        self.assertIn('STATE\trecovery_required\n', receipt.read_text())

    def test_completed_receipts_rotate_at_capacity_without_blocking_new_work(self):
        receipt = self.completed_review(False)
        receipt.write_text(receipt.read_text().replace('STATE\trecovery_required', 'STATE\tcancelled'))
        original = receipt.read_text()
        receipt.unlink()
        for index in range(128):
            identifier = f'{index:032x}'
            path = receipt.parent / identifier
            path.write_text(original.replace('ID\t' + receipt.name, 'ID\t' + identifier))
            path.chmod(0o600)
        process, sender, receiver, new_receipt = self.start_owner()
        self.review_owner(sender, receiver)
        sender.write('CANCEL\n')
        self.finish(process, 1)
        self.assertIn('STATE\tcancelled\n', new_receipt.read_text())
        self.assertEqual(len([p for p in receipt.parent.iterdir()
                              if backend.IDENTIFIER.fullmatch(p.name)]), 128)


if __name__ == '__main__':
    unittest.main()
