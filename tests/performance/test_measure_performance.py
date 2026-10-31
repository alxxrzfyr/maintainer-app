"""Deterministic fixtures; no GTK launch or benchmark workload."""
import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    'measure_performance', Path(__file__).resolve().parents[2] / 'scripts/measure-performance.py')
measure = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(measure)


def stat_line(pid=12, ppid=1, own=5, reaped=2, start=100, comm='tricky ) ( process'):
    fields = ['S', str(ppid)] + ['0'] * 22
    fields[11], fields[12] = str(own), '0'
    fields[13], fields[14] = str(reaped), '0'
    fields[19] = str(start)
    return f'{pid} ({comm}) ' + ' '.join(fields)


def event(name, timestamp):
    return (json.dumps({'event': name, 'monotonic_ns': timestamp}) + '\n').encode()


class ParserTests(unittest.TestCase):
    def test_stat_with_parentheses_and_spaces(self):
        value = measure.parse_stat(stat_line())
        self.assertEqual(value['own_ticks'], 5)
        self.assertEqual(value['reaped_ticks'], 2)
        self.assertEqual(measure.identity(value), (12, 100))

    def test_stat_truncated_or_invalid(self):
        for text in ('garbage', '12 (x) S 1', stat_line(own=-1)):
            with self.assertRaises(measure.MeasurementError):
                measure.parse_stat(text)

    def test_smaps_aggregates_all_regions_and_kernel_kib(self):
        region = '\n'.join(f'{key}: 2 kB' for key in measure.MEMORY_KEYS)
        values = measure.parse_smaps(region + '\n' + region)
        self.assertEqual(values['Pss'], 4096)
        self.assertEqual(values['SwapPss'], 4096)

    def test_smaps_missing_or_invalid_never_zero_fills(self):
        for text in ('Pss: 1 kB', 'Pss: 1 MB', 'Pss: -1 kB', 'Pss: nope kB'):
            with self.assertRaises(measure.MeasurementError):
                measure.parse_smaps(text)

    def test_memory_revalidates_pid_start_identity(self):
        before = measure.parse_stat(stat_line())
        after = measure.parse_stat(stat_line(start=101))
        text = '\n'.join(f'{key}: 2 kB' for key in measure.MEMORY_KEYS)
        with patch.object(measure, 'process_stat', side_effect=[before, after]), \
                patch.object(Path, 'read_text', return_value=text):
            with self.assertRaises(measure.MeasurementError):
                measure.process_memory((12, 100))

    def test_memory_falls_back_to_smaps(self):
        value = measure.parse_stat(stat_line())
        text = '\n'.join(f'{key}: 2 kB' for key in measure.MEMORY_KEYS)
        with patch.object(measure, 'process_stat', return_value=value), \
                patch.object(Path, 'read_text', side_effect=[FileNotFoundError(), text]):
            self.assertEqual(measure.process_memory((12, 100))['Rss'], 2048)


