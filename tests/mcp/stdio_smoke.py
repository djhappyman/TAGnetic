#!/usr/bin/env python3
"""Exercise a real installed MCP stdio connector without a model or extra packages.

Example (pair/enable through the trusted desktop first):
  python tests/mcp/stdio_smoke.py --report /tmp/mcp-smoke.json -- \
    /path/to/tagrex --mcp-stdio --mcp-profile /path/to/profile \
    --mcp-credential /path/to/client.credential

The report deliberately excludes response bodies, paths, credentials and tags.
This proves the wire contract, not the tagging workflow or host compatibility.
"""

import argparse
import asyncio
import contextlib
import json
import pathlib
import time


class Connector:
    def __init__(self, command, timeout):
        self.command, self.timeout = command, timeout
        self.sequence = 0
        self.notifications = 0
        self.process = None
        self.stderr = bytearray()

    async def __aenter__(self):
        self.process = await asyncio.create_subprocess_exec(
            *self.command, stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
            limit=2 * 1024 * 1024,
        )
        self.stderr_task = asyncio.create_task(self.drain_stderr())
        return self

    async def drain_stderr(self):
        while chunk := await self.process.stderr.read(4096):
            # Drain continuously so stderr cannot deadlock the connector.
            self.stderr.extend(chunk)
            del self.stderr[:-65536]

    async def send(self, message):
        self.process.stdin.write(json.dumps(message).encode() + b"\n")
        await self.process.stdin.drain()

    async def request(self, method, params):
        self.sequence += 1
        request_id = self.sequence
        await self.send({"jsonrpc": "2.0", "id": request_id,
                         "method": method, "params": params})
        while True:
            line = await asyncio.wait_for(self.process.stdout.readline(), self.timeout)
            if not line:
                raise AssertionError("Connector closed before its JSON-RPC response")
            # Any logs on stdout fail here instead of being silently ignored.
            message = json.loads(line)
            assert message.get("jsonrpc") == "2.0", "Non-JSON-RPC stdout"
            if "id" not in message:
                self.notifications += 1
                continue
            assert message["id"] == request_id, "Unexpected JSON-RPC response id"
            assert "error" not in message, f"JSON-RPC error for {method}"
            assert "result" in message, "Missing JSON-RPC result"
            return message["result"]

    async def __aexit__(self, *unused):
        if self.process.stdin:
            self.process.stdin.close()
            with contextlib.suppress(BrokenPipeError, ConnectionResetError):
                await self.process.stdin.wait_closed()
        try:
            await asyncio.wait_for(self.process.wait(), 5)
        except asyncio.TimeoutError:
            self.process.terminate()
            try:
                await asyncio.wait_for(self.process.wait(), 5)
            except asyncio.TimeoutError:
                self.process.kill()
                await self.process.wait()
        await self.stderr_task


async def smoke(args):
    started = time.monotonic()
    async with Connector(args.command, args.timeout) as client:
        initialized = await client.request("initialize", {
            "protocolVersion": args.protocol,
            "capabilities": {},
            "clientInfo": {"name": "tagnetic-wire-smoke", "version": "1"},
        })
        assert initialized.get("protocolVersion"), "No negotiated protocol"
        assert "tools" in initialized.get("capabilities", {}), "No tool capability"
        await client.send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        await client.request("ping", {})
        tools = []
        cursor = None
        seen = set()
        while True:
            page = await client.request("tools/list", {"cursor": cursor} if cursor else {})
            for tool in page["tools"]:
                assert tool["name"] not in seen, "Duplicate tool name"
                assert tool.get("description"), "Undocumented tool"
                assert tool.get("inputSchema", {}).get("type") == "object", "Invalid input schema"
                seen.add(tool["name"])
                tools.append(tool)
            cursor = page.get("nextCursor")
            if not cursor:
                break
        assert tools, "Empty MCP tool catalog"
        # Unknown names must produce a tool or JSON-RPC error, never execute.
        client.sequence += 1
        unknown_id = client.sequence
        await client.send({"jsonrpc": "2.0", "id": unknown_id, "method": "tools/call",
                           "params": {"name": "tagnetic_nonexistent_tool", "arguments": {}}})
        while True:
            line = await asyncio.wait_for(client.process.stdout.readline(), args.timeout)
            assert line, "Connector closed on an unknown tool"
            reply = json.loads(line)
            if "id" not in reply:
                continue
            assert reply["id"] == unknown_id
            assert "error" in reply or reply.get("result", {}).get("isError") is True
            break
        # The session must still work after a refused request.
        await client.request("ping", {})
        for name in args.read_tool:
            assert name in seen, "Requested read tool missing from catalog"
            result = await client.request("tools/call", {"name": name, "arguments": {}})
            assert not result.get("isError", False), "Read tool failed"
        report = {"protocol": initialized["protocolVersion"], "tool_count": len(tools),
                  "tool_names": sorted(seen), "unknown_tool_refused": True,
                  "stdout_jsonrpc_only": True, "read_tools": args.read_tool,
                  "duration_seconds": round(time.monotonic() - started, 3)}
    report["connector_exit_code"] = client.process.returncode
    assert client.process.returncode == 0, "Connector did not exit cleanly on EOF"
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: v for k, v in report.items() if k != "tool_names"}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--protocol", default="2025-11-25")
    parser.add_argument("--timeout", default=30, type=float)
    parser.add_argument("--report", type=pathlib.Path)
    parser.add_argument("--read-tool", action="append", default=[])
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.command and args.command[0] == "--":
        args.command.pop(0)
    if not args.command:
        parser.error("pass the connector command after --")
    asyncio.run(smoke(args))


if __name__ == "__main__":
    main()
