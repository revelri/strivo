// StriVo SPA — vanilla JS, hash routing, *arr-inspired chrome. (W4 MVP.)
//
// This is the minimum-viable shippable webui that uses the W1+W2+W3
// backend. SvelteKit conversion is the W4 phase 2 follow-up; this
// file deliberately stays small + dependency-free.

const API = {
  async _fetch(path, opts = {}) {
    const headers = { Accept: "application/json", ...(opts.headers || {}) };
    if (opts.body && typeof opts.body !== "string") {
      headers["Content-Type"] = "application/json";
      opts.body = JSON.stringify(opts.body);
    }
    const res = await fetch(`/api/v1${path}`, {
      credentials: "same-origin",
      ...opts,
      headers,
    });
    if (res.status === 401) {
      route("login");
      throw new Error("unauthorized");
    }
    if (!res.ok) {
      const text = await res.text();
      throw new Error(`HTTP ${res.status}: ${text}`);
    }
    return res.headers.get("content-type")?.includes("json")
      ? res.json()
      : res.text();
  },
  channels: () => API._fetch("/channels"),
  recordings: () => API._fetch("/recordings"),
  startRecording: (body) =>
    API._fetch("/recordings", { method: "POST", body }),
  stopRecording: (id) =>
    API._fetch(`/recordings/${id}`, { method: "DELETE" }),
  toggleAutoRecord: (channelKey, enabled) =>
    API._fetch(`/channels/${encodeURIComponent(channelKey)}/auto_record`, {
      method: "PUT",
      body: { enabled },
    }),
  pollNow: () => API._fetch("/poll_now", { method: "POST" }),
  health: () => API._fetch("/health"),
  storage: () => API._fetch("/storage"),
  gantt: () => API._fetch("/gantt"),
  pluginRpc: (plugin, verb, body) =>
    API._fetch(`/plugins/${encodeURIComponent(plugin)}/${encodeURIComponent(verb)}`, {
      method: "POST",
      body,
    }),
  bulkDownload: (channelId, body) =>
    API._fetch(`/channels/${encodeURIComponent(channelId)}/bulk`, {
      method: "POST",
      body,
    }),
  requestPlaylists: (channelId) =>
    API._fetch(`/channels/${encodeURIComponent(channelId)}/playlists`, {
      method: "POST",
    }),
  login: (apiKey) =>
    API._fetch("/auth/login", { method: "POST", body: { api_key: apiKey } }),
  logout: () => API._fetch("/auth/logout", { method: "POST" }),
};

// ── SSE event stream ─────────────────────────────────────────────────
const events = {
  source: null,
  listeners: new Set(),
  start() {
    if (this.source) return;
    this.source = new EventSource("/events", { withCredentials: true });
    this.source.onmessage = (e) => {
      try {
        const data = JSON.parse(e.data);
        this.listeners.forEach((fn) => fn(data));
      } catch (_) {}
    };
    this.source.onerror = () => {
      // Auto-reconnect via the browser; if we're 401-ing the user is
      // probably logged out and a route('login') will reset us.
    };
  },
  on(fn) {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  },
};

// Activity event ring (most-recent-first, capped at 50).
const activityLog = [];
// #74 — per-channel bulk-download status, keyed by channel_id:
// { done, total, active }. Fed by the `bulk-progress` SSE event.
const bulkStatus = {};
function pushActivity(event) {
  const kind = Object.keys(event)[0] || "event";
  const summary = summarizeEvent(event);
  activityLog.unshift({
    kind,
    summary,
    at: new Date(),
  });
  if (activityLog.length > 50) activityLog.pop();
  renderActivityRail();
}
function summarizeEvent(event) {
  if (event.ChannelWentLive)
    return `${event.ChannelWentLive.display_name || event.ChannelWentLive.name} went LIVE`;
  if (event.ChannelWentOffline)
    return `${event.ChannelWentOffline.display_name || event.ChannelWentOffline.name} went offline`;
  if (event.RecordingStarted)
    return `Started: ${event.RecordingStarted.job.channel_name}`;
  if (event.RecordingFinished)
    return `Finished: ${event.RecordingFinished.final_state}`;
  if (event.RecordingProgress)
    return `Progress: ${(event.RecordingProgress.bytes_written / 1e6).toFixed(1)} MB`;
  if (event.ScheduleFired)
    return `Schedule fired: ${event.ScheduleFired.channel}`;
  if (event.Notification)
    return `${event.Notification.title}: ${event.Notification.body}`;
  if (event.PlatformAuthenticated)
    return `Authenticated: ${event.PlatformAuthenticated.kind}`;
  if (event.DeviceCodeRequired) return `Device-code prompt`;
  return JSON.stringify(event).slice(0, 80);
}

