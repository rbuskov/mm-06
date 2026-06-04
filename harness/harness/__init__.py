"""`dsp-harness` — the MM-06 calibration agent's hands.

Wraps the offline `render` binary + the Python `analysis/` layer and exposes the
`calibration-and-testing-strategy.md` §2.3 tool surface. Every tool is a plain
typed function in :mod:`harness.tools` (unit-testable without an MCP client);
:mod:`harness.server` registers the same functions with an MCP server.
"""

from __future__ import annotations

from .tools import (
    analyze,
    apply_param_edit,
    compare_to_reference,
    compare_to_targets,
    list_params,
    parity_check,
    propose_param_edit,
    render_mix,
    render_voice,
    run_invariants,
)

__all__ = [
    "render_voice",
    "render_mix",
    "analyze",
    "run_invariants",
    "parity_check",
    "compare_to_targets",
    "compare_to_reference",
    "propose_param_edit",
    "apply_param_edit",
    "list_params",
]
