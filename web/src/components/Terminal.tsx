import { useEffect, useRef, useState } from "react";
import { api } from "../api";

/* ============================================================
   Terminal Component - Embedded terminal for bot server
   ============================================================ */

interface TerminalLine {
  id: number;
  type: "prompt" | "output" | "error" | "warn" | "success" | "info" | "system";
  content: string;
  timestamp: number;
}

export default function Terminal() {
  const [lines, setLines] = useState<TerminalLine[]>([]);
  const [input, setInput] = useState("");
  const [connected, setConnected] = useState(false);
  const [history, setHistory] = useState<string[]>([]);
  const [historyIndex, setHistoryIndex] = useState(-1);
  const terminalRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const lineId = useRef(0);
  const wsRef = useRef<WebSocket | null>(null);

  /* ============================================================
     WebSocket Connection to Terminal Stream
     ============================================================ */
  useEffect(() => {
    const connectTerminal = async () => {
      try {
        const { ticket } = await api.wsTicket();
        const proto = location.protocol === "https:" ? "wss" : "ws";
        const ws = new WebSocket(`${proto}://${location.host}/api/terminal?ticket=${encodeURIComponent(ticket)}`);

        ws.onopen = () => {
          setConnected(true);
          addLine("system", "Terminal connected to bot server");
          addLine("system", "Type 'help' for available commands");
        };

        ws.onclose = () => {
          setConnected(false);
          addLine("system", "Terminal disconnected");
          // Reconnect after 3 seconds
          setTimeout(connectTerminal, 3000);
        };

        ws.onmessage = (ev) => {
          try {
            const data = JSON.parse(ev.data);
            if (data.type === "output") {
              addLine(data.level || "output", data.content);
            } else if (data.type === "prompt") {
              addLine("prompt", data.content);
            }
          } catch {
            // If not JSON, treat as raw output
            addLine("output", ev.data);
          }
        };

        ws.onerror = (err) => {
          console.error("Terminal WebSocket error:", err);
          addLine("error", "Connection error");
        };

        wsRef.current = ws;
      } catch (err) {
        console.error("Terminal connection failed:", err);
        addLine("error", "Failed to connect to terminal");
        setTimeout(connectTerminal, 5000);
      }
    };

    connectTerminal();

    return () => {
      if (wsRef.current) {
        wsRef.current.close();
        wsRef.current = null;
      }
    };
  }, []);

  /* ============================================================
     Add Line to Terminal
     ============================================================ */
  const addLine = (type: TerminalLine["type"], content: string) => {
    setLines((prev) => {
      const newLines = [...prev, { id: lineId.current++, type, content, timestamp: Date.now() }];
      // Keep only last 500 lines
      return newLines.slice(-500);
    });
    // Auto-scroll
    setTimeout(() => {
      terminalRef.current?.scrollTo({ top: terminalRef.current.scrollHeight, behavior: "smooth" });
    }, 0);
  };

  /* ============================================================
     Send Command
     ============================================================ */
  const sendCommand = async (cmd: string) => {
    if (!cmd.trim()) return;
    
    addLine("prompt", `$ ${cmd}`);
    setHistory((prev) => [...prev.slice(-49), cmd]);
    setHistoryIndex(-1);
    setInput("");

    try {
      // Send command via WebSocket
      if (wsRef.current?.readyState === WebSocket.OPEN) {
        wsRef.current.send(JSON.stringify({ type: "command", command: cmd }));
      } else {
        // Fallback to REST API
        const resp = await fetch("/api/terminal/command", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ command: cmd }),
        });
        const data = await resp.json();
        if (data.output) {
          data.output.split("\n").forEach((line: string) => {
            if (line.trim()) addLine("output", line);
          });
        }
      }
    } catch (err) {
      addLine("error", `Command failed: ${err}`);
    }
  };

  /* ============================================================
     Handle Input Key Events
     ============================================================ */
  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      sendCommand(input);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (history.length > 0 && historyIndex < history.length - 1) {
        const newIndex = historyIndex + 1;
        setHistoryIndex(newIndex);
        setInput(history[history.length - 1 - newIndex]);
      }
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      if (historyIndex > 0) {
        const newIndex = historyIndex - 1;
        setHistoryIndex(newIndex);
        setInput(history[history.length - 1 - newIndex]);
      } else if (historyIndex === 0) {
        setHistoryIndex(-1);
        setInput("");
      }
    } else if (e.key === "Tab") {
      e.preventDefault();
      // Simple tab completion for common commands
      const commands = ["help", "status", "start", "stop", "shadow", "paper", "live", "emergency", "logs", "pools", "opportunities", "executions", "config", "pnl", "providers", "clear", "exit"];
      const matches = commands.filter((c) => c.startsWith(input.toLowerCase()));
      if (matches.length === 1) {
        setInput(matches[0]);
      }
    }
  };

  /* ============================================================
     Clear Terminal
     ============================================================ */
  const clearTerminal = () => {
    setLines([]);
    addLine("system", "Terminal cleared");
  };

  /* ============================================================
     Render
     ============================================================ */
  return (
    <div className="terminal" style={{ 
      display: "flex", 
      flexDirection: "column", 
      height: "100%", 
      minHeight: "400px",
      background: "#000",
      border: "1px solid var(--border)",
      borderRadius: "10px",
      overflow: "hidden"
    }}>
      {/* Terminal Header */}
      <div className="terminal-header" style={{ 
        display: "flex", 
        alignItems: "center", 
        gap: "8px", 
        padding: "10px 14px", 
        background: "#0a0a0a", 
        borderBottom: "1px solid var(--border)" 
      }}>
        <div className="terminal-dots" style={{ display: "flex", gap: "6px" }}>
          <div className="terminal-dot red" style={{ width: "10px", height: "10px", borderRadius: "50%", background: "#ff5f57" }} />
          <div className="terminal-dot yellow" style={{ width: "10px", height: "10px", borderRadius: "50%", background: "#ffbd2e" }} />
          <div className="terminal-dot green" style={{ width: "10px", height: "10px", borderRadius: "50%", background: "#28ca42" }} />
        </div>
        <div className="terminal-title" style={{ flex: 1, fontSize: "11px", color: "#888", textAlign: "center" }}>
          bot-terminal@server
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
          <span className={`status-dot ${connected ? "connected" : "disconnected"}`} style={{ 
            width: "8px", 
            height: "8px", 
            borderRadius: "50%", 
            background: connected ? "#28ca42" : "#ff5f57",
            boxShadow: connected ? "0 0 8px #28ca42" : "0 0 8px #ff5f57"
          }} />
          <span style={{ fontSize: "10px", color: connected ? "#28ca42" : "#ff5f57", fontWeight: 600 }}>
            {connected ? "CONNECTED" : "DISCONNECTED"}
          </span>
          <button 
            onClick={clearTerminal}
            style={{
              background: "transparent",
              border: "1px solid #333",
              color: "#888",
              padding: "4px 8px",
              borderRadius: "4px",
              fontSize: "10px",
              cursor: "pointer",
              transition: "all 0.2s"
            }}
            onMouseOver={(e) => { e.currentTarget.style.borderColor = "#555"; e.currentTarget.style.color = "#fff"; }}
            onMouseOut={(e) => { e.currentTarget.style.borderColor = "#333"; e.currentTarget.style.color = "#888"; }}
          >
            Clear
          </button>
        </div>
      </div>

      {/* Terminal Body */}
      <div 
        className="terminal-body" 
        ref={terminalRef}
        style={{ 
          flex: 1, 
          padding: "14px", 
          overflowY: "auto", 
          fontFamily: '"JetBrains Mono", "Fira Code", monospace', 
          fontSize: "12px", 
          lineHeight: "1.6",
          color: "#d4d4d4"
        }}
      >
        {lines.map((line) => (
          <div key={line.id} className="terminal-line" style={{ 
            display: "flex", 
            gap: "10px", 
            marginBottom: "2px",
            whiteSpace: "pre-wrap",
            wordBreak: "break-word"
          }}>
            <span className={`terminal-${line.type}`} style={{ 
              color: 
                line.type === "prompt" ? "#00ff88" :
                line.type === "error" ? "#ff4d6a" :
                line.type === "warn" ? "#ffb800" :
                line.type === "success" ? "#00ff88" :
                line.type === "info" ? "#00d4ff" :
                line.type === "system" ? "#b877ff" :
                "#ccc"
            }}>
              {line.type === "prompt" ? "$" : line.type === "system" ? "●" : "▸"}
            </span>
            <span style={{ flex: 1, color: 
              line.type === "error" ? "#ff4d6a" :
              line.type === "warn" ? "#ffb800" :
              line.type === "success" ? "#00ff88" :
              line.type === "info" ? "#00d4ff" :
              line.type === "system" ? "#b877ff" :
              "#d4d4d4"
            }}>
              {line.content}
            </span>
          </div>
        ))}
      </div>

      {/* Terminal Input */}
      <div style={{ 
        display: "flex", 
        alignItems: "center", 
        gap: "10px", 
        padding: "10px 14px", 
        background: "#0a0a0a", 
        borderTop: "1px solid #222" 
      }}>
        <span style={{ color: "#00ff88", fontFamily: '"JetBrains Mono", monospace', fontSize: "12px" }}>
          root@arb-bot:~#
        </span>
        <input
          ref={inputRef}
          type="text"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          style={{
            flex: 1,
            background: "transparent",
            border: "none",
            color: "#d4d4d4",
            fontFamily: '"JetBrains Mono", monospace',
            fontSize: "12px",
            outline: "none",
            caretColor: "#00ff88"
          }}
          placeholder="Type command... (Tab for completion, ↑/↓ for history)"
          autoFocus
        />
      </div>
    </div>
  );
}

/* ============================================================
   Terminal Commands Reference (for help)
   ============================================================ */
export const TERMINAL_COMMANDS = [
  { cmd: "help", desc: "Show available commands" },
  { cmd: "status", desc: "Show bot status and mode" },
  { cmd: "start", desc: "Start execution" },
  { cmd: "stop", desc: "Stop execution" },
  { cmd: "shadow", desc: "Switch to shadow mode" },
  { cmd: "paper", desc: "Switch to paper mode" },
  { cmd: "live", desc: "Switch to live mode (requires CONFIRM-LIVE)" },
  { cmd: "emergency", desc: "Engage emergency stop" },
  { cmd: "clear-emergency", desc: "Clear emergency stop latch" },
  { cmd: "logs [level]", desc: "Show recent logs (info/warn/error)" },
  { cmd: "pools", desc: "List tracked pools" },
  { cmd: "opportunities", desc: "Show live opportunities" },
  { cmd: "executions", desc: "Show recent executions" },
  { cmd: "config", desc: "Show current configuration" },
  { cmd: "pnl", desc: "Show profit/loss summary" },
  { cmd: "providers", desc: "Show provider status" },
  { cmd: "clear", desc: "Clear terminal" },
  { cmd: "exit", desc: "Disconnect terminal" },
];