// ── Hash router ──────────────────────────────────────────────────────
const ROUTES = [
  "library",
  "recordings",
  "schedule",
  "pipelines",
  "plugins",
  "activity",
  "settings",
  "system",
  "login",
];

function currentRoute() {
  const hash = window.location.hash.replace(/^#\/?/, "") || "library";
  return ROUTES.includes(hash) ? hash : "library";
}

function route(name) {
  window.location.hash = `#/${name}`;
}

window.addEventListener("hashchange", render);

// ── Render ───────────────────────────────────────────────────────────
const root = document.getElementById("app");

async function render() {
  const r = currentRoute();
  // Probe auth — if /health returns 401-ish, we land on login.
  if (r !== "login") {
    try {
      await API.health();
    } catch (e) {
      // health is unauthenticated; this catch means real network/server
      // issue. Surface and continue.
      console.warn(e);
    }
    // The first real call that hits an auth check will redirect to
    // /login on 401 via the API._fetch path.
  }
  switch (r) {
    case "login":
      renderLogin();
      break;
    case "library":
      await renderLibrary();
      break;
    case "recordings":
      await renderRecordings();
      break;
    case "schedule":
      renderStub("Schedule", "Calendar view — webui parity follow-up.");
      break;
    case "pipelines":
      renderPipelines();
      break;
    case "plugins":
      renderPlugins();
      break;
    case "activity":
      renderActivityPage();
      break;
    case "settings":
      renderStub("Settings", "Settings page — webui parity follow-up.");
      break;
    case "system":
      renderStub("System", "Health checks + log files — webui parity follow-up.");
      break;
  }
}

function chrome(content) {
  return `
    <div class="chrome">
      <header class="topbar" role="banner">
        <span class="brand">StriVo</span>
        <span id="live-pill" class="live-pill" style="display: none"
              aria-label="Live recording count"></span>
        <span id="storage-pill" class="storage-pill" style="display: none"
              aria-label="Storage usage"></span>
        <span class="spacer"></span>
        <button id="activity-toggle" title="Toggle activity feed (a)"
                aria-label="Toggle activity rail">⌘ Activity</button>
        <button id="poll-now" title="Poke channel monitor (p)"
                aria-label="Trigger immediate channel poll">↻ Poll</button>
        <button id="logout" title="Logout"
                aria-label="Sign out">⏻</button>
      </header>
      <nav class="leftrail" aria-label="Main navigation">
        <a href="#/library" data-route="library" data-key="l">
          <span class="glyph" aria-hidden="true">▣</span> Library
        </a>
        <a href="#/recordings" data-route="recordings" data-key="r">
          <span class="glyph" aria-hidden="true">📁</span> Recordings
        </a>
        <a href="#/schedule" data-route="schedule" data-key="s">
          <span class="glyph" aria-hidden="true">📅</span> Schedule
        </a>
        <a href="#/pipelines" data-route="pipelines" data-key="d">
          <span class="glyph" aria-hidden="true">🔁</span> Pipelines
        </a>
        <a href="#/plugins" data-route="plugins" data-key="g">
          <span class="glyph" aria-hidden="true">🧩</span> Plugins
        </a>
        <a href="#/activity" data-route="activity" data-key="i">
          <span class="glyph" aria-hidden="true">⚡</span> Activity
        </a>
        <a href="#/settings" data-route="settings" data-key="c">
          <span class="glyph" aria-hidden="true">⚙</span> Settings
        </a>
        <a href="#/system" data-route="system" data-key="y">
          <span class="glyph" aria-hidden="true">🛠</span> System
        </a>
      </nav>
      <main class="content" id="content">${content}</main>
      <aside class="activity-rail" id="activity-rail">
        <h3>
          Activity
          <button class="close-btn" id="activity-close">×</button>
        </h3>
        <div id="activity-list"></div>
      </aside>
    </div>
  `;
}

function setupChromeHandlers() {
  const r = currentRoute();
  document.querySelectorAll(".leftrail a").forEach((a) => {
    a.classList.toggle("active", a.dataset.route === r);
  });
  document.getElementById("poll-now")?.addEventListener("click", async () => {
    try {
      await API.pollNow();
    } catch (e) {
      console.error(e);
    }
  });
  document.getElementById("logout")?.addEventListener("click", async () => {
    await API.logout().catch(() => {});
    route("login");
  });
  document
    .getElementById("activity-toggle")
    ?.addEventListener("click", () => {
      document.getElementById("activity-rail")?.classList.toggle("open");
      renderActivityRail();
    });
  document.getElementById("activity-close")?.addEventListener("click", () => {
    document.getElementById("activity-rail")?.classList.remove("open");
  });
  // W5 — refresh the topbar storage pill on every chrome mount.
  refreshStoragePill();
}

// Storage pill refresh — debounced to once per chrome render.
async function refreshStoragePill() {
  const pill = document.getElementById("storage-pill");
  if (!pill) return;
  try {
    const s = await API.storage();
    const used = s.bytes_used_by_recordings || 0;
    const avail = s.filesystem_avail_bytes || 0;
    if (avail > 0 || used > 0) {
      pill.textContent = `💾 ${formatBytes(used)} used · ${formatBytes(avail)} free`;
      pill.style.display = "";
    }
  } catch (_) {
    pill.style.display = "none";
  }
}

function renderActivityRail() {
  const list = document.getElementById("activity-list");
  if (!list) return;
  list.innerHTML = activityLog
    .map(
      (e) => `
    <div class="activity-event">
      <span class="kind">${escape(e.kind)}</span>
      <span class="timestamp">${e.at.toLocaleTimeString()}</span>
      <div class="summary">${escape(e.summary)}</div>
    </div>
  `,
    )
    .join("");
}

// ── Login ────────────────────────────────────────────────────────────
function renderLogin(errorMsg) {
  root.removeAttribute("aria-busy");
  root.innerHTML = `
    <div class="login-screen">
      <form class="login-card" id="login-form">
        <h1>StriVo</h1>
        <p class="subtitle">Sign in to the web console</p>
        <label for="api-key">API Key</label>
        <input type="password" id="api-key" autocomplete="current-password" autofocus />
        <button type="submit" class="primary">Sign in</button>
        ${errorMsg ? `<div class="error">${escape(errorMsg)}</div>` : ""}
        <div class="hint">
          API key lives in <code>~/.config/strivo/config.toml</code> under
          <code>[web]</code>. <br />
          Or run: <code>strivo config get web.api_key</code>
        </div>
      </form>
    </div>
  `;
  document.getElementById("login-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const key = document.getElementById("api-key").value.trim();
    if (!key) return;
    try {
      await API.login(key);
      route("library");
    } catch (err) {
      renderLogin("Invalid API key");
    }
  });
}

