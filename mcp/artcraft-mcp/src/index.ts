#!/usr/bin/env node
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { BridgeClient } from "./bridge.js";
import { registerTools } from "./tools.js";

const server = new McpServer({ name: "artcraft", version: "0.1.0" });
registerTools(server, new BridgeClient());
await server.connect(new StdioServerTransport());
