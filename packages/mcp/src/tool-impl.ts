/**
 * Local mirror of @cut/agent's ToolImpl contract so MCP tools can be merged
 * straight into an AgentSession ToolMap without importing the agent package.
 */
export interface ToolImpl<I = unknown, O = unknown> {
  description?: string;
  inputSchema?: I;
  inputJsonSchema?: Record<string, unknown>;
  execute(input: I): Promise<O>;
}