// ── Library (channels grid + LIVE NOW strip) ──────────────────────────
async function renderLibrary() {
  let channels = [];
  try {
    const data = await API.channels();
    channels = data.channels || [];
  } catch (e) {
    if (e.message.includes("unauthorized")) return;
    root.innerHTML = chrome(
      `<div class="empty"><div class="glyph">⚠</div>${escape(e.message)}</div>`,
    );
    setupChromeHandlers();
    return;
  }
  root.removeAttribute("aria-busy");

  const live = channels.filter((c) => c.is_live);
  const offline = channels.filter((c) => !c.is_live);
  updateLiveCount(live.length);

  const liveStrip = live.length
    ? `
    <div class="live-now">
      <h2><span class="rec-dot">●</span> LIVE NOW (${live.length})</h2>
      <div class="live-now-grid">
        ${live.map(channelCard).join("")}
      </div>
    </div>
  `
    : "";

  // W5 — 24h Gantt strip just below the LIVE NOW pane.
  let ganttHtml = "";
  try {
    const g = await API.gantt();
    ganttHtml = renderGantt(g.items || []);
  } catch (_) {
    /* Gantt is decorative; silent fail. */
  }

  root.innerHTML = chrome(`
    <h1 class="page-title">Library</h1>
    <p class="page-subtitle">${channels.length} channels monitored</p>
    ${liveStrip}
    ${ganttHtml}
    <div class="channel-grid">
      ${offline.map(channelCard).join("") ||
        '<div class="empty">No offline channels yet</div>'}
    </div>
  `);
  setupChromeHandlers();
  document.querySelectorAll("[data-action=record]").forEach((btn) => {
    btn.addEventListener("click", () => startRecordingFromCard(btn.dataset));
  });
  document.querySelectorAll("[data-action=auto-record]").forEach((btn) => {
    btn.addEventListener("click", () => toggleAutoRecord(btn.dataset));
  });
  document.querySelectorAll("[data-action=bulk]").forEach((btn) => {
    btn.addEventListener("click", () => toggleBulk(btn.dataset));
  });
  document.querySelectorAll("[data-action=bulk-playlist]").forEach((btn) => {
    btn.addEventListener("click", () => openPlaylistPicker(btn.dataset));
  });
}

