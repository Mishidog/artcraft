import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { BridgeClient } from "./bridge.js";

const text = (v: unknown) => ({ content: [{ type: "text" as const, text: typeof v === "string" ? v : JSON.stringify(v, null, 2) }] });
const fail = (m: string) => ({ isError: true, content: [{ type: "text" as const, text: m }] });

const genShape = {
  confirm: z.literal(true).describe("Must be true. Confirms you intend to SPEND ArtCraft credits."),
  model: z.string().describe("Model id from list_models."),
  prompt: z.string().describe("Text prompt."),
  max_credits: z.number().int().nonnegative().optional().describe("Optional lower per-job cap. Can only lower the app's cap, never raise it."),
  options: z.record(z.any()).optional().describe("Extra model fields passed through (e.g. aspect_ratio, resolution, duration_seconds, image_media_token for image-to-video)."),
};

const TERMINAL = new Set(["complete_success", "complete_failure", "dead", "cancelled_by_user", "cancelled_by_provider", "cancelled_by_us"]);

export function isTerminal(status: unknown): boolean {
  return typeof status === "string" && (TERMINAL.has(status) || status.startsWith("complete") || status.startsWith("cancel") || status === "failed");
}

export function registerTools(server: McpServer, client: BridgeClient, opts: { pollMs?: number } = {}) {
  const call = async (fn: () => Promise<unknown>) => { try { return text(await fn()); } catch (e) { return fail((e as Error).message); } };

  server.tool("artcraft_status", "Show ArtCraft app version, whether it is logged in, the credit balance, and the per-job credit cap. Free; spends nothing.", {},
    async () => call(() => client.request("GET", "/v1/status")));

  server.tool("list_models", "List ArtCraft generation models for a modality. Free.", { modality: z.enum(["image", "video"]) },
    async ({ modality }) => call(() => client.request("GET", `/v1/models?modality=${modality}`)));

  server.tool("estimate_cost", "Estimate the credit cost of an image or video generation without spending credits, and report whether it fits the per-job cap.",
    { modality: z.enum(["image", "video"]), model: z.string(), prompt: z.string().optional(), max_credits: z.number().int().nonnegative().optional(), options: z.record(z.any()).optional() },
    async ({ modality, model, prompt, max_credits, options }) => call(() => client.request("POST", "/v1/estimate", { ...(options ?? {}), modality, model, prompt, max_credits })));

  const gen = (kind: "image" | "video") => async (a: { confirm: true; model: string; prompt: string; max_credits?: number; options?: Record<string, unknown> }) => {
    if (a.confirm !== true) return fail("Refused: set confirm: true to spend credits.");
    return call(() => client.request("POST", `/v1/generate/${kind}`, { ...(a.options ?? {}), model: a.model, prompt: a.prompt, max_credits: a.max_credits }));
  };

  server.tool("generate_image", "SPENDS ArtCraft CREDITS. Generates an image via the signed-in ArtCraft account. The app first estimates cost and refuses if it exceeds the per-job cap. Requires confirm: true. Returns job tokens; use wait_for_task then download_media.", genShape, gen("image"));
  server.tool("generate_video", "SPENDS ArtCraft CREDITS (video is usually expensive). Generates a video via the signed-in ArtCraft account. The app first estimates cost and refuses if it exceeds the per-job cap (default cap blocks most videos until raised by the user). Requires confirm: true. Returns job tokens.", genShape, gen("video"));

  server.tool("get_tasks", "List ArtCraft's task queue (the same tasks shown in the app UI). Free.", {},
    async () => call(() => client.request("GET", "/v1/tasks")));

  server.tool("get_task", "Get one task by task id or by the job token returned from generate_*. Free.", { id: z.string() },
    async ({ id }) => call(() => client.request("GET", `/v1/tasks/${encodeURIComponent(id)}`)));

  server.tool("wait_for_task", "Poll a task until it completes or fails, or until timeout_seconds passes. Free. Returns the task, including completed_item.primary_media_file.token for download_media.",
    { id: z.string(), timeout_seconds: z.number().int().min(1).max(1800).default(300) },
    async ({ id, timeout_seconds }) => {
      const deadline = Date.now() + timeout_seconds * 1000;
      let last: unknown = null;
      while (Date.now() < deadline) {
        try {
          last = await client.request("GET", `/v1/tasks/${encodeURIComponent(id)}`);
          if (isTerminal((last as { task_status?: unknown })?.task_status)) return text(last);
        } catch (e) {
          if (!/404/.test((e as Error).message)) return fail((e as Error).message);
        }
        await new Promise((r) => setTimeout(r, opts.pollMs ?? 3000));
      }
      return fail(`Timed out after ${timeout_seconds}s. Last state: ${JSON.stringify(last)}`);
    });

  server.tool("download_media", "Download a finished media file into ~/Artcraft/downloads/mcp/ and return its local path. Free.", { media_token: z.string().regex(/^[A-Za-z0-9_-]+$/) },
    async ({ media_token }) => call(() => client.request("POST", `/v1/download/${media_token}`)));
}