class ReadinessTests(unittest.TestCase):
    def test_readiness_pipe_eof_reports_a_crash_instead_of_an_environment_error(self):
        class Crashing:
            pid = 123
            def poll(self):
                return None
            def wait(self, **kwargs):
                return -11
        with patch.object(measure.subprocess, 'Popen', return_value=Crashing()), \
                patch.object(measure, 'process_stat', return_value=measure.parse_stat(stat_line())), \
                patch.object(measure, 'await_ready', side_effect=measure.MeasurementError('readiness pipe closed before semantic milestones')), \
                patch.object(measure.os, 'killpg'):
            result = measure.run_once(Path('/unused'), 'Overview', 300, 30)
        self.assertEqual(result['category'], 'application-failure')
        self.assertEqual(result['application_exit_code'], -11)

    def test_semantic_canary_and_renderer_fragmented_pipe(self):
        readiness = measure.Readiness(100)
        data = b''.join(event(name, 101 + index) for index, name in enumerate(
            ('process-start', 'mapped', 'renderer:GskGLRenderer', 'usable', 'first-data')))
        for byte in data:
            readiness.feed(bytes([byte]), 200)
        self.assertTrue(readiness.complete)
        self.assertEqual(readiness.renderer, 'GskGLRenderer')
        self.assertEqual(readiness.events['usable'], 4 / 1_000_000)

    def test_missing_data_or_early_eof_is_failure(self):
        readiness = measure.Readiness(100)
        readiness.feed(event('process-start', 101) + event('mapped', 102) + event('usable', 103), 200)
        self.assertFalse(readiness.complete)
        with self.assertRaises(measure.MeasurementError):
            readiness.feed(b'', 200)

    def test_data_before_navigation_canary_is_an_independent_milestone(self):
        readiness = measure.Readiness(100)
        for index, name in enumerate(('process-start', 'mapped', 'first-data', 'usable')):
            readiness.feed(event(name, 101 + index), 200)
        self.assertTrue(readiness.complete)
        self.assertLess(readiness.events['first-data'], readiness.events['usable'])

    def test_out_of_order_bad_timestamp_and_invalid_utf8(self):
        for data in (event('usable', 101), event('process-start', 99),
                     event('process-start', 1000), b'\xff\n', b'[]\n'):
            with self.assertRaises(measure.MeasurementError):
                measure.Readiness(100).feed(data, 200)

    def test_preexisting_instance_exit_does_not_pass(self):
        class Exited:
            def poll(self):
                return 0
        read_fd, write_fd = measure.os.pipe()
        try:
            with self.assertRaises(measure.MeasurementError):
                measure.await_ready(Exited(), read_fd, 100)
        finally:
            measure.os.close(read_fd)
            measure.os.close(write_fd)

    def test_crashed_launch_is_recorded_as_failure_with_exit_status(self):
        class Crashed:
            pid = 123
            def poll(self):
                return -11
            def wait(self, **kwargs):
                return -11
        with patch.object(measure.subprocess, 'Popen', return_value=Crashed()), \
                patch.object(measure, 'process_stat', side_effect=measure.MeasurementError('process identity unavailable')), \
                patch.object(measure.os, 'killpg'):
            result = measure.run_once(Path('/unused'), 'Overview', 300, 30)
        self.assertEqual(result['category'], 'application-failure')
        self.assertEqual(result['application_exit_code'], -11)
        self.assertEqual(result['raw_samples'], [])

    def test_early_exit_preserves_readiness_without_reporting_a_pass(self):
        class Exited:
            pid = 123
            def poll(self):
                return 0
            def wait(self, **kwargs):
                return 0
        readiness = measure.Readiness(measure.time.monotonic_ns())
        readiness.events = {'usable': 123.0}
        readiness.renderer = 'GskNglRenderer'
        with patch.object(measure.subprocess, 'Popen', return_value=Exited()), \
                patch.object(measure, 'process_stat', return_value=measure.parse_stat(stat_line())), \
                patch.object(measure, 'await_ready', return_value=readiness), \
                patch.object(measure, 'detect_renderer', return_value='hardware'), \
                patch.object(measure.os, 'killpg'):
            result = measure.run_once(Path('/unused'), 'Overview', 300, 30)
        self.assertEqual(result['category'], 'application-failure')
        self.assertEqual(result['application_exit_code'], 0)
        self.assertEqual(result['startup_ms'], {'usable': 123.0})
        self.assertEqual(result['renderer_class'], 'GskNglRenderer')

    def test_lifecycle_records_are_bounded_and_separate_from_readiness(self):
        readiness = measure.Readiness(100)
        readiness.feed(event("process-start", 101), 200)
        readiness.feed(event("close-requested", 102), 200)
        self.assertFalse(readiness.complete)
        self.assertEqual(readiness.lifecycle[0]["event"], "close-requested")
        for _ in range(15):
            readiness.feed(event("application-shutdown", 102), 200)
        with self.assertRaises(measure.MeasurementError):
            readiness.feed(event("window-destroyed", 102), 200)

    def test_probe_size_limit(self):
        with self.assertRaises(measure.MeasurementError):
            measure.Readiness(100).feed(b'x' * 65537, 200)