// #74 — start/stop a per-channel bulk download.
async function toggleBulk(ds) {
  const active = ds.bulkActive === "true";
  try {
    await API.bulkDownload(ds.channelId, {
      channel_name: ds.channelName,
      platform: ds.platform,
      action: active ? "stop" : "start",
    });
    // Optimistic: flip local state; SSE bulk-progress will correct it.
    bulkStatus[ds.channelId] = active
      ? { done: 0, total: 0, active: false }
      : { done: 0, total: 0, active: true };
    if (currentRoute() === "library") render();
  } catch (e) {
    alert(`Bulk download failed: ${e.message}`);
  }
}

// #74 / #73 — request the channel's playlists; the picker modal opens
// when the `playlist-list` SSE event arrives.
let pendingPlaylistChannel = null;
async function openPlaylistPicker(ds) {
  pendingPlaylistChannel = { id: ds.channelId, name: ds.channelName };
  try {
    await API.requestPlaylists(ds.channelId);
    showPlaylistModal({ loading: true, name: ds.channelName, playlists: [] });
  } catch (e) {
    alert(`Couldn't load playlists: ${e.message}`);
  }
}

function showPlaylistModal(opts) {
  let modal = document.getElementById("playlist-modal");
  if (!modal) {
    modal = document.createElement("div");
    modal.id = "playlist-modal";
    modal.className = "kbd-help"; // reuse the centered-overlay styling
    document.body.appendChild(modal);
    modal.addEventListener("click", (e) => {
      if (e.target === modal) modal.classList.remove("open");
    });
  }
  const rows = opts.loading
    ? "<div>Loading playlists…</div>"
    : [
        `<div class="pl-row" data-pl=""><b>▣ Whole channel</b> (all uploads)</div>`,
        ...opts.playlists.map(
          (p) =>
            `<div class="pl-row" data-pl="${escape(p.id)}">≡ ${escape(p.title)}${
              p.item_count != null ? ` (${p.item_count})` : ""
            }</div>`,
        ),
      ].join("");
  modal.innerHTML = `
    <div class="card">
      <h2>Bulk download — ${escape(opts.name)}</h2>
      <div class="pl-list">${rows}</div>
    </div>`;
  modal.classList.add("open");
  modal.querySelectorAll(".pl-row").forEach((row) => {
    row.addEventListener("click", async () => {
      const ch = pendingPlaylistChannel;
      if (!ch) return;
      const playlist_id = row.dataset.pl || null;
      try {
        await API.bulkDownload(ch.id, {
          channel_name: ch.name,
          platform: "YouTube",
          action: "start",
          playlist_id,
        });
        bulkStatus[ch.id] = { done: 0, total: 0, active: true };
        modal.classList.remove("open");
        if (currentRoute() === "library") render();
      } catch (e) {
        alert(`Bulk download failed: ${e.message}`);
      }
    });
  });
}

function channelCard(c) {
  const platformClass = c.platform.toLowerCase();
  const liveClass = c.is_live ? "live" : "";
  const channelKey = `${c.platform}:${c.id}`;
  return `
    <div class="channel-card ${liveClass}">
      <div class="row">
        <span class="platform-icon ${platformClass}">${c.platform}</span>
        <span class="name">${escape(c.display_name || c.name)}</span>
        ${c.is_live ? '<span class="status live">LIVE</span>' : '<span class="status">offline</span>'}
      </div>
      ${c.stream_title ? `<div class="stream-title">${escape(c.stream_title)}</div>` : ""}
      <div class="meta">
        ${c.viewer_count ? `<span>${formatCount(c.viewer_count)} viewers</span>` : ""}
        ${c.game_or_category ? `<span>${escape(c.game_or_category)}</span>` : ""}
        ${c.auto_record ? '<span style="color: var(--secondary)">★ auto</span>' : ""}
      </div>
      <div class="actions">
        ${c.is_live ? `
          <button class="primary" data-action="record" data-channel-id="${c.id}"
                  data-channel-name="${escape(c.name)}"
                  data-display-name="${escape(c.display_name || c.name)}"
                  data-platform="${c.platform}"
                  data-stream-title="${escape(c.stream_title || '')}">
            ● Record
          </button>
          <button data-action="record" data-from-start="true"
                  data-channel-id="${c.id}"
                  data-channel-name="${escape(c.name)}"
                  data-display-name="${escape(c.display_name || c.name)}"
                  data-platform="${c.platform}"
                  data-stream-title="${escape(c.stream_title || '')}">
            ● From start
          </button>
        ` : ""}
        <button data-action="auto-record"
                data-channel-key="${channelKey}"
                data-enabled="${!c.auto_record}">
          ${c.auto_record ? "Disable auto" : "Enable auto"}
        </button>
        ${bulkButton(c)}
        ${c.platform === "YouTube" ? `
          <button data-action="bulk-playlist" data-channel-id="${c.id}"
                  data-channel-name="${escape(c.display_name || c.name)}">
            ⛁ Playlist…
          </button>
        ` : ""}
      </div>
    </div>
  `;
}

