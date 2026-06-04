"""The MCP server entry must import cleanly even without the `mcp` SDK.

`harness.server` imports the SDK lazily inside `build_server()`, so importing the
module never requires `mcp`. If the SDK *is* installed, `build_server()` builds a
real server with every tool registered.
"""

from __future__ import annotations

import importlib

import pytest


def test_server_module_imports_without_sdk():
    # Module import must not require the mcp SDK.
    mod = importlib.import_module("harness.server")
    assert hasattr(mod, "build_server")
    assert hasattr(mod, "main")


def test_build_server_when_sdk_present():
    pytest.importorskip("mcp", reason="mcp SDK not installed offline")
    from harness.server import build_server

    server = build_server()
    # FastMCP names the server; the tools are registered as MCP tools.
    assert server is not None