class EvaluationTests(unittest.TestCase):
    def test_first_launch_uses_empty_private_data_and_cache_and_cleans_them(self):
        directories = []
        class Exited:
            pid = 123
            def poll(self):
                return 0
            def wait(self, **kwargs):
                return 0
        def launch(*args, **kwargs):
            for name in ('XDG_CACHE_HOME', 'XDG_DATA_HOME'):
                directory = Path(kwargs['env'][name])
                self.assertTrue(directory.is_dir())
                self.assertEqual(list(directory.iterdir()), [])
                self.assertEqual(directory.parent.stat().st_mode & 0o077, 0)
                directories.append(directory)
            return Exited()
        with patch.object(measure.subprocess, 'Popen', side_effect=launch), \
                patch.object(measure, 'process_stat', return_value=measure.parse_stat(stat_line())), \
                patch.object(measure, 'await_ready', side_effect=measure.MeasurementError('exited')), \
                patch.object(measure.os, 'killpg'):
            result = measure.run_once(Path('/unused'), 'Overview', 300, 30, first_launch=True)
        self.assertEqual(result['category'], 'application-failure')
        self.assertEqual(len(directories), 2)
        self.assertTrue(all(not directory.exists() for directory in directories))

    def test_first_launch_has_four_seconds_but_later_launches_keep_two(self):
        memory = {key: {'max_bytes': 1, 'median_bytes': 1} for key in measure.MEMORY_KEYS}
        self.assertEqual(measure.evaluate('hardware', memory, 0.4, 4000, first_launch=True), 'pass')
        self.assertEqual(measure.evaluate('hardware', memory, 0.4, 4001, first_launch=True), 'ceiling-failure')
        self.assertEqual(measure.evaluate('hardware', memory, 0.4, 2001), 'ceiling-failure')
        self.assertEqual(measure.evaluate('hardware', memory, 0.5, 3000, first_launch=True), 'target-miss')

    def test_vulkan_requires_advancing_physical_drm_counters(self):
        sample = 'drm-driver: i915\ndrm-client-id: 7\ndrm-engine-render: 100 ns\n'
        self.assertEqual(measure.parse_drm_render(sample), ('i915:7', 100))
        self.assertIsNone(measure.parse_drm_render(sample.replace('i915', 'lavapipe')))
        self.assertIsNone(measure.parse_drm_render(sample.replace('100 ns', 'invalid')))
        before, after = {'i915:7': 100}, {'i915:7': 200}
        self.assertEqual(measure.renderer_with_drm('unknown', 'GskVulkanRenderer', before, after), 'hardware')
        self.assertEqual(measure.renderer_with_drm('unknown', 'GskVulkanRenderer', before, before), 'unknown')
        self.assertEqual(measure.renderer_with_drm('unknown', 'GskVulkanRenderer', before, {'i915:8': 200}), 'unknown')
        self.assertEqual(measure.renderer_with_drm('unknown', 'GskVulkanRenderer', before, after, True), 'unknown')
        self.assertEqual(measure.renderer_with_drm('software', 'GskVulkanRenderer', before, after), 'software')

    def test_renderer_requires_actual_evidence(self):
        self.assertEqual(measure.renderer_mode(None, virtual_display=True), 'unknown')
        self.assertEqual(measure.renderer_mode('GskMysteryRenderer', virtual_display=True), 'unknown')
        self.assertEqual(measure.renderer_mode('GskCairoRenderer'), 'software')
        self.assertEqual(measure.renderer_mode('GskGLRenderer'), 'unknown')
        self.assertEqual(measure.renderer_mode('GskGLRenderer', 'OpenGL renderer string: llvmpipe'), 'software')
        self.assertEqual(measure.renderer_mode('GskGLRenderer', 'direct rendering: Yes\nOpenGL renderer string: Mesa Intel'), 'hardware')

    def test_budgets_and_target_are_distinct(self):
        memory = {key: {'max_bytes': 1, 'median_bytes': 1} for key in measure.MEMORY_KEYS}
        self.assertEqual(measure.evaluate('hardware', memory, 0.3, 100), 'pass')
        self.assertEqual(measure.evaluate('hardware', memory, 0.35, 100), 'pass')
        self.assertEqual(measure.evaluate('hardware', memory, 0.350001, 100), 'pass')
        self.assertEqual(measure.evaluate('hardware', memory, 0.4, 100), 'pass')
        self.assertEqual(measure.evaluate('hardware', memory, 0.499999, 100), 'pass')
        self.assertEqual(measure.evaluate('hardware', memory, 0.5, 100), 'target-miss')
        self.assertEqual(measure.evaluate('hardware', memory, 0.6, 100), 'target-miss')
        self.assertEqual(measure.evaluate('hardware', memory, 1.1, 100), 'ceiling-failure')
        self.assertEqual(measure.evaluate('software', memory, 1.1, 100), 'target-miss')
        self.assertEqual(measure.evaluate('unknown', memory, 0, 100), 'unverified')
        self.assertEqual(measure.evaluate('hardware', memory, 0, 2001), 'ceiling-failure')
        memory['Pss']['max_bytes'] = 100_000_000
        self.assertEqual(measure.evaluate('hardware', memory, 0, 100), 'ceiling-failure')
        self.assertEqual(measure.evaluate('software', memory, 0, 100), 'pass')

    def test_cpu_diagnostics_separate_root_and_reaped_child_work(self):
        root = measure.parse_stat(stat_line(own=5, reaped=2))
        family = measure.Family(root)
        family.last[family.root] = measure.parse_stat(stat_line(own=9, reaped=12))
        family.own_delta = 6
        self.assertEqual(family.cpu_details()["root_own_ticks"], 4)
        self.assertEqual(family.cpu_details()["root_reaped_child_ticks"], 10)
        self.assertEqual(family.cpu_details()["observed_identity_own_ticks"], 6)

    def test_reaped_children_are_not_added_twice(self):
        root = measure.parse_stat(stat_line())
        family = measure.Family(root)
        family.own_delta = 20
        family.root_total = family.root_total_start + 15
        self.assertEqual(family.cpu_ticks(), 20)
        family.root_total = family.root_total_start + 30
        self.assertEqual(family.cpu_ticks(), 30)

    def test_empty_samples_fail(self):
        with self.assertRaises(measure.MeasurementError):
            measure.summarize([])


if __name__ == '__main__':
    unittest.main()
