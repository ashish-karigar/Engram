import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { opBrief, opExpand, opNote, opSearch, type OpContext, type OpResult } from "./ops.ts";

const memoryTypes = z.enum(["decision", "fact", "preference", "rejected_option", "dead_end", "episode", "task"]);

const asContent = (r: OpResult) => ({ content: [{ type: "text" as const, text: r.text }] });

/** A deliberately small tool surface: every tool definition costs the agent context tokens. */
export function createMcpServer(ctx: OpContext): McpServer {
  const server = new McpServer({ name: "engram", version: "0.1.0" });

  server.registerTool(
    "brief",
    {
      description:
        "Call at the start of a task and after context compaction. Returns the project map, key decisions and reasons, and recent agent activity. Pass `task` to focus it.",
      inputSchema: { task: z.string().optional(), budget: z.number().int().positive().optional() },
    },
    async (args) => asContent(opBrief(ctx, args)),
  );

  server.registerTool(
    "search",
    {
      description:
        "Ask project memory a question (e.g. 'why is auth session based', 'what depends on the lidar filter'). Returns ranked memories with ids you can pass to `expand`.",
      inputSchema: { query: z.string(), budget: z.number().int().positive().optional(), asOf: z.string().optional() },
    },
    async (args) => asContent(opSearch(ctx, args)),
  );

  server.registerTool(
    "expand",
    {
      description: "Open a memory or code node by id: reasons, version history, evidence, and connected code and memories.",
      inputSchema: { id: z.string(), depth: z.number().int().min(1).max(3).optional() },
    },
    async (args) => asContent(opExpand(ctx, args)),
  );

  server.registerTool(
    "note",
    {
      description:
        "Record a decision, its reason, a rejected option, or a dead end as you work, so other agents and future sessions know it. Name the files involved.",
      inputSchema: {
        title: z.string(),
        claim: z.string(),
        type: memoryTypes.optional(),
        rationale: z.string().optional(),
        files: z.array(z.string()).optional(),
      },
    },
    async (args) => asContent(opNote(ctx, args)),
  );

  return server;
}
