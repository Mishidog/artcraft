import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";

export interface BridgeConnection { port: number; token: string }

export function defaultConnectionPath(): string {
  return process.env.ARTCRAFT_MCP_BRIDGE_FILE ?? join(homedir(), "Artcraft", "state", "mcp-bridge.json");
}

/** Re-read on every call so an app restart (new port/token) is picked up. */
export async function readConnection(path = defaultConnectionPath()): Promise<BridgeConnection> {
  let text: string;
  try {
    text = await readFile(path, "utf8");
  } catch {
    throw new Error(`ArtCraft bridge not found at ${path}. Launch ArtCraft built with --features mcp-bridge and ARTCRAFT_MCP_BRIDGE=1.`);
  }
  const data = JSON.parse(text);
  if (typeof data.port !== "number" || typeof data.token !== "string") throw new Error("mcp-bridge.json is malformed");
  return { port: data.port, token: data.token };
}

export class BridgeClient {
  constructor(private connectionPath = defaultConnectionPath(), private host = "127.0.0.1") {}

  async request(method: "GET" | "POST", path: string, body?: unknown): Promise<unknown> {
    const { port, token } = await readConnection(this.connectionPath);
    let res: Response;
    try {
      res = await fetch(`http://${this.host}:${port}${path}`, {
        method,
        headers: { Authorization: `Bearer ${token}`, ...(body !== undefined ? { "Content-Type": "application/json" } : {}) },
        body: body !== undefined ? JSON.stringify(body) : undefined,
      });
    } catch (e) {
      throw new Error(`Could not reach ArtCraft on port ${port}; is the app running? (${(e as Error).message})`);
    }
    const text = await res.text();
    let json: unknown = text;
    try { json = JSON.parse(text); } catch { /* keep text */ }
    if (!res.ok) throw new Error(`ArtCraft bridge ${res.status}: ${typeof json === "string" ? json : JSON.stringify(json)}`);
    return json;
  }
}