// #74 — bulk-download toggle button reflecting live SSE progress.
function bulkButton(c) {
  const st = bulkStatus[c.id];
  if (st && st.active) {
    const label = st.total > 0 ? `⇩ ${st.done}/${st.total} — Stop` : "⇩ … — Stop";
    return `<button data-action="bulk" data-bulk-active="true"
              data-channel-id="${c.id}"
              data-channel-name="${escape(c.display_name || c.name)}"
              data-platform="${c.platform}">${label}</button>`;
  }
  return `<button data-action="bulk" data-bulk-active="false"
            data-channel-id="${c.id}"
            data-channel-name="${escape(c.display_name || c.name)}"
            data-platform="${c.platform}">⇩ Bulk DL</button>`;
}

async function startRecordingFromCard(d) {
  try {
    await API.startRecording({
      channel_id: d.channelId,
      channel_name: d.channelName,
      display_name: d.displayName,
      platform: d.platform,
      from_start: d.fromStart === "true",
      stream_title: d.streamTitle || null,
      transcode: false,
    });
  } catch (e) {
    alert(`Start failed: ${e.message}`);
  }
}

async function toggleAutoRecord(d) {
  try {
    await API.toggleAutoRecord(d.channelKey, d.enabled === "true");
    await render();
  } catch (e) {
    alert(`Auto-record toggle failed: ${e.message}`);
  }
}

// ── Recordings table ─────────────────────────────────────────────────
async function renderRecordings() {
  let recordings = [];
  try {
    const data = await API.recordings();
    recordings = data.recordings || [];
  } catch (e) {
    if (e.message.includes("unauthorized")) return;
    root.innerHTML = chrome(
      `<div class="empty"><div class="glyph">⚠</div>${escape(e.message)}</div>`,
    );
    setupChromeHandlers();
    return;
  }
  root.removeAttribute("aria-busy");
  if (recordings.length === 0) {
    root.innerHTML = chrome(`
      <h1 class="page-title">Recordings</h1>
      <div class="empty">
        <div class="glyph">📁</div>
        No recordings yet. Start one from the Library tab.
      </div>
    `);
    setupChromeHandlers();
    return;
  }
  root.innerHTML = chrome(`
    <h1 class="page-title">Recordings</h1>
    <p class="page-subtitle">${recordings.length} total</p>
    <table class="recordings-table">
      <thead>
        <tr>
          <th>State</th>
          <th>Channel</th>
          <th>Title</th>
          <th>Started</th>
          <th>Size</th>
          <th></th>
        </tr>
      </thead>
      <tbody>
        ${recordings.map(recordingRow).join("")}
      </tbody>
    </table>
  `);
  setupChromeHandlers();
  document.querySelectorAll("[data-action=stop]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      try {
        await API.stopRecording(btn.dataset.jobId);
        setTimeout(render, 500);
      } catch (e) {
        alert(`Stop failed: ${e.message}`);
      }
    });
  });
}

function recordingRow(r) {
  const state = stateLabel(r.state);
  const stateClass = stateClassName(r.state);
  const isActive = stateClass === "recording";
  return `
    <tr>
      <td><span class="state-pill ${stateClass}">${state}</span></td>
      <td>${escape(r.channel_name)}</td>
      <td>${escape(r.stream_title || "(no title)")}</td>
      <td>${new Date(r.started_at).toLocaleString()}</td>
      <td>${formatBytes(r.bytes_written || 0)}</td>
      <td>
        ${isActive
          ? `<button class="danger" data-action="stop" data-job-id="${r.id}">Stop</button>`
          : ""}
      </td>
    </tr>
  `;
}

