#!/usr/bin/env python3
"""Sanitized, unprivileged GTK semantic-readiness and process-family measurements."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import signal
import statistics
import subprocess
import sys
import time
import tempfile

PAGES = ('Overview', 'System', 'Processes', 'Network')
LIFECYCLE_EVENTS = ('close-requested', 'window-destroyed', 'application-shutdown')
MEMORY_KEYS = ('Pss', 'Private_Dirty', 'Rss', 'Swap', 'SwapPss')
IDLE_CPU_TARGET_PERCENT = 0.35
IDLE_CPU_ACCEPTANCE_EXCLUSIVE_PERCENT = 0.5
UNVERIFIED = [
    'cold-boot startup repetitions and cache state',
    'extended soak and leak trend',
    'hidden-window refresh and CPU behavior',
    'physical 4 GB hardware profile',
    'compositor and distribution matrix',
    'application-only memory above measured GTK baseline',
    'short-lived descendant CPU attribution is approximate',
]


class MeasurementError(Exception):
    """Only fixed, non-sensitive diagnostics cross the report boundary."""


def parse_stat(text):
    """Linux comm can contain spaces and closing parentheses; fields start after last ') '."""
    prefix, separator, suffix = text.rpartition(') ')
    if not separator:
        raise MeasurementError('invalid process stat format')
    fields = suffix.split()
    try:
        pid = int(prefix.split(' ', 1)[0])
        result = {'pid': pid, 'state': fields[0], 'ppid': int(fields[1]),
                  'own_ticks': int(fields[11]) + int(fields[12]),
                  'reaped_ticks': int(fields[13]) + int(fields[14]),
                  'start_ticks': int(fields[19])}
    except (IndexError, ValueError):
        raise MeasurementError('invalid process stat counters') from None
    if any(result[key] < 0 for key in ('pid', 'ppid', 'own_ticks', 'start_ticks')):
        raise MeasurementError('invalid process stat counters')
    return result


def parse_smaps(text):
    values = {key: 0 for key in MEMORY_KEYS}
    present = set()
    for line in text.splitlines():
        key, separator, rest = line.partition(':')
        if separator and key in values:
            fields = rest.split()
            if len(fields) != 2 or fields[1] != 'kB':
                raise MeasurementError('invalid memory units')
            try:
                value = int(fields[0])
            except ValueError:
                raise MeasurementError('invalid memory counter') from None
            if value < 0:
                raise MeasurementError('invalid memory counter')
            values[key] += value * 1024
            present.add(key)
    if present != set(MEMORY_KEYS):
        raise MeasurementError('required memory counters unavailable')
    return values


def process_stat(pid):
    try:
        return parse_stat(Path(f'/proc/{pid}/stat').read_text())
    except OSError:
        raise MeasurementError('process identity unavailable') from None


def identity(stat):
    return stat['pid'], stat['start_ticks']


def process_memory(expected):
    before = process_stat(expected[0])
    if identity(before) != expected:
        raise MeasurementError('process identity changed before memory sample')
    if before['state'] == 'Z':
        return {key: 0 for key in MEMORY_KEYS}
    try:
        try:
            text = Path(f'/proc/{expected[0]}/smaps_rollup').read_text()
        except FileNotFoundError:
            text = Path(f'/proc/{expected[0]}/smaps').read_text()
    except OSError:
        raise MeasurementError('process memory sample unavailable') from None
    values = parse_smaps(text)
    after = process_stat(expected[0])
    if identity(after) != expected:
        raise MeasurementError('process identity changed during memory sample')
    return values


class Readiness:
    def __init__(self, started_ns):
        self.started_ns = started_ns
        self.buffer = bytearray()
        self.events = {}
        self.renderer = None
        self.lifecycle = []
        self.last_ns = started_ns

    @property
    def complete(self):
        return all(name in self.events for name in ('process-start', 'mapped', 'usable', 'first-data'))

    def feed(self, chunk, now_ns):
        if not chunk:
            if not self.complete:
                raise MeasurementError('readiness pipe closed before semantic milestones')
            return
        self.buffer.extend(chunk)
        if len(self.buffer) > 65536:
            raise MeasurementError('readiness protocol exceeded bounded buffer')
        while b'\n' in self.buffer:
            line, _, rest = self.buffer.partition(b'\n')
            self.buffer = bytearray(rest)
            try:
                event = json.loads(line)
                name, timestamp = event['event'], event['monotonic_ns']
            except (ValueError, KeyError, TypeError, UnicodeError):
                raise MeasurementError('invalid readiness event') from None
            if not isinstance(name, str) or type(timestamp) is not int:
                raise MeasurementError('invalid readiness event types')
            if not self.last_ns <= timestamp <= now_ns:
                raise MeasurementError('invalid readiness monotonic timestamp')
            self.last_ns = timestamp
            if name in LIFECYCLE_EVENTS:
                if len(self.lifecycle) >= 16:
                    raise MeasurementError('lifecycle event limit exceeded')
                self.lifecycle.append({'event': name, 'elapsed_ms': (timestamp - self.started_ns) / 1_000_000})
                continue
            if name.startswith('renderer:'):
                renderer = name.partition(':')[2]
                if ('mapped' not in self.events or self.renderer is not None
                        or not re.fullmatch(r'Gsk[A-Za-z0-9]{1,48}', renderer)):
                    raise MeasurementError('invalid renderer evidence')
                self.renderer = renderer
                continue
            if name not in ('process-start', 'mapped', 'usable', 'first-data') or name in self.events:
                raise MeasurementError('readiness milestones missing or out of order')
            # Data can arrive before the navigation canary. These are separate
            # milestones; neither is a proxy for the other.
            if (name != 'process-start' and 'process-start' not in self.events
                    or name == 'usable' and 'mapped' not in self.events):
                raise MeasurementError('readiness milestones missing or out of order')
            self.events[name] = (timestamp - self.started_ns) / 1_000_000


def await_ready(process, fd, started_ns, timeout_seconds=15):
    readiness = Readiness(started_ns)
    deadline = time.monotonic() + timeout_seconds
    with selectors.DefaultSelector() as selector:
        selector.register(fd, selectors.EVENT_READ)
        while not readiness.complete:
            if process.poll() is not None:
                raise MeasurementError('application exited before semantic readiness')
            if time.monotonic() >= deadline:
                raise MeasurementError('semantic readiness timed out')
            for key, _ in selector.select(timeout=0.05):
                readiness.feed(os.read(key.fd, 4096), time.monotonic_ns())
    if process.poll() is not None:
        raise MeasurementError('application exited at semantic readiness')
    return readiness


def renderer_mode(renderer, glx_text=None, virtual_display=False):
    if not renderer:
        return 'unknown'
    if 'cairo' in renderer.lower():
        return 'software'
    if renderer not in ('GskGLRenderer', 'GskNglRenderer', 'GskVulkanRenderer'):
        return 'unknown'
    if virtual_display:
        return 'software'
    if glx_text is None:
        return 'unknown'
    text = glx_text.lower()
    if any(word in text for word in ('llvmpipe', 'softpipe', 'swrast', 'software rasterizer')):
        return 'software'
    # A GSK GL class alone does not prove hardware execution.
    if 'gl' in renderer.lower() and 'direct rendering: yes' in text and 'opengl renderer string:' in text:
        return 'hardware'
    return 'unknown'


def detect_renderer(renderer):
    if renderer == 'GskVulkanRenderer':
        return 'unknown'  # GLX cannot identify the Vulkan device; use DRM activity.
    glx_text = None
    try:
        result = subprocess.run(['glxinfo', '-B'], stdout=subprocess.PIPE,
                                stderr=subprocess.DEVNULL, timeout=3, check=False)
        if result.returncode == 0:
            glx_text = result.stdout[:65536].decode('utf-8', errors='replace')
    except (OSError, subprocess.TimeoutExpired):
        pass
    return renderer_mode(renderer, glx_text, os.environ.get('PERF_HARNESS_XVFB') == '1')


def parse_drm_render(text):
    driver = re.search(r'^drm-driver:\s*(\w+)\s*$', text, re.M)
    client = re.search(r'^drm-client-id:\s*(\d+)\s*$', text, re.M)
    engine = re.search(r'^drm-engine-(?:render|gfx):\s*(\d+) ns\s*$', text, re.M)
    physical = {'i915', 'xe', 'amdgpu', 'nouveau', 'v3d', 'vc4', 'panfrost', 'panthor', 'msm', 'asahi'}
    if not driver or driver[1] not in physical or not client or not engine:
        return None
    return driver[1] + ':' + client[1], int(engine[1])


def read_drm_render(root):
    try:
        if identity(process_stat(root['pid'])) != identity(root):
            return {}
        counters = {}
        for index, path in enumerate((Path('/proc') / str(root['pid']) / 'fdinfo').iterdir()):
            if index >= 512:
                return {}
            with path.open() as stream:
                text = stream.read(65537)
            sample = parse_drm_render(text) if len(text) <= 65536 else None
            if sample:
                key, value = sample
                counters[key] = max(counters.get(key, 0), value)
        return counters if identity(process_stat(root['pid'])) == identity(root) else {}
    except (OSError, UnicodeError, MeasurementError):
        return {}


def renderer_with_drm(mode, renderer, before, after, virtual_display=False):
    if mode == 'unknown' and renderer == 'GskVulkanRenderer' and not virtual_display:
        if any(after.get(client, 0) > value for client, value in before.items()):
            return 'hardware'
    return mode


class Family:
    """Retain discovered identities even after reparenting; never follow reused PIDs."""
    def __init__(self, root):
        self.root = identity(root)
        self.known = {self.root}
        self.last = {self.root: root}
        self.own_delta = 0
        self.root_own_start = root['own_ticks']
        self.root_reaped_start = root['reaped_ticks']
        self.root_total_start = root['own_ticks'] + root['reaped_ticks']
        self.root_total = self.root_total_start
        self.disappeared = False

    def snapshot(self):
        all_stats = {}
        for path in Path('/proc').iterdir():
            if not path.name.isdecimal():
                continue
            try:
                stat = parse_stat((path / 'stat').read_text())
            except (OSError, MeasurementError):
                continue
            all_stats[identity(stat)] = stat
        if self.root not in all_stats:
            raise MeasurementError('application identity disappeared during measurement')
        active = {key: all_stats[key] for key in self.known if key in all_stats}
        while True:
            pids = {key[0] for key in active}
            found = {key: value for key, value in all_stats.items()
                     if value['ppid'] in pids and key not in self.known}
            if not found:
                break
            self.known.update(found)
            active.update(found)
        for key, stat in active.items():
            previous = self.last.get(key)
            # Descendants born during the measurement contribute their full lifetime.
            baseline = previous['own_ticks'] if previous else 0
            self.own_delta += max(0, stat['own_ticks'] - baseline)
            self.last[key] = stat
        if self.known - active.keys():
            self.disappeared = True
        root = active[self.root]
        self.root_total = root['own_ticks'] + root['reaped_ticks']
        return active

    def cpu_details(self):
        root = self.last[self.root]
        return {
            "observed_identity_own_ticks": self.own_delta,
            "root_own_ticks": root["own_ticks"] - self.root_own_start,
            "root_reaped_child_ticks": root["reaped_ticks"] - self.root_reaped_start,
            "clock_ticks_per_second": os.sysconf("SC_CLK_TCK"),
        }

    def cpu_ticks(self):
        # Root's reaped counters recover unseen short-lived children. Taking the
        # larger estimate avoids adding reaped CPU twice to observed child CPU.
        # Nested reaping and between-sample lifetimes remain attribution uncertainty.
        return max(self.own_delta, self.root_total - self.root_total_start)


def ensure_alive(process):
    if process.poll() is not None:
        raise MeasurementError('application exited during measurement')


def summarize(samples):
    if not samples:
        raise MeasurementError('no memory samples collected')
    return {key: {'median_bytes': statistics.median(sample[key] for sample in samples),
                  'max_bytes': max(sample[key] for sample in samples)} for key in MEMORY_KEYS}


def evaluate(renderer, memory, cpu_percent, usable_ms, first_launch=False):
    if renderer == 'unknown':
        return 'unverified'
    if usable_ms > (4000 if first_launch else 2000) or cpu_percent > (2.0 if renderer == 'software' else 1.0):
        return 'ceiling-failure'
    if renderer == 'hardware' and (memory['Pss']['max_bytes'] >= 100_000_000 or
                                   memory['Private_Dirty']['max_bytes'] > 130_000_000):
        return 'ceiling-failure'
    return 'target-miss' if cpu_percent >= IDLE_CPU_ACCEPTANCE_EXCLUSIVE_PERCENT else 'pass'


def run_once(binary, page, duration, settle, first_launch=False):
    read_fd, write_fd = os.pipe()
    os.set_blocking(read_fd, False)
    environment = os.environ.copy()
    environment.update(MAINTAINER_PROBE_FD=str(write_fd), MAINTAINER_PROBE_PAGE=page)
    process = None
    samples = []
    raw_samples = []
    readiness = None
    family = None
    start = None
    private_state = None
    drm_before = {}
    try:
        if first_launch:
            private_state = tempfile.TemporaryDirectory(prefix='maintainer-first-launch-')
            for variable, directory in (('XDG_CACHE_HOME', 'cache'), ('XDG_DATA_HOME', 'data')):
                path = Path(private_state.name) / directory
                path.mkdir()
                environment[variable] = str(path)
        started_ns = time.monotonic_ns()
        process = subprocess.Popen([str(binary)], env=environment, pass_fds=(write_fd,),
                                   stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                   stderr=subprocess.DEVNULL, start_new_session=True)
        spawn_ms = (time.monotonic_ns() - started_ns) / 1_000_000
        os.close(write_fd)
        write_fd = None
        root = process_stat(process.pid)
        readiness = await_ready(process, read_fd, started_ns)
        renderer = detect_renderer(readiness.renderer)
        # Keep draining later probe bytes while settling and sampling; the pipe
        # remains bounded and never becomes a source of GUI backpressure.
        end_settle = time.monotonic() + settle
        samples = []
        family = None
        start = None
        next_sample = None
        while True:
            ensure_alive(process)
            try:
                chunk = os.read(read_fd, 4096)
                if chunk:
                    readiness.feed(chunk, time.monotonic_ns())
            except BlockingIOError:
                pass
            now = time.monotonic()
            if now >= end_settle and family is None:
                current = process_stat(process.pid)
                if identity(current) != identity(root):
                    raise MeasurementError('application identity changed during settle')
                family = Family(current)
                drm_before = read_drm_render(root)
                family.snapshot()
                # Descendants already running at measurement start need baselines.
                family.own_delta = 0
                start = now
                next_sample = now
            if family is not None:
                active = family.snapshot()
                if now >= next_sample or now - start >= duration:
                    total = {key: 0 for key in MEMORY_KEYS}
                    for member in active:
                        values = process_memory(member)
                        for key in MEMORY_KEYS:
                            total[key] += values[key]
                    samples.append(total)
                    raw_samples.append({'elapsed_seconds': now - start, **total})
                    next_sample = now + 5
                if now - start >= duration:
                    break
            time.sleep(min(0.25, max(0.001, duration - (now - start))) if start else 0.05)
        family.snapshot()
        elapsed = time.monotonic() - start
        cpu = family.cpu_ticks() / os.sysconf('SC_CLK_TCK') / elapsed * 100
        memory = summarize(samples)
        ensure_alive(process)
        drm_after = read_drm_render(root)
        renderer = renderer_with_drm(renderer, readiness.renderer, drm_before, drm_after,
                                     os.environ.get('PERF_HARNESS_XVFB') == '1')
        return {'page': page, 'category': evaluate(renderer, memory, cpu, readiness.events['usable'], first_launch),
                'renderer_class': readiness.renderer, 'renderer_mode': renderer,
                'drm_render_ns_before': drm_before, 'drm_render_ns_after': drm_after,
                'first_launch': first_launch, 'startup_limit_ms': 4000 if first_launch else 2000,
                'startup_ms': readiness.events, 'spawn_overhead_ms': spawn_ms,
                'duration_seconds': elapsed, 'settle_seconds': settle, 'cpu_percent': cpu,
                'preferred_cpu_target_percent': IDLE_CPU_TARGET_PERCENT,
                'preferred_cpu_target_met': cpu <= IDLE_CPU_TARGET_PERCENT,
                'accepted_cpu_limit_exclusive_percent': IDLE_CPU_ACCEPTANCE_EXCLUSIVE_PERCENT,
                'memory_family': memory, 'memory_samples': len(samples), 'raw_samples': raw_samples,
                'lifecycle_events': readiness.lifecycle, 'cpu_counters': family.cpu_details(),
                'descendant_disappearance_observed': family.disappeared,
                'cpu_attribution': 'maximum of observed identity CPU deltas and root own plus reaped-child deltas; nested and unseen descendant attribution remains approximate'}
    except (OSError, ValueError, MeasurementError) as error:
        message = str(error) if isinstance(error, MeasurementError) else 'measurement environment unavailable'
        code = process.poll() if process is not None else None
        if process is not None and code is None and message == 'readiness pipe closed before semantic milestones':
            try:
                code = process.wait(timeout=0.2)
            except subprocess.TimeoutExpired:
                pass
        partial = {}
        if readiness is not None:
            try:
                readiness.feed(os.read(read_fd, 4096), time.monotonic_ns())
            except (BlockingIOError, MeasurementError):
                pass
            partial.update(startup_ms=readiness.events, renderer_class=readiness.renderer,
                           lifecycle_events=readiness.lifecycle)
        if samples:
            partial.update(memory_family=summarize(samples), memory_samples=len(samples))
        if family is not None and start is not None:
            elapsed = time.monotonic() - start
            partial.update(observed_duration_seconds=elapsed,
                           observed_cpu_percent=family.cpu_ticks() / os.sysconf('SC_CLK_TCK') / elapsed * 100,
                           partial_measurement=True, cpu_counters=family.cpu_details())
        return {'page': page, **partial,
                'category': 'application-failure' if code is not None else 'environment-error',
                'reason': message, 'application_exit_code': code, 'raw_samples': raw_samples}
    finally:
        os.close(read_fd)
        if write_fd is not None:
            os.close(write_fd)
        if process is not None:
            try:
                os.killpg(process.pid, signal.SIGTERM)
                process.wait(timeout=2)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=2)
                except (ProcessLookupError, subprocess.TimeoutExpired):
                    pass
        if private_state is not None:
            private_state.cleanup()


def artifact_hash(binary):
    digest = hashlib.sha256()
    with binary.open('rb') as artifact:
        for chunk in iter(lambda: artifact.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', nargs='?', default='target/release/maintainer-app')
    parser.add_argument('--page', choices=PAGES, default='Overview')
    parser.add_argument('--runs', type=int)
    parser.add_argument('--matrix', action='store_true', help='Three runs on each supported page')
    parser.add_argument('--first-launch', action='store_true', help='Use empty private data and shader caches; four-second first-launch budget')
    args = parser.parse_args(argv)
    try:
        duration = float(os.environ.get('PERF_DURATION_SECS', '300'))
        settle = float(os.environ.get('PERF_SETTLE_SECS', '30'))
        if not (0 < duration <= 86400 and 0 <= settle <= 3600):
            raise ValueError
        runs = args.runs if args.runs is not None else (3 if args.matrix else 1)
        if runs < 1 or runs > 100:
            raise ValueError
        if os.geteuid() == 0:
            raise MeasurementError('run measurements as an unprivileged user')
        binary = Path(args.binary).resolve(strict=True)
        if not os.access(binary, os.X_OK) or not binary.is_file():
            raise MeasurementError('release binary is not executable')
        digest = artifact_hash(binary)
    except (OSError, ValueError, MeasurementError) as error:
        print(json.dumps({'category': 'environment-error', 'reason': str(error) if isinstance(error, MeasurementError) else 'invalid measurement configuration or unavailable artifact'}))
        return 2
    pages = PAGES if args.matrix else (args.page,)
    results = [dict(run_once(binary, page, duration, settle, first_launch=args.first_launch), run=index + 1)
               for page in pages for index in range(runs)]
    remaining = list(UNVERIFIED)
    if duration < 300 or settle < 30:
        remaining.append('short diagnostic duration or settle is not acceptance evidence')
    if not args.matrix or runs < 3:
        remaining.append('three runs on each required page')
    try:
        unchanged = artifact_hash(binary) == digest
    except OSError:
        unchanged = False
    if not unchanged:
        results.append({'category': 'environment-error', 'reason': 'artifact changed or became unavailable during measurement'})
    categories = [result['category'] for result in results]
    overall = next((category for category in ('application-failure', 'environment-error', 'ceiling-failure', 'target-miss') if category in categories), 'unverified')
    print(json.dumps({'category': overall, 'artifact_sha256': digest,
                      'protocol_unverified': remaining, 'release_acceptance': False,
                      'runs': results}, indent=2, allow_nan=False))
    return 2 if overall == 'environment-error' else 1


if __name__ == '__main__':
    sys.exit(main())
