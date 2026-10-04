/**
 * A tool's short display name: "Bash" stays "Bash", an MCP tool such as
 * `mcp__hermes-bus__send_message` reads "Send message". Raw `mcp__…` ids
 * never reach copy the owner reads (ux-glossary §3).
 */
export function toolDisplayName(tool: string): string {
  const mcp = /^mcp__.+?__(.+)$/.exec(tool);
  if (mcp === null) {
    return tool;
  }
  const words = (mcp[1] ?? tool).replaceAll("_", " ").trim();
  return words.length === 0 ? tool : words.charAt(0).toUpperCase() + words.slice(1);
}
