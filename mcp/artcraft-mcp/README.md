# artcraft-mcp

A local **stdio MCP server** that lets an AI agent drive the ArtCraft desktop app
(images, video, task status, downloads) through ArtCraft's opt-in local bridge.

It talks only to `http://127.0.0.1:<port>` using the port and bearer token that
ArtCraft writes to `~/Artcraft/state/mcp-bridge.json` (0600) when built with
`--features mcp-bridge` and launched with `ARTCRAFT_MCP_BRIDGE=1`. It never sees
your ArtCraft login/cookies. See `../../MAC-SETUP.md` for build + setup.

## Tools

| Tool | Spends credits? | What it does |
|---|---|---|
| `artcraft_status` | no | version, logged-in, credit balance, per-job cap |
| `list_models` | no | image or video models |
| `estimate_cost` | no | cost estimate + whether it fits the cap |
| `generate_image` | **yes** | requires `confirm: true`; app estimates first and refuses over-cap jobs |
| `generate_video` | **yes** | same; default cap blocks most videos until you raise it |
| `get_tasks` / `get_task` | no | the app's task queue (same tasks shown in the UI) |
| `wait_for_task` | no | polls until done / failed / timeout |
| `download_media` | no | saves into `~/Artcraft/downloads/mcp/`, returns path |

Typical flow: `estimate_cost` → `generate_image {confirm:true}` → `wait_for_task {id: job_token}`
→ `download_media {media_token: completed_item.primary_media_file.token}`.

Extra model fields (aspect ratio, resolution, duration, `image_media_token` for
image-to-video, etc.) go in `options` and are passed through to ArtCraft's own
request format. `max_credits` may lower the cap for one call; it can never raise it.

## Develop

```bash
npm install
npm run build
npm test   # mock-bridge tests; no ArtCraft, no credits
```

Run: `node dist/index.js` (stdio).
