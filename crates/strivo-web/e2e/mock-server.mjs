// Deterministic mock backend for the StriVo webui E2E suite (W7).
// Serves the real SPA assets and stubs /api/v1 + /events so the browser
// tests run without a live daemon or platform auth.
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ASSETS = join(__dirname, "..", "assets");
const PORT = process.env.PORT || 8199;

const CHANNELS = [
  {
    id: "UClive0000000000000000aa",
    platform: "YouTube",
    name: "livechan",
    display_name: "Live Channel",
    is_live: true,
    stream_title: "Live test stream",
    game_or_category: "Just Chatting",
    viewer_count: 1234,
    started_at: new Date().toISOString(),
    thumbnail_url: null,
    auto_record: false,
  },
  {
    id: "twitch:offlinechan",
    platform: "Twitch",
    name: "offlinechan",
    display_name: "Offline Channel",
    is_live: false,
    stream_title: null,
    game_or_category: null,
    viewer_count: null,
    started_at: null,
    thumbnail_url: null,
    auto_record: true,
  },
];

const RECORDINGS = {
  recordings: [
    {
      id: "11111111-1111-1111-1111-111111111111",
      channel_name: "Alpha",
      stream_title: "Zebra stream",
      state: "Finished",
      started_at: "2026-05-20T10:00:00Z",
      bytes_written: 5_000_000_000,
    },
    {
      id: "22222222-2222-2222-2222-222222222222",
      channel_name: "Bravo",
      stream_title: "Apple stream",
      state: "Recording",
      started_at: "2026-05-26T09:00:00Z",
      bytes_written: 1_000_000_000,
    },
    {
      id: "33333333-3333-3333-3333-333333333333",
      channel_name: "Charlie",
      stream_title: "Mango stream",
      state: "Failed",
      started_at: "2026-05-22T12:00:00Z",
      bytes_written: 200_000_000,
    },
  ],
};

const CONTENT_TYPES = { ".js": "text/javascript", ".css": "text/css", ".html": "text/html" };

function json(res, code, body) {
  res.writeHead(code, { "Content-Type": "application/json" });
  res.end(JSON.stringify(body));
}

// Open SSE connections, so POSTs that resolve asynchronously (vods,
// playlists) can push their answer like the real daemon does.
const sseClients = new Set();
function broadcast(eventObj) {
  const frame = `data: ${JSON.stringify(eventObj)}\n\n`;
  for (const c of sseClients) c.write(frame);
}

const SCHEDULE = [
  {
    channel: "Alpha",
    cron: "0 20 * * *",
    duration: "4h",
    next_fire: new Date(Date.now() + 3600_000).toISOString(),
  },
];

