import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { BridgeClient } from "../src/bridge.js";
import { registerTools, isTerminal } from "../src/tools.js";

const TOKEN = "t".repeat(64);

async function setup() {
  const calls: { method: string; url: string; auth?: string; body?: any }[] = [];
  let polls = 0;
  const http = createServer((req, res) => {
    let raw = "";
    req.on("data", (c) => (raw += c));
    req.on("end", () => {
      calls.push({ method: req.method!, url: req.url!, auth: req.headers.authorization, body: raw ? JSON.parse(raw) : undefined });
      if (req.headers.authorization !== `Bearer ${TOKEN}`) { res.writeHead(401); return res.end('{"error":"bad token"}'); }
      const send = (c: number, v: unknown) => { res.writeHead(c, { "content-type": "application/json" }); res.end(JSON.stringify(v)); };
      if (req.url === "/v1/status") return send(200, { app_version: "0.42.0", logged_in: true, max_credits_per_job: 10 });
      if (req.url === "/v1/generate/image") return send(200, { job_tokens: ["job_1"], estimated_credits: 4, cap: 10 });
      if (req.url === "/v1/generate/video") return send(402, { error: "estimated cost exceeds per-job credit cap", estimated_credits: 80, cap: 10 });
      if (req.url === "/v1/tasks/job_1") { polls++; return send(200, { id: "task_1", task_status: polls < 2 ? "started" : "complete_success", completed_item: { primary_media_file: { token: "m_1" } } }); }
      if (req.url === "/v1/download/m_1") return send(200, { path: "/Users/jim/Artcraft/downloads/mcp/m_1.png" });
      send(404, { error: "nope" });
    });
  });
  await new Promise<void>((r) => http.listen(0, "127.0.0.1", r));
  const port = (http.address() as any).port;
  const dir = mkdtempSync(join(tmpdir(), "acmcp-"));
  const file = join(dir, "mcp-bridge.json");
  writeFileSync(file, JSON.stringify({ port, token: TOKEN }));
  const server = new McpServer({ name: "artcraft", version: "test" });
  registerTools(server, new BridgeClient(file), { pollMs: 10 });
  const [a, b] = InMemoryTransport.createLinkedPair();
  await server.connect(a);
  const client = new Client({ name: "t", version: "1" });
  await client.connect(b);
  return { client, calls, close: () => { http.close(); client.close(); } };
}

const body = (r: any) => r.content[0].text as string;

test("lists all tools", async () => {
  const { client, close } = await setup();
  const names = (await client.listTools()).tools.map((t) => t.name).sort();
  assert.deepEqual(names, ["artcraft_status", "download_media", "estimate_cost", "generate_image", "generate_video", "get_task", "get_tasks", "list_models", "wait_for_task"]);
  close();
});

test("status sends bearer token", async () => {
  const { client, calls, close } = await setup();
  const r: any = await client.callTool({ name: "artcraft_status", arguments: {} });
  assert.match(body(r), /logged_in/);
  assert.equal(calls[0].auth, `Bearer ${TOKEN}`);
  close();
});

test("generate requires confirm: true and never reaches bridge without it", async () => {
  const { client, calls, close } = await setup();
  const r: any = await client.callTool({ name: "generate_image", arguments: { model: "m", prompt: "p" } });
  assert.equal(r.isError, true);
  const r2: any = await client.callTool({ name: "generate_image", arguments: { model: "m", prompt: "p", confirm: false } });
  assert.equal(r2.isError, true);
  assert.equal(calls.filter((c) => c.url.startsWith("/v1/generate")).length, 0);
  close();
});

test("generate passes cap and surfaces over-cap refusal", async () => {
  const { client, calls, close } = await setup();
  const ok: any = await client.callTool({ name: "generate_image", arguments: { model: "m", prompt: "a cat", confirm: true, max_credits: 5 } });
  assert.match(body(ok), /job_1/);
  assert.equal(calls.at(-1)!.body.max_credits, 5);
  assert.equal(calls.at(-1)!.body.confirm, undefined);
  const no: any = await client.callTool({ name: "generate_video", arguments: { model: "v", prompt: "x", confirm: true } });
  assert.equal(no.isError, true);
  assert.match(body(no), /402/);
  close();
});

test("wait_for_task polls until terminal then download", async () => {
  const { client, close } = await setup();
  const r: any = await client.callTool({ name: "wait_for_task", arguments: { id: "job_1", timeout_seconds: 5 } });
  assert.match(body(r), /complete_success/);
  const d: any = await client.callTool({ name: "download_media", arguments: { media_token: "m_1" } });
  assert.match(body(d), /downloads\/mcp/);
  close();
});

test("terminal status detection", () => {
  assert.ok(isTerminal("complete_success"));
  assert.ok(isTerminal("complete_failure"));
  assert.ok(!isTerminal("pending"));
  assert.ok(!isTerminal(undefined));
});
