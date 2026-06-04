"""Make `import harness` work when running pytest from the harness/ dir.

When the harness is installed editable (`pip install -e .`) this is a no-op;
this conftest just guarantees the package is importable when it isn't installed.
"""

from __future__ import annotations

import sys
from pathlib import Path

HARNESS_DIR = Path(__file__).resolve().parent.parent
if str(HARNESS_DIR) not in sys.path:
    sys.path.insert(0, str(HARNESS_DIR))
