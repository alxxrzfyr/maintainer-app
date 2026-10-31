#!/usr/bin/python3
"""Root-only, short-lived native APT transaction owner.

The GUI never invokes this entry point. The authorized D-Bus owner passes a
fixed intent and later a fingerprint over its private pipe. One depcache and
native system/list locks remain alive from resolution through commit. Existing
root-owned APT policy/hooks are part of the host's trusted computing base.
"""
from dataclasses import dataclass
import fcntl
import hashlib
import os
from pathlib import Path
import queue
import re
import selectors
import subprocess
import signal
import stat
import sys
import tempfile
import threading
import time

# Prevent inherited APT_CONFIG from being consulted during apt's import-time
# initialization. The production caller is trusted root, but config paths are
# still fixed by policy rather than inherited environment.
if __name__ == '__main__':
    if os.geteuid() != 0:
        sys.exit(2)
    os.environ.clear()
    os.environ.update({'PATH': '/usr/sbin:/usr/bin:/sbin:/bin', 'LC_ALL': 'C',
                       'DEBIAN_FRONTEND': 'noninteractive'})

import apt
import apt.progress.base
import apt_pkg

MAX_ITEMS = 4096
MAX_SELECTED = 256
MAX_BYTES = 2 * 1024 * 1024
APPROVAL_SECONDS = 90
LOCK_SECONDS = 15
RECEIPT_ROOT = Path('/var/lib/maintainer/transactions')
NAME = re.compile(r'[a-z0-9][a-z0-9+.-]{0,127}', re.ASCII)
ARCH = re.compile(r'[a-z0-9][a-z0-9-]{0,31}', re.ASCII)
VERSION = re.compile(r'[A-Za-z0-9][A-Za-z0-9+.:~-]{0,255}', re.ASCII)
IDENTIFIER = re.compile(r'[0-9a-f]{32}', re.ASCII)
DIGEST = re.compile(r'[0-9a-f]{64}', re.ASCII)
KERNEL = re.compile(r'linux-(?:image-(?:unsigned-)?|headers-|modules-(?:extra-)?)[0-9][a-z0-9+.-]*', re.ASCII)
POLICY = b'v1\tretain-depcache\tno-purge\tno-held-changes\tprotect-essential-running-kernel\tpreserve-local-conffiles\tblock-shutdown-sleep-through-postconditions\n'
TERMINAL = {'succeeded', 'cancelled', 'failed', 'recovery_required'}


class Rejected(ValueError):
    """A fixed, non-sensitive failure category suitable for a private receipt."""


class Cancelled(Exception):
    pass


INHIBITOR_COMMAND = (
    '/usr/bin/systemd-inhibit', '--what=shutdown:sleep', '--mode=block',
    '--who=Maintainer', '--why=Applying reviewed package changes', '/usr/bin/cat',
)
INHIBITOR_READY = b'MAINTAINER-INHIBITOR-READY\n'


class ShutdownInhibitor:
    """A private child retains logind's lock FD after the GUI/owner disconnects."""
    def __init__(self):
        self.process = None
        try:
            self.process = subprocess.Popen(
                INHIBITOR_COMMAND, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL, start_new_session=True, close_fds=True,
                env={'PATH': '/usr/sbin:/usr/bin:/sbin:/bin', 'LC_ALL': 'C'},
            )
            self.process.stdin.write(INHIBITOR_READY)
            self.process.stdin.flush()
            response = bytearray()
            deadline = time.monotonic() + 5
            with selectors.DefaultSelector() as selector:
                selector.register(self.process.stdout, selectors.EVENT_READ)
                while len(response) < len(INHIBITOR_READY):
                    remaining = deadline - time.monotonic()
                    if remaining <= 0 or not selector.select(remaining):
                        raise Rejected('inhibition_unavailable')
                    chunk = os.read(self.process.stdout.fileno(), len(INHIBITOR_READY) + 1 - len(response))
                    if not chunk:
                        raise Rejected('inhibition_unavailable')
                    response.extend(chunk)
                    if not INHIBITOR_READY.startswith(response):
                        raise Rejected('inhibition_unavailable')
            self.ensure_alive()
        except BaseException:
            self.close()
            raise Rejected('inhibition_unavailable') from None

    def ensure_alive(self):
        if self.process is None or self.process.poll() is not None:
            raise Rejected('inhibition_lost')

    def close(self):
        process, self.process = self.process, None
        if process is None:
            return
        # EOF lets cat exit normally, releasing the inhibitor. Only this new
        # session's helper group may be terminated; never signal APT or dpkg.
        if process.stdin is not None:
            try:
                process.stdin.close()
            except OSError:
                pass
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=2)
        finally:
            if process.stdout is not None:
                process.stdout.close()