function stateLabel(s) {
  if (typeof s === "string") return s;
  if (s && typeof s === "object") return Object.keys(s)[0];
  return "?";
}
function stateClassName(s) {
  const label = stateLabel(s).toLowerCase();
  if (label.includes("record")) return "recording";
  if (label.includes("finish")) return "finished";
  if (label.includes("fail")) return "failed";
  return "";
}

// ── Gantt strip (W5 — last 24h of recordings as horizontal bars) ──────
function renderGantt(items) {
  if (items.length === 0) return "";
  // Bucket by channel for the vertical axis; horizontal axis is the
  // last 24 hours.
  const now = Date.now();
  const windowMs = 24 * 60 * 60 * 1000;
  const start = now - windowMs;
  const byChannel = new Map();
  for (const it of items) {
    const ch = it.channel_name || "(unknown)";
    if (!byChannel.has(ch)) byChannel.set(ch, []);
    byChannel.get(ch).push(it);
  }
  const channels = [...byChannel.keys()];
  if (channels.length === 0) return "";
  const rowH = 22;
  const totalH = channels.length * rowH + 24;
  // SVG width is responsive via 100%; bars use percentage coordinates.
  const bars = channels
    .map((ch, i) => {
      const y = i * rowH + 20;
      const chBars = byChannel
        .get(ch)
        .map((it) => {
          const s = new Date(it.start_at).getTime();
          const e = new Date(it.end_at).getTime();
          const xPct = Math.max(0, ((s - start) / windowMs) * 100);
          const wPct = Math.max(0.3, Math.min(100 - xPct, ((e - s) / windowMs) * 100));
          const stateColor =
            it.state.toLowerCase().includes("record")
              ? "var(--recording)"
              : it.state.toLowerCase().includes("finish")
              ? "var(--live)"
              : it.state.toLowerCase().includes("fail")
              ? "var(--secondary)"
              : "var(--muted)";
          return `<rect x="${xPct}%" y="${y + 3}" width="${wPct}%" height="14"
                     fill="${stateColor}" rx="2"
                     data-title="${escape(it.stream_title || ch)} · ${formatBytes(it.bytes_written || 0)}"></rect>`;
        })
        .join("");
      return `
        <text x="0" y="${y + 14}" fill="var(--muted)" font-size="11" font-family="ui-monospace, monospace">
          ${escape(ch.slice(0, 18))}
        </text>
        ${chBars}
      `;
    })
    .join("");
  // Vertical "now" marker at the right edge (100%).
  const nowMarker = `<line x1="100%" x2="100%" y1="20" y2="${totalH - 4}" stroke="var(--primary)" stroke-width="2" stroke-dasharray="2 2"/>`;
  return `
    <div style="background: var(--surface); border: 1px solid var(--border); border-radius: 8px; padding: 1rem; margin-bottom: 2rem;">
      <h2 style="margin: 0 0 0.5rem 0; font-size: 0.875rem; color: var(--muted);">
        24h timeline · ${items.length} recording${items.length === 1 ? "" : "s"}
      </h2>
      <svg viewBox="0 0 100 ${totalH}" preserveAspectRatio="none"
           style="width: 100%; height: ${totalH}px; padding-left: 120px; box-sizing: border-box;"
           role="img" aria-label="24-hour recording timeline">
        ${bars}
        ${nowMarker}
      </svg>
      <div style="display: flex; justify-content: space-between; color: var(--dim); font-size: 0.75rem; padding-left: 120px;">
        <span>24h ago</span><span>12h</span><span>now</span>
      </div>
    </div>
  `;
}

// ── Pipelines (W5 — read PluginRpc dispatch state from daemon) ────────
async function renderPipelines() {
  root.removeAttribute("aria-busy");
  root.innerHTML = chrome(`
    <h1 class="page-title">Pipelines</h1>
    <p class="page-subtitle">
      Cross-plugin DAG mirror — Ctrl+G overlay equivalent.
    </p>
    <div class="empty" role="status">
      <div class="glyph" aria-hidden="true">🔁</div>
      Daemon pipeline registry is empty.<br>
      Pipelines appear here when plugins submit them via <code>PluginAction::SubmitPipeline</code>.<br>
      <small>(Daemon plugins load at startup but verb dispatch over IPC is W2-phase-3.)</small>
    </div>
  `);
  setupChromeHandlers();
}

