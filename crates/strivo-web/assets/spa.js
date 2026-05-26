// StriVo SPA — vanilla JS, hash routing, *arr-inspired chrome. (W4 MVP.)
//
// This is the minimum-viable shippable webui that uses the W1+W2+W3
// backend. SvelteKit conversion is the W4 phase 2 follow-up; this
// file deliberately stays small + dependency-free.

const API = {
  async _fetch(path, opts = {}) {
    // X-Strivo-CSRF is a custom header browsers can't attach cross-site
    // without a (denied) preflight, so it gates cookie-authed mutations
    // against CSRF. Harmless on GETs. See crates/strivo-web/src/csrf.rs.
    const headers = {
      Accept: "application/json",
      "X-Strivo-CSRF": "1",
      ...(opts.headers || {}),
    };
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
  healthChecks: () => API._fetch("/health/checks"),
  logs: (level, lines = 300) =>
    API._fetch(`/logs?level=${encodeURIComponent(level || "trace")}&lines=${lines}`),
  backupCreate: () => API._fetch("/backup", { method: "POST" }),
  backups: () => API._fetch("/backups"),
  backupRestore: (name) =>
    API._fetch(`/backups/${encodeURIComponent(name)}/restore`, { method: "POST" }),
  history: () => API._fetch("/history"),
  blocklist: () => API._fetch("/blocklist"),
  blockAdd: (body) => API._fetch("/blocklist", { method: "POST", body }),
  blockRemove: (body) => API._fetch("/blocklist", { method: "DELETE", body }),
  storage: () => API._fetch("/storage"),
  settings: () => API._fetch("/settings"),
  patreon: () => API._fetch("/patreon"),
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
  resolveChannel: (platform, query) =>
    API._fetch("/channels/resolve", { method: "POST", body: { platform, query } }),
  requestChannelVods: (channelId, platform) =>
    API._fetch(`/channels/${encodeURIComponent(channelId)}/vods`, {
      method: "POST",
      body: { platform },
    }),
  schedule: () => API._fetch("/schedule"),
  patreonPull: (body) =>
    API._fetch("/patreon/pull", { method: "POST", body }),
  login: (apiKey) =>
    API._fetch("/auth/login", { method: "POST", body: { api_key: apiKey } }),
  logout: () => API._fetch("/auth/logout", { method: "POST" }),
};

// ── SSE event stream ─────────────────────────────────────────────────
const events = {
  source: null,
  listeners: new Set(),
  degradedPoll: null,
  start() {
    if (this.source) return;
    this.source = new EventSource("/events", { withCredentials: true });
    this.source.onopen = () => this.setConnected(true);
    this.source.onmessage = (e) => {
      this.setConnected(true);
      try {
        const data = JSON.parse(e.data);
        this.listeners.forEach((fn) => fn(data));
      } catch (_) {}
    };
    this.source.onerror = () => {
      // Make the stale-data state VISIBLE (research §A/§5: silent
      // real-time breakage is the #1 cited gotcha). The browser
      // auto-reconnects; meanwhile we show a pill and degrade to a
      // slow poll so list views don't go stale.
      this.setConnected(false);
    };
  },
  // Show/hide the topbar "reconnecting…" pill and arm/disarm a 10s
  // degraded re-poll of the current data route.
  setConnected(ok) {
    const pill = document.getElementById("conn-status");
    if (pill) pill.hidden = ok;
    if (ok) {
      if (this.degradedPoll) {
        clearInterval(this.degradedPoll);
        this.degradedPoll = null;
      }
    } else if (!this.degradedPoll) {
      this.degradedPoll = setInterval(() => {
        const r = currentRoute();
        if (r === "library") renderHome().catch(() => {});
        else if (r === "recordings") renderRecordings().catch(() => {});
      }, 10000);
    }
  },
  on(fn) {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  },
};

// #74 — per-channel bulk-download status, keyed by channel_id:
// { done, total, active }. Fed by the `bulk-progress` SSE event.
const bulkStatus = {};
// #75 — Patreon snapshot, fed by the `patreon-state` SSE event:
// { creators: [ChannelEntry], posts: { campaign_id: [PatreonPost] } }.
const patreonState = { creators: [], posts: {} };
// W4-alt — recordings grid sort/filter state + last-fetched cache.
let recSort = { col: "started", dir: "desc" };
let recFilter = "";
let recCache = [];
// TUI-redesign — left-rail channel cache, current selection, per-channel
// VOD cache (channel_id -> [VodEntry]), and the recordings dashboard cache.
let channelCache = [];
let selectedChannelKey = null;
const channelVods = {};
let dashRecordings = [];
let dashSchedule = [];

// True for recording states still in flight.
function isInProgress(state) {
  const s = stateLabel(state).toLowerCase();
  return s.includes("record") || s.includes("resolv") || s.includes("stopp");
}

// ── Toasts (research §D) ──────────────────────────────────────────────
// One singleton with two pre-created ARIA live regions: polite for
// success/info, assertive for errors. Errors are sticky (action-needed);
// success/info auto-dismiss with hover-pause. Toasts are non-interactive
// (message + close only).
const Toast = (() => {
  let polite, assertive;
  function ensure() {
    if (polite) return;
    const wrap = document.createElement("div");
    wrap.className = "toast-wrap";
    const mk = (role, live) => {
      const r = document.createElement("div");
      r.className = "toast-region";
      r.setAttribute("role", role);
      r.setAttribute("aria-live", live);
      return r;
    };
    assertive = mk("alert", "assertive");
    polite = mk("status", "polite");
    wrap.append(assertive, polite);
    document.body.appendChild(wrap);
  }
  function show(kind, msg, sticky) {
    ensure();
    const region = kind === "error" ? assertive : polite;
    const el = document.createElement("div");
    el.className = `toast ${kind}`;
    el.innerHTML = `<span class="toast-msg"></span><button class="toast-close" aria-label="Dismiss">×</button>`;
    el.querySelector(".toast-msg").textContent = msg;
    const close = () => {
      el.classList.add("out");
      setTimeout(() => el.remove(), 200);
    };
    el.querySelector(".toast-close").addEventListener("click", close);
    region.appendChild(el);
    while (region.children.length > 4) region.firstChild.remove();
    if (!sticky) {
      const ttl = 5000;
      let timer = setTimeout(close, ttl);
      el.addEventListener("mouseenter", () => clearTimeout(timer));
      el.addEventListener("mouseleave", () => (timer = setTimeout(close, ttl)));
    }
    return close;
  }
  return {
    success: (m) => show("success", m, false),
    info: (m) => show("info", m, false),
    error: (m) => show("error", m, true), // sticky — user must see/dismiss
  };
})();

// Focus-trapped confirmation dialog for destructive actions. Resolves
// true/false. (research §D: modals only for irreversible actions.)
function confirmDialog(message, opts = {}) {
  return new Promise((resolve) => {
    const prev = document.activeElement;
    const modal = document.createElement("div");
    modal.className = "kbd-help open confirm-modal";
    modal.innerHTML = `<div class="card" role="alertdialog" aria-modal="true">
      <p class="confirm-msg"></p>
      <div class="confirm-actions">
        <button class="confirm-cancel">${escape(opts.cancel || "Cancel")}</button>
        <button class="confirm-ok ${opts.danger ? "danger" : "primary"}">${escape(opts.ok || "Confirm")}</button>
      </div></div>`;
    modal.querySelector(".confirm-msg").textContent = message;
    document.body.appendChild(modal);
    const ok = modal.querySelector(".confirm-ok");
    const cancel = modal.querySelector(".confirm-cancel");
    const done = (v) => {
      modal.remove();
      if (prev && prev.focus) prev.focus();
      resolve(v);
    };
    ok.addEventListener("click", () => done(true));
    cancel.addEventListener("click", () => done(false));
    modal.addEventListener("click", (e) => {
      if (e.target === modal) done(false);
    });
    modal.addEventListener("keydown", (e) => {
      if (e.key === "Escape") done(false);
      if (e.key === "Tab") {
        e.preventDefault();
        (document.activeElement === ok ? cancel : ok).focus();
      }
    });
    ok.focus();
  });
}

// Run an async action with a busy/debounced button: aria-busy + label
// swap + double-fire guard. Safe even if the handler re-renders the page.
async function withBusy(btn, busyLabel, fn) {
  if (btn) {
    if (btn.dataset.busy === "1") return; // debounce
    btn.dataset.busy = "1";
    btn.setAttribute("aria-busy", "true");
    btn.classList.add("busy");
    if (busyLabel) {
      btn.dataset.prevLabel = btn.textContent;
      btn.textContent = busyLabel;
    }
  }
  try {
    return await fn();
  } finally {
    if (btn && btn.isConnected) {
      btn.dataset.busy = "0";
      btn.removeAttribute("aria-busy");
      btn.classList.remove("busy");
      if (btn.dataset.prevLabel) btn.textContent = btn.dataset.prevLabel;
    }
  }
}

// ── Hash router ──────────────────────────────────────────────────────
const ROUTES = [
  "library",
  "recordings",
  "schedule",
  "pipelines",
  "plugins",
  "settings",
  "system",
  "logs",
  "history",
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
      await renderHome();
      break;
    case "recordings":
      await renderRecordings();
      break;
    case "schedule":
      await renderSchedule();
      break;
    case "pipelines":
      renderPipelines();
      break;
    case "plugins":
      renderPlugins();
      break;
    case "settings":
      await renderSettings();
      break;
    case "system":
      await renderSystem();
      break;
    case "logs":
      await renderLogs();
      break;
    case "history":
      await renderHistory();
      break;
  }
}