@dataclass(frozen=True, order=True)
class Change:
    name: str
    architecture: str
    old_version: str
    new_version: str
    operation: str

    def line(self):
        return '\t'.join(('ITEM', self.operation, self.name, self.architecture,
                          self.old_version or '-', self.new_version or '-'))


def qualified_name(value):
    parts = value.split(':')
    if not NAME.fullmatch(parts[0]) or len(parts) > 2:
        raise Rejected('invalid_request')
    if len(parts) == 2 and not ARCH.fullmatch(parts[1]):
        raise Rejected('invalid_request')
    return value


def parse_request(line):
    if not line.isascii() or len(line) > MAX_BYTES:
        raise Rejected('invalid_request')
    fields = line.split('\t')
    if fields == ['FULL']:
        return 'full', ()
    if fields[0] == 'SELECTED' and 1 <= len(fields) - 1 <= MAX_SELECTED:
        names = set()
        for spec in fields[1:]:
            if spec.count('=') != 1 or len(spec) > 512:
                raise Rejected('invalid_request')
            name, version = spec.split('=')
            qualified_name(name)
            if not VERSION.fullmatch(version) or name in names:
                raise Rejected('invalid_request')
            names.add(name)
        return 'selected', tuple(sorted(fields[1:]))
    if len(fields) == 2 and fields[0] == 'KERNEL' and KERNEL.fullmatch(fields[1]):
        return 'kernel', (fields[1],)
    raise Rejected('invalid_request')


def plan(cache):
    changes = []
    for package in cache.get_changes():
        old = package.installed
        new = None if package.marked_delete else package.candidate
        if new is None and old is None:
            raise Rejected('unsupported_plan')
        operation = ('remove' if package.marked_delete else
                     'downgrade' if package.marked_downgrade else
                     'reinstall' if package.marked_reinstall else
                     'upgrade' if old else 'install')
        change = Change(package.shortname, (new or old).architecture,
                        old.version if old else '', new.version if new else '', operation)
        if not NAME.fullmatch(change.name) or not ARCH.fullmatch(change.architecture):
            raise Rejected('unsupported_plan')
        for version in (change.old_version, change.new_version):
            if version and not VERSION.fullmatch(version):
                raise Rejected('unsupported_plan')
        changes.append(change)
        if len(changes) > MAX_ITEMS:
            raise Rejected('plan_too_large')
    return tuple(sorted(changes))


def installed(cache):
    return {(p.shortname, p.installed.architecture): p.installed.version
            for p in cache if p.is_installed}


def running_kernel_package(name):
    release = os.uname().release
    # Ubuntu's common headers have an ABI name without the flavour suffix.
    abi = '-'.join(release.split('-')[:2])
    return any(name == prefix + suffix
               for prefix in ('linux-image-', 'linux-image-unsigned-', 'linux-headers-',
                              'linux-modules-', 'linux-modules-extra-')
               for suffix in (release, abi))


