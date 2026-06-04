"""Make the ``analysis`` package importable when running pytest from anywhere.

The package lives one directory up from ``tests/`` (``analysis/analysis``); add
the parent so ``import analysis`` works without an editable install.
"""

import os
import sys

_PKG_PARENT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if _PKG_PARENT not in sys.path:
    sys.path.insert(0, _PKG_PARENT)