// ── Plugins (W5 — mirror the TUI's Shift+P browser) ────────────────────
async function renderPlugins() {
  root.removeAttribute("aria-busy");
  root.innerHTML = chrome(`
    <h1 class="page-title">Plugins</h1>
    <p class="page-subtitle">
      Loaded first-party plugins. Verbs hit <code>POST /api/v1/plugins/&lt;plugin&gt;/&lt;verb&gt;</code>.
    </p>
    <div class="channel-grid">
      ${pluginCard("crunchr", "Crunchr", "Transcription + analysis", ["Re-transcribe", "Show transcript"])}
      ${pluginCard("archiver", "Archiver", "Back-catalog VOD pulls", ["Re-archive channel"])}
      ${pluginCard("editor", "Editor", "Lossless transcript-as-timeline clipper", [])}
      ${pluginCard("insights", "Insights", "Word freq / speakers / topics", [])}
    </div>
    <div class="empty" style="margin-top: 2rem; font-size: 0.875rem;">
      Verb dispatch is W2-phase-3 — buttons here POST to the daemon, which logs the request
      and returns 202. Full dispatch lands when the daemon AppState wrapper does.
    </div>
  `);
  setupChromeHandlers();
  document.querySelectorAll("[data-action=plugin-verb]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      try {
        const r = await API.pluginRpc(btn.dataset.plugin, btn.dataset.verb, {
          selection: [],
        });
        alert(`Dispatched: ${btn.dataset.plugin}: ${btn.dataset.verb}\n${r.note || ""}`);
      } catch (e) {
        alert(`Plugin RPC failed: ${e.message}`);
      }
    });
  });
}

function pluginCard(slug, name, desc, verbs) {
  const verbButtons = verbs
    .map(
      (v) => `
    <button data-action="plugin-verb" data-plugin="${slug}" data-verb="${escape(v)}">${escape(v)}</button>
  `,
    )
    .join("");
  return `
    <div class="channel-card">
      <div class="row">
        <span class="platform-icon" style="background: var(--secondary); color: var(--bg);">
          ${escape(slug.toUpperCase())}
        </span>
        <span class="name">${escape(name)}</span>
        <span class="status">ready</span>
      </div>
      <div class="stream-title">${escape(desc)}</div>
      <div class="actions">${verbButtons || '<span style="color: var(--muted); font-size: 0.75rem">no item-scoped verbs</span>'}</div>
    </div>
  `;
}

// ── Activity (W5 — full-page version of the right-rail tail) ───────────
function renderActivityPage() {
  root.removeAttribute("aria-busy");
  const items = activityLog.length
    ? activityLog
        .map(
          (e) => `
      <div class="activity-event" style="border-bottom-color: var(--border);">
        <span class="kind">${escape(e.kind)}</span>
        <span class="timestamp">${e.at.toLocaleString()}</span>
        <div class="summary">${escape(e.summary)}</div>
      </div>
    `,
        )
        .join("")
    : `<div class="empty"><div class="glyph">⚡</div>No events yet. Things appear here as the daemon emits them.</div>`;
  root.innerHTML = chrome(`
    <h1 class="page-title">Activity</h1>
    <p class="page-subtitle">Last ${activityLog.length} events (live).</p>
    <div style="max-width: 720px;">${items}</div>
  `);
  setupChromeHandlers();
}

// ── Stub routes ──────────────────────────────────────────────────────
function renderStub(title, msg) {
  root.removeAttribute("aria-busy");
  root.innerHTML = chrome(`
    <h1 class="page-title">${escape(title)}</h1>
    <div class="empty">
      <div class="glyph">🚧</div>
      ${escape(msg)}
    </div>
  `);
  setupChromeHandlers();
}

// ── Live-count ticker ────────────────────────────────────────────────
function updateLiveCount(n) {
  const pill = document.getElementById("live-pill");
  if (!pill) return;
  if (n > 0) {
    pill.textContent = `● LIVE NOW: ${n}`;
    pill.style.display = "";
  } else {
    pill.style.display = "none";
  }
}

// ── Utilities ────────────────────────────────────────────────────────
function escape(s) {
  if (s == null) return "";
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}
function formatCount(n) {
  if (n >= 1000000) return (n / 1000000).toFixed(1) + "M";
  if (n >= 1000) return (n / 1000).toFixed(1) + "k";
  return String(n);
}
function formatBytes(n) {
  if (n >= 1e9) return (n / 1e9).toFixed(2) + " GB";
  if (n >= 1e6) return (n / 1e6).toFixed(1) + " MB";
  if (n >= 1e3) return (n / 1e3).toFixed(0) + " KB";
  return n + " B";
}