// Top-bar route nav (functional pages). The left rail is the channel
// list now; these icon links reach the management pages.
const TOPNAV = [
  ["library", "▣", "Home", "l"],
  ["recordings", "📁", "Recordings", "r"],
  ["schedule", "📅", "Schedule", "s"],
  ["pipelines", "🔁", "Pipelines", "d"],
  ["plugins", "🧩", "Plugins", "g"],
  ["settings", "⚙", "Settings", "c"],
  ["system", "🛠", "System", "y"],
  ["logs", "📜", "Logs", "o"],
  ["history", "🗂", "History", "h"],
];

function chrome(content) {
  const r = currentRoute();
  const nav = TOPNAV.map(
    ([route, glyph, label, key]) =>
      `<a class="topnav-link ${route === r ? "active" : ""}"
          href="#/${route}" data-route="${route}" data-key="${key}"
          title="${label}" aria-label="${label}">
        <span aria-hidden="true">${glyph}</span>
      </a>`,
  ).join("");
  return `
    <div class="chrome">
      <header class="topbar" role="banner">
        <a class="brand" href="#/library" id="brand-home" title="Home">StriVo</a>
        <span id="live-pill" class="live-pill" style="display: none"
              aria-label="Live recording count"></span>
        <span id="storage-pill" class="storage-pill" style="display: none"
              aria-label="Storage usage"></span>
        <span id="conn-status" class="conn-status" role="status" hidden
              title="Live updates connection">● reconnecting…</span>
        <a id="health-pill" class="health-pill" href="#/system" hidden
           role="status" title="System health — click for details"></a>
        <span class="spacer"></span>
        <nav class="topnav" aria-label="Main navigation">${nav}</nav>
        <button id="add-channel" title="Add a channel to monitor"
                aria-label="Add channel">＋ Add</button>
        <button id="poll-now" title="Poke channel monitor (p)"
                aria-label="Trigger immediate channel poll">↻ Poll</button>
        <button id="logout" title="Logout" aria-label="Sign out">⏻</button>
      </header>
      <nav class="leftrail" id="channel-list" aria-label="Channels"></nav>
      <main class="content" id="content">${content}</main>
    </div>
  `;
}

