// Permission rules for tools an employee was blocked on. Mirrors `claude::mcp_server_rule` in the backend.

const MCP_TOOL = /^mcp__(.+?)__(.+)$/;

/** The rule that allows every tool of an MCP server: `mcp__claude-in-chrome__*` for
 *  `mcp__claude-in-chrome__javascript_tool`. `null` for a tool that is not an MCP tool. */
export function mcpServerRule(tool: string): string | null {
  const m = MCP_TOOL.exec(tool);
  return m ? `mcp__${m[1]}__*` : null;
}

/** A readable name for the server behind an MCP tool: "Microsoft 365" for `mcp__claude_ai_Microsoft_365__x`. */
export function mcpServerName(tool: string): string | null {
  const m = MCP_TOOL.exec(tool);
  if (!m) return null;
  const name = m[1].replace(/^claude_ai_/, "").replace(/[_-]+/g, " ").trim();
  return name === "claude in chrome" ? "Chrome" : name || m[1];
}

/** The tool name of an ask for a folder alone. Mirrors `access::FOLDER_TOOL` in the backend. */
export const FOLDER_TOOL = "folder";

/** The tool as a person would say it: "javascript_tool" for `mcp__claude-in-chrome__javascript_tool`. */
export function toolName(tool: string): string {
  if (tool === FOLDER_TOOL) return "a folder";
  const m = MCP_TOOL.exec(tool);
  return m ? m[2] : tool;
}