def kernel_metapackage(name):
    """Tracking packages must survive resolver-induced kernel removals."""
    if name.startswith(('linux-image-', 'linux-headers-')):
        suffix = name.split('-', 2)[2].removeprefix('unsigned-')
        return not suffix[:1].isdigit()
    return name in {'linux-image', 'linux-headers', 'linux-generic', 'linux-virtual', 'linux-lowlatency', 'linux-oem'} or name.startswith(
        ('linux-generic-', 'linux-virtual-', 'linux-lowlatency-', 'linux-oem-',
         'linux-signed-', 'linux-aws', 'linux-azure', 'linux-gcp', 'linux-kvm',
         'linux-oracle', 'linux-raspi'))


class Backend:
    """Shared production/fixture engine. Configuration is never caller supplied."""
    def __init__(self, events=None, cancelled=None, inhibitor_factory=None):
        self.inhibitor_factory = inhibitor_factory or ShutdownInhibitor
        self.inhibitor = None
        self.events = events or (lambda owner: None)
        self.cancelled = cancelled or (lambda: False)
        self.phase = 'locking'
        self.error = ''
        self.review = ()
        self.identifier = '-'
        self.download_bytes = 0
        self.lock = None
        self.list_lock = None
        self.mutation_started = False
        try:
            updates = Path(apt_pkg.config.find_file('Dir::State::status')).parent / 'updates'
            if any(entry.name and entry.name[0].isdigit() for entry in updates.iterdir()):
                raise Rejected('recovery_required')
            lock = apt_pkg.SystemLock()
            lock.__enter__()
            self.lock = lock
            list_lock = apt_pkg.FileLock(
                apt_pkg.config.find_dir('Dir::State::lists') + 'lock')
            list_lock.__enter__()
            self.list_lock = list_lock
            self._state('resolving')
            self.cache = apt.Cache()
            if self.cache.dpkg_journal_dirty or self.cache.broken_count:
                raise Rejected('recovery_required')
            safe_states = {apt_pkg.CURSTATE_NOT_INSTALLED,
                           apt_pkg.CURSTATE_CONFIG_FILES, apt_pkg.CURSTATE_INSTALLED}
            if any(p._pkg.current_state not in safe_states for p in self.cache):
                raise Rejected('recovery_required')
            self.before = installed(self.cache)
            self.status_file = Path(apt_pkg.config.find_file('Dir::State::status'))
            self.status_digest = hashlib.sha256(self.status_file.read_bytes()).digest()
        except BaseException:
            self.close()
            raise

    def _state(self, phase, error=''):
        self.phase, self.error = phase, error
        self.events(self)

    def _check_cancel(self):
        if not self.mutation_started and self.cancelled():
            raise Cancelled()

    def _fingerprint(self):
        digest = hashlib.sha256(POLICY)
        digest.update(str(self.download_bytes).encode('ascii') + b'\n')
        for change in plan(self.cache):
            digest.update(change.line().encode('ascii') + b'\n')
            if change.operation != 'remove':
                package = self.cache[change.name if change.architecture == 'all' else change.name + ':' + change.architecture]
                archive_hash = package.candidate.record.get('SHA256', '')
                if not DIGEST.fullmatch(archive_hash):
                    raise Rejected('unsupported_archive_hash')
                digest.update(archive_hash.encode('ascii') + b'\n')
        return digest.hexdigest()

    def resolve(self, selected=(), remove=(), full=False, kernel=False):
        if self.phase != 'resolving':
            raise Rejected('invalid_state')
        self._check_cancel()
        if full:
            self.cache.upgrade(dist_upgrade=True)
        selected_identities = set()
        for name, version in selected:
            qualified_name(name)
            if not VERSION.fullmatch(version):
                raise Rejected('invalid_request')
            package = self.cache[name]
            identity = (package.shortname, package.architecture())
            if identity in selected_identities:
                raise Rejected('duplicate_package_identity')
            selected_identities.add(identity)
            if package._pkg.selected_state == apt_pkg.SELSTATE_HOLD:
                raise Rejected('held_package')
            package.candidate = package.versions[version]
            package.mark_install()
        for name in remove:
            package = self.cache[name]
            if not package.is_installed:
                raise Rejected('package_not_installed')
            package.mark_delete(purge=False)
        if self.cache.broken_count:
            raise Rejected('broken_plan')
        for name, version in selected:
            package = self.cache[name]
            if package.marked_delete or package.candidate.version != version:
                raise Rejected('requested_version_changed')
        self.review = plan(self.cache)
        for change in self.review:
            if change.name == 'maintainer':
                raise Rejected('maintainer_requires_external_update')
            package = self.cache[change.name if change.architecture == 'all' else change.name + ':' + change.architecture]
            if package._pkg.selected_state == apt_pkg.SELSTATE_HOLD:
                raise Rejected('held_package')
            if change.operation == 'remove' and (package.essential or
                                                  running_kernel_package(change.name) or
                                                  kernel_metapackage(change.name)):
                raise Rejected('protected_package')
        if kernel or any(change.operation == 'remove' and
                         re.fullmatch(r'linux-image-(?:unsigned-)?[0-9].*', change.name)
                         for change in self.review):
            remaining_images = {re.sub(r'^linux-image-(?:unsigned-)?', '', p.shortname)
                                for p in self.cache if p.is_installed and not p.marked_delete
                                and re.fullmatch(r'linux-image-(?:unsigned-)?[0-9].*', p.shortname)}
            if len(remaining_images) < 2:
                raise Rejected('kernel_fallback_required')
        self.download_bytes = max(0, int(self.cache.required_download))
        self.identifier = self._fingerprint()
        self._check_cancel()
        self._state('review')
        return self.review, self.identifier

    def resolve_intent(self, intent, requests):
        if intent == 'full':
            return self.resolve(full=True)
        if intent == 'selected':
            return self.resolve(selected=[spec.split('=', 1) for spec in requests])
        if intent == 'kernel':
            # Re-derive protection inside the lock; UI/helper assertions alone
            # do not protect resolver-induced removals of a running kernel.
            if running_kernel_package(requests[0]):
                raise Rejected('protected_package')
            return self.resolve(remove=requests, kernel=True)
        raise Rejected('invalid_request')

    def cancel(self):
        if self.phase not in ('resolving', 'review'):
            return False
        self._state('cancelled')
        self.close()
        return True

    def commit(self, approved_id):
        if self.phase != 'review':
            raise Rejected('invalid_state')
        if approved_id != self.identifier or plan(self.cache) != self.review or self._fingerprint() != self.identifier:
            raise Rejected('review_binding_changed')
        owner = self

        class Acquire(apt.progress.base.AcquireProgress):
            def pulse(self, acquire):
                return owner.mutation_started or not owner.cancelled()

        class Install(apt.progress.base.InstallProgress):
            def run(self, manager):
                owner._check_cancel()
                if plan(owner.cache) != owner.review or owner._fingerprint() != owner.identifier:
                    raise Rejected('review_binding_changed')
                if any(change.operation == 'remove' and running_kernel_package(change.name)
                       for change in owner.review):
                    raise Rejected('protected_package')
                if hashlib.sha256(owner.status_file.read_bytes()).digest() != owner.status_digest:
                    raise Rejected('status_changed')
                if owner.inhibitor is None:
                    owner.inhibitor = owner.inhibitor_factory()
                owner.inhibitor.ensure_alive()
                owner._check_cancel()
                # Persist before the first dpkg fork. A crash from here onward
                # is conservatively recovery_required; never replay or kill it.
                owner._state('mutating')
                owner.mutation_started = True
                return super().run(manager)

        progress = Install()
        try:
            self._check_cancel()
            self._state('fetching')
            result = self.cache.commit(fetch_progress=Acquire(), install_progress=progress,
                                       allow_unauthenticated=False)
            if not result:
                raise Rejected('install_failed')
            if self.inhibitor is not None:
                self.inhibitor.ensure_alive()
            after_cache = apt.Cache()
            expected = dict(self.before)
            for change in self.review:
                key = (change.name, change.architecture)
                if change.operation == 'remove':
                    expected.pop(key, None)
                else:
                    expected[key] = change.new_version
            if installed(after_cache) != expected or any(
                    p.is_installed and p._pkg.current_state != apt_pkg.CURSTATE_INSTALLED
                    for p in after_cache):
                raise Rejected('postcondition_failed')
            if self.inhibitor is not None:
                self.inhibitor.ensure_alive()
            self._state('succeeded')
            return True
        except BaseException as error:
            if self.mutation_started:
                self._state('recovery_required', str(error) if isinstance(error, Rejected) else 'install_failed')
            elif isinstance(error, Cancelled) or self.cancelled():
                self._state('cancelled')
            else:
                self._state('failed', str(error) if isinstance(error, Rejected) else 'fetch_or_install_failed')
            raise
        finally:
            progress.write_stream.close()
            progress.status_stream.close()
            self.close()

    def close(self):
        try:
            if self.list_lock is not None:
                self.list_lock.__exit__(None, None, None)
                self.list_lock = None
        finally:
            try:
                if self.lock is not None:
                    self.lock.__exit__(None, None, None)
                    self.lock = None
            finally:
                if self.inhibitor is not None:
                    self.inhibitor.close()
                    self.inhibitor = None