function setupChromeHandlers() {
  // Brand → home: clear any selected channel and go to the dashboard.
  document.getElementById("brand-home")?.addEventListener("click", (e) => {
    e.preventDefault();
    selectedChannelKey = null;
    if (currentRoute() === "library") render();
    else route("library");
  });
  document.getElementById("poll-now")?.addEventListener("click", async () => {
    try {
      await API.pollNow();
    } catch (e) {
      console.error(e);
    }
  });
  document.getElementById("add-channel")?.addEventListener("click", () => openAddChannelWizard());
  document.getElementById("logout")?.addEventListener("click", async () => {
    await API.logout().catch(() => {});
    route("login");
  });
  // W5 — refresh the topbar storage pill on every chrome mount.
  refreshStoragePill();
  // Health pill — amber/red when any check is degraded (roadmap item 13).
  refreshHealthPill();
  // Channel list lives in the left rail on every page.
  paintChannelList();
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

// Topbar health pill: only shown when the worst check is warn/error, so a
// healthy system stays uncluttered. Links to the System page. (Item 13.)
async function refreshHealthPill() {
  const pill = document.getElementById("health-pill");
  if (!pill) return;
  try {
    const h = await API.healthChecks();
    const worst = h.status || "ok";
    if (worst === "ok") {
      pill.hidden = true;
      return;
    }
    const bad = (h.checks || []).filter((c) => c.severity !== "ok");
    pill.className = `health-pill ${worst}`;
    pill.textContent = `${worst === "error" ? "✕" : "▲"} ${bad.length} issue${bad.length === 1 ? "" : "s"}`;
    pill.title = bad.map((c) => `${c.domain}/${c.name}: ${c.message}`).join("\n");
    pill.hidden = false;
  } catch (_) {
    pill.hidden = true;
  }
}

// ── Channel list (left rail) ─────────────────────────────────────────
// Merges /channels (Twitch/YT) with Patreon creators (patreonState),
// live first + bold, then offline. Clicking selects a channel and shows
// its detail in the center (home route only).
function paintChannelList() {
  const rail = document.getElementById("channel-list");
  if (!rail) return;

  const merged = [...channelCache, ...patreonState.creators];
  // De-dupe by platform:id in case a Patreon creator is also in /channels.
  const seen = new Set();
  const channels = merged.filter((c) => {
    const k = `${c.platform}:${c.id}`;
    if (seen.has(k)) return false;
    seen.add(k);
    return true;
  });

  const live = channels.filter((c) => c.is_live);
  const offline = channels
    .filter((c) => !c.is_live)
    .sort((a, b) =>
      (a.display_name || a.name).localeCompare(b.display_name || b.name),
    );
  updateLiveCount(recCache.filter((r) => isInProgress(r.state)).length);

  const recordingChannelIds = new Set(
    recCache.filter((r) => isInProgress(r.state)).map((r) => r.channel_id),
  );

  const row = (c) => {
    const key = `${c.platform}:${c.id}`;
    const sel = key === selectedChannelKey ? "sel" : "";
    const isPatreon = c.platform === "Patreon";
    const rec = recordingChannelIds.has(c.id)
      ? '<span class="ch-rec" title="recording">●</span>'
      : "";
    const liveDot = c.is_live ? '<span class="ch-live">◉</span>' : "";
    const viewers = c.is_live && c.viewer_count
      ? `<span class="ch-viewers">${formatCount(c.viewer_count)}</span>`
      : "";
    // Patreon rows are visually distinct (item 6): a pledged-tier chip
    // (stored in stream_title) and a patreon-accented platform glyph.
    const tier = isPatreon && c.stream_title
      ? `<span class="ch-tier" title="pledged tier">${escape(c.stream_title)}</span>`
      : "";
    return `
      <a class="ch-row ${c.is_live ? "live" : ""} ${isPatreon ? "patreon" : ""} ${sel}"
         data-channel-key="${key}" data-channel-id="${c.id}"
         data-platform="${c.platform}" href="#/library">
        <span class="ch-plat ${c.platform.toLowerCase()}" aria-hidden="true">${platformGlyph(c.platform)}</span>
        <span class="ch-name">${escape(c.display_name || c.name)}</span>
        ${tier}${viewers}${liveDot}${rec}
      </a>`;
  };

  const section = (title, list) =>
    list.length
      ? `<div class="ch-section-title">${title} <span class="ch-count">${list.length}</span></div>${list.map(row).join("")}`
      : "";

  // Offline channels grouped by platform (item 5). Twitch / YouTube /
  // Patreon each get their own header; Patreon thus becomes a distinct,
  // always-visible section (item 6).
  const byPlat = (plat) => offline.filter((c) => c.platform === plat);

  // Preserve scroll position across repaints (the rail is rebuilt
  // wholesale, which would otherwise jump it to the top on every event).
  const prevScroll = rail.scrollTop;
  rail.innerHTML =
    channels.length === 0
      ? `<div class="ch-empty">No channels yet.<br><br>
           Connect Twitch / YouTube / Patreon and follow channels — they
           appear here automatically.<br>
           <a href="#/settings">Check Settings →</a></div>`
      : section(`● LIVE`, live) +
        section("Twitch", byPlat("Twitch")) +
        section("YouTube", byPlat("YouTube")) +
        section("Patreon", byPlat("Patreon"));

  rail.querySelectorAll(".ch-row").forEach((el) => {
    el.addEventListener("click", (e) => {
      e.preventDefault();
      selectChannel(el.dataset.channelKey);
    });
  });
  rail.scrollTop = prevScroll;
}

function platformGlyph(p) {
  return p === "Twitch" ? "🟣" : p === "YouTube" ? "🔴" : "◈";
}

// Seed patreonState from the daemon snapshot (/patreon) so Patreon shows
// immediately on load, instead of only after the next ~5-min poll's
// patreon-state SSE event. Idempotent; refreshed live by SSE thereafter.
async function seedPatreon() {
  try {
    const p = await API.patreon();
    patreonState.creators = p.creators || [];
    patreonState.posts = {};
    for (const post of p.posts || []) {
      (patreonState.posts[post.campaign_id] ||= []).push(post);
    }
    for (const list of Object.values(patreonState.posts)) {
      list.sort((a, b) => (b.published_at || "").localeCompare(a.published_at || ""));
    }
  } catch (_) {
    /* non-fatal — SSE still refreshes it */
  }
}

function selectChannel(key) {
  selectedChannelKey = key;
  if (currentRoute() !== "library") {
    route("library"); // hashchange triggers render()
  } else {
    render();
  }
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

// ── Home: channel detail (if selected) + recordings dashboard ─────────
async function renderHome() {
  // Refresh the channel + recordings caches that feed the left rail and
  // the dashboard. Both are cheap snapshots.
  try {
    const [ch, rec] = await Promise.all([API.channels(), API.recordings()]);
    channelCache = ch.channels || [];
    recCache = rec.recordings || [];
    dashRecordings = recCache;
  } catch (e) {
    if (e.message.includes("unauthorized")) return;
  }
  await seedPatreon();
  try {
    dashSchedule = (await API.schedule()).schedule || [];
  } catch (_) {
    dashSchedule = [];
  }
  root.removeAttribute("aria-busy");

  const selected = selectedChannelKey
    ? [...channelCache, ...patreonState.creators].find(
        (c) => `${c.platform}:${c.id}` === selectedChannelKey,
      )
    : null;

  const center = selected
    ? `${channelDetailHtml(selected)}
       <div class="dash-band"><div id="dash">${recordingsDashboardHtml(true)}</div></div>`
    : `<div id="dash">${recordingsDashboardHtml(false)}</div>`;

  root.innerHTML = chrome(center);
  setupChromeHandlers();

  if (selected) {
    wireChannelDetail(selected);
    loadChannelDetailData(selected);
  }
  wireDashboard();
}

// Repaint ONLY the recordings dashboard subtree (#dash) — never the chrome,
// left rail, or channel-detail iframe. Driven by high-frequency recording
// events so they don't reload the live preview or reset rail scroll.
function paintDashboard() {
  const el = document.getElementById("dash");
  if (!el) return;
  el.innerHTML = recordingsDashboardHtml(!!selectedChannelKey);
  wireDashboard();
}

// ── Recordings dashboard ─────────────────────────────────────────────
function recordingsDashboardHtml(compact) {
  const inProgress = dashRecordings.filter((r) => isInProgress(r.state));
  const recent = dashRecordings
    .filter((r) => !isInProgress(r.state))
    .slice(0, compact ? 6 : 24);
  const upcoming = [...dashSchedule]
    .filter((s) => s.next_fire)
    .sort((a, b) => new Date(a.next_fire) - new Date(b.next_fire));

  const recCardEl = (r) => {
    const cls = stateClassName(r.state);
    const stopBtn = isInProgress(r.state)
      ? `<button class="danger sm" data-action="stop" data-job-id="${r.id}">Stop</button>`
      : "";
    return `<div class="rec-card ${cls}">
      <div class="rec-card-title">${escape(r.stream_title || r.channel_name || "(recording)")}</div>
      <div class="rec-card-meta">
        <span class="state-pill ${cls}">${escape(stateLabel(r.state))}</span>
        <span>${escape(r.channel_name || "")}</span>
        <span>${formatBytes(r.bytes_written || 0)}</span>
      </div>
      ${stopBtn}
    </div>`;
  };
  const schedCardEl = (s) => `
    <div class="rec-card upcoming">
      <div class="rec-card-title">${escape(s.channel)}</div>
      <div class="rec-card-meta">
        <span>${new Date(s.next_fire).toLocaleString()}</span>
        <span>${escape(s.duration || "")}</span>
      </div>
    </div>`;

  const rowEl = (title, count, html, empty) => `
    <section class="dash-row">
      <h2 class="dash-row-title">${title}${count != null ? ` <span class="dash-count">${count}</span>` : ""}</h2>
      <div class="dash-strip">${html || `<div class="empty sm">${empty}</div>`}</div>
    </section>`;

  const heading = compact ? "" : `<h1 class="page-title">Recordings dashboard</h1>`;
  return `${heading}
    ${rowEl("In progress", inProgress.length, inProgress.map(recCardEl).join(""), "Nothing recording")}
    ${rowEl("Recent", null, recent.map(recCardEl).join(""), "No recordings yet")}
    ${rowEl("Upcoming", upcoming.length, upcoming.map(schedCardEl).join(""), "No scheduled recordings")}`;
}

function wireDashboard() {
  document.querySelectorAll('[data-action="stop"]').forEach((btn) => {
    btn.addEventListener("click", async () => {
      if (!(await confirmDialog("Stop this recording?", { ok: "Stop", danger: true })))
        return;
      try {
        await API.stopRecording(btn.dataset.jobId);
        Toast.success("Recording stopped");
        setTimeout(render, 500);
      } catch (e) {
        Toast.error(`Stop failed: ${e.message}`);
      }
    });
  });
}

// ── Channel detail (center) ──────────────────────────────────────────
function channelDetailHtml(c) {
  const key = `${c.platform}:${c.id}`;
  const isPatreon = c.platform === "Patreon";
  const liveBadge = c.is_live
    ? '<span class="status live">LIVE</span>'
    : '<span class="status">offline</span>';
  const actions = `
    <div class="actions">
      ${c.is_live ? `
        <button class="primary" data-action="record" data-channel-id="${c.id}"
                data-channel-name="${escape(c.name)}"
                data-display-name="${escape(c.display_name || c.name)}"
                data-platform="${c.platform}"
                data-stream-title="${escape(c.stream_title || "")}">● Record</button>
        <button data-action="record" data-from-start="true" data-channel-id="${c.id}"
                data-channel-name="${escape(c.name)}"
                data-display-name="${escape(c.display_name || c.name)}"
                data-platform="${c.platform}"
                data-stream-title="${escape(c.stream_title || "")}">● From start</button>
      ` : ""}
      ${!isPatreon ? `
        <button data-action="auto-record" data-channel-key="${key}"
                data-enabled="${!c.auto_record}">
          ${c.auto_record ? "Disable auto" : "Enable auto"}
        </button>
        ${bulkButton(c)}
        ${c.platform === "YouTube" ? `
          <button data-action="bulk-playlist" data-channel-id="${c.id}"
                  data-channel-name="${escape(c.display_name || c.name)}">⛁ Playlist…</button>` : ""}
        <button data-action="block-channel" data-channel-id="${c.id}"
                data-platform="${c.platform}"
                data-channel-name="${escape(c.display_name || c.name)}"
                title="Stop auto-grabbing this channel">⊘ Block</button>
      ` : ""}
    </div>`;

  // Section placeholders filled by loadChannelDetailData (async).
  let sections;
  if (isPatreon) {
    sections = `<div id="cd-posts" class="cd-section"></div>`;
  } else if (c.platform === "YouTube") {
    sections = `
      <div id="cd-playlists" class="cd-section"></div>
      <div id="cd-streams" class="cd-section"></div>
      <div id="cd-uploads" class="cd-section"></div>`;
  } else {
    sections = `<div id="cd-streams" class="cd-section"></div>`;
  }

  return `
    <div class="channel-detail">
      <div class="cd-header">
        <span class="platform-icon ${c.platform.toLowerCase()}">${c.platform}</span>
        <h1 class="cd-name">${escape(c.display_name || c.name)}</h1>
        ${liveBadge}
        ${c.viewer_count ? `<span class="cd-viewers">${formatCount(c.viewer_count)} viewers</span>` : ""}
        <button class="cd-close" data-action="cd-close" title="Close">×</button>
      </div>
      ${c.stream_title ? `<div class="stream-title">${escape(c.stream_title)}</div>` : ""}
      ${livePreviewHtml(c)}
      ${actions}
      ${sections}
    </div>`;
}

// Live, muted preview when a live channel is opened (item 4). Uses the
// platform's own embed player (loopback-robust, no proxy/CORS): Twitch
// player.twitch.tv with parent=<host>, YouTube embed/live_stream?channel.
// Muted + autoplay so it plays without a gesture on desktop; the player's
// own controls cover mobile. Patreon has no live concept.
function livePreviewHtml(c) {
  if (!c.is_live) return "";
  const host = location.hostname || "127.0.0.1";
  let src = null;
  if (c.platform === "Twitch") {
    src = `https://player.twitch.tv/?channel=${encodeURIComponent(c.name)}` +
      `&parent=${encodeURIComponent(host)}&muted=true&autoplay=true`;
  } else if (c.platform === "YouTube") {
    src = `https://www.youtube.com/embed/live_stream?channel=${encodeURIComponent(c.id)}` +
      `&autoplay=1&mute=1&playsinline=1`;
  }
  if (!src) return "";
  return `<div class="cd-preview">
    <iframe src="${src}" title="Live preview" loading="lazy"
            allow="autoplay; fullscreen; picture-in-picture" allowfullscreen></iframe>
  </div>`;
}

function wireChannelDetail() {
  document.querySelector('[data-action="cd-close"]')?.addEventListener("click", () => {
    selectedChannelKey = null;
    render();
  });
  document.querySelectorAll("[data-action=record]").forEach((btn) =>
    btn.addEventListener("click", () => startRecordingFromCard(btn.dataset)),
  );
  document.querySelectorAll("[data-action=auto-record]").forEach((btn) =>
    btn.addEventListener("click", () => toggleAutoRecord(btn.dataset)),
  );
  document.querySelectorAll("[data-action=bulk]").forEach((btn) =>
    btn.addEventListener("click", () => toggleBulk(btn.dataset)),
  );
  document.querySelectorAll("[data-action=bulk-playlist]").forEach((btn) =>
    btn.addEventListener("click", () => openPlaylistPicker(btn.dataset)),
  );
  document.querySelectorAll("[data-action=block-channel]").forEach((btn) =>
    btn.addEventListener("click", async () => {
      const d = btn.dataset;
      if (
        !(await confirmDialog(
          `Block ${d.channelName}? StriVo will stop auto-grabbing this channel's VODs.`,
          { ok: "Block", danger: true },
        ))
      )
        return;
      try {
        await API.blockAdd({ platform: d.platform, channel_id: d.channelId });
        Toast.success(`Blocked ${d.channelName}`);
      } catch (e) {
        Toast.error(`Block failed: ${e.message}`);
      }
    }),
  );
}

// Fetch + render the per-channel VOD lists. Patreon uses cached posts;
// YouTube/Twitch request VODs over IPC (result arrives via SSE) and also
// request playlists for YouTube.
function loadChannelDetailData(c) {
  if (c.platform === "Patreon") {
    renderPatreonPosts(c);
    return;
  }
  // Render from cache immediately if we have it, then (re)request.
  paintChannelVods(c.id, c.platform);
  API.requestChannelVods(c.id, c.platform).catch(() => {});
  if (c.platform === "YouTube") {
    API.requestPlaylists(c.id).catch(() => {});
  }
  // Don't hang on "Loading…" forever — if the channel-vods SSE answer
  // hasn't arrived in 15s (slow/failed platform fetch), show an error
  // state for whichever sections are still loading.
  const id = c.id;
  setTimeout(() => {
    if (!channelVods[id] && `${c.platform}:${id}` === selectedChannelKey) {
      for (const sid of ["cd-streams", "cd-uploads"]) {
        const el = document.getElementById(sid);
        if (el && el.textContent.includes("Loading")) {
          const title = sid === "cd-streams"
            ? (c.platform === "Twitch" ? "Past broadcasts" : "Recent live streams")
            : "Recent uploads";
          el.innerHTML = `<h2 class="cd-section-title">${title}</h2>` +
            `<div class="empty sm">Couldn't load — the daemon may be fetching, or the platform isn't authed. <a href="#" data-action="cd-retry">Retry</a></div>`;
        }
      }
      document.querySelector('[data-action="cd-retry"]')?.addEventListener("click", (e) => {
        e.preventDefault();
        loadChannelDetailData(c);
      });
    }
  }, 15000);
}

function paintChannelVods(channelId, platform) {
  const vods = channelVods[channelId];
  const streamsEl = document.getElementById("cd-streams");
  const uploadsEl = document.getElementById("cd-uploads");
  if (!vods) {
    if (streamsEl) streamsEl.innerHTML = vodSectionHtml("Recent live streams", null);
    if (uploadsEl) uploadsEl.innerHTML = vodSectionHtml("Recent uploads", null);
    return;
  }
  const streams = vods.filter((v) => v.kind === "LiveBroadcast");
  const uploads = vods.filter((v) => v.kind !== "LiveBroadcast");
  if (streamsEl) {
    streamsEl.innerHTML = vodSectionHtml(
      platform === "Twitch" ? "Past broadcasts" : "Recent live streams",
      streams,
    );
  }
  if (uploadsEl) uploadsEl.innerHTML = vodSectionHtml("Recent uploads", uploads);
}

function vodSectionHtml(title, vods) {
  if (vods === null) {
    return `<h2 class="cd-section-title">${title}</h2><div class="empty sm">Loading…</div>`;
  }
  if (vods.length === 0) {
    return `<h2 class="cd-section-title">${title}</h2><div class="empty sm">None</div>`;
  }
  const rows = vods
    .map(
      (v) => `
    <a class="vod-row" href="${escape(v.url)}" target="_blank" rel="noopener">
      <span class="vod-date">${escape((v.published_at || "").slice(0, 10))}</span>
      <span class="vod-title">${escape(v.title)}</span>
    </a>`,
    )
    .join("");
  return `<h2 class="cd-section-title">${title} <span class="dash-count">${vods.length}</span></h2>
    <div class="vod-list">${rows}</div>`;
}

// Patreon channel detail: render cached posts with a pull action.
function renderPatreonPosts(c) {
  const el = document.getElementById("cd-posts");
  if (!el) return;
  const posts = patreonState.posts[c.id] || [];
  const rows = posts.length
    ? posts
        .map(
          (p) => `
      <div class="vod-row" data-action="patreon-pull"
           data-embed="${escape(p.embed_url || "")}"
           data-creator="${escape(c.display_name || c.name)}"
           data-title="${escape(p.title)}">
        <span class="vod-date">${escape((p.published_at || "").slice(0, 10))}</span>
        <span class="vod-title">${escape(p.title)}</span>
        ${p.embed_url ? '<span class="vod-pull">⇩ pull</span>' : ""}
      </div>`,
        )
        .join("")
    : '<div class="empty sm">No video posts.</div>';
  el.innerHTML = `<h2 class="cd-section-title">Posts</h2><div class="vod-list">${rows}</div>`;
  el.querySelectorAll("[data-action=patreon-pull]").forEach((row) => {
    if (!row.dataset.embed) return;
    row.addEventListener("click", async () => {
      try {
        await API.patreonPull({
          embed_url: row.dataset.embed,
          creator_name: row.dataset.creator,
          post_title: row.dataset.title,
        });
        row.querySelector(".vod-pull")?.replaceChildren(document.createTextNode("queued ✓"));
        Toast.success(`Pull queued — ${row.dataset.title}`);
      } catch (e) {
        Toast.error(`Pull failed: ${e.message}`);
      }
    });
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
    Toast.success(
      active
        ? `Stopped bulk download — ${ds.channelName}`
        : `Bulk download started — ${ds.channelName}`,
    );
    if (currentRoute() === "library") render();
  } catch (e) {
    Toast.error(`Bulk download failed: ${e.message}`);
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
    Toast.error(`Couldn't load playlists: ${e.message}`);
  }
}

// ── Add-Channel two-phase wizard (item 19) ───────────────────────────
// Phase 1: pick platform + type a name → resolve (live, via SSE).
// Phase 2: show the resolved channel → confirm → enable auto-record.
// Config is deferred until the entity is confirmed.
let addWizard = null; // { platform, query } while a resolve is in flight

function openAddChannelWizard() {
  let modal = document.getElementById("add-channel-modal");
  if (!modal) {
    modal = document.createElement("div");
    modal.id = "add-channel-modal";
    modal.className = "kbd-help";
    document.body.appendChild(modal);
    modal.addEventListener("click", (e) => {
      if (e.target === modal) modal.classList.remove("open");
    });
  }
  paintAddWizardSearch(modal);
  modal.classList.add("open");
}

function paintAddWizardSearch(modal, opts = {}) {
  modal = modal || document.getElementById("add-channel-modal");
  if (!modal) return;
  const plat = opts.platform || "Twitch";
  const sel = (p) => (p === plat ? " selected" : "");
  modal.innerHTML = `
    <div class="card">
      <h2>Add channel</h2>
      <p class="wizard-step">Step 1 of 2 — find the channel</p>
      <div class="wizard-row">
        <select id="aw-platform">
          <option value="Twitch"${sel("Twitch")}>Twitch</option>
          <option value="YouTube"${sel("YouTube")}>YouTube</option>
          <option value="Patreon"${sel("Patreon")}>Patreon</option>
        </select>
        <input id="aw-query" type="text" placeholder="Twitch login, or YouTube/Patreon id"
               value="${escape(opts.query || "")}" autofocus />
        <button id="aw-search" class="primary">Search</button>
      </div>
      <div id="aw-result" class="wizard-result">${opts.message || ""}</div>
    </div>`;
  const doSearch = async () => {
    const platform = modal.querySelector("#aw-platform").value;
    const query = modal.querySelector("#aw-query").value.trim();
    if (!query) return;
    addWizard = { platform, query };
    modal.querySelector("#aw-result").innerHTML = '<div class="empty sm">Searching…</div>';
    try {
      await API.resolveChannel(platform, query);
    } catch (e) {
      modal.querySelector("#aw-result").innerHTML = `<div class="empty sm">Search failed: ${escape(e.message)}</div>`;
    }
  };
  modal.querySelector("#aw-search")?.addEventListener("click", doSearch);
  modal.querySelector("#aw-query")?.addEventListener("keydown", (e) => {
    if (e.key === "Enter") doSearch();
  });
}

// Phase 2: render the resolved entity for confirmation (called from the
// ChannelResolved SSE handler).
function paintAddWizardConfirm(ev) {
  const modal = document.getElementById("add-channel-modal");
  if (!modal || !modal.classList.contains("open") || !addWizard) return;
  if (ev.platform !== addWizard.platform || ev.query !== addWizard.query) return;
  const result = modal.querySelector("#aw-result");
  if (!result) return;
  if (ev.error || !ev.channel_id) {
    result.innerHTML = `<div class="empty sm">Not found: ${escape(ev.error || "no match")}</div>`;
    return;
  }
  const name = ev.display_name || ev.channel_id;
  result.innerHTML = `
    <div class="wizard-confirm">
      <p class="wizard-step">Step 2 of 2 — confirm</p>
      <div class="task-row">
        <div class="task-info">
          <span class="task-name">${escape(name)}</span>
          <span class="task-cadence">${escape(ev.platform)} · ${escape(ev.channel_id)}</span>
        </div>
      </div>
      <button id="aw-confirm" class="primary" data-key="${escape(ev.platform)}:${escape(ev.channel_id)}">
        Add &amp; enable auto-record
      </button>
    </div>`;
  result.querySelector("#aw-confirm")?.addEventListener("click", async (e) => {
    const key = e.currentTarget.dataset.key;
    try {
      await API.toggleAutoRecord(key, true);
      Toast.success(`Added ${name} — auto-record on`);
      modal.classList.remove("open");
      addWizard = null;
      if (currentRoute() === "library") render();
    } catch (err) {
      Toast.error(`Add failed: ${err.message}`);
    }
  });
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
        Toast.success(`Bulk download started — ${ch.name}`);
        if (currentRoute() === "library") render();
      } catch (e) {
        Toast.error(`Bulk download failed: ${e.message}`);
      }
    });
  });
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
    Toast.success(
      `Recording ${d.fromStart === "true" ? "from start " : ""}— ${d.displayName || d.channelName}`,
    );
  } catch (e) {
    Toast.error(`Start failed: ${e.message}`);
  }
}

