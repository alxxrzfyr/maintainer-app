"""Maintainer scripts run exclusively against temporary paths and mocked services."""
import os
import fcntl
from pathlib import Path
import subprocess
import tempfile
import unittest


class HelperUpgrade(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.tools = self.base / 'tools'
        self.tools.mkdir()
        self.runtime = self.base / 'run/systemd/system'
        self.runtime.mkdir(parents=True)
        (self.base / 'etc/systemd/system').mkdir(parents=True)
        self.mask = self.runtime / 'maintainer-helper.service'
        self.state = self.base / 'run/maintainer-package-upgrade'
        self.log = self.base / 'calls'
        self.environment = dict(os.environ, PATH=str(self.tools) + os.pathsep + os.environ['PATH'],
                                FIXTURE_LOG=str(self.log), FIXTURE_MODE='idle', FIXTURE_CLOCK=str(self.base / 'clock'))
        self.tool('systemctl', '''printf 'systemctl %s\\n' "$*" >> "$FIXTURE_LOG"
case "$*" in
  *LoadState*) [ "$FIXTURE_MODE" = override ] && echo loaded || echo masked ;;
  *ActiveState*) [ "$FIXTURE_MODE" = busy ] && echo active || echo inactive ;;
  *MainPID*) [ "$FIXTURE_MODE" = busy ] && echo 123 || echo 0 ;;
  daemon-reload) : ;;
  *) exit 99 ;;
esac
''')
        self.tool('busctl', '''printf 'busctl %s\\n' "$*" >> "$FIXTURE_LOG"
[ "$FIXTURE_MODE" != bus-error ] || exit 1
[ "$FIXTURE_MODE" = busy ] && echo 'b true' || echo 'b false'
''')
        self.tool('date', '''if [ -e "$FIXTURE_CLOCK" ]; then echo 200; else touch "$FIXTURE_CLOCK"; echo 0; fi
''')
        for tool in ('sleep', 'glib-compile-schemas', 'gtk-update-icon-cache', 'update-desktop-database'):
            self.tool(tool, ':\n')

    def tool(self, name, body):
        path = self.tools / name
        path.write_text('#!/bin/sh\nset -eu\n' + body)
        path.chmod(0o755)

    def run_script(self, name, argument):
        root = Path(__file__).resolve().parents[2]
        text = (root / 'packaging/debian' / name).read_text()
        for prefix in ('/run/', '/etc/', '/usr/share/', '/var/lib/'):
            text = text.replace(prefix, str(self.base / prefix.lstrip('/')) + '/')
        text = text.replace('= 0:700', '= ' + str(os.getuid()) + ':700')
        script = self.base / name
        script.write_text(text)
        return subprocess.run(['sh', str(script), argument], env=self.environment,
                              capture_output=True, text=True, check=False)

    def test_idle_old_helper_drains_then_configure_restores_owned_mask(self):
        result = self.run_script('preinst', 'upgrade')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.mask.is_symlink())
        self.assertTrue((self.runtime / "maintainer-package-owner.service").is_symlink())
        self.assertTrue(self.state.is_dir())
        self.assertEqual(self.run_script('preinst', 'upgrade').returncode, 0)
        result = self.run_script('postinst', 'configure')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.mask.is_symlink())
        self.assertFalse((self.runtime / "maintainer-package-owner.service").is_symlink())
        self.assertFalse(self.state.exists())
        calls = self.log.read_text()
        self.assertNotIn(' stop ', calls)
        self.assertNotIn(' restart ', calls)
        self.assertNotIn('kill', calls)

    def test_busy_helper_refuses_upgrade_without_stopping_work(self):
        self.environment['FIXTURE_MODE'] = 'busy'
        result = self.run_script('preinst', 'upgrade')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('busy', result.stderr)
        self.assertFalse(self.mask.is_symlink())
        self.assertFalse(self.state.exists())
        self.assertNotIn('restart', self.log.read_text())
        self.assertNotIn('stop', self.log.read_text())

    def test_orphan_backend_lock_refuses_upgrade_and_purge_without_signals(self):
        transactions = self.base / 'var/lib/maintainer/transactions'
        transactions.mkdir(parents=True, mode=0o700)
        with (transactions / 'current.lock').open('w') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            result = self.run_script('preinst', 'upgrade')
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('backend is busy', result.stderr)
            self.assertFalse(self.mask.is_symlink())
            self.assertFalse((self.runtime / 'maintainer-package-owner.service').is_symlink())
            result = self.run_script('postrm', 'purge')
            self.assertNotEqual(result.returncode, 0)
            self.assertTrue(transactions.is_dir())
        calls = self.log.read_text()
        self.assertNotIn(' stop ', calls)
        self.assertNotIn('kill', calls)

    def test_administrator_runtime_mask_survives_configuration(self):
        self.mask.symlink_to('/dev/null')
        result = self.run_script('preinst', 'upgrade')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.state.exists())
        result = self.run_script('postinst', 'configure')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.mask.is_symlink())

    def test_bus_failure_restores_activation_and_abort_upgrade_restores_mask(self):
        self.environment['FIXTURE_MODE'] = 'bus-error'
        self.assertNotEqual(self.run_script('preinst', 'upgrade').returncode, 0)
        self.assertFalse(self.mask.is_symlink())
        self.environment['FIXTURE_MODE'] = 'idle'
        self.assertEqual(self.run_script('preinst', 'upgrade').returncode, 0)
        self.assertEqual(self.run_script('postrm', 'abort-upgrade').returncode, 0)
        self.assertFalse(self.mask.is_symlink())

    def test_effective_etc_override_refuses_upgrade_and_preserves_override(self):
        override = self.base / 'etc/systemd/system/maintainer-helper.service'
        override.write_text('[Service]\nExecStart=/administrator/helper\n')
        self.environment['FIXTURE_MODE'] = 'override'
        result = self.run_script('preinst', 'upgrade')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('effective', result.stderr)
        self.assertFalse(self.mask.is_symlink())
        self.assertFalse(self.state.exists())
        self.assertIn('/administrator/helper', override.read_text())
        self.assertNotIn('stop', self.log.read_text())

    def test_required_schema_failure_fails_configuration_and_restores_own_mask(self):
        self.assertEqual(self.run_script('preinst', 'upgrade').returncode, 0)
        self.tool('glib-compile-schemas', 'exit 1\n')
        self.assertNotEqual(self.run_script('postinst', 'configure').returncode, 0)
        self.assertFalse(self.mask.is_symlink())
        self.assertFalse(self.state.exists())

    def test_replaced_mask_is_never_removed(self):
        self.assertEqual(self.run_script('preinst', 'upgrade').returncode, 0)
        self.mask.unlink()
        self.mask.symlink_to('/administrator/override')
        self.assertNotEqual(self.run_script('postinst', 'configure').returncode, 0)
        self.assertEqual(os.readlink(self.mask), '/administrator/override')


if __name__ == '__main__':
    unittest.main()
