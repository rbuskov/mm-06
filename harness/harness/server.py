"""MCP server entry for the dsp-harness.

Registers the :mod:`harness.tools` functions with the official MCP Python SDK
(`mcp`, modelcontextprotocol) and serves them over stdio. The SDK is imported
**lazily inside `build_server()`** so this module imports cleanly even where the
`mcp` package isn't installed offline — the tool functions themselves never need
the SDK (`harness.tools`), and the function-level tests run without it.

Run the server::

    python -m harness.server            # or: dsp-harness   (console script)

It speaks MCP over stdio, suitable for a `claude_desktop_config.json` /
`mcp.json` stdio server entry pointing at this module's `main`.
"""

from __future__ import annotations

from . import tools


def build_server():
    """Construct the `FastMCP` server with every §2.3 tool registered.

    Imports `mcp` lazily so the module stays importable without the SDK.
    """
    try:
        from mcp.server.fastmcp import FastMCP
    except ImportError as e:  # pragma: no cover - exercised only without the SDK
        raise ImportError(
            "the `mcp` SDK is required to run the server "
            "(pip install -r harness/requirements.txt). The tool functions in "
            "harness.tools work without it."
        ) from e

    mcp = FastMCP("dsp-harness")

    @mcp.tool()
    def render_voice(
        voice: str,
        sample_rate: float = tools.DEFAULT_SAMPLE_RATE,
        block_size: int = tools.DEFAULT_BLOCK_SIZE,
        seed: int = tools.DEFAULT_SEED,
        variance_depth: float = tools.DEFAULT_VARIANCE_DEPTH,
        accent: bool = False,
        events: list | None = None,
        length_samples: int = tools.DEFAULT_LENGTH_SAMPLES,
    ) -> str:
        """Render one voice in isolation (pre-mix) → wav_id."""
        return tools.render_voice(
            voice,
            sample_rate=sample_rate,
            block_size=block_size,
            seed=seed,
            variance_depth=variance_depth,
            accent=accent,
            events=events,
            length_samples=length_samples,
        )

    @mcp.tool()
    def render_mix(request: dict) -> str:
        """Render the full main output → wav_id."""
        return tools.render_mix(request)

    @mcp.tool()
    def analyze(wav_id: str, feature_set: list | str | None = None) -> dict:
        """Extract a metrics JSON from a rendered wav_id (wraps analysis/)."""
        return tools.analyze(wav_id, feature_set=feature_set)

    @mcp.tool()
    def run_invariants(scope: str = "regime-a") -> dict:
        """Run the regime-A invariant suite → structured pass/fail report."""
        return tools.run_invariants(scope)

    @mcp.tool()
    def parity_check(request: dict | None = None) -> dict:
        """WASM↔native parity gate → max bit divergence (0 when bit-identical)."""
        return tools.parity_check(request)

    @mcp.tool()
    def compare_to_targets(metrics: dict, voice: str) -> dict:
        """Regime B: deltas from measured metrics to the spec targets."""
        return tools.compare_to_targets(metrics, voice)

    @mcp.tool()
    def compare_to_reference(wav_id: str, voice: str) -> dict:
        """Regime C: shape distance to references/<voice>.wav."""
        return tools.compare_to_reference(wav_id, voice)

    @mcp.tool()
    def list_params() -> list:
        """List the whitelist of calibratable constants + active values."""
        return tools.list_params()

    @mcp.tool()
    def propose_param_edit(key: str, new_value: float) -> dict:
        """Validate (don't persist) a calibratable-constant edit."""
        return tools.propose_param_edit(key, new_value)

    @mcp.tool()
    def apply_param_edit(key: str, new_value: float) -> dict:
        """Persist an in-scope, in-bounds calibratable-constant edit."""
        return tools.apply_param_edit(key, new_value)

    return mcp


def main() -> None:
    """Console entry: build the server and serve over stdio."""
    build_server().run()


if __name__ == "__main__":
    main()