async function toggleAutoRecord(d) {
  const enabling = d.enabled === "true";
  try {
    await API.toggleAutoRecord(d.channelKey, enabling);
    Toast.success(enabling ? "Auto-record enabled" : "Auto-record disabled");
    await render();
  } catch (e) {
    Toast.error(`Auto-record toggle failed: ${e.message}`);
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
  recCache = recordings;
  // W4-alt: sortable + filterable data grid. Column headers toggle sort;
  // the filter box narrows by channel/title live without refetching.
  root.innerHTML = chrome(`
    <h1 class="page-title">Recordings</h1>
    <input id="rec-filter" class="grid-filter" type="search"
           placeholder="Filter by channel or title… (/)"
           aria-label="Filter recordings" value="${escape(recFilter)}">
    <p class="page-subtitle" id="rec-count"></p>
    <table class="recordings-table">
      <thead>
        <tr>
          ${recHeader("state", "State")}
          ${recHeader("channel", "Channel")}
          ${recHeader("title", "Title")}
          ${recHeader("started", "Started")}
          ${recHeader("size", "Size")}
          <th></th>
        </tr>
      </thead>
      <tbody id="rec-body"></tbody>
    </table>
  `);
  setupChromeHandlers();
  paintRecordings();

  document.getElementById("rec-filter")?.addEventListener("input", (e) => {
    recFilter = e.target.value;
    paintRecordings();
  });
  document.querySelectorAll("th[data-sort]").forEach((th) => {
    th.addEventListener("click", () => {
      const col = th.dataset.sort;
      if (recSort.col === col) {
        recSort.dir = recSort.dir === "asc" ? "desc" : "asc";
      } else {
        recSort = { col, dir: "asc" };
      }
      renderRecordings(); // re-render header arrows + body
    });
  });
}

function recHeader(key, label) {
  const arrow =
    recSort.col === key ? (recSort.dir === "asc" ? " ▲" : " ▼") : "";
  return `<th data-sort="${key}" style="cursor:pointer">${label}${arrow}</th>`;
}

// Apply the live filter + sort to recCache and repaint the table body.
function paintRecordings() {
  const body = document.getElementById("rec-body");
  if (!body) return;
  const q = recFilter.trim().toLowerCase();
  let rows = recCache.filter((r) => {
    if (!q) return true;
    return (
      (r.channel_name || "").toLowerCase().includes(q) ||
      (r.stream_title || "").toLowerCase().includes(q)
    );
  });
  const dir = recSort.dir === "asc" ? 1 : -1;
  const key = (r) => {
    switch (recSort.col) {
      case "state": return stateLabel(r.state).toLowerCase();
      case "channel": return (r.channel_name || "").toLowerCase();
      case "title": return (r.stream_title || "").toLowerCase();
      case "size": return r.bytes_written || 0;
      case "started":
      default: return new Date(r.started_at).getTime() || 0;
    }
  };
  rows.sort((a, b) => {
    const ka = key(a), kb = key(b);
    return ka < kb ? -dir : ka > kb ? dir : 0;
  });
  body.innerHTML = rows.map(recordingRow).join("");
  const count = document.getElementById("rec-count");
  if (count) {
    count.textContent =
      q || rows.length !== recCache.length
        ? `${rows.length} of ${recCache.length}`
        : `${recCache.length} total`;
  }
  body.querySelectorAll("[data-action=stop]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      if (!(await confirmDialog("Stop this recording?", { ok: "Stop", danger: true })))
        return;
      try {
        await API.stopRecording(btn.dataset.jobId);
        Toast.success("Recording stopped");
        setTimeout(render, 500);
      } catch (e) {
        Toast.error(`Stop failed: ${e.message}`);
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
        Toast.success(`${btn.dataset.plugin}: ${btn.dataset.verb} — ${r.note ? "dispatched" : "queued"}`);
      } catch (e) {
        Toast.error(`Plugin RPC failed: ${e.message}`);
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

// ── Settings (item 7) — real, domain-grouped read of the daemon config.
// Editing still lives in the TUI / config.toml; this surfaces the live
// configuration so the page is informative rather than a stub.
async function renderSettings() {
  let s = {};
  try {
    s = await API.settings();
  } catch (e) {
    if (e.message.includes("unauthorized")) return;
  }
  root.removeAttribute("aria-busy");

  const yesno = (b) => (b ? "yes" : "no");
  const badge = (ok, okText, noText) =>
    `<span class="cfg-badge ${ok ? "ok" : "warn"}">${ok ? okText : noText}</span>`;
  const rec = s.recording || {};
  const arc = s.archiver || {};
  const ui = s.ui || {};

  const card = (title, rows) => `
    <section class="cfg-card">
      <h2 class="cfg-title">${title}</h2>
      <dl class="cfg-list">${rows}</dl>
    </section>`;
  const kv = (k, v) => `<dt>${escape(k)}</dt><dd>${v}</dd>`;

  root.innerHTML = chrome(`
    <h1 class="page-title">Settings</h1>
    <p class="page-subtitle">Live daemon configuration. Edit via the TUI or <code>~/.config/strivo/config.toml</code>.</p>
    <div class="cfg-grid">
      ${card("Platforms", [
        kv("Twitch", badge(s.twitch_configured, "configured", "not configured")),
        kv("YouTube", badge(s.youtube_configured, "configured", "not configured")),
        kv("Patreon", badge(s.patreon_configured, "configured", "not configured")),
        kv("Auto-record channels", `${(s.auto_record_channels || []).length}`),
        kv("Poll interval", `${s.poll_interval_secs ?? "?"}s`),
      ].join(""))}
      ${card("Recording", [
        kv("Directory", `<code>${escape(s.recording_dir || "?")}</code>`),
        kv("Filename template", `<code>${escape(rec.filename_template || "?")}</code>`),
        kv("Transcode", yesno(rec.transcode)),
        kv("Twitch from-start", yesno(rec.twitch_live_from_start)),
        kv("Auto VOD backfill", yesno(rec.auto_vod_backfill)),
        kv("Auto-trim ads", yesno(rec.auto_trim_ads)),
      ].join(""))}
      ${card("Plugins", [
        kv("Archiver", badge(arc.enabled, "enabled", "disabled")),
        kv("Archiver dir", `<code>${escape(arc.archive_dir || "—")}</code>`),
        kv("Archiver format", escape(arc.format || "—")),
        kv("Concurrent fragments", `${arc.concurrent_fragments ?? "—"}`),
      ].join(""))}
      ${card("Interface", [
        kv("Reduce motion", yesno(ui.reduce_motion)),
        kv("Verbose status", yesno(ui.verbose_status)),
        kv("Scheduled recordings", `${(s.schedule || []).length}`),
      ].join(""))}
    </div>
  `);
  setupChromeHandlers();
}

// ── System (item 7) — version, daemon connectivity, severity-tiered
// health checks, disk gauge, tasks. (research §E)
async function renderSystem() {
  const [health, storage, checksResp, settings] = await Promise.all([
    API.health().catch(() => null),
    API.storage().catch(() => null),
    API.healthChecks().catch(() => null),
    API.settings().catch(() => null),
  ]);
  root.removeAttribute("aria-busy");

  // Server-side health-check registry is the single source of truth
  // (roadmap item 13): {domain, name, severity, message, fix}.
  const serverChecks = (checksResp && checksResp.checks) || [
    { domain: "Network", name: "Daemon IPC", severity: "error", message: "not reachable", fix: "" },
  ];
  const checks = serverChecks.map((c) => ({ sev: c.severity, label: c.name, msg: c.message }));
  const activeRec = recCache.filter((r) => isInProgress(r.state)).length;

  const sevGlyph = { ok: "✓", warn: "▲", error: "✕" };
  // Group rows by domain so related checks (Storage / Platform Auth /
  // Network) sit together, each with its remediation hint.
  const domains = [...new Set(serverChecks.map((c) => c.domain))];
  const healthRows = domains
    .map((domain) => {
      const rows = serverChecks
        .filter((c) => c.domain === domain)
        .map(
          (c) => `
    <div class="sys-check ${c.severity}">
      <span class="sys-sev">${sevGlyph[c.severity] || "•"}</span>
      <span class="sys-label">${escape(c.name)}</span>
      <span class="sys-msg">${escape(c.message)}${c.fix ? ` <span class="sys-fix">— ${escape(c.fix)}</span>` : ""}</span>
    </div>`,
        )
        .join("");
      return `<div class="sys-domain"><h3 class="sys-domain-title">${escape(domain)}</h3>${rows}</div>`;
    })
    .join("");

  // Disk gauge.
  const gauge = storage && storage.filesystem_total_bytes
    ? (() => {
        const usedPct = (1 - storage.filesystem_avail_bytes / storage.filesystem_total_bytes) * 100;
        return `<div class="sys-gauge"><div class="sys-gauge-fill" style="width:${usedPct.toFixed(1)}%"></div></div>
          <div class="sys-gauge-label">${formatBytes(storage.bytes_used_by_recordings || 0)} recordings ·
          ${formatBytes(storage.filesystem_avail_bytes)} free of ${formatBytes(storage.filesystem_total_bytes)}</div>`;
      })()
    : '<div class="empty sm">Disk stats unavailable</div>';

  const worst = checks.some((c) => c.sev === "error")
    ? "error"
    : checks.some((c) => c.sev === "warn")
    ? "warn"
    : "ok";

  root.innerHTML = chrome(`
    <h1 class="page-title">System</h1>
    <p class="page-subtitle">StriVo v${health ? escape(health.version || "?") : "?"} ·
      overall <span class="cfg-badge ${worst === "ok" ? "ok" : worst === "warn" ? "warn" : "err"}">${worst}</span></p>
    <div class="cfg-grid">
      <section class="cfg-card">
        <h2 class="cfg-title">Health</h2>
        <div class="sys-checks">${healthRows}</div>
      </section>
      <section class="cfg-card">
        <h2 class="cfg-title">Storage</h2>
        ${gauge}
      </section>
      <section class="cfg-card" id="backup-card">
        <h2 class="cfg-title">Backup</h2>
        <div class="task-row">
          <div class="task-info">
            <span class="task-name">Config + jobs DB</span>
            <span class="task-cadence">on-demand snapshot</span>
          </div>
          <button id="backup-now" class="sm">＋ Backup now</button>
        </div>
        <div id="backup-list"><div class="empty sm">Loading backups…</div></div>
      </section>
      <section class="cfg-card" id="blocklist-card">
        <h2 class="cfg-title">Blocklist</h2>
        <div id="blocklist-list"><div class="empty sm">Loading blocklist…</div></div>
      </section>
      <section class="cfg-card">
        <h2 class="cfg-title">Tasks</h2>
        <div class="task-row">
          <div class="task-info">
            <span class="task-name">Channel poll</span>
            <span class="task-cadence">every ${settings ? settings.poll_interval_secs : "?"}s</span>
          </div>
          <button id="task-poll-now" class="sm" title="Run the channel poll now">↻ Run now</button>
        </div>
        ${(settings && settings.schedule && settings.schedule.length
          ? settings.schedule
          : []
        )
          .map(
            (s) => `
        <div class="task-row">
          <div class="task-info">
            <span class="task-name">⏱ ${escape(s.channel || "scheduled")}</span>
            <span class="task-cadence">${escape(s.cron || "")}${s.duration ? ` · ${escape(s.duration)}` : ""}</span>
          </div>
        </div>`,
          )
          .join("")}
        <div class="task-row">
          <div class="task-info">
            <span class="task-name">Active recordings</span>
            <span class="task-cadence">${activeRec} running${activeRec ? " · stop from the dashboard" : ""}</span>
          </div>
          ${activeRec ? '<a class="sm" href="#/library">View</a>' : ""}
        </div>
      </section>
    </div>
  `);
  setupChromeHandlers();
  // Run-now duality: poll task enqueues the same command as the scheduled poll.
  document.getElementById("task-poll-now")?.addEventListener("click", async (e) => {
    const btn = e.currentTarget;
    btn.disabled = true;
    try {
      await API.pollNow();
      Toast.success("Channel poll triggered");
    } catch (err) {
      Toast.error(`Poll failed: ${err.message}`);
    } finally {
      btn.disabled = false;
    }
  });
  // Backup/restore (item 16).
  document.getElementById("backup-now")?.addEventListener("click", async (e) => {
    await withBusy(e.currentTarget, "Backing up…", async () => {
      const r = await API.backupCreate();
      Toast.success(`Backup created — ${r.name}`);
      await paintBackups();
    }).catch((err) => Toast.error(`Backup failed: ${err.message}`));
  });
  paintBackups();
  paintBlocklist();
}

async function paintBlocklist() {
  const el = document.getElementById("blocklist-list");
  if (!el) return;
  try {
    const r = await API.blocklist();
    const rows = r.blocklist || [];
    if (!rows.length) {
      el.innerHTML = '<div class="empty sm">Nothing blocked.</div>';
      return;
    }
    el.innerHTML = rows
      .map((b) => {
        const scope = b.vod_id ? `VOD ${escape(b.vod_id)}` : "whole channel";
        return `
      <div class="task-row">
        <div class="task-info">
          <span class="task-name">${escape(b.platform)} · ${escape(b.channel_id)}</span>
          <span class="task-cadence">${scope}${b.reason ? ` · ${escape(b.reason)}` : ""}</span>
        </div>
        <button class="sm unblock" data-platform="${escape(b.platform)}"
                data-channel="${escape(b.channel_id)}" data-vod="${escape(b.vod_id || "")}">Unblock</button>
      </div>`;
      })
      .join("");
    el.querySelectorAll(".unblock").forEach((btn) => {
      btn.addEventListener("click", async () => {
        const d = btn.dataset;
        try {
          await API.blockRemove({
            platform: d.platform,
            channel_id: d.channel,
            vod_id: d.vod || null,
          });
          Toast.success("Unblocked");
          paintBlocklist();
        } catch (e) {
          Toast.error(`Unblock failed: ${e.message}`);
        }
      });
    });
  } catch (e) {
    el.innerHTML = `<div class="empty sm">Could not load blocklist: ${escape(e.message)}</div>`;
  }
}

async function paintBackups() {
  const el = document.getElementById("backup-list");
  if (!el) return;
  try {
    const r = await API.backups();
    const rows = r.backups || [];
    if (!rows.length) {
      el.innerHTML = '<div class="empty sm">No backups yet.</div>';
      return;
    }
    el.innerHTML = rows
      .map(
        (b) => `
      <div class="task-row">
        <div class="task-info">
          <span class="task-name">${escape(b.name)}</span>
          <span class="task-cadence">${formatBytes(b.bytes || 0)} · ${(b.files || []).join(", ")}</span>
        </div>
        <button class="sm restore-backup" data-name="${escape(b.name)}">Restore</button>
      </div>`,
      )
      .join("");
    el.querySelectorAll(".restore-backup").forEach((btn) => {
      btn.addEventListener("click", async () => {
        const name = btn.dataset.name;
        if (
          !(await confirmDialog(
            `Restore config + jobs DB from ${name}? This overwrites the current files; restart the daemon to apply.`,
            { ok: "Restore", danger: true },
          ))
        )
          return;
        try {
          const res = await API.backupRestore(name);
          Toast.success(`Restored ${(res.restored || []).join(", ")} — restart the daemon to apply`);
        } catch (err) {
          Toast.error(`Restore failed: ${err.message}`);
        }
      });
    });
  } catch (e) {
    el.innerHTML = `<div class="empty sm">Could not load backups: ${escape(e.message)}</div>`;
  }
}

// ── Logs viewer (item 15) — tails the rolling log with a level selector. ──
let logsLevel = "info";
async function renderLogs() {
  const levels = ["error", "warn", "info", "debug", "trace"];
  const options = levels
    .map((l) => `<option value="${l}"${l === logsLevel ? " selected" : ""}>${l.toUpperCase()}</option>`)
    .join("");
  root.innerHTML = chrome(`
    <h1 class="page-title">Logs</h1>
    <div class="logs-toolbar">
      <label>Min level
        <select id="logs-level">${options}</select>
      </label>
      <button id="logs-refresh" class="sm" title="Reload">↻ Refresh</button>
      <span id="logs-file" class="logs-file"></span>
    </div>
    <pre id="logs-output" class="logs-output" aria-live="polite">Loading…</pre>
  `);
  setupChromeHandlers();

  async function load() {
    const out = document.getElementById("logs-output");
    const fileEl = document.getElementById("logs-file");
    try {
      const r = await API.logs(logsLevel, 500);
      const lines = r.lines || [];
      out.textContent = lines.length ? lines.join("\n") : "No log lines at this level.";
      if (fileEl) fileEl.textContent = r.file ? `· ${r.file} · ${lines.length} lines` : "";
      out.scrollTop = out.scrollHeight;
    } catch (e) {
      out.textContent = `Failed to load logs: ${e.message}`;
    }
  }
  document.getElementById("logs-level")?.addEventListener("change", (e) => {
    logsLevel = e.target.value;
    load();
  });
  document.getElementById("logs-refresh")?.addEventListener("click", load);
  await load();
}

// ── Upcoming agenda (item 18) — first-class calendar of known upcoming
// recordings. Source = scheduled (cron) entries with their server-computed
// next_fire. (Platform-side scheduled broadcasts aren't available via API.) ──
function dayBucket(d) {
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const that = new Date(d.getFullYear(), d.getMonth(), d.getDate());
  const diff = Math.round((that - today) / 86400000);
  if (diff === 0) return "Today";
  if (diff === 1) return "Tomorrow";
  return d.toLocaleDateString(undefined, { weekday: "long", month: "short", day: "numeric" });
}

async function renderSchedule() {
  let entries = [];
  try {
    const r = await API.schedule();
    entries = r.schedule || [];
  } catch (_) {}
  root.removeAttribute("aria-busy");

  const dated = entries
    .filter((e) => e.next_fire)
    .map((e) => ({ ...e, when: new Date(e.next_fire) }))
    .sort((a, b) => a.when - b.when);
  const undated = entries.filter((e) => !e.next_fire);

  // Group by day bucket, preserving sorted order.
  const groups = [];
  for (const e of dated) {
    const label = dayBucket(e.when);
    let g = groups.find((x) => x.label === label);
    if (!g) {
      g = { label, items: [] };
      groups.push(g);
    }
    g.items.push(e);
  }

  const row = (e) => `
    <div class="task-row">
      <div class="task-info">
        <span class="task-name">${escape(e.channel || "scheduled")}</span>
        <span class="task-cadence">${escape(e.cron || "")}${e.duration ? ` · ${escape(e.duration)}` : ""}</span>
      </div>
      <span class="agenda-time">${e.when ? e.when.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : ""}</span>
    </div>`;

  const groupsHtml = groups
    .map(
      (g) => `
    <section class="cfg-card">
      <h2 class="cfg-title">${escape(g.label)}</h2>
      ${g.items.map(row).join("")}
    </section>`,
    )
    .join("");

  const undatedHtml = undated.length
    ? `<section class="cfg-card">
         <h2 class="cfg-title">Unscheduled</h2>
         ${undated.map((e) => `<div class="task-row"><div class="task-info"><span class="task-name">${escape(e.channel || "")}</span><span class="task-cadence">${escape(e.cron || "")} · unparsed cron</span></div></div>`).join("")}
       </section>`
    : "";

  const empty = !entries.length
    ? '<div class="empty">No scheduled recordings. Add a schedule entry in config.toml.</div>'
    : "";

  root.innerHTML = chrome(`
    <h1 class="page-title">Schedule</h1>
    <p class="page-subtitle">Upcoming scheduled recordings · ${dated.length} upcoming</p>
    ${empty}
    <div class="cfg-grid">${groupsHtml}${undatedHtml}</div>
  `);
  setupChromeHandlers();
}

// ── Durable History (item 17) — completed/failed audit from the jobs DB,
// survives restarts (unlike the in-memory /recordings snapshot). ──
async function renderHistory() {
  let rows = [];
  try {
    const r = await API.history();
    rows = r.history || [];
  } catch (_) {}
  root.removeAttribute("aria-busy");
  const body = rows.length
    ? rows
        .map((j) => {
          const when = j.started_at
            ? new Date(j.started_at).toLocaleString()
            : "—";
          return `
      <tr>
        <td><span class="state-pill ${stateClassName(j.state)}">${escape(stateLabel(j.state))}</span></td>
        <td>${escape(j.channel_name || "")}</td>
        <td>${escape(j.stream_title || "")}</td>
        <td>${escape(when)}</td>
        <td>${formatBytes(j.bytes_written || 0)}</td>
      </tr>`;
        })
        .join("")
    : `<tr><td colspan="5" class="empty sm">No recording history yet.</td></tr>`;
  root.innerHTML = chrome(`
    <h1 class="page-title">History</h1>
    <p class="page-subtitle">Durable record of every capture (survives restarts) · ${rows.length} entries</p>
    <table class="recordings-table">
      <thead><tr><th>State</th><th>Channel</th><th>Title</th><th>Started</th><th>Size</th></tr></thead>
      <tbody>${body}</tbody>
    </table>
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
  // ⌘K / Ctrl+K — command palette. Handled before the input guard so it
  // works from anywhere, including while a field is focused. (W4-alt.)
  if ((e.metaKey || e.ctrlKey) && (e.key === "k" || e.key === "K")) {
    e.preventDefault();
    toggleCommandPalette();
    return;
  }
  // If the palette is open, it owns the keyboard.
  if (document.getElementById("cmdk")?.classList.contains("open")) {
    handleCmdkKey(e);
    return;
  }

  // Don't intercept while typing in an input.
  const tag = (e.target.tagName || "").toLowerCase();
  if (tag === "input" || tag === "textarea") return;
  if (e.ctrlKey || e.metaKey || e.altKey) return;

  // `/` focuses the recordings filter when on that route.
  if (e.key === "/" && currentRoute() === "recordings") {
    const f = document.getElementById("rec-filter");
    if (f) {
      e.preventDefault();
      f.focus();
      return;
    }
  }

  if (e.key === "?") {
    e.preventDefault();
    document.getElementById("kbd-help")?.classList.add("open");
    return;
  }
  if (e.key === "Escape") {
    document.getElementById("kbd-help")?.classList.remove("open");
    if (selectedChannelKey) {
      selectedChannelKey = null;
      render();
    }
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
    const link = document.querySelector(`.topnav a[data-key="${e.key}"]`);
    if (link) {
      e.preventDefault();
      route(link.dataset.route);
    }
  }
});

// ── ⌘K command palette (W4-alt) ───────────────────────────────────────
let cmdkItems = [];
let cmdkSelected = 0;

function commandList() {
  const nav = [
    ["library", "Go to Home"],
    ["recordings", "Go to Recordings"],
    ["schedule", "Go to Schedule"],
    ["pipelines", "Go to Pipelines"],
    ["plugins", "Go to Plugins"],
    ["settings", "Go to Settings"],
    ["system", "Go to System"],
  ].map(([r, label]) => ({ label, run: () => route(r) }));
  const actions = [
    { label: "Poll channels now", run: () => API.pollNow().catch(() => {}) },
    {
      label: "Stop all recordings",
      run: () => API._fetch("/recordings/stop_all", { method: "POST" }).catch(() => {}),
    },
    { label: "Logout", run: () => API.logout().then(() => route("login")) },
  ];
  return [...nav, ...actions];
}

function toggleCommandPalette() {
  let el = document.getElementById("cmdk");
  if (!el) {
    el = document.createElement("div");
    el.id = "cmdk";
    el.className = "kbd-help";
    el.innerHTML = `
      <div class="card">
        <input id="cmdk-input" class="grid-filter" type="text"
               placeholder="Type a command…" autocomplete="off" aria-label="Command palette">
        <div id="cmdk-list" class="pl-list"></div>
      </div>`;
    document.body.appendChild(el);
    el.addEventListener("click", (ev) => {
      if (ev.target === el) el.classList.remove("open");
    });
    el.querySelector("#cmdk-input").addEventListener("input", paintCmdk);
  }
  const open = el.classList.toggle("open");
  if (open) {
    cmdkSelected = 0;
    const input = el.querySelector("#cmdk-input");
    input.value = "";
    paintCmdk();
    input.focus();
  }
}

function paintCmdk() {
  const q = (document.getElementById("cmdk-input")?.value || "")
    .trim()
    .toLowerCase();
  const all = commandList();
  cmdkItems = q
    ? all.filter((c) => c.label.toLowerCase().includes(q))
    : all;
  if (cmdkSelected >= cmdkItems.length) cmdkSelected = 0;
  const list = document.getElementById("cmdk-list");
  if (!list) return;
  list.innerHTML = cmdkItems
    .map(
      (c, i) =>
        `<div class="pl-row ${i === cmdkSelected ? "sel" : ""}" data-i="${i}">${escape(
          c.label,
        )}</div>`,
    )
    .join("");
  list.querySelectorAll(".pl-row").forEach((row) => {
    row.addEventListener("click", () => runCmdk(parseInt(row.dataset.i, 10)));
  });
}

function handleCmdkKey(e) {
  const el = document.getElementById("cmdk");
  if (e.key === "Escape") {
    e.preventDefault();
    el.classList.remove("open");
  } else if (e.key === "ArrowDown") {
    e.preventDefault();
    cmdkSelected = Math.min(cmdkSelected + 1, cmdkItems.length - 1);
    paintCmdk();
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    cmdkSelected = Math.max(cmdkSelected - 1, 0);
    paintCmdk();
  } else if (e.key === "Enter") {
    e.preventDefault();
    runCmdk(cmdkSelected);
  }
}

function runCmdk(i) {
  const item = cmdkItems[i];
  document.getElementById("cmdk")?.classList.remove("open");
  if (item) item.run();
}

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
        <dt>⌘K</dt><dd>Command palette</dd>
        <dt>/</dt><dd>Filter recordings</dd>
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
events.on((event) => {
  const onHome = currentRoute() === "library";

  // Surgical updates only — NEVER full renderHome on background events.
  // renderHome rebuilds the whole page (chrome + rail + channel-detail
  // iframe), so doing it on the ~2s RecordingProgress stream reloaded the
  // live preview and reset the rail scroll. Each handler now touches the
  // smallest subtree: paintChannelList (rail, scroll-preserved) or
  // paintDashboard (#dash only), leaving the detail iframe untouched.

  if (event.ChannelsUpdated) {
    channelCache = event.ChannelsUpdated;
    paintChannelList();
  }
  if (event.ChannelWentLive || event.ChannelWentOffline) {
    // Refetch so the new live state (and ordering) is reflected.
    API.channels()
      .then((d) => {
        channelCache = d.channels || [];
        paintChannelList();
      })
      .catch(() => {});
  }

  // High-frequency progress: update the in-memory job + the dashboard
  // subtree in place. No rail/detail rebuild.
  if (event.RecordingProgress) {
    const p = event.RecordingProgress;
    const j = recCache.find((r) => r.id === p.job_id);
    if (j) {
      j.bytes_written = p.bytes_written;
      j.duration_secs = p.duration_secs;
    }
    updateLiveCount(recCache.filter((r) => isInProgress(r.state)).length);
    if (currentRoute() === "recordings") paintRecordings();
    else paintDashboard();
  }

  // Lifecycle state changes (rare): refetch recordings, refresh the
  // dashboard + rail rec-dots, without rebuilding the detail.
  if (event.RecordingStarted || event.RecordingFinished || event.AllRecordingsStopped) {
    API.recordings()
      .then((d) => {
        recCache = d.recordings || [];
        dashRecordings = recCache;
        updateLiveCount(recCache.filter((r) => isInProgress(r.state)).length);
        if (currentRoute() === "recordings") renderRecordings().catch(() => {});
        else {
          paintDashboard();
          paintChannelList();
        }
      })
      .catch(() => {});
  }

  // #74 — bulk-download progress: update state + the rail bulk badge only.
  if (event.BulkProgress) {
    const p = event.BulkProgress;
    if (p.active) {
      bulkStatus[p.channel_id] = { done: p.done, total: p.total, active: true };
    } else {
      delete bulkStatus[p.channel_id];
    }
    paintChannelList();
  }

  // #75 — Patreon snapshot feeds the channel list + Patreon detail.
  if (event.PatreonState) {
    const ps = event.PatreonState;
    patreonState.creators = ps.creators || [];
    patreonState.posts = {};
    for (const post of ps.posts || []) {
      (patreonState.posts[post.campaign_id] ||= []).push(post);
    }
    for (const list of Object.values(patreonState.posts)) {
      list.sort((a, b) => (b.published_at || "").localeCompare(a.published_at || ""));
    }
    paintChannelList();
    // Refresh an open Patreon detail.
    if (onHome && selectedChannelKey && selectedChannelKey.startsWith("Patreon:")) {
      const id = selectedChannelKey.slice("Patreon:".length);
      const c = patreonState.creators.find((x) => x.id === id);
      if (c) renderPatreonPosts(c);
    }
  }

  // Channel VODs answer the detail-pane request.
  if (event.ChannelVods) {
    const cv = event.ChannelVods;
    channelVods[cv.channel_id] = cv.vods || [];
    if (onHome && selectedChannelKey && selectedChannelKey.endsWith(`:${cv.channel_id}`)) {
      const platform = selectedChannelKey.split(":")[0];
      paintChannelVods(cv.channel_id, platform);
    }
  }

  // #19 — Add-Channel wizard resolve reply.
  if (event.ChannelResolved) {
    paintAddWizardConfirm(event.ChannelResolved);
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
// Seed Patreon from the daemon snapshot before first paint, then render,
// so the Patreon section is populated on load (not after the next poll).
seedPatreon().finally(render);
