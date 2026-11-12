# Awesome-RE-MCP — Setup on this laptop

Source: https://github.com/crowdere/Awesome-RE-MCP (curated list of RE tools with MCP servers).

## Already generalized across all IDEs/agents on this laptop
- **shadcn MCP** (`npx shadcn@latest mcp`) — Cursor, Claude Code, VS Code, OpenCode, Codex (+ Aceternity registry in `components.json`).
- **REA** (`npx -y rea-agents@6.7.0 mcp`) — 21 clients (Cursor, Claude Code/Desktop, Codex, Copilot CLI, VS Code, OpenCode, Windsurf, Devin, Droid, DSH, Gemini CLI, Antigravity, Hermes, Kimi, Pi, Qoder, Qwen Code, Trae, Grok Build, Grok Bot [manual]).
- **frida-mcp** (`npx -y frida-mcp`) — Cursor, Claude Code, VS Code, OpenCode, Codex. (Requires Frida installed; not present yet.)

## From Awesome-RE-MCP — install prerequisites first
Most entries below need the underlying RE tool installed (many are paid/licensed). Add their MCP entry to your client config once the tool is available.

### Disassemblers & Decompilers
- IDA Pro → ida-pro-mcp (mrexodia) — requires IDA Pro license
- Ghidra → GhidraMCP (LaurieWired) / ReVa — requires Ghidra
- Binary Ninja → binary_ninja_mcp (fosdickio) — requires Binary Ninja
- radare2 → radare2-mcp (radareorg) — requires radare2/r2
- x64dbg → x64dbgMCP (Wasdubya) — Windows, requires x64dbg
- Cutter → CutterMCP (ap425q) — requires Cutter/Rizin

### Debuggers
- LLDB → native MCP support (official, June 2025)
- GDB → GDB MCP Server (pansila)

### Static / Dynamic Analysis
- YaraFlux (ThreatFlux) — YARA malware detection (Docker)
- frida-mcp (dnakov) — already added above (requires Frida)

### Network Analysis
- WireMCP (0xKoda) — Wireshark (requires tshark)
- Burp Suite MCP (PortSwigger) — requires Burp Suite
- BurpMCP (swgee) — requires Burp
- ZAP-MCP (ajtazer) — requires OWASP ZAP

### Android/Mobile
- Jadx MCP Plugin (mobilehackinglab) / jadx-mcp-server (zinja-coder) — requires JADX
- apktool-mcp-server (zinja-coder) — requires apktool

### Security Testing
- mcp-for-security (cyproxio) — wraps SQLMap, FFUF, NMAP, Masscan (requires those tools)

## Missing (no MCP yet — community gap)
UPX, PEiD, Detect It Easy, ExeinfoPE, MalCat, Cuckoo, CAPE, Joe Sandbox, ANY.RUN, pestudio, AFL++, libFuzzer, honggfuzz, Radamsa, ExifTool, Foremost, binwalk, dex2jar, objection, MobSF, APK Analyzer, Androguard, WinDbg, OllyDbg, Immunity Debugger, rr, Fiddler, mitmproxy.

## How to enable an MCP (example: frida-mcp)
Add to your client config (already done for frida-mcp):
- Cursor/ Claude: mcpServers["frida-mcp"] = { command: "npx", args: ["-y","frida-mcp"] }
- OpenCode: mcp["frida-mcp"] = { type: "stdio", command: "npx", args: ["-y","frida-mcp"] }
- Codex: [mcp_servers.frida-mcp] command="npx" args=["-y","frida-mcp"]
- VS Code: servers["frida-mcp"] = { type: "stdio", command: "npx", args: ["-y","frida-mcp"] }

For tools requiring a host binary, replace the command with the tool's MCP launcher (see each repo's README).