const server = createServer(async (req, res) => {
  const url = new URL(req.url, `http://localhost:${PORT}`);
  const path = url.pathname;

  if (path === "/events") {
    res.writeHead(200, {
      "Content-Type": "text/event-stream",
      "Cache-Control": "no-cache",
      Connection: "keep-alive",
    });
    res.write(": connected\n\n");
    sseClients.add(res);
    const ka = setInterval(() => res.write(": keepalive\n\n"), 1000);
    req.on("close", () => {
      clearInterval(ka);
      sseClients.delete(res);
    });
    return;
  }

  // API surface.
  if (path.startsWith("/api/v1/")) {
    const p = path.slice("/api/v1".length);
    if (p === "/health") return json(res, 200, { status: "ok" });
    if (p === "/backup") return json(res, 201, { name: "2026-05-26T00-00-00Z", files: ["config.toml", "jobs.db"], bytes: 1234 });
    if (p === "/backups")
      return json(res, 200, {
        backups: [{ name: "2026-05-26T00-00-00Z", bytes: 1234, files: ["config.toml", "jobs.db"] }],
      });
    if (p.startsWith("/logs"))
      return json(res, 200, {
        file: "strivo.2026-05-26.log",
        level: "info",
        lines: [
          "2026-05-26T22:00:00Z  INFO strivo_core::daemon: StriVo daemon starting",
          "2026-05-26T22:00:01Z  WARN strivo_core::monitor: example warning",
        ],
      });
    if (p === "/health/checks")
      return json(res, 200, {
        status: "ok",
        checks: [
          { domain: "Network", name: "Daemon IPC", severity: "ok", message: "Daemon reachable.", fix: "" },
          { domain: "Storage", name: "Disk space", severity: "ok", message: "3 TB free.", fix: "" },
        ],
      });
    if (p === "/auth/login") return json(res, 200, { status: "ok" });
    if (p === "/auth/logout") return json(res, 200, { status: "ok" });
    if (p === "/channels") return json(res, 200, { channels: CHANNELS });
    if (p === "/patreon")
      return json(res, 200, {
        creators: [
          {
            id: "camp123",
            platform: "Patreon",
            name: "creatorslug",
            display_name: "Cool Creator",
            is_live: false,
            stream_title: "Premium Tier",
            game_or_category: null,
            viewer_count: null,
            started_at: null,
            thumbnail_url: null,
            auto_record: false,
          },
        ],
        posts: [
          {
            id: "post1",
            campaign_id: "camp123",
            title: "Behind the scenes",
            url: "https://patreon.com/posts/post1",
            published_at: "2026-05-25T00:00:00Z",
            embed_url: "https://example.com/embed/post1",
          },
        ],
      });
    if (p === "/recordings" && req.method === "GET") return json(res, 200, RECORDINGS);
    if (p === "/storage")
      return json(res, 200, {
        bytes_used_by_recordings: 6_200_000_000,
        filesystem_avail_bytes: 900_000_000_000,
      });
    if (p === "/gantt") return json(res, 200, { items: [] });
    if (p === "/schedule") return json(res, 200, { schedule: SCHEDULE });
    if (p === "/settings") return json(res, 200, {});

    // Channel VODs request → answer asynchronously over SSE, like the daemon.
    const vodsMatch = p.match(/^\/channels\/([^/]+)\/vods$/);
    if (vodsMatch && req.method === "POST") {
      const channelId = decodeURIComponent(vodsMatch[1]);
      setTimeout(() => {
        broadcast({
          ChannelVods: {
            channel_id: channelId,
            vods: [
              {
                id: "stream1",
                platform: "YouTube",
                channel_id: channelId,
                title: "Yesterday's livestream",
                published_at: "2026-05-25T20:00:00Z",
                url: "https://youtu.be/stream1",
                kind: "LiveBroadcast",
              },
              {
                id: "upload1",
                platform: "YouTube",
                channel_id: channelId,
                title: "How I edit my videos",
                published_at: "2026-05-24T12:00:00Z",
                url: "https://youtu.be/upload1",
                kind: "Upload",
              },
            ],
          },
        });
      }, 50);
      return json(res, 202, { status: "requested" });
    }

    // Mutations / verb dispatch — accept and echo queued.
    if (req.method === "POST" || req.method === "PUT" || req.method === "DELETE") {
      return json(res, 202, { status: "queued", path: p });
    }
    return json(res, 200, {});
  }

  // Static assets + SPA shell.
  let file;
  if (path === "/" || path === "/app") file = join(ASSETS, "spa.html");
  else if (path.startsWith("/assets/")) file = join(ASSETS, path.slice("/assets/".length));
  if (file) {
    try {
      const buf = await readFile(file);
      const ext = file.slice(file.lastIndexOf("."));
      res.writeHead(200, { "Content-Type": CONTENT_TYPES[ext] || "application/octet-stream" });
      res.end(buf);
      return;
    } catch {
      res.writeHead(404);
      res.end("not found");
      return;
    }
  }
  res.writeHead(404);
  res.end("not found");
});

server.listen(PORT, () => console.log(`mock server on http://localhost:${PORT}`));
