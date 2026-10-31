"""Fixture-only root guard around the installed backend's shared engine."""
import importlib.util
import os
from pathlib import Path
import sys
from unittest.mock import patch

import apt_pkg

source = Path(__file__).resolve().parents[2] / 'scripts/maintainer-apt-backend.py'
spec = importlib.util.spec_from_file_location('maintainer_apt_backend', source)
backend = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = backend
spec.loader.exec_module(backend)


def fixture_inhibitor():
    # Exercise the real holder protocol without contacting this host's logind.
    with patch.object(backend, 'INHIBITOR_COMMAND', ('/usr/bin/cat',)):
        return backend.ShutdownInhibitor()


class RetainedPlan(backend.Backend):
    def __init__(self, root, **kwargs):
        root = Path(root).resolve(strict=True)
        if os.geteuid() == 0 or not (root / '.maintainer-fixture').is_file():
            raise ValueError('only unprivileged disposable fixtures are supported')
        status = Path(apt_pkg.config.find_file('Dir::State::status')).resolve()
        if status != root / 'var/lib/dpkg/status':
            raise ValueError('APT configuration does not target fixture')
        kwargs.setdefault('inhibitor_factory', fixture_inhibitor)
        super().__init__(**kwargs)