class Receipt:
    def __init__(self, directory, identifier, uid, intent, requests, output):
        self.directory, self.identifier, self.uid = Path(directory), identifier, uid
        self.intent, self.requests, self.output = intent, requests, output
        self.pid = os.getpid()
        fields = Path('/proc/self/stat').read_text().rsplit(')', 1)[1].split()
        self.start_ticks = int(fields[19])

    def write(self, owner):
        lines = ['VERSION\t1', 'ID\t' + self.identifier, 'UID\t' + str(self.uid),
                 'PID\t' + str(self.pid), 'START\t' + str(self.start_ticks),
                 'INTENT\t' + self.intent]
        lines.extend('REQUEST\t' + spec for spec in self.requests)
        lines.extend(['STATE\t' + owner.phase,
                      'PLAN\t' + owner.identifier + '\t' + str(owner.download_bytes)])
        lines.extend(change.line() for change in owner.review)
        if owner.error:
            lines.append('ERROR\t' + owner.error)
        lines.append('END')
        data = ('\n'.join(lines) + '\n').encode('ascii')
        if len(data) > MAX_BYTES:
            raise Rejected('plan_too_large')
        descriptor, name = tempfile.mkstemp(prefix='.receipt-', dir=self.directory)
        try:
            with os.fdopen(descriptor, 'wb') as stream:
                stream.write(data)
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(name, self.directory / self.identifier)
            directory_fd = os.open(self.directory, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
            try:
                os.fsync(directory_fd)
            finally:
                os.close(directory_fd)
        finally:
            if os.path.exists(name):
                os.unlink(name)
        # The receipt is authoritative. A disconnected D-Bus owner/GUI cannot
        # turn an approved transaction into a broken-pipe cancellation.
        try:
            self.output.write('STATE\t' + owner.phase + '\n')
            if owner.phase == 'review':
                self.output.write('PLAN\t' + owner.identifier + '\t' + str(owner.download_bytes) + '\n')
                for change in owner.review:
                    self.output.write(change.line() + '\n')
                self.output.write('END\n')
            self.output.flush()
        except (BrokenPipeError, OSError):
            pass


class Commands:
    def __init__(self, stream):
        self.messages = queue.Queue(maxsize=8)
        self.cancel_signal = False
        self.approved = False
        self.stream = stream
        self.thread = threading.Thread(target=self._read, daemon=True)
        self.thread.start()

    def _read(self):
        while True:
            line = self.stream.readline(MAX_BYTES + 1)
            if not line:
                self.messages.put(None)
                return
            if len(line) > MAX_BYTES or not line.endswith(b'\n'):
                self.messages.put('INVALID')
                return
            try:
                value = line[:-1].decode('ascii')
            except UnicodeDecodeError:
                value = 'INVALID'
            self.messages.put(value)

    def before_deadline(self, deadline):
        while time.monotonic() < deadline:
            if self.cancel_signal:
                raise Cancelled()
            try:
                value = self.messages.get(timeout=min(.2, max(.001, deadline - time.monotonic())))
                if value is None or value == 'CANCEL':
                    raise Cancelled()
                return value
            except queue.Empty:
                continue
        raise Cancelled()

    def cancelled(self):
        if self.cancel_signal:
            return True
        # After approval, EOF/disconnect does not cancel the owner. Only an
        # explicit CANCEL accepted before Install.run can stop acquisition.
        while True:
            try:
                value = self.messages.get_nowait()
            except queue.Empty:
                return False
            if value == 'CANCEL' or (value is None and not self.approved):
                self.cancel_signal = True
                return True


def private_directory(path):
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    for directory in (path.parent, path):
        metadata = directory.lstat()
        if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != 0 or metadata.st_mode & 0o022:
            raise Rejected('unsafe_receipt_directory')
    if stat.S_IMODE(path.stat().st_mode) != 0o700:
        raise Rejected('unsafe_receipt_directory')


def read_record(path):
    metadata = path.lstat()
    if (not stat.S_ISREG(metadata.st_mode) or metadata.st_size > MAX_BYTES
            or metadata.st_uid != os.geteuid() or metadata.st_nlink != 1
            or metadata.st_mode & 0o022):
        raise Rejected('prior_recovery_required')
    text = path.read_text(encoding='ascii')
    if not text.endswith('END\n'):
        raise Rejected('prior_recovery_required')
    headers, requests, changes = {}, [], []
    for line in text.splitlines()[:-1]:
        fields = line.split('\t')
        if fields[0] == 'ITEM' and len(fields) == 6:
            changes.append(Change(fields[2], fields[3], '' if fields[4] == '-' else fields[4],
                                  '' if fields[5] == '-' else fields[5], fields[1]))
        elif fields[0] == 'REQUEST' and len(fields) == 2:
            requests.append(fields[1])
        elif fields[0] == 'PLAN' and len(fields) == 3 and 'PLAN' not in headers:
            headers['PLAN'], headers['BYTES'] = fields[1:]
        elif (len(fields) == 2 and fields[0] in
              {'VERSION', 'ID', 'UID', 'PID', 'START', 'STATE', 'INTENT', 'ERROR'}
              and fields[0] not in headers):
            headers[fields[0]] = fields[1]
        else:
            raise Rejected('prior_recovery_required')
    if (headers.get('VERSION') != '1' or headers.get('ID') != path.name
            or len(changes) > MAX_ITEMS or len(requests) > MAX_SELECTED):
        raise Rejected('prior_recovery_required')
    required = {'VERSION', 'ID', 'UID', 'PID', 'START', 'STATE', 'INTENT', 'PLAN', 'BYTES'}
    if (not required <= headers.keys() or headers.keys() - required - {'ERROR'}
            or not re.fullmatch(r'[0-9]{1,10}', headers['UID'])
            or not 0 < int(headers['UID']) <= 4294967294
            or not re.fullmatch(r'[0-9]{1,10}', headers['PID'])
            or not 2 <= int(headers['PID']) <= 2147483647
            or not re.fullmatch(r'[0-9]{1,20}', headers['START']) or int(headers['START']) == 0
            or not re.fullmatch(r'[0-9]{1,20}', headers['BYTES'])
            or (headers['PLAN'] != '-' and not DIGEST.fullmatch(headers['PLAN']))
            or headers['STATE'] not in TERMINAL | {'locking', 'resolving', 'review', 'fetching', 'mutating'}):
        raise Rejected('prior_recovery_required')
    prefix = {'full': 'FULL', 'selected': 'SELECTED', 'kernel': 'KERNEL'}.get(headers['INTENT'])
    if not prefix or parse_request('\t'.join((prefix, *requests))) != (headers['INTENT'], tuple(requests)):
        raise Rejected('prior_recovery_required')
    identities = set()
    for change in changes:
        identity = (change.name, change.architecture)
        if (identity in identities or not NAME.fullmatch(change.name) or not ARCH.fullmatch(change.architecture)
                or change.operation not in {'install', 'upgrade', 'downgrade', 'reinstall', 'remove'}
                or any(version and not VERSION.fullmatch(version) for version in (change.old_version, change.new_version))
                or (change.operation == 'remove' and (not change.old_version or change.new_version))
                or (change.operation != 'remove' and not change.new_version)):
            raise Rejected('prior_recovery_required')
        identities.add(identity)
    return headers, tuple(requests), tuple(changes)


def reconcile(uid, identifier, output, directory=RECEIPT_ROOT):
    """Read-only audit after a dead backend. Never configure or replay packages."""
    headers, requests, changes = read_record(directory / identifier)
    if int(headers['UID']) != uid:
        raise Rejected('wrong_user')
    try:
        fields = Path('/proc/' + headers['PID'] + '/stat').read_text().rsplit(')', 1)[1].split()
        if fields[19] == headers['START']:
            raise Rejected('busy')
    except FileNotFoundError:
        pass
    descriptor = os.open(directory / 'current.lock', os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC)
    owner = None
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        receipt = Receipt(directory, identifier, uid, headers['INTENT'], requests, output)
        class Audited:
            phase, error = 'recovery_required', 'package_repair_required'
            identifier, download_bytes, review = headers['PLAN'], int(headers['BYTES']), changes
        result = Audited()
        try:
            owner = Backend()
            current = installed(owner.cache)
            matches = all((current.get((c.name, c.architecture)) == c.new_version
                           if c.operation != 'remove' else
                           (c.name, c.architecture) not in current) for c in changes)
            result.phase = 'succeeded' if matches and headers['PLAN'] != '-' else 'failed'
            result.error = '' if result.phase == 'succeeded' else 'repaired_outcome_unknown'
        except Rejected:
            pass
        receipt.write(result)
        return 0 if result.phase != 'recovery_required' else 1
    finally:
        if owner is not None:
            owner.close()
        os.close(descriptor)


def run_owner(uid, identifier, commands, output, directory=RECEIPT_ROOT):
    """The directory parameter exists for module-level unprivileged fixtures only."""
    request = commands.before_deadline(time.monotonic() + APPROVAL_SECONDS)
    if request == 'RECONCILE':
        return reconcile(uid, identifier, output, directory)
    intent, requests = parse_request(request)
    descriptor = os.open(directory / 'current.lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    owner = None
    receipt = Receipt(directory, identifier, uid, intent, requests, output)
    try:
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise Rejected('busy') from None
        if os.path.lexists(directory / identifier):
            raise Rejected('duplicate_request')
        receipts = [entry for entry in directory.iterdir() if IDENTIFIER.fullmatch(entry.name)]
        pending_files = sum(1 for entry in directory.iterdir()
                            if entry.name.startswith('.receipt-'))
        completed = []
        for previous in receipts:
            record, _, _ = read_record(previous)
            if record.get('STATE') not in {'succeeded', 'failed', 'cancelled'}:
                raise Rejected('prior_recovery_required')
            completed.append((previous.stat().st_mtime, previous))
        # Keep up to 128 receipts for seven days. Unresolved work is never pruned.
        remaining = len(receipts) + pending_files
        for modified, previous in sorted(completed):
            if remaining < 128 and time.time() - modified <= 7 * 86400:
                continue
            previous.unlink()
            remaining -= 1
        if remaining >= 128:
            raise Rejected('receipt_capacity')
        # A restart must never replay work merely because the native lock is
        # now free. The D-Bus owner reconciles incomplete receipts explicitly.
        class Initial:
            phase, error, identifier, download_bytes, review = 'locking', '', '-', 0, ()
        receipt.write(Initial())
        lock_deadline = time.monotonic() + LOCK_SECONDS
        while owner is None:
            if commands.cancelled():
                raise Cancelled()
            try:
                owner = Backend(events=receipt.write, cancelled=commands.cancelled)
            except apt_pkg.Error:
                if time.monotonic() >= lock_deadline:
                    raise Rejected('package_lock_unavailable') from None
                time.sleep(.1)
        owner.resolve_intent(intent, requests)
        approval = commands.before_deadline(time.monotonic() + APPROVAL_SECONDS)
        if approval != 'APPROVE\t' + owner.identifier:
            raise Rejected('review_binding_changed')
        commands.approved = True
        owner.commit(owner.identifier)
        return 0
    except BaseException as error:
        if owner is not None and owner.phase not in TERMINAL:
            phase = 'recovery_required' if owner.mutation_started else 'cancelled' if isinstance(error, Cancelled) else 'failed'
            owner._state(phase, '' if isinstance(error, Cancelled) else error.args[0] if isinstance(error, Rejected) else 'backend_failed')
        elif owner is None and isinstance(error, (Cancelled, Rejected)) and str(error) not in ('busy', 'duplicate_request', 'receipt_capacity', 'prior_recovery_required'):
            class Failed:
                phase = ('cancelled' if isinstance(error, Cancelled) else
                         'recovery_required' if str(error) == 'recovery_required' else 'failed')
                identifier, download_bytes, review = '-', 0, ()
            failed = Failed()
            failed.error = '' if isinstance(error, Cancelled) else str(error)
            receipt.write(failed)
        return 1
    finally:
        if owner is not None:
            owner.close()
        os.close(descriptor)


def main():
    if os.geteuid() != 0 or len(sys.argv) != 3:
        return 2
    uid_text, identifier = sys.argv[1:]
    if not re.fullmatch(r'[0-9]{1,10}', uid_text) or not 0 < int(uid_text) <= 4294967294 or not IDENTIFIER.fullmatch(identifier):
        return 2
    # Neither environment nor terminal state is part of the request protocol.
    os.environ.clear()
    os.environ.update({'PATH': '/usr/sbin:/usr/bin:/sbin:/bin', 'LC_ALL': 'C',
                       'DEBIAN_FRONTEND': 'noninteractive'})
    os.umask(0o077)
    output = os.fdopen(os.dup(1), 'w', encoding='ascii', buffering=1)
    command_stream = os.fdopen(os.dup(0), 'rb', buffering=0)
    null_fd = os.open('/dev/null', os.O_RDWR | os.O_CLOEXEC)
    for target in (0, 1, 2):
        os.dup2(null_fd, target)
    os.close(null_fd)
    commands = Commands(command_stream)
    def stop_before_mutation(_number, _frame):
        commands.cancel_signal = True
    signal.signal(signal.SIGTERM, stop_before_mutation)
    signal.signal(signal.SIGINT, stop_before_mutation)
    signal.signal(signal.SIGHUP, signal.SIG_IGN)
    try:
        private_directory(RECEIPT_ROOT)
        # Discard inherited options. These fixed flags preserve local conffiles
        # without asking the protocol pipe or silently replacing local edits.
        apt_pkg.config.clear('DPkg::Options')
        apt_pkg.config.set('DPkg::Options::', '--force-confdef')
        apt_pkg.config.set('DPkg::Options::', '--force-confold')
        apt_pkg.config.set('Debug::NoLocking', 'false')
        apt_pkg.config.set('DPkg::Lock::Timeout', '0')
        apt_pkg.config.set('APT::Get::AllowUnauthenticated', 'false')
        return run_owner(int(uid_text), identifier, commands, output)
    except BaseException:
        return 1
    finally:
        try:
            output.close()
        except (BrokenPipeError, OSError):
            pass


if __name__ == '__main__':
    sys.exit(main())