// ── W6 keyboard shortcuts ────────────────────────────────────────────
// Linear-/GitHub-style: prefix `g` then route letter to jump (gl/gr/gs
// etc.), `?` to open the help overlay, `Esc` to close, `a` to toggle
// the activity rail, `p` to trigger Poll.
let prefixActive = false;
let prefixTimer = null;

document.addEventListener("keydown", (e) => {
  // Don't intercept while typing in an input.
  const tag = (e.target.tagName || "").toLowerCase();
  if (tag === "input" || tag === "textarea") return;
  if (e.ctrlKey || e.metaKey || e.altKey) return;

  if (e.key === "?") {
    e.preventDefault();
    document.getElementById("kbd-help")?.classList.add("open");
    return;
  }
  if (e.key === "Escape") {
    document.getElementById("kbd-help")?.classList.remove("open");
    document.getElementById("activity-rail")?.classList.remove("open");
    return;
  }
  if (e.key === "a") {
    e.preventDefault();
    document.getElementById("activity-rail")?.classList.toggle("open");
    renderActivityRail();
    return;
  }
  if (e.key === "p") {
    e.preventDefault();
    API.pollNow().catch(() => {});
    return;
  }
  if (e.key === "g" && !prefixActive) {
    prefixActive = true;
    prefixTimer = setTimeout(() => (prefixActive = false), 1000);
    return;
  }
  if (prefixActive) {
    clearTimeout(prefixTimer);
    prefixActive = false;
    const link = document.querySelector(`.leftrail a[data-key="${e.key}"]`);
    if (link) {
      e.preventDefault();
      const r = link.dataset.route;
      route(r);
    }
  }
});

function injectKeyboardHelp() {
  if (document.getElementById("kbd-help")) return;
  const div = document.createElement("div");
  div.id = "kbd-help";
  div.className = "kbd-help";
  div.setAttribute("role", "dialog");
  div.setAttribute("aria-label", "Keyboard shortcuts");
  div.innerHTML = `
    <div class="card">
      <h2>Keyboard shortcuts</h2>
      <dl>
        <dt>?</dt><dd>This help</dd>
        <dt>g l</dt><dd>Library</dd>
        <dt>g r</dt><dd>Recordings</dd>
        <dt>g s</dt><dd>Schedule</dd>
        <dt>g d</dt><dd>Pipelines (DAG)</dd>
        <dt>g g</dt><dd>Plugins</dd>
        <dt>g i</dt><dd>Activity feed (page)</dd>
        <dt>g c</dt><dd>Settings</dd>
        <dt>g y</dt><dd>System</dd>
        <dt>a</dt><dd>Toggle activity rail</dd>
        <dt>p</dt><dd>Poke channel monitor</dd>
        <dt>Esc</dt><dd>Close overlay</dd>
      </dl>
    </div>
  `;
  div.addEventListener("click", (e) => {
    if (e.target === div) div.classList.remove("open");
  });
  document.body.appendChild(div);
}

// ── Boot ─────────────────────────────────────────────────────────────
events.on(pushActivity);
events.on((event) => {
  // Cheap re-render gate: refresh the visible page on relevant events.
  if (
    currentRoute() === "library" &&
    (event.ChannelWentLive ||
      event.ChannelWentOffline ||
      event.ChannelsUpdated)
  ) {
    renderLibrary().catch(console.error);
  }
  if (
    currentRoute() === "recordings" &&
    (event.RecordingStarted ||
      event.RecordingFinished ||
      event.RecordingProgress)
  ) {
    renderRecordings().catch(console.error);
  }
  // #74 — bulk-download progress drives the per-channel button.
  if (event.BulkProgress) {
    const p = event.BulkProgress;
    if (p.active) {
      bulkStatus[p.channel_id] = { done: p.done, total: p.total, active: true };
    } else {
      delete bulkStatus[p.channel_id];
    }
    if (currentRoute() === "library") renderLibrary().catch(console.error);
  }
  // #74 / #73 — playlist list answers the picker request.
  if (event.PlaylistList) {
    const pl = event.PlaylistList;
    if (pendingPlaylistChannel && pl.channel_id === pendingPlaylistChannel.id) {
      showPlaylistModal({
        loading: false,
        name: pendingPlaylistChannel.name,
        playlists: pl.playlists || [],
      });
    }
  }
});
events.start();
injectKeyboardHelp();
render();
