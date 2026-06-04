"""Repo-path resolution for the dsp-harness.

Everything is resolved **relative to this file** so the harness works from any
worktree without hard-coded absolute paths. The layout it assumes::

    <repo>/
      crates/render/...        # the offline render binary (cargo workspace)
      analysis/analysis/...    # the Python feature-extraction package
      references/<voice>.wav    # regime-C reference clips
      target/debug/render       # built render binary (cargo build -p render)
      harness/harness/...       # this package
      harness/.work/            # generated WAV/JSON artifacts (git-ignored)
"""

from __future__ import annotations

import os
from pathlib import Path

# harness/harness/paths.py -> harness/harness -> harness -> <repo>
HARNESS_PKG_DIR = Path(__file__).resolve().parent
HARNESS_DIR = HARNESS_PKG_DIR.parent
REPO_ROOT = HARNESS_DIR.parent

CRATES_DIR = REPO_ROOT / "crates"
ANALYSIS_DIR = REPO_ROOT / "analysis"
ANALYSIS_PKG_PARENT = ANALYSIS_DIR  # `import analysis` lives directly under analysis/
REFERENCES_DIR = REPO_ROOT / "references"

# Generated artifacts live here; git-ignored via harness/.gitignore.
WORK_DIR = HARNESS_DIR / ".work"


def render_binary() -> Path:
    """Absolute path to the built `render` binary (cargo build -p render).

    Honours `CARGO_TARGET_DIR` if set; otherwise the workspace `target/`.
    Returns the debug build path; callers should build it first
    (`cargo build -p render`) — `ensure_render_built()` does that on demand.
    """
    target = os.environ.get("CARGO_TARGET_DIR")
    target_dir = Path(target) if target else (REPO_ROOT / "target")
    exe = "render.exe" if os.name == "nt" else "render"
    return target_dir / "debug" / exe


def ensure_work_dir() -> Path:
    WORK_DIR.mkdir(parents=True, exist_ok=True)
    return WORK_DIR


def add_analysis_to_syspath() -> None:
    """Make `import analysis` work by putting `analysis/` on `sys.path`.

    Keeps `analysis/` unmodified — we never pip-install it into the harness.
    """
    import sys

    p = str(ANALYSIS_PKG_PARENT)
    if p not in sys.path:
        sys.path.insert(0, p)
