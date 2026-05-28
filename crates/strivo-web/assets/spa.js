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
  recordingOne: (id) =>
    API._fetch(`/recordings/${encodeURIComponent(id)}`),
  recordingProbe: (id) =>
    API._fetch(`/recordings/${encodeURIComponent(id)}/probe`),
  deleteRecordingFile: (id) =>
    API._fetch(`/recordings/${encodeURIComponent(id)}/file`, { method: "DELETE" }),
  clearErroredRecordings: () =>
    API._fetch("/recordings/clear_errored", { method: "POST" }),
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
  setPollInterval: (secs) =>
    API._fetch("/settings/poll_interval", { method: "POST", body: { secs } }),
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
  // ── Plugin data (read-only, served from each plugin's SQLite DB) ──
  plugins: () => API._fetch("/plugins"),
  crunchrRecordings: () => API._fetch("/plugins/crunchr/recordings"),
  crunchrRecording: (id) =>
    API._fetch(`/plugins/crunchr/recordings/${encodeURIComponent(id)}`),
  crunchrSearch: (q) =>
    API._fetch(`/plugins/crunchr/search?q=${encodeURIComponent(q)}`),
  archiverChannels: () => API._fetch("/plugins/archiver/channels"),
  archiverVideos: (channelId) =>
    API._fetch(`/plugins/archiver/channels/${encodeURIComponent(channelId)}/videos`),
  viewguardVerdicts: () => API._fetch("/plugins/viewguard/verdicts"),
  viewguardSamples: (channelId) =>
    API._fetch(`/plugins/viewguard/channels/${encodeURIComponent(channelId)}/samples`),
  insightsWords: (opts = {}) => {
    const p = new URLSearchParams();
    if (opts.scope) p.set("scope", opts.scope);
    if (opts.recording) p.set("recording", opts.recording);
    if (opts.stopwords) p.set("stopwords", "true");
    if (opts.limit) p.set("limit", String(opts.limit));
    return API._fetch(`/plugins/insights/words?${p.toString()}`);
  },
  insightsTopics: () => API._fetch("/plugins/insights/topics"),
  insightsSpeakers: (id) =>
    API._fetch(`/plugins/insights/recordings/${encodeURIComponent(id)}/speakers`),
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
  vodDownload: (body) =>
    API._fetch("/vods/download", { method: "POST", body }),
  login: (apiKey) =>
    API._fetch("/auth/login", { method: "POST", body: { api_key: apiKey } }),
  logout: () => API._fetch("/auth/logout", { method: "POST" }),
  // ── Strivo Pro licensing (Phase 1: status only; activate/trial 501) ──
  licenceStatus: () => API._fetch("/licence/status"),
  licenceTrial: () => API._fetch("/licence/trial", { method: "POST" }),
  licenceActivate: (key) =>
    API._fetch("/licence/activate", { method: "POST", body: { key } }),
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
      // auto-reconnects on transient errors; meanwhile we show a pill
      // and degrade to a slow poll so list views don't go stale.
      this.setConnected(false);
      // On a hard close (e.g. a 401 before login — /events is now
      // authenticated), EventSource will NOT auto-reconnect. Recreate it on
      // a timer so the stream comes back once the session cookie is set.
      if (this.source && this.source.readyState === EventSource.CLOSED) {
        this.source.close();
        this.source = null;
        setTimeout(() => this.start(), 3000);
      }
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
// Item 22 — recordings index density (compact|comfortable) + multi-select.
let recDensity = localStorage.getItem("strivo-rec-density") || "comfortable";
let recSelected = new Set();
// Anchor for shift+click range selection. Tracks the last row whose
// selection state was toggled by direct interaction (click on checkbox or
// modifier+click on row). Reset when the recordings page re-renders.
let recAnchorId = null;
// TUI-redesign — left-rail channel cache, current selection, per-channel
// VOD cache (channel_id -> [VodEntry]), and the recordings dashboard cache.
let channelCache = [];
let selectedChannelKey = null;
const channelVods = {};
// Per-VOD download state for the Past Broadcasts / Recent uploads pills.
// Keys: VOD URL. Values: "downloading" | "downloaded". Absence = idle.
// Seeded from recCache on every recordings refresh via
// `seedVodDownloadStateFromRecCache()` — correlation is by exact source_url
// match (RecordingJob.source_url, stamped on DownloadVod), so a page reload
// or a previously-finished download both surface correctly without a FIFO
// guess.
const vodDownloadState = {};
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
    if (polite && document.body.contains(polite)) return;
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
  // Pre-create the live regions at load so screen readers register them
  // BEFORE any message is injected — injecting a region and its content in
  // the same frame is unreliably announced (item 24).
  if (typeof document !== "undefined" && document.body) ensure();
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
async function withBusy(btn, busyLabel, fn, timeoutMs = 30000) {
  if (btn) {
    if (btn.dataset.busy === "1") return; // debounce double-submit
    btn.dataset.busy = "1";
    btn.setAttribute("aria-busy", "true");
    btn.classList.add("busy");
    if (busyLabel) {
      btn.dataset.prevLabel = btn.textContent;
      btn.textContent = busyLabel;
    }
  }
  // Never strand a spinner: race the work against a timeout so a hung
  // request still tears the busy state down and surfaces an error (item 25).
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error("timed out")), timeoutMs);
  });
  try {
    return await Promise.race([fn(), timeout]);
  } finally {
    clearTimeout(timer);
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
  // Sub-routes (e.g. #/plugins/crunchr) highlight their base tab.
  const base = hash.split("/")[0];
  return ROUTES.includes(base) ? base : "library";
}

// Path segments after the leading "#/", e.g. #/plugins/crunchr/rec/<id>
// → ["plugins", "crunchr", "rec", "<id>"].
function routeParts() {
  return window.location.hash
    .replace(/^#\/?/, "")
    .split("/")
    .filter(Boolean)
    .map((s) => decodeURIComponent(s));
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
      await renderPipelines();
      break;
    case "plugins":
      await renderPlugins();
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
// Tuple: [route, fallbackGlyph, label, key, iconHref?]
// Eight slots ship Eliver Lara's candy-icons (GPL-3.0, vendored under
// /assets/icons/candy/ with the upstream LICENSE + ATTRIBUTION). History
// keeps its Unicode glyph by the user's choice.
const TOPNAV = [
  ["library", "▣", "Home", "l", "/assets/icons/candy/home.svg"],
  ["recordings", "📁", "Recordings", "r", "/assets/icons/candy/recordings.svg"],
  ["schedule", "📅", "Schedule", "s", "/assets/icons/candy/schedule.svg"],
  ["pipelines", "🔁", "Pipelines", "d", "/assets/icons/candy/pipelines.svg"],
  ["plugins", "🧩", "Plugins", "g", "/assets/icons/candy/plugins.svg"],
  ["settings", "⚙", "Settings", "c", "/assets/icons/candy/settings.svg"],
  ["system", "🛠", "System", "y", "/assets/icons/candy/system.svg"],
  ["logs", "📜", "Logs", "o", "/assets/icons/candy/logs.svg"],
  ["history", "🗂", "History", "h"],
];

function chrome(content) {
  const r = currentRoute();
  const nav = TOPNAV.map(([route, glyph, label, key, iconHref]) => {
    const inner = iconHref
      ? `<img class="topnav-icon" src="${iconHref}" alt="" />`
      : `<span aria-hidden="true">${glyph}</span>`;
    return `<a class="topnav-link ${route === r ? "active" : ""}"
              href="#/${route}" data-route="${route}" data-key="${key}"
              title="${label}" aria-label="${label}">
            ${inner}
          </a>`;
  }).join("");
  return `
    <div class="chrome">
      <header class="topbar" role="banner">
        <a class="brand" href="#/library" id="brand-home" title="Home">StriVo</a>
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
        <button id="logout" title="Logout" aria-label="Sign out">
          <img class="topnav-icon" src="/assets/icons/candy/logout.svg" alt="" />
        </button>
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
  // Health pill — amber/red when any check is degraded (roadmap item 13).
  refreshHealthPill();
  // Channel list lives in the left rail on every page.
  paintChannelList();
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
    // Live → viewer count; offline Twitch/YT → "last live: N ago" in the same
    // slot (when StriVo has observed it live at least once).
    let viewers = "";
    if (c.is_live && c.viewer_count) {
      viewers = `<span class="ch-viewers">${formatCount(c.viewer_count)}</span>`;
    } else if (!c.is_live && !isPatreon && c.last_live_at) {
      viewers = `<span class="ch-lastlive" title="last live: ${escape(lastLiveLong(c.last_live_at))}">${escape(relTime(c.last_live_at))}</span>`;
    }
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
      events.start(); // (re)connect the now-authorized SSE stream
      route("library");
    } catch (err) {
      renderLogin("Invalid API key");
    }
  });
}

// ── Home: channel detail (if selected) + recordings dashboard ─────────
// First-run gate (item 20): a fresh install with no platform connected gets
// a guided setup checklist instead of an empty/half-configured dashboard.
// (Platform auth + config writes happen in the TUI/CLI, not the webui, so
// this screen reports live status and tells the user what to do.)
let firstRunDismissed = false;

function renderFirstRun(setup) {
  root.removeAttribute("aria-busy");
  const step = (done, label, detail) => `
    <div class="fr-step ${done ? "done" : "todo"}">
      <span class="fr-mark">${done ? "✓" : "○"}</span>
      <div class="fr-body">
        <div class="fr-label">${escape(label)}</div>
        <div class="fr-detail">${detail}</div>
      </div>
    </div>`;
  const plat = (name, ok) =>
    `<span class="fr-pill ${ok ? "ok" : ""}">${ok ? "✓" : "○"} ${escape(name)}</span>`;
  const anyPlatform =
    setup.twitch_configured || setup.youtube_configured || setup.patreon_configured;
  const recDir = setup.recording_dir || "(unset)";
  const chanCount = (setup.auto_record_channels || []).length;

  root.innerHTML = chrome(`
    <h1 class="page-title">Welcome to StriVo</h1>
    <p class="page-subtitle">Finish setup before the dashboard fills in.</p>
    <div class="cfg-card fr-card">
      ${step(
        anyPlatform,
        "1 · Connect a platform",
        `Authenticate Twitch / YouTube / Patreon by running <code>strivo</code>
         in a terminal (device-code login). Then re-check below.
         <div class="fr-pills">${plat("Twitch", setup.twitch_configured)}
           ${plat("YouTube", setup.youtube_configured)}
           ${plat("Patreon", setup.patreon_configured)}</div>`,
      )}
      ${step(
        !!setup.recording_dir,
        "2 · Recording directory",
        `Where captures are written: <code>${escape(recDir)}</code>.
         Change it in <code>~/.config/strivo/config.toml</code> if needed.`,
      )}
      ${step(
        chanCount > 0,
        "3 · Pick channels to record",
        `Use the <b>＋ Add</b> button (top bar) to find a channel and enable
         auto-record. ${chanCount} channel(s) configured so far.`,
      )}
      <div class="fr-actions">
        <button id="fr-recheck">↻ Re-check</button>
        <button id="fr-continue" class="primary">${anyPlatform ? "Continue to dashboard" : "Continue anyway"}</button>
      </div>
    </div>
  `);
  setupChromeHandlers();
  document.getElementById("fr-recheck")?.addEventListener("click", () => renderHome());
  document.getElementById("fr-continue")?.addEventListener("click", () => {
    firstRunDismissed = true;
    renderHome();
  });
}

async function renderHome() {
  let setup = null;
  try {
    setup = await API.settings();
  } catch (e) {
    if (e.message.includes("unauthorized")) return;
  }
  const anyPlatform =
    setup &&
    (setup.twitch_configured || setup.youtube_configured || setup.patreon_configured);
  if (setup && !anyPlatform && !firstRunDismissed) {
    renderFirstRun(setup);
    return;
  }
  // Refresh the channel + recordings caches that feed the left rail and
  // the dashboard. Both are cheap snapshots.
  try {
    const [ch, rec] = await Promise.all([API.channels(), API.recordings()]);
    channelCache = ch.channels || [];
    recCache = rec.recordings || [];
    dashRecordings = recCache;
    seedVodDownloadStateFromRecCache();
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

  // The recordings dashboard (In progress / Recent / Upcoming) lives only on
  // the home view; opening a channel shows just its detail.
  const center = selected
    ? channelDetailHtml(selected)
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

  const schedPillEl = (s) => `
    <div class="media-pill">
      <div class="mp-thumb"></div>
      <div class="mp-info">
        <div class="mp-title">${escape(s.channel)}</div>
        <div class="mp-sub">${escape(new Date(s.next_fire).toLocaleString())}${s.duration ? ` · ${escape(s.duration)}` : ""}</div>
      </div>
      <div class="mp-meta"><span class="mp-badge">scheduled</span></div>
    </div>`;

  const rowEl = (title, count, html, empty) => `
    <section class="dash-row">
      <h2 class="dash-row-title">${title}${count != null ? ` <span class="dash-count">${count}</span>` : ""}</h2>
      <div class="media-list">${html || `<div class="empty sm">${empty}</div>`}</div>
    </section>`;

  const heading = compact ? "" : `<h1 class="page-title">Recordings dashboard</h1>`;
  return `${heading}
    ${rowEl("In progress", inProgress.length, inProgress.map(recordingPillHtml).join(""), "Nothing recording")}
    ${rowEl("Recent", null, recent.map(recordingPillHtml).join(""), "No recordings yet")}
    ${rowEl("Upcoming", upcoming.length, upcoming.map(schedPillEl).join(""), "No scheduled recordings")}`;
}

// Shared recording media-pill (used by the home dashboard + History): cover
// thumbnail + title + channel·date + state/size, with a Stop on active rows.
function recordingPillHtml(j) {
  const when = j.started_at ? new Date(j.started_at).toLocaleString() : "—";
  const stop = isInProgress(j.state)
    ? `<button class="danger sm" data-action="stop" data-job-id="${escape(j.id)}">Stop</button>`
    : "";
  return `
    <div class="media-pill">
      <div class="mp-thumb"><img class="mp-thumb-img" loading="lazy" alt=""
        src="/api/v1/recordings/${encodeURIComponent(j.id)}/thumb" onerror="this.remove()"></div>
      <div class="mp-info">
        <div class="mp-title">${escape(j.stream_title || j.channel_name || "(recording)")}</div>
        <div class="mp-sub">${escape(j.channel_name || "")} · ${escape(when)}</div>
      </div>
      <div class="mp-meta">
        ${(() => { const d = recordingDisplayState(j); return `<span class="state-pill ${d.className}">${escape(d.label)}</span>`; })()}
        <span class="mp-size">${formatBytes(j.bytes_written || 0)}</span>
        ${stop}
      </div>
    </div>`;
}

function wireDashboard() {
  document.querySelectorAll('[data-action="stop"]').forEach((btn) => {
    btn.addEventListener("click", async () => {
      if (!(await confirmDialog("Stop this recording?", { ok: "Stop", danger: true })))
        return;
      await withBusy(btn, "Stopping…", async () => {
        await API.stopRecording(btn.dataset.jobId);
        Toast.success("Recording stopped");
        setTimeout(() => render().catch(() => {}), 500);
      }).catch((e) => Toast.error(`Stop failed: ${e.message}`));
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
                data-thumbnail="${escape(c.thumbnail_url || "")}"
                data-stream-title="${escape(c.stream_title || "")}">● Record</button>
        <button data-action="record" data-from-start="true" data-channel-id="${c.id}"
                data-channel-name="${escape(c.name)}"
                data-display-name="${escape(c.display_name || c.name)}"
                data-platform="${c.platform}"
                data-thumbnail="${escape(c.thumbnail_url || "")}"
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

// Live preview when a live channel is opened (items 4 + 23). Progressive
// model: show a refreshing thumbnail poster first, upgrade to the platform's
// embed player on click (tap-to-play — avoids auto-spinning a player for every
// open and works on mobile). Patreon has no live concept (thumbnail-only).
function liveEmbedSrc(c) {
  const host = location.hostname || "127.0.0.1";
  if (c.platform === "Twitch") {
    return `https://player.twitch.tv/?channel=${encodeURIComponent(c.name)}` +
      `&parent=${encodeURIComponent(host)}&muted=true&autoplay=true`;
  }
  if (c.platform === "YouTube") {
    return `https://www.youtube.com/embed/live_stream?channel=${encodeURIComponent(c.id)}` +
      `&autoplay=1&mute=1&playsinline=1`;
  }
  return null;
}

// Substitute Twitch's {width}x{height} placeholders and cache-bust so the
// poster refreshes to a near-live frame.
function liveThumbUrl(c) {
  if (!c.thumbnail_url) return null;
  const sized = c.thumbnail_url
    .replace("{width}", "440")
    .replace("{height}", "248");
  return `${sized}${sized.includes("?") ? "&" : "?"}t=${Date.now()}`;
}

function livePreviewHtml(c) {
  if (!c.is_live) return "";
  const src = liveEmbedSrc(c);
  const thumb = liveThumbUrl(c);
  // No thumbnail but we have an embed → mount the player directly.
  // No `loading="lazy"`: this iframe is the live player. Chromium
  // viewport-throttles lazy iframes during the top-layer transition that
  // fullscreen triggers on cross-origin embeds, which stalls Twitch playback.
  if (!thumb && src) {
    return `<div class="cd-preview" data-embed-src="${escape(src)}">
      <iframe src="${escape(src)}" title="Live preview"
              allow="autoplay; fullscreen; picture-in-picture" allowfullscreen></iframe>
    </div>`;
  }
  if (!thumb) return "";
  // Poster + (if embeddable) a play overlay to upgrade to the player.
  return `<div class="cd-preview poster" ${src ? `data-embed-src="${escape(src)}"` : ""}>
    <img id="cd-poster-img" src="${escape(thumb)}" alt="Live thumbnail" />
    ${src ? `<button class="cd-play" id="cd-play" aria-label="Play live preview">▶</button>` : ""}
  </div>`;
}

let cdPosterTimer = null;
function teardownLivePreview() {
  if (cdPosterTimer) {
    clearInterval(cdPosterTimer);
    cdPosterTimer = null;
  }
}

function wireChannelDetail() {
  // Clear any preview refresh timer from a previously-open detail (item 23).
  teardownLivePreview();
  document.querySelector('[data-action="cd-close"]')?.addEventListener("click", () => {
    teardownLivePreview();
    selectedChannelKey = null;
    render();
  });

  // Live preview: refresh the poster thumbnail every 30s, and upgrade to the
  // embed player on click (tap-to-play). Tears down when detail re-renders.
  const poster = document.querySelector(".cd-preview.poster");
  if (poster) {
    const img = poster.querySelector("#cd-poster-img");
    if (img) {
      const base = img.src.split(/[?&]t=/)[0];
      cdPosterTimer = setInterval(() => {
        // Only refresh while still on-screen (cheap visibility guard).
        if (!document.body.contains(img)) {
          teardownLivePreview();
          return;
        }
        img.src = `${base}${base.includes("?") ? "&" : "?"}t=${Date.now()}`;
      }, 30000);
    }
    const playBtn = poster.querySelector("#cd-play");
    const src = poster.dataset.embedSrc;
    if (playBtn && src) {
      playBtn.addEventListener("click", () => {
        teardownLivePreview();
        poster.classList.remove("poster");
        poster.innerHTML = `<iframe src="${escape(src)}" title="Live preview"
          allow="autoplay; fullscreen; picture-in-picture" allowfullscreen></iframe>`;
      });
    }
  }
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
            ? "Past Broadcasts"
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
  // Look up channel context once so each VOD pill can carry the
  // channel_name + platform the download route needs.
  const channel = channelCache.find((c) => c.id === channelId);
  const ctx = {
    channelName: (channel && (channel.display_name || channel.name)) || "",
    platform: platform || (channel && channel.platform) || "",
  };
  if (!vods) {
    if (streamsEl) streamsEl.innerHTML = vodSectionHtml("Past Broadcasts", null, ctx);
    if (uploadsEl) uploadsEl.innerHTML = vodSectionHtml("Recent uploads", null, ctx);
    return;
  }
  const streams = vods.filter((v) => v.kind === "LiveBroadcast");
  const uploads = vods.filter((v) => v.kind !== "LiveBroadcast");
  if (streamsEl) {
    streamsEl.innerHTML = vodSectionHtml("Past Broadcasts", streams, ctx);
  }
  if (uploadsEl) uploadsEl.innerHTML = vodSectionHtml("Recent uploads", uploads, ctx);
  wireVodDownloadButtons();
}

// Click handler for [data-action=vod-download] buttons inside the media-list
// pills. Optimistically flips to "downloading"; the SSE RecordingFinished
// handler flips to "downloaded" when a matching recording completes.
function wireVodDownloadButtons() {
  document.querySelectorAll("[data-action=vod-download]").forEach((btn) => {
    if (btn.dataset.wired === "1") return;
    btn.dataset.wired = "1";
    btn.addEventListener("click", async (e) => {
      e.preventDefault();
      e.stopPropagation();
      const url = btn.dataset.url;
      const channel_name = btn.dataset.channel;
      const platform = btn.dataset.platform;
      const post_title = btn.dataset.title || null;
      if (!url || vodDownloadState[url] === "downloading" || vodDownloadState[url] === "downloaded") {
        return;
      }
      vodDownloadState[url] = "downloading";
      setVodButtonState(btn, "downloading");
      try {
        // `data-via=patreon` routes through PatreonPull (its IPC arm
        // builds the Patreon-shaped output path + threads the patron
        // cookies); everything else lands on the generic DownloadVod
        // path. Both produce a RecordingJob with `source_url == url`,
        // so the state map + progress bar pipeline are identical.
        if (btn.dataset.via === "patreon") {
          await API.patreonPull({
            embed_url: url,
            creator_name: channel_name,
            post_title: post_title || "",
          });
        } else {
          await API.vodDownload({ url, channel_name, platform, post_title });
        }
        Toast.success(`Downloading: ${post_title || url}`);
        // The RecordingStarted SSE that follows will land in recCache with
        // source_url == this url; seedVodDownloadStateFromRecCache() then
        // confirms our optimistic state. When the recording reaches
        // Finished, the same path flips the pill to Downloaded by exact
        // source_url match — no FIFO guess.
      } catch (err) {
        // Roll back to idle so the user can retry.
        delete vodDownloadState[url];
        setVodButtonState(btn, "idle");
        Toast.error(`Download failed: ${err.message}`);
      }
    });
  });
}

// Walk recCache and reflect each recording whose source_url points at a VOD
// into vodDownloadState. Called whenever recCache is refreshed so the
// channel-detail view (and a fresh page reload) shows correct button state
// without any FIFO guess.
function seedVodDownloadStateFromRecCache() {
  for (const r of recCache) {
    if (!r.source_url) continue;
    if (r.state === "Finished") {
      vodDownloadState[r.source_url] = "downloaded";
    } else if (isInProgress(r.state)) {
      // Don't downgrade a "downloaded" entry if a stale in-progress row
      // sneaks in (rare, but be safe).
      if (vodDownloadState[r.source_url] !== "downloaded") {
        vodDownloadState[r.source_url] = "downloading";
      }
    }
  }
}

function setVodButtonState(btn, state) {
  btn.classList.remove("vod-dl-idle", "vod-dl-downloading", "vod-dl-downloaded");
  btn.classList.add(`vod-dl-${state}`);
  btn.disabled = state !== "idle";
  if (state === "downloading") {
    // Try to seed initial bar from any cached progress on the matching job.
    const url = btn.dataset.url;
    const job = recCache.find((r) => r.source_url === url);
    btn.innerHTML = vodProgressHtml(
      job && job.download_pct,
      job && job.download_eta_secs,
      job && job.download_rate_bps,
    );
  } else {
    btn.textContent = state === "downloaded" ? "Downloaded" : "Download";
  }
}

// Inner HTML for the in-flight download widget: gradient-filled bar +
// "NN% · Xm Ys left · R MB/s" label. Bar gradient runs amber → green so the
// rightmost fill colour shifts greener as the pull completes.
function vodProgressHtml(pct, etaSecs, rateBps) {
  const p = Math.max(0, Math.min(100, Math.round(pct == null ? 0 : pct)));
  const eta = etaSecs == null ? "" : fmtEta(etaSecs);
  const rate = rateBps == null ? "" : `${formatBytes(rateBps)}/s`;
  const meta = [eta && `${eta} left`, rate].filter(Boolean).join(" · ");
  return `
    <span class="vod-dl-bar"><span class="vod-dl-fill" style="width:${p}%"></span></span>
    <span class="vod-dl-label">${p}%${meta ? " · " + meta : ""}</span>
  `;
}

function fmtEta(secs) {
  const s = Math.max(0, Math.floor(secs));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  const r = s % 60;
  if (m < 60) return r ? `${m}m ${r}s` : `${m}m`;
  const h = Math.floor(m / 60);
  const mm = m % 60;
  return mm ? `${h}h ${mm}m` : `${h}h`;
}

// Surgical DOM patch: find every visible VOD pill bound to this job's
// source_url and refresh its progress widget. Skips pills that have
// transitioned to "downloaded" (which the seed function will finalize).
function updateVodProgressDom(job) {
  if (!job || !job.source_url) return;
  if (vodDownloadState[job.source_url] !== "downloading") return;
  const sel = `[data-action=vod-download][data-url="${CSS.escape(job.source_url)}"]`;
  document.querySelectorAll(sel).forEach((btn) => {
    btn.innerHTML = vodProgressHtml(
      job.download_pct,
      job.download_eta_secs,
      job.download_rate_bps,
    );
  });
}

// Resolve a VOD/stream thumbnail URL, substituting Twitch's templated
// dimension placeholders ({width}/%{width}). VOD thumbnails are static.
function vodThumb(url) {
  if (!url) return null;
  return url
    .replace(/%?\{width\}/g, "440")
    .replace(/%?\{height\}/g, "248");
}
// Compact duration from a serde std::time::Duration ({secs, nanos}) or number.
function fmtDur(d) {
  const s = typeof d === "number" ? d : d && d.secs;
  if (!s || s <= 0) return "";
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60);
  return h ? `${h}h ${m}m` : `${m}m`;
}

function vodSectionHtml(title, vods, ctx) {
  // Past Broadcasts gets the larger, centered treatment. Uploads keep the
  // smaller original style.
  const isPast = title === "Past Broadcasts";
  const titleCls = isPast ? "cd-section-title past-broadcasts" : "cd-section-title";
  if (vods === null) {
    return `<h2 class="${titleCls}">${title}</h2><div class="empty sm">Loading…</div>`;
  }
  if (vods.length === 0) {
    return `<h2 class="${titleCls}">${title}</h2><div class="empty sm">None</div>`;
  }
  const channelName = (ctx && ctx.channelName) || "";
  const platform = (ctx && ctx.platform) || "";
  // Jellyseerr/*arr-style horizontal media pills: thumbnail + rich info block,
  // with a sibling download button. The link wraps thumb+info; the button
  // sits next to it so we don't nest interactive elements.
  const rows = vods
    .map((v) => {
      const href = /^https?:\/\//i.test(v.url || "") ? escape(v.url) : "#";
      const thumb = vodThumb(v.thumbnail_url);
      const date = (v.published_at || "").slice(0, 10);
      const dur = fmtDur(v.duration);
      const live = v.kind === "Live" || v.kind === "live";
      const meta = [date, dur].filter(Boolean).map(escape).join(" · ");
      const downloadable = !!(v.url && channelName && platform);
      const state = vodDownloadState[v.url] || "idle";
      // For the downloading state, embed a live progress widget instead of
      // plain text. Seed pct/eta/rate from any matching cached job so a
      // re-render between SSE ticks doesn't reset the bar to 0%.
      let inner;
      if (state === "downloading") {
        const job = recCache.find((r) => r.source_url === v.url);
        inner = vodProgressHtml(
          job && job.download_pct,
          job && job.download_eta_secs,
          job && job.download_rate_bps,
        );
      } else if (state === "downloaded") {
        inner = "Downloaded";
      } else {
        inner = "Download";
      }
      const btn = downloadable
        ? `<button class="vod-dl vod-dl-${state}" data-action="vod-download"
              data-url="${escape(v.url)}"
              data-channel="${escape(channelName)}"
              data-platform="${escape(platform)}"
              data-title="${escape(v.title || "")}"
              ${state !== "idle" ? "disabled" : ""}>${inner}</button>`
        : "";
      return `
    <div class="media-pill">
      <a class="mp-link" href="${href}" target="_blank" rel="noopener">
        <div class="mp-thumb">${thumb ? `<img class="mp-thumb-img" loading="lazy" alt="" src="${escape(thumb)}" onerror="this.remove()">` : ""}</div>
        <div class="mp-info">
          <div class="mp-title">${escape(v.title)}</div>
          <div class="mp-sub">${meta}</div>
        </div>
        <div class="mp-meta">${live ? '<span class="mp-badge live">LIVE VOD</span>' : '<span class="mp-badge">Upload</span>'}</div>
      </a>
      ${btn}
    </div>`;
    })
    .join("");
  return `<h2 class="${titleCls}">${title}</h2>
    <div class="media-list">${rows}</div>`;
}

// Patreon channel detail: render cached posts with a pull action.
function renderPatreonPosts(c) {
  const el = document.getElementById("cd-posts");
  if (!el) return;
  const posts = patreonState.posts[c.id] || [];
  const channelName = c.display_name || c.name;
  // Each post pill carries the same `.vod-dl` button the past-broadcasts
  // list uses; state is keyed by embed_url (== source_url on the resulting
  // RecordingJob), so seedVodDownloadStateFromRecCache surfaces in-flight /
  // completed pulls across navigation just like past broadcasts.
  const rows = posts.length
    ? posts
        .map((p) => {
          const thumb = p.thumbnail_url
            ? `<img class="mp-thumb-img" loading="lazy" alt="" src="${escape(p.thumbnail_url)}" onerror="this.remove()">`
            : "";
          const url = p.embed_url || "";
          const state = vodDownloadState[url] || "idle";
          const cachedJob = recCache.find((r) => r.source_url === url);
          const inner = state === "downloading"
            ? vodProgressHtml(
                cachedJob && cachedJob.download_pct,
                cachedJob && cachedJob.download_eta_secs,
                cachedJob && cachedJob.download_rate_bps,
              )
            : state === "downloaded" ? "Downloaded" : "Download";
          const btn = url
            ? `<button class="vod-dl vod-dl-${state}" data-action="vod-download"
                  data-via="patreon"
                  data-url="${escape(url)}"
                  data-channel="${escape(channelName)}"
                  data-platform="Patreon"
                  data-title="${escape(p.title)}"
                  ${state !== "idle" ? "disabled" : ""}>${inner}</button>`
            : "";
          return `
      <div class="media-pill">
        <div class="mp-link" style="cursor: default;">
          <div class="mp-thumb">${thumb}</div>
          <div class="mp-info">
            <div class="mp-title">${escape(p.title)}</div>
            <div class="mp-sub">${escape((p.published_at || "").slice(0, 10))}</div>
          </div>
          <div class="mp-meta"></div>
        </div>
        ${btn}
      </div>`;
        })
        .join("")
    : '<div class="empty sm">No video posts.</div>';
  el.innerHTML = `<h2 class="cd-section-title">Posts</h2><div class="media-list">${rows}</div>`;
  wireVodDownloadButtons();
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
      <div id="aw-result" class="wizard-result">${escape(opts.message || "")}</div>
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
      thumbnail_url: d.thumbnail || null,
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
  seedVodDownloadStateFromRecCache();
  // W4-alt: sortable + filterable data grid. Column headers toggle sort;
  // the filter box narrows by channel/title live without refetching.
  root.innerHTML = chrome(`
    <h1 class="page-title">Recordings</h1>
    <div class="rec-toolbar">
      <input id="rec-filter" class="grid-filter" type="search"
             placeholder="Filter by channel or title… (/)"
             aria-label="Filter recordings" value="${escape(recFilter)}">
      <button id="rec-density" class="sm" title="Toggle row density">
        ${recDensity === "compact" ? "▤ Comfortable" : "▥ Compact"}
      </button>
      ${(() => {
        const errored = recCache.filter((r) => stateClassName(r.state) === "failed" || stateLabel(r.state).toLowerCase().includes("interrupt")).length;
        return errored > 0
          ? `<button id="rec-clear-errored" class="danger sm" title="Trash all failed/interrupted recordings">✕ Clear errored (${errored})</button>`
          : "";
      })()}
    </div>
    <p class="page-subtitle" id="rec-count"></p>
    <div id="rec-massbar" class="massbar" hidden></div>
    <table class="recordings-table ${recDensity === "compact" ? "compact" : ""}">
      <thead>
        <tr>
          <th class="rec-check"><input type="checkbox" id="rec-select-all" aria-label="Select all"></th>
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
  document.getElementById("rec-density")?.addEventListener("click", () => {
    recDensity = recDensity === "compact" ? "comfortable" : "compact";
    localStorage.setItem("strivo-rec-density", recDensity);
    renderRecordings().catch((e) => Toast.error(e.message));
  });
  document.getElementById("rec-clear-errored")?.addEventListener("click", async (e) => {
    const btn = e.currentTarget;
    const errored = recCache.filter((r) => {
      const c = stateClassName(r.state);
      const l = stateLabel(r.state).toLowerCase();
      return c === "failed" || l.includes("interrupt");
    });
    if (errored.length === 0) return;
    if (!(await confirmDialog(`Trash ${errored.length} errored recording(s)? Files move to the 7-day trash.`, { ok: "Clear", danger: true })))
      return;
    await withBusy(btn, "Clearing…", async () => {
      await API.clearErroredRecordings();
      Toast.success(`Cleared ${errored.length}`);
      // Optimistic prune; SSE refetch confirms.
      const erroredIds = new Set(errored.map((r) => r.id));
      recCache = recCache.filter((r) => !erroredIds.has(r.id));
      renderRecordings().catch(() => {});
    }).catch((err) => Toast.error(`Clear failed: ${err.message}`));
  });
  document.getElementById("rec-select-all")?.addEventListener("change", (e) => {
    const visible = visibleRecordingIds();
    if (e.target.checked) visible.forEach((id) => recSelected.add(id));
    else visible.forEach((id) => recSelected.delete(id));
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
      renderRecordings().catch((e) => Toast.error(e.message)); // re-render header arrows + body
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
  recVisible = rows;
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
      await withBusy(btn, "Stopping…", async () => {
        await API.stopRecording(btn.dataset.jobId);
        Toast.success("Recording stopped");
        setTimeout(() => render().catch(() => {}), 500);
      }).catch((e) => Toast.error(`Stop failed: ${e.message}`));
    });
  });
  body.querySelectorAll("[data-action=rec-play]").forEach((btn) => {
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      openRecordingPlayer(btn.dataset.jobId);
    });
  });
  body.querySelectorAll("[data-action=rec-info]").forEach((btn) => {
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      openRecordingInfo(btn.dataset.jobId);
    });
  });
  body.querySelectorAll("[data-action=rec-delete]").forEach((btn) => {
    btn.addEventListener("click", async (e) => {
      e.stopPropagation();
      if (!(await confirmDialog("Delete this recording? The file moves to the 7-day trash.", { ok: "Delete", danger: true })))
        return;
      await withBusy(btn, "Deleting…", async () => {
        await API.deleteRecordingFile(btn.dataset.jobId);
        Toast.success("Deleted");
        // Optimistic: drop from local cache + repaint; the SSE refetch
        // confirms shortly.
        recCache = recCache.filter((r) => r.id !== btn.dataset.jobId);
        renderRecordings().catch(() => {});
      }).catch((err) => Toast.error(`Delete failed: ${err.message}`));
    });
  });
  // Row click:
  //   plain                           → open Info modal
  //   Shift+click                     → select range (anchor → here)
  //   Ctrl/Cmd+click                  → toggle just this row
  // Buttons/inputs/anchors still get their own handlers (early-return).
  body.querySelectorAll("tr[data-rec-row]").forEach((tr) => {
    tr.addEventListener("click", (e) => {
      if (e.target.closest("button, input, a")) return;
      const id = tr.dataset.recRow;
      if (e.shiftKey && recAnchorId) {
        e.preventDefault();
        const ids = visibleRecordingIds();
        const i = ids.indexOf(recAnchorId);
        const j = ids.indexOf(id);
        if (i >= 0 && j >= 0) {
          const [lo, hi] = i < j ? [i, j] : [j, i];
          for (let k = lo; k <= hi; k++) recSelected.add(ids[k]);
          paintRecordings();
        }
        return;
      }
      if (e.ctrlKey || e.metaKey) {
        e.preventDefault();
        if (recSelected.has(id)) recSelected.delete(id);
        else recSelected.add(id);
        recAnchorId = id;
        paintRecordings();
        return;
      }
      openRecordingInfo(id);
    });
  });
  // Selection model:
  //   Click checkbox            → toggle this row
  //   Shift+click checkbox/row  → select range from anchor to here
  //   Ctrl/Cmd+click row body   → toggle this row (without opening Info)
  //   Plain click row body      → open Info modal (handled below)
  body.querySelectorAll(".rec-row-check").forEach((cb) => {
    // Suppress the native `change` (it fires on Space too); a `click`
    // handler with `preventDefault` lets us implement range semantics.
    cb.addEventListener("click", (e) => {
      e.preventDefault();
      const id = cb.dataset.jobId;
      if (e.shiftKey && recAnchorId) {
        const ids = visibleRecordingIds();
        const i = ids.indexOf(recAnchorId);
        const j = ids.indexOf(id);
        if (i >= 0 && j >= 0) {
          const [lo, hi] = i < j ? [i, j] : [j, i];
          for (let k = lo; k <= hi; k++) recSelected.add(ids[k]);
        }
      } else {
        if (recSelected.has(id)) recSelected.delete(id);
        else recSelected.add(id);
        recAnchorId = id;
      }
      paintRecordings();
    });
  });
  const all = document.getElementById("rec-select-all");
  if (all) {
    const vis = visibleRecordingIds();
    all.checked = vis.length > 0 && vis.every((id) => recSelected.has(id));
  }
  updateMassbar();
}

// IDs currently visible after filter/sort (for select-all + mass actions).
let recVisible = [];
function visibleRecordingIds() {
  return recVisible.map((r) => r.id);
}

// Show/hide the multi-select mass-action bar (item 22). Acts on the selection
// intersected with currently-visible rows.
function updateMassbar() {
  const bar = document.getElementById("rec-massbar");
  if (!bar) return;
  const visible = new Set(visibleRecordingIds());
  const sel = recVisible.filter((r) => recSelected.has(r.id) && visible.has(r.id));
  if (sel.length === 0) {
    bar.hidden = true;
    bar.innerHTML = "";
    return;
  }
  const active = sel.filter((r) => stateClassName(r.state) === "recording");
  bar.hidden = false;
  bar.innerHTML = `
    <span class="massbar-count">${sel.length} selected</span>
    ${active.length ? `<button id="mass-stop" class="danger sm">Stop ${active.length} active</button>` : ""}
    <button id="mass-rerecord" class="sm">Re-record ${sel.length}</button>
    <button id="mass-clear" class="sm">Clear</button>`;
  document.getElementById("mass-clear")?.addEventListener("click", () => {
    recSelected.clear();
    paintRecordings();
  });
  document.getElementById("mass-stop")?.addEventListener("click", async () => {
    if (!(await confirmDialog(`Stop ${active.length} active recording(s)?`, { ok: "Stop", danger: true })))
      return;
    let ok = 0;
    for (const r of active) {
      try {
        await API.stopRecording(r.id);
        ok++;
      } catch (_) {}
    }
    Toast.success(`Stopped ${ok}/${active.length}`);
    recSelected.clear();
    setTimeout(() => render().catch(() => {}), 500);
  });
  document.getElementById("mass-rerecord")?.addEventListener("click", async () => {
    if (!(await confirmDialog(`Re-record ${sel.length} channel(s) now?`, { ok: "Re-record" })))
      return;
    let ok = 0;
    for (const r of sel) {
      try {
        await API.startRecording({
          channel_id: r.channel_id,
          channel_name: r.channel_name,
          platform: r.platform,
          from_start: true,
        });
        ok++;
      } catch (_) {}
    }
    Toast.success(`Re-record queued ${ok}/${sel.length}`);
    recSelected.clear();
    setTimeout(() => render().catch(() => {}), 500);
  });
}

// Cover thumbnail for a recording. The wrapper renders a channel-initials
// tile coloured by a hash of the channel name; the inner <img> sits on top
// and covers it when /thumb returns a real jpg. On 404 the img self-removes
// and the initials show through, so old recordings (made before the source-
// thumbnail snapshot landed, and missed by ffmpeg fallback on the server)
// still look intentional rather than broken.
function recThumb(r) {
  const initials = (r.channel_name || r.stream_title || "?")
    .trim()
    .replace(/[^\p{L}\p{N} ]/gu, "")
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0].toUpperCase())
    .join("") || "?";
  const hue = thumbHue(r.channel_name || r.id || "");
  // r.file_exists is set by the backend's augment_recording; when false the
  // recording's output_path is gone from disk (moved / deleted / external
  // drive offline) so we surface it as a red-caps overlay over the thumb.
  const missing = r.file_exists === false ? " rec-thumb-missing" : "";
  return `<span class="rec-thumb-wrap${missing}" data-init="${escape(initials)}"
    style="--ch-hue:${hue}deg">
    <img class="rec-thumb" loading="lazy" alt=""
      src="/api/v1/recordings/${encodeURIComponent(r.id)}/thumb"
      onerror="this.remove()" />
  </span>`;
}

// Stable hash → hue so the same channel always gets the same colour, but
// different channels get different ones across the rail.
function thumbHue(s) {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0;
  return Math.abs(h) % 360;
}

function recordingRow(r) {
  const disp = recordingDisplayState(r);
  const state = disp.label;
  const stateClass = disp.className;
  // Active includes both live captures (Recording) and VOD pulls (Downloading);
  // both are in-flight and offer Stop.
  const isActive = stateClass === "recording" || stateClass === "downloading";
  const isFinished = stateClass === "finished";
  // Action set per state:
  //   active   → Stop
  //   finished → ▶ Play  ⓘ Info
  //   anything → ⓘ Info  ✕ Delete
  const actions = isActive
    ? `<button class="danger sm" data-action="stop" data-job-id="${r.id}">Stop</button>`
    : `${isFinished
        ? `<button class="primary sm" data-action="rec-play" data-job-id="${r.id}" title="Open player (Enter)">▶ Play</button>`
        : ""}
       <button class="sm" data-action="rec-info" data-job-id="${r.id}" title="Recording details (I)">ⓘ Info</button>
       <button class="danger sm" data-action="rec-delete" data-job-id="${r.id}" title="Delete (Del)">✕</button>`;
  return `
    <tr class="${recSelected.has(r.id) ? "rec-sel" : ""}" data-rec-row="${escape(r.id)}">
      <td class="rec-check"><input type="checkbox" class="rec-row-check" data-job-id="${escape(r.id)}" ${recSelected.has(r.id) ? "checked" : ""} aria-label="Select recording"></td>
      <td><span class="state-pill ${stateClass}">${state}</span></td>
      <td>${escape(r.channel_name)}</td>
      <td><div class="rec-title-cell">${recThumb(r)}<span>${escape(r.stream_title || "(no title)")}</span></div></td>
      <td>${new Date(r.started_at).toLocaleString()}</td>
      <td>${formatBytes(r.bytes_written || 0)}</td>
      <td class="rec-actions"><div class="rec-actions-inner">${actions}</div></td>
    </tr>
  `;
}

// VOD pulls and live captures both ride `RecordingState::Recording`, but
// "Recording" reads wrong for a yt-dlp-backed VOD pull. Distinguish by
// `source_url`: when set, label + colour as a download instead. Other
// states (Finished/Failed/etc) read the same regardless.
function recordingDisplayState(j) {
  const cls = stateClassName(j.state);
  const lbl = stateLabel(j.state);
  // A row whose file is gone overrides every other state — the journal
  // says "Finished" but the recording has no file behind it, so reading
  // that as a green Finished pill misleads.
  if (j && j.file_exists === false) {
    return { label: "File Error", className: "file-error" };
  }
  if (j && j.source_url && cls === "recording") {
    return { label: "Downloading", className: "downloading" };
  }
  return { label: lbl, className: cls };
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
// Top-level Plugins route. Sub-routes select a plugin and its sub-views:
//   #/plugins                       → hub
//   #/plugins/crunchr               → transcribed-recordings list + search
//   #/plugins/crunchr/rec/<id>      → transcript + analysis
//   #/plugins/archiver              → archived channels
//   #/plugins/archiver/<channelId>  → channel catalog
//   #/plugins/viewguard             → fraud verdicts
//   #/plugins/insights              → word freq / topics / speakers
async function renderPlugins() {
  const parts = routeParts(); // ["plugins", <slug?>, …]
  const slug = parts[1];
  try {
    switch (slug) {
      case "crunchr":
        if (parts[2] === "rec" && parts[3]) return await renderCrunchrRecording(parts[3]);
        return await renderCrunchr();
      case "archiver":
        if (parts[2]) return await renderArchiverVideos(parts[2]);
        return await renderArchiver();
      case "viewguard":
        return await renderViewguard();
      case "insights":
        return await renderInsights();
      default:
        return await renderPluginHub();
    }
  } catch (e) {
    if (e.message && e.message.includes("unauthorized")) return;
    root.removeAttribute("aria-busy");
    root.innerHTML = chrome(
      `${pluginHeader("Plugins", "")}<div class="empty"><div class="glyph">⚠</div>${escape(e.message)}</div>`,
    );
    setupChromeHandlers();
  }
}

// Shared page header with an optional "← back to Plugins" trail.
function pluginHeader(title, subtitle, backHref) {
  const back = backHref
    ? `<a class="pg-back" href="${backHref}">← back</a>`
    : "";
  return `
    ${back}
    <h1 class="page-title">${escape(title)}</h1>
    ${subtitle ? `<p class="page-subtitle">${subtitle}</p>` : ""}
  `;
}

async function renderPluginHub() {
  // Fetch licence + plugins in parallel; licence failure must not block
  // the hub render — it just means we hide the upgrade card this paint.
  const [resp, licence] = await Promise.all([
    API.plugins(),
    API.licenceStatus().catch(() => null),
  ]);
  root.removeAttribute("aria-busy");
  const plugins = (resp && resp.plugins) || [];
  const upgrade = renderUpgradeCard(licence);
  const cards = plugins
    .map((p) => {
      const statBits = Object.entries(p.stats || {})
        .map(
          ([k, v]) =>
            `<span class="pg-stat"><strong>${formatCount(v)}</strong> ${escape(k.replace(/_/g, " "))}</span>`,
        )
        .join("");
      const status = p.available
        ? `<span class="cfg-badge ok">ready</span>`
        : `<span class="cfg-badge">idle</span>`;
      const href = p.available ? `#/plugins/${p.name}` : null;
      const body = `
        <div class="pg-card-head">
          <span class="pg-icon pg-icon-${p.name}" aria-hidden="true">${escape((p.display || p.name)[0])}</span>
          <span class="pg-card-name">${escape(p.display || p.name)}</span>
          ${status}
        </div>
        <p class="pg-card-desc">${escape(p.description || "")}</p>
        <div class="pg-stats">${statBits || '<span class="pg-stat muted">no data yet</span>'}</div>`;
      return href
        ? `<a class="pg-card" href="${href}" data-plugin="${p.name}">${body}</a>`
        : `<div class="pg-card pg-card-idle" data-plugin="${p.name}">${body}</div>`;
    })
    .join("");
  root.innerHTML = chrome(`
    ${pluginHeader("Plugins", "First-party plugins. Pick one to browse what it has produced.")}
    ${upgrade}
    <div class="pg-grid">${cards || '<div class="empty">No plugins loaded.</div>'}</div>
  `);
  setupChromeHandlers();
  wireUpgradeCard();
}

// Upgrade card — shown on the Plugins hub when the user is not entitled.
// Phase 1: stubbed backend, so the "Activate" button is disabled until
// the licence service implements (returns `implemented: false`). The
// trial CTA is wired to a placeholder endpoint that returns 501 today;
// the surface stays so the design is locked in.
function renderUpgradeCard(licence) {
  if (!licence || licence.entitled) return ""; // dev unlock + future paid users
  const implemented = licence.implemented === true;
  const trialDisabled = implemented ? "" : "disabled";
  return `
    <section class="upgrade-card" data-tier="${escape(licence.tier || "free")}">
      <img class="upgrade-logo" src="/assets/img/chorosyne-logo.png" alt="Chorosyne" />
      <div class="upgrade-body">
        <h2 class="upgrade-title">Strivo Pro</h2>
        <p class="upgrade-tagline">Unlock every plugin — Crunchr, Archiver, Viewguard, Insights — and everything we ship next.</p>
        <ul class="upgrade-bullets">
          <li>One-time <strong>$25</strong> — no subscription, no recurring fees.</li>
          <li>Single-machine licence with auto-refresh every 72h (works offline).</li>
          <li>3-day free trial — no card required.</li>
        </ul>
        <div class="upgrade-actions">
          <button class="upgrade-trial btn-primary" ${trialDisabled}>Start 3-day trial</button>
          <button class="upgrade-activate btn-ghost" ${trialDisabled}>I have a key</button>
        </div>
        ${implemented ? "" : '<p class="upgrade-hint">Activation backend wires up in the next phase — surface preview only.</p>'}
      </div>
    </section>
  `;
}

function wireUpgradeCard() {
  const trial = document.querySelector(".upgrade-trial");
  const activate = document.querySelector(".upgrade-activate");
  if (trial) {
    trial.addEventListener("click", async () => {
      try {
        await API.licenceTrial();
        location.reload();
      } catch (e) {
        Toast.error(e.message || "Trial unavailable");
      }
    });
  }
  if (activate) {
    activate.addEventListener("click", async () => {
      const key = prompt("Paste your Strivo Pro licence key:");
      if (!key) return;
      try {
        await API.licenceActivate(key.trim());
        location.reload();
      } catch (e) {
        Toast.error(e.message || "Activation failed");
      }
    });
  }
}

// ── Crunchr ──────────────────────────────────────────────────────────
async function renderCrunchr() {
  const resp = await API.crunchrRecordings();
  root.removeAttribute("aria-busy");
  const recs = (resp && resp.recordings) || [];
  const rows = recs
    .map((r) => {
      const an = r.has_analysis
        ? '<span class="cfg-badge ok">analyzed</span>'
        : "";
      return `
        <a class="pg-row" href="#/plugins/crunchr/rec/${encodeURIComponent(r.recording_id)}">
          <span class="pg-row-main">
            <span class="pg-row-title">${escape(r.title || "(untitled)")}</span>
            <span class="pg-row-sub">${escape(r.channel_name)} · ${escape(r.created_at || "")}</span>
          </span>
          <span class="pg-row-meta">
            <span class="cfg-badge status-${escape(r.status)}">${escape(r.status)}</span>
            <span class="pg-row-num">${formatCount(r.segment_count)} segs</span>
            ${an}
          </span>
        </a>`;
    })
    .join("");
  root.innerHTML = chrome(`
    ${pluginHeader("Crunchr", "Transcribed recordings. Click one to read its transcript and analysis.", "#/plugins")}
    <div class="pg-search">
      <input id="crunchr-q" type="search" placeholder="Search transcripts…"
             autocomplete="off" aria-label="Search transcripts" />
    </div>
    <div id="crunchr-search-results"></div>
    <div class="pg-list">${rows || '<div class="empty">Nothing transcribed yet.</div>'}</div>
  `);
  setupChromeHandlers();

  const q = document.getElementById("crunchr-q");
  const out = document.getElementById("crunchr-search-results");
  let timer = null;
  q.addEventListener("input", () => {
    clearTimeout(timer);
    const term = q.value.trim();
    if (!term) {
      out.innerHTML = "";
      return;
    }
    timer = setTimeout(async () => {
      try {
        const r = await API.crunchrSearch(term);
        const hits = (r && r.results) || [];
        out.innerHTML = hits.length
          ? `<div class="pg-list pg-search-hits">${hits
              .map(
                (h) => `
            <a class="pg-row" href="#/plugins/crunchr/rec/${encodeURIComponent(findRecIdForHit(recs, h))}">
              <span class="pg-row-main">
                <span class="pg-row-title">${escape(h.snippet)}</span>
                <span class="pg-row-sub">${escape(h.video_title)} · ${escape(h.channel_name)} · ${fmtClock(h.start_sec)}</span>
              </span>
            </a>`,
              )
              .join("")}</div>`
          : '<div class="empty sm">No matches.</div>';
      } catch (e) {
        out.innerHTML = `<div class="empty sm">${escape(e.message)}</div>`;
      }
    }, 220);
  });
}

// FTS rows don't carry a recording_id; match on title+channel against the
// already-loaded list so a hit links to the right transcript.
function findRecIdForHit(recs, hit) {
  const m = recs.find(
    (r) => r.title === hit.video_title && r.channel_name === hit.channel_name,
  );
  return m ? m.recording_id : "";
}

async function renderCrunchrRecording(id) {
  const d = await API.crunchrRecording(id);
  root.removeAttribute("aria-busy");
  const topics = (d.topics || [])
    .map((t) => `<span class="pg-chip">${escape(t)}</span>`)
    .join("");
  const sentiment = d.sentiment
    ? `<span class="cfg-badge sentiment-${escape(d.sentiment)}">${escape(d.sentiment)}</span>`
    : "";
  const analysis = d.summary || topics || sentiment
    ? `<section class="cfg-card">
         <h2 class="cfg-title">Analysis ${sentiment}</h2>
         ${d.summary ? `<p class="pg-summary">${escape(d.summary)}</p>` : ""}
         ${topics ? `<div class="pg-chips">${topics}</div>` : ""}
       </section>`
    : "";
  const segs = (d.segments || [])
    .map(
      (s) => `
      <div class="pg-seg">
        <span class="pg-seg-time">${fmtClock(s.start_sec)}</span>
        ${s.speaker ? `<span class="pg-seg-speaker">${escape(s.speaker)}</span>` : ""}
        <span class="pg-seg-text">${escape(s.text)}</span>
      </div>`,
    )
    .join("");
  root.innerHTML = chrome(`
    ${pluginHeader(d.title || "Transcript", `${escape(d.channel_name)} · ${escape(d.status)}`, "#/plugins/crunchr")}
    <div class="pg-verbs">
      <button id="retranscribe" data-rec="${escape(d.recording_id)}">↻ Re-transcribe</button>
      <a class="pg-linkbtn" href="#/plugins/insights/rec/${encodeURIComponent(d.recording_id)}">View insights →</a>
    </div>
    ${analysis}
    <section class="cfg-card">
      <h2 class="cfg-title">Transcript</h2>
      <div class="pg-transcript">${segs || '<div class="empty sm">No segments — transcription may still be running.</div>'}</div>
    </section>
  `);
  setupChromeHandlers();
  const btn = document.getElementById("retranscribe");
  if (btn) {
    btn.addEventListener("click", () =>
      dispatchVerb("crunchr", "Re-transcribe", [btn.dataset.rec], btn),
    );
  }
}

// ── Archiver ─────────────────────────────────────────────────────────
async function renderArchiver() {
  const resp = await API.archiverChannels();
  root.removeAttribute("aria-busy");
  const chans = (resp && resp.channels) || [];
  const rows = chans
    .map((c) => {
      const pct = c.video_count
        ? Math.round((c.downloaded_count / c.video_count) * 100)
        : 0;
      return `
        <a class="pg-row" href="#/plugins/archiver/${encodeURIComponent(c.id)}">
          <span class="pg-row-main">
            <span class="pg-row-title">${escape(c.name)}</span>
            <span class="pg-row-sub plat-${escape((c.platform || "").toLowerCase())}">${escape(c.platform)} · ${escape(c.last_scan || "never scanned")}</span>
          </span>
          <span class="pg-row-meta">
            <span class="pg-row-num">${formatCount(c.downloaded_count)} / ${formatCount(c.video_count)}</span>
            <span class="pg-mini-gauge"><span style="width:${pct}%"></span></span>
          </span>
        </a>`;
    })
    .join("");
  root.innerHTML = chrome(`
    ${pluginHeader("Archiver", "Tracked channels and their back-catalog download status.", "#/plugins")}
    <div class="pg-list">${rows || '<div class="empty">No channels archived yet.</div>'}</div>
  `);
  setupChromeHandlers();
}

async function renderArchiverVideos(channelId) {
  const resp = await API.archiverVideos(channelId);
  root.removeAttribute("aria-busy");
  const vids = (resp && resp.videos) || [];
  const rows = vids
    .map(
      (v) => `
      <div class="pg-row pg-row-static">
        <span class="pg-row-main">
          <span class="pg-row-title">${escape(v.title)}</span>
          <span class="pg-row-sub">${escape(v.upload_date || "")}${v.playlist ? " · " + escape(v.playlist) : ""}${v.duration ? " · " + fmtClock(v.duration) : ""}</span>
        </span>
        <span class="pg-row-meta">
          ${v.downloaded ? '<span class="cfg-badge ok">downloaded</span>' : '<span class="cfg-badge">pending</span>'}
        </span>
      </div>`,
    )
    .join("");
  root.innerHTML = chrome(`
    ${pluginHeader("Catalog", `${vids.length} videos`, "#/plugins/archiver")}
    <div class="pg-list">${rows || '<div class="empty">No catalog entries.</div>'}</div>
  `);
  setupChromeHandlers();
}

// ── Viewguard ────────────────────────────────────────────────────────
async function renderViewguard() {
  const resp = await API.viewguardVerdicts();
  root.removeAttribute("aria-busy");
  const verdicts = (resp && resp.verdicts) || [];
  const cards = verdicts
    .map((v) => {
      const pct = Math.round((v.final_score || 0) * 100);
      const contributors = Array.isArray(v.contributors)
        ? v.contributors
        : v.contributors && v.contributors.contributors
          ? v.contributors.contributors
          : [];
      const bars = contributors
        .map((c) => {
          const name = c.kind || c.detector || c.name || "signal";
          const score = c.score != null ? c.score : c.weight != null ? c.weight : 0;
          return `<div class="vg-contrib">
              <span class="vg-contrib-name">${escape(String(name))}</span>
              <span class="vg-bar"><span style="width:${Math.round(score * 100)}%"></span></span>
            </div>`;
        })
        .join("");
      return `
        <section class="cfg-card vg-card">
          <div class="vg-head">
            <span class="vg-channel">${escape(v.channel_id)}</span>
            <span class="cfg-badge vg-band vg-band-${escape((v.band || "").toLowerCase())}">${escape(v.band)}</span>
          </div>
          <div class="vg-score">
            <span class="vg-score-num">${pct}%</span>
            <span class="vg-score-label">suspicion</span>
          </div>
          ${bars ? `<div class="vg-contribs">${bars}</div>` : ""}
          <div class="vg-when">${escape(v.stream_started_at || "")}</div>
        </section>`;
    })
    .join("");
  root.innerHTML = chrome(`
    ${pluginHeader("Viewguard", "Latest viewbot-fraud verdict per channel. Higher = more suspicious.", "#/plugins")}
    <div class="cfg-grid">${cards || '<div class="empty">No verdicts yet — viewers are sampled while channels are live.</div>'}</div>
  `);
  setupChromeHandlers();
}

// ── Insights ─────────────────────────────────────────────────────────
let insightsState = { stopwords: false };
async function renderInsights() {
  const parts = routeParts();
  const recId = parts[2] === "rec" ? parts[3] : null;
  const [wordsResp, topicsResp] = await Promise.all([
    API.insightsWords({ stopwords: insightsState.stopwords, limit: 40 }),
    API.insightsTopics(),
  ]);
  root.removeAttribute("aria-busy");
  const words = (wordsResp && wordsResp.words) || [];
  const max = words.reduce((m, w) => Math.max(m, w.count), 0) || 1;
  const wordRows = words
    .map(
      (w) => `
      <div class="wf-row">
        <span class="wf-word">${escape(w.word)}</span>
        <span class="wf-bar"><span style="width:${Math.round((w.count / max) * 100)}%"></span></span>
        <span class="wf-count">${formatCount(w.count)}</span>
      </div>`,
    )
    .join("");
  const topics = (topicsResp && topicsResp.topics) || [];
  const topicChips = topics
    .slice(0, 60)
    .map(
      (t) =>
        `<span class="pg-chip" title="${escape(t.first_seen)} → ${escape(t.last_seen)}">${escape(t.topic)} <em>${t.count}</em></span>`,
    )
    .join("");

  root.innerHTML = chrome(`
    ${pluginHeader("Insights", "Aggregate signals across every transcribed recording.", "#/plugins")}
    <div class="cfg-grid">
      <section class="cfg-card">
        <h2 class="cfg-title">Top words</h2>
        <div class="pg-toolbar">
          <label class="pg-toggle"><input type="checkbox" id="ins-stopwords" ${insightsState.stopwords ? "checked" : ""}/> include stopwords</label>
          <a class="pg-linkbtn" href="/api/v1/plugins/insights/export?fmt=csv${insightsState.stopwords ? "&stopwords=true" : ""}">Export CSV</a>
          <a class="pg-linkbtn" href="/api/v1/plugins/insights/export?fmt=json${insightsState.stopwords ? "&stopwords=true" : ""}">JSON</a>
        </div>
        <div class="wf-list">${wordRows || '<div class="empty sm">No word data yet.</div>'}</div>
      </section>
      <section class="cfg-card">
        <h2 class="cfg-title">Topics</h2>
        <div class="pg-chips">${topicChips || '<div class="empty sm">No analyzed recordings yet.</div>'}</div>
      </section>
      <section class="cfg-card" id="ins-speakers-card">
        <h2 class="cfg-title">Speaker airtime</h2>
        <div id="ins-speakers"><div class="empty sm">Open a transcript and choose “View insights” to load speaker airtime.</div></div>
      </section>
    </div>
  `);
  setupChromeHandlers();
  const cb = document.getElementById("ins-stopwords");
  if (cb) {
    cb.addEventListener("change", () => {
      insightsState.stopwords = cb.checked;
      renderInsights();
    });
  }
  if (recId) await loadInsightsSpeakers(recId);
}

async function loadInsightsSpeakers(recId) {
  const host = document.getElementById("ins-speakers");
  if (!host) return;
  try {
    const r = await API.insightsSpeakers(recId);
    const speakers = (r && r.speakers) || [];
    const max = speakers.reduce((m, s) => Math.max(m, s.seconds), 0) || 1;
    host.innerHTML = speakers.length
      ? `${r.sentiment ? `<p class="page-subtitle">sentiment: <span class="cfg-badge sentiment-${escape(r.sentiment)}">${escape(r.sentiment)}</span></p>` : ""}
         ${speakers
           .map(
             (s) => `
        <div class="wf-row">
          <span class="wf-word">${escape(s.speaker)}</span>
          <span class="wf-bar"><span style="width:${Math.round((s.seconds / max) * 100)}%"></span></span>
          <span class="wf-count">${fmtClock(s.seconds)}</span>
        </div>`,
           )
           .join("")}`
      : '<div class="empty sm">No diarized speakers for this recording.</div>';
  } catch (e) {
    host.innerHTML = `<div class="empty sm">${escape(e.message)}</div>`;
  }
}

// ── Verb dispatch (actions over IPC) ─────────────────────────────────
async function dispatchVerb(plugin, verb, selection, btn) {
  if (btn) {
    btn.disabled = true;
    btn.dataset.prevLabel = btn.textContent;
    btn.textContent = "…";
  }
  try {
    await API.pluginRpc(plugin, verb, { selection });
    Toast.success(`${verb} queued in the daemon`);
  } catch (e) {
    Toast.error(`${verb} failed: ${e.message}`);
  } finally {
    if (btn) {
      btn.disabled = false;
      if (btn.dataset.prevLabel) btn.textContent = btn.dataset.prevLabel;
    }
  }
}

// mm:ss / h:mm:ss from a float-seconds value.
function fmtClock(sec) {
  const s = Math.max(0, Math.floor(sec || 0));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

// ── Recording info modal + in-app player ─────────────────────────────
//
// Two overlays — `#rec-info-modal` (stats + plugin quick-actions) and
// `#rec-player-modal` (custom mpv-style HTML5 player). Both close on
// Esc / backdrop click; opening one closes any other.

function ensureModalContainer(id) {
  let el = document.getElementById(id);
  if (!el) {
    el = document.createElement("div");
    el.id = id;
    el.className = "modal-overlay";
    el.setAttribute("role", "dialog");
    el.setAttribute("aria-modal", "true");
    document.body.appendChild(el);
  }
  return el;
}

function closeRecordingModals() {
  document.getElementById("rec-info-modal")?.remove();
  const pl = document.getElementById("rec-player-modal");
  if (pl) {
    const v = pl.querySelector("video");
    if (v) { v.pause(); v.removeAttribute("src"); v.load(); }
    pl.remove();
  }
  document.body.classList.remove("modal-open");
}

document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  if (document.getElementById("rec-player-modal") || document.getElementById("rec-info-modal")) {
    closeRecordingModals();
    e.preventDefault();
  }
});

// Format bytes/sec into "1.2 Mbps" / "320 kbps" / "12 bps".
function fmtBitrate(bps) {
  if (!bps || bps <= 0) return "";
  if (bps >= 1_000_000) return `${(bps / 1_000_000).toFixed(1)} Mbps`;
  if (bps >= 1_000) return `${Math.round(bps / 1_000)} kbps`;
  return `${bps} bps`;
}
function fmtHz(hz) {
  if (!hz || hz <= 0) return "";
  if (hz >= 1_000) return `${(hz / 1_000).toFixed(hz % 1_000 === 0 ? 0 : 1)} kHz`;
  return `${hz} Hz`;
}

// Stream section for the Info modal: container + per-track summaries from
// ffprobe. Renders empty when the probe failed (ffprobe missing, file
// missing, codec parse failure) — the rest of the modal still shows.
function probeSectionHtml(p) {
  if (!p) {
    return `<section class="rec-info-stream rec-info-stream-missing">
      <h3>Stream</h3>
      <div class="empty sm">ffprobe unavailable or recording file missing.</div>
    </section>`;
  }
  const meta = (k, v) => v ? `<dt>${escape(k)}</dt><dd>${v}</dd>` : "";
  const headBits = [
    p.container && escape(p.container),
    fmtBitrate(p.bit_rate || 0),
  ].filter(Boolean).join(" · ");
  const vRows = (p.video || []).map((v) => {
    const bits = [
      v.codec && escape(v.codec),
      (v.width && v.height) ? `${v.width}×${v.height}` : null,
      v.fps ? `${(+v.fps).toFixed(v.fps % 1 === 0 ? 0 : 2)} fps` : null,
      fmtBitrate(v.bit_rate || 0),
      v.pix_fmt && escape(v.pix_fmt),
    ].filter(Boolean).join(" · ");
    return bits ? `<div class="rec-info-track">${bits}</div>` : "";
  }).join("");
  const aRows = (p.audio || []).map((a) => {
    const bits = [
      a.codec && escape(a.codec),
      a.channel_layout ? escape(a.channel_layout) : (a.channels ? `${a.channels} ch` : null),
      fmtHz(a.sample_rate || 0),
      fmtBitrate(a.bit_rate || 0),
      a.language && escape(a.language),
    ].filter(Boolean).join(" · ");
    return bits ? `<div class="rec-info-track">${bits}</div>` : "";
  }).join("");
  const sRows = (p.subtitle || []).map((s) => {
    const bits = [s.codec && escape(s.codec), s.language && escape(s.language)]
      .filter(Boolean).join(" · ");
    return bits ? `<div class="rec-info-track">${bits}</div>` : "";
  }).join("");
  return `
    <section class="rec-info-stream">
      <h3>Stream</h3>
      <dl class="rec-info-stats rec-info-stream-stats">
        ${meta("Container", headBits)}
        ${vRows ? meta(`Video${p.video.length > 1 ? ` ×${p.video.length}` : ""}`, vRows) : ""}
        ${aRows ? meta(`Audio${p.audio.length > 1 ? ` ×${p.audio.length}` : ""}`, aRows) : ""}
        ${sRows ? meta(`Subtitle${p.subtitle.length > 1 ? ` ×${p.subtitle.length}` : ""}`, sRows) : ""}
      </dl>
    </section>`;
}

async function openRecordingInfo(jobId) {
  closeRecordingModals();
  const overlay = ensureModalContainer("rec-info-modal");
  overlay.innerHTML = `<div class="modal-card rec-info-card"><div class="empty sm">Loading…</div></div>`;
  document.body.classList.add("modal-open");
  overlay.addEventListener("click", (e) => { if (e.target === overlay) closeRecordingModals(); });

  let rec, plugins, probe;
  try {
    // Probe is best-effort (ffprobe may not be installed, file may be
    // missing); a failure must not block the modal from rendering.
    [rec, plugins, probe] = await Promise.all([
      API.recordingOne(jobId),
      API.plugins().catch(() => ({ plugins: [] })),
      API.recordingProbe(jobId).catch(() => null),
    ]);
  } catch (e) {
    overlay.querySelector(".modal-card").innerHTML =
      `<div class="empty"><div class="glyph">⚠</div>${escape(e.message)}</div>`;
    return;
  }

  const state = stateLabel(rec.state);
  const stateClass = stateClassName(rec.state);
  const isFinished = stateClass === "finished";
  const meta = (k, v) => `<dt>${escape(k)}</dt><dd>${v}</dd>`;
  // Bullet-proof scope match: accept the canonical lowercase "recording",
  // the Rust-debug form "Item(Recording)", or any string whose lowercase
  // contains "recording". Keeps the SPA right whether the index handler
  // hardcodes the string or eventually serializes the live enum.
  const isRecordingScope = (s) => {
    if (!s) return false;
    const t = String(s).toLowerCase();
    return t === "recording" || t.includes("recording");
  };
  const recordingVerbs = ((plugins && plugins.plugins) || [])
    .flatMap((p) => (p.verbs || [])
      .filter((v) => isRecordingScope(v.scope))
      .map((v) => ({ ...v, plugin: p.name, available: p.available })))
    .filter((v) => v.available);
  // SPA-native action: if Crunchr is available, surface a transcript-view
  // link rather than a no-op IPC dispatch. (`Show transcript` on the
  // plugin returns TUI-only `ActivatePane` actions when handled headless,
  // so we'd otherwise just queue and visibly do nothing.)
  const crunchr = ((plugins && plugins.plugins) || []).find((p) => p.name === "crunchr");
  const showTranscriptHtml = crunchr && crunchr.available
    ? `<a class="sm rec-info-verb-link"
          href="#/plugins/crunchr/rec/${encodeURIComponent(jobId)}"
          data-action="rec-info-route-close">📜 Show transcript</a>`
    : "";
  const verbBtns = recordingVerbs.map((v) => `
      <button class="sm" data-action="rec-info-verb"
              data-plugin="${escape(v.plugin)}"
              data-verb="${escape(v.verb)}">
        ${escape(v.label || v.verb)}
      </button>`).join("");
  const actionsHtml = (verbBtns + showTranscriptHtml) ||
    `<div class="empty sm">No plugin actions available.</div>`;

  overlay.querySelector(".modal-card").innerHTML = `
    <header class="rec-info-head">
      <span class="state-pill ${stateClass}">${escape(state)}</span>
      <h2>${escape(rec.stream_title || "(no title)")}</h2>
      <button class="modal-close" aria-label="Close" data-action="modal-close">✕</button>
    </header>
    <div class="rec-info-body">
      <div class="rec-info-thumb">${recThumb(rec)}</div>
      <dl class="rec-info-stats">
        ${meta("Channel", escape(rec.channel_name || ""))}
        ${meta("Platform", `<span class="plat-${escape((rec.platform || "").toLowerCase())}">${escape(rec.platform || "")}</span>`)}
        ${meta("Started", escape(rec.started_at ? new Date(rec.started_at).toLocaleString() : "—"))}
        ${meta("Duration", escape(rec.duration_secs ? fmtClock(rec.duration_secs) : "—"))}
        ${meta("Size", escape(formatBytes(rec.bytes_written || 0)))}
        ${meta("Transcode", rec.transcode ? "yes" : "no")}
        ${rec.source_url ? meta("Source", `<a href="${escape(rec.source_url)}" target="_blank" rel="noopener">${escape(rec.source_url)}</a>`) : ""}
        ${rec.output_path ? meta("File", `<code class="rec-info-path">${escape(rec.output_path)}</code>`) : ""}
        ${rec.error ? meta("Error", `<span class="cfg-badge err">${escape(rec.error)}</span>`) : ""}
      </dl>
    </div>
    ${probeSectionHtml(probe)}
    <section class="rec-info-actions">
      <h3>Plugin actions</h3>
      <div class="rec-info-verbs">${actionsHtml}</div>
    </section>
    <footer class="rec-info-foot">
      ${isFinished ? `<button class="primary" data-action="rec-info-play">▶ Open in player</button>` : ""}
      <button class="danger" data-action="rec-info-delete">✕ Delete</button>
    </footer>`;

  overlay.querySelectorAll("[data-action=modal-close]").forEach((b) =>
    b.addEventListener("click", closeRecordingModals));
  overlay.querySelector("[data-action=rec-info-play]")?.addEventListener("click", () => {
    closeRecordingModals();
    openRecordingPlayer(jobId);
  });
  overlay.querySelector("[data-action=rec-info-delete]")?.addEventListener("click", async (e) => {
    if (!(await confirmDialog("Delete this recording? File moves to the 7-day trash.", { ok: "Delete", danger: true })))
      return;
    const btn = e.currentTarget;
    await withBusy(btn, "Deleting…", async () => {
      await API.deleteRecordingFile(jobId);
      Toast.success("Deleted");
      recCache = recCache.filter((r) => r.id !== jobId);
      closeRecordingModals();
      if (currentRoute() === "recordings") renderRecordings().catch(() => {});
    }).catch((err) => Toast.error(`Delete failed: ${err.message}`));
  });
  overlay.querySelectorAll("[data-action=rec-info-route-close]").forEach((a) =>
    a.addEventListener("click", () => closeRecordingModals()));
  overlay.querySelectorAll("[data-action=rec-info-verb]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      await withBusy(btn, "Queued…", async () => {
        await API.pluginRpc(btn.dataset.plugin, btn.dataset.verb, { selection: [jobId] });
        Toast.success(`${btn.dataset.verb} queued`);
      }).catch((err) => Toast.error(`${btn.dataset.verb} failed: ${err.message}`));
    });
  });
}

// ── In-app player ────────────────────────────────────────────────────
// Custom controls — power-user keyboard maps mirror mpv where the HTML5
// video API allows. State is owned by the modal; no globals (except the
// modal-open class) leak out.

async function openRecordingPlayer(jobId) {
  closeRecordingModals();
  const overlay = ensureModalContainer("rec-player-modal");
  overlay.innerHTML = `<div class="modal-card rec-player-card"><div class="empty sm">Loading…</div></div>`;
  document.body.classList.add("modal-open");
  overlay.addEventListener("click", (e) => { if (e.target === overlay) closeRecordingModals(); });

  let rec;
  try {
    rec = await API.recordingOne(jobId);
  } catch (e) {
    overlay.querySelector(".modal-card").innerHTML =
      `<div class="empty"><div class="glyph">⚠</div>${escape(e.message)}</div>`;
    return;
  }

  const src = `/api/v1/recordings/${encodeURIComponent(jobId)}/download`;
  const captionsUrl = `/api/v1/recordings/${encodeURIComponent(jobId)}/captions.vtt`;
  overlay.querySelector(".modal-card").innerHTML = `
    <header class="rec-player-head">
      <h2 class="rec-player-title">${escape(rec.stream_title || rec.channel_name || "Recording")}</h2>
      <button class="modal-close" aria-label="Close" data-action="modal-close">✕</button>
    </header>
    <div class="rec-player-stage">
      <video id="rec-player-vid" preload="metadata" tabindex="-1"></video>
      <div class="rec-player-overlay" id="rec-player-overlay" hidden>
        <div class="rec-player-overlay-msg" id="rec-player-overlay-msg"></div>
      </div>
    </div>
    <div class="rec-player-controls">
      <button class="rec-pc-btn" id="rec-pc-play" title="Play / Pause (Space)">▶</button>
      <span class="rec-pc-time" id="rec-pc-cur">0:00</span>
      <input type="range" class="rec-pc-seek" id="rec-pc-seek" min="0" max="1000" value="0" step="1" aria-label="Seek">
      <span class="rec-pc-time" id="rec-pc-dur">0:00</span>
      <span class="rec-pc-ab" id="rec-pc-ab" title="A-B loop (I / O / C)"></span>
      <label class="rec-pc-speed">
        Speed
        <select id="rec-pc-speed-sel">
          <option value="0.25">0.25×</option>
          <option value="0.5">0.5×</option>
          <option value="0.75">0.75×</option>
          <option value="1" selected>1×</option>
          <option value="1.25">1.25×</option>
          <option value="1.5">1.5×</option>
          <option value="1.75">1.75×</option>
          <option value="2">2×</option>
          <option value="3">3×</option>
          <option value="4">4×</option>
        </select>
      </label>
      <button class="rec-pc-btn" id="rec-pc-mute" title="Mute (M)">🔊</button>
      <input type="range" class="rec-pc-vol" id="rec-pc-vol" min="0" max="1" step="0.05" value="1" aria-label="Volume">
      <button class="rec-pc-btn" id="rec-pc-cc" title="Captions (T)" hidden>CC</button>
      <button class="rec-pc-btn" id="rec-pc-pip" title="Picture-in-picture (P)">⧉</button>
      <button class="rec-pc-btn" id="rec-pc-fs" title="Fullscreen (F)">⛶</button>
      <button class="rec-pc-btn" id="rec-pc-help" title="Keyboard help (?)">?</button>
    </div>
    <div class="rec-player-help" id="rec-player-help" hidden>
      <div class="rec-player-help-card">
        <h3>Keyboard</h3>
        <dl>
          <dt>Space</dt><dd>Play / Pause</dd>
          <dt>← / →</dt><dd>Skip ±5 s</dd>
          <dt>J / L</dt><dd>Skip ±10 s</dd>
          <dt>K</dt><dd>Play / Pause</dd>
          <dt>, / .</dt><dd>Frame step (1/30 s)</dd>
          <dt>&lt; / &gt;</dt><dd>Speed −/+</dd>
          <dt>↑ / ↓</dt><dd>Volume</dd>
          <dt>M</dt><dd>Mute</dd>
          <dt>I / O / C</dt><dd>Set A loop / Set B loop / Clear</dd>
          <dt>F</dt><dd>Fullscreen</dd>
          <dt>P</dt><dd>Picture-in-picture</dd>
          <dt>T</dt><dd>Toggle captions</dd>
          <dt>0 – 9</dt><dd>Seek to N · 10 %</dd>
          <dt>Esc</dt><dd>Close player</dd>
        </dl>
      </div>
    </div>`;

  const v = overlay.querySelector("#rec-player-vid");
  v.src = src;
  v.focus();
  // Probe captions sidecar; reveal the CC button only when present.
  fetch(captionsUrl, { method: "HEAD", credentials: "same-origin" })
    .then((r) => {
      if (r.ok) {
        const t = document.createElement("track");
        t.kind = "subtitles";
        t.src = captionsUrl;
        t.default = true;
        t.label = "Captions";
        v.appendChild(t);
        overlay.querySelector("#rec-pc-cc").hidden = false;
      }
    })
    .catch(() => {});

  wirePlayer(overlay, v);
  overlay.querySelectorAll("[data-action=modal-close]").forEach((b) =>
    b.addEventListener("click", closeRecordingModals));
}

function wirePlayer(overlay, v) {
  const playBtn = overlay.querySelector("#rec-pc-play");
  const seek = overlay.querySelector("#rec-pc-seek");
  const cur = overlay.querySelector("#rec-pc-cur");
  const dur = overlay.querySelector("#rec-pc-dur");
  const speedSel = overlay.querySelector("#rec-pc-speed-sel");
  const muteBtn = overlay.querySelector("#rec-pc-mute");
  const vol = overlay.querySelector("#rec-pc-vol");
  const ccBtn = overlay.querySelector("#rec-pc-cc");
  const pipBtn = overlay.querySelector("#rec-pc-pip");
  const fsBtn = overlay.querySelector("#rec-pc-fs");
  const helpBtn = overlay.querySelector("#rec-pc-help");
  const helpEl = overlay.querySelector("#rec-player-help");
  const abEl = overlay.querySelector("#rec-pc-ab");
  const overlayMsgEl = overlay.querySelector("#rec-player-overlay");
  const overlayMsgText = overlay.querySelector("#rec-player-overlay-msg");
  const state = { a: null, b: null, lastFlash: 0 };

  function flash(msg) {
    overlayMsgText.textContent = msg;
    overlayMsgEl.hidden = false;
    state.lastFlash = Date.now();
    setTimeout(() => {
      if (Date.now() - state.lastFlash >= 700) overlayMsgEl.hidden = true;
    }, 750);
  }
  function paintAb() {
    if (state.a == null && state.b == null) { abEl.textContent = ""; return; }
    const fmt = (s) => s == null ? "—" : fmtClock(s);
    abEl.textContent = `A ${fmt(state.a)} ↔ B ${fmt(state.b)}`;
  }

  v.addEventListener("loadedmetadata", () => {
    dur.textContent = fmtClock(v.duration || 0);
    seek.max = Math.max(1, Math.floor(v.duration * 10));
    // Audio-only files have 0×0 video boxes — collapse the 16:9 stage so
    // the player isn't a giant black rectangle, and hide PiP + fullscreen
    // (PiP throws on a no-video-track stream; fullscreen is pointless).
    const audioOnly = !v.videoWidth && !v.videoHeight;
    overlay.classList.toggle("audio-only", audioOnly);
  });
  v.addEventListener("timeupdate", () => {
    cur.textContent = fmtClock(v.currentTime || 0);
    seek.value = Math.floor((v.currentTime || 0) * 10);
    if (state.a != null && state.b != null && v.currentTime >= state.b) {
      v.currentTime = state.a;
    }
  });
  v.addEventListener("play", () => playBtn.textContent = "❚❚");
  v.addEventListener("pause", () => playBtn.textContent = "▶");
  v.addEventListener("error", () => {
    flash("Playback failed — your browser may not support this codec. Try Download from the row menu.");
    overlayMsgEl.hidden = false;
  });

  playBtn.addEventListener("click", () => { v.paused ? v.play() : v.pause(); });
  seek.addEventListener("input", () => { v.currentTime = Number(seek.value) / 10; });
  speedSel.addEventListener("change", () => { v.playbackRate = Number(speedSel.value); flash(`${v.playbackRate}×`); });
  muteBtn.addEventListener("click", () => { v.muted = !v.muted; muteBtn.textContent = v.muted ? "🔇" : "🔊"; });
  vol.addEventListener("input", () => { v.volume = Number(vol.value); v.muted = v.volume === 0; muteBtn.textContent = v.muted ? "🔇" : "🔊"; });
  ccBtn.addEventListener("click", () => {
    const tracks = v.textTracks;
    if (!tracks.length) return;
    const cur = tracks[0];
    cur.mode = cur.mode === "showing" ? "hidden" : "showing";
    ccBtn.classList.toggle("on", cur.mode === "showing");
  });
  pipBtn.addEventListener("click", async () => {
    try {
      if (document.pictureInPictureElement === v) await document.exitPictureInPicture();
      else await v.requestPictureInPicture();
    } catch (e) { flash(e.message); }
  });
  fsBtn.addEventListener("click", () => {
    if (document.fullscreenElement) document.exitFullscreen();
    else overlay.querySelector(".rec-player-stage").requestFullscreen?.();
  });
  helpBtn.addEventListener("click", () => { helpEl.hidden = !helpEl.hidden; });

  // Keyboard map.
  function onKey(e) {
    // Don't grab keystrokes inside the speed dropdown / sliders.
    if (e.target.closest("select, input")) return;
    const k = e.key;
    if (k === "?") { helpEl.hidden = !helpEl.hidden; e.preventDefault(); return; }
    if (k === " ") { v.paused ? v.play() : v.pause(); e.preventDefault(); return; }
    if (k === "ArrowLeft") { v.currentTime = Math.max(0, v.currentTime - 5); e.preventDefault(); return; }
    if (k === "ArrowRight") { v.currentTime = Math.min((v.duration || 0), v.currentTime + 5); e.preventDefault(); return; }
    if (k === "j" || k === "J") { v.currentTime = Math.max(0, v.currentTime - 10); e.preventDefault(); return; }
    if (k === "l" || k === "L") { v.currentTime = Math.min((v.duration || 0), v.currentTime + 10); e.preventDefault(); return; }
    if (k === "k" || k === "K") { v.paused ? v.play() : v.pause(); e.preventDefault(); return; }
    if (k === ",") { v.pause(); v.currentTime = Math.max(0, v.currentTime - 1/30); e.preventDefault(); return; }
    if (k === ".") { v.pause(); v.currentTime = Math.min((v.duration || 0), v.currentTime + 1/30); e.preventDefault(); return; }
    if (k === "<") { speedSel.selectedIndex = Math.max(0, speedSel.selectedIndex - 1); speedSel.dispatchEvent(new Event("change")); e.preventDefault(); return; }
    if (k === ">") { speedSel.selectedIndex = Math.min(speedSel.options.length - 1, speedSel.selectedIndex + 1); speedSel.dispatchEvent(new Event("change")); e.preventDefault(); return; }
    if (k === "ArrowUp") { vol.value = Math.min(1, Number(vol.value) + 0.05); vol.dispatchEvent(new Event("input")); e.preventDefault(); return; }
    if (k === "ArrowDown") { vol.value = Math.max(0, Number(vol.value) - 0.05); vol.dispatchEvent(new Event("input")); e.preventDefault(); return; }
    if (k === "m" || k === "M") { muteBtn.click(); e.preventDefault(); return; }
    if (k === "i" || k === "I") { state.a = v.currentTime; paintAb(); flash(`A = ${fmtClock(state.a)}`); e.preventDefault(); return; }
    if (k === "o" || k === "O") { state.b = v.currentTime; paintAb(); flash(`B = ${fmtClock(state.b)}`); e.preventDefault(); return; }
    if (k === "c" || k === "C") { state.a = null; state.b = null; paintAb(); flash("A-B cleared"); e.preventDefault(); return; }
    if (k === "f" || k === "F") { fsBtn.click(); e.preventDefault(); return; }
    if (k === "p" || k === "P") { pipBtn.click(); e.preventDefault(); return; }
    if (k === "t" || k === "T") { ccBtn.click(); e.preventDefault(); return; }
    if (/^[0-9]$/.test(k)) {
      const frac = Number(k) / 10;
      if (v.duration) v.currentTime = v.duration * frac;
      e.preventDefault(); return;
    }
  }
  overlay.addEventListener("keydown", onKey);
  // Tear the global keydown when modal closes — done implicitly because
  // the overlay is removed from the DOM in closeRecordingModals.
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

// ── Settings (Jellyfin-style two-pane shell) ────────────────────────
// Left rail = section nav (sub-route via #/settings/<section>).
// Right pane = section content. All knobs the daemon exposes get a
// visible row — read-only for now (Phase 2a). Phase 2b wires writes;
// Phase 2c adds the platforms wizard + keyring. Tooltip hints (the
// `title` attribute on .stg-hint) explain non-obvious knobs without
// cluttering the layout.
const SETTINGS_SECTIONS = [
  { slug: "general", label: "General", icon: "⚙" },
  { slug: "recording", label: "Recording", icon: "⏺" },
  { slug: "platforms", label: "Platforms", icon: "🔌" },
  { slug: "plugins", label: "Plugins", icon: "🧩" },
  { slug: "interface", label: "Interface", icon: "🎨" },
  { slug: "advanced", label: "Advanced", icon: "🛠" },
  { slug: "about", label: "About", icon: "ℹ" },
];

async function renderSettings() {
  const parts = routeParts(); // ["settings", <slug?>]
  const slug = parts[1] || "general";
  const known = SETTINGS_SECTIONS.find((s) => s.slug === slug)
    ? slug
    : "general";

  let s = {};
  try {
    s = await API.settings();
  } catch (e) {
    if (e.message && e.message.includes("unauthorized")) return;
  }
  root.removeAttribute("aria-busy");

  const rail = SETTINGS_SECTIONS.map((sec) => `
    <a class="stg-rail-item ${sec.slug === known ? "is-active" : ""}"
       href="#/settings/${sec.slug}">
      <span class="stg-rail-icon" aria-hidden="true">${sec.icon}</span>
      <span class="stg-rail-label">${escape(sec.label)}</span>
    </a>`).join("");

  const pane = renderSettingsPane(known, s);

  root.innerHTML = chrome(`
    <h1 class="page-title">Settings</h1>
    <p class="page-subtitle">Live daemon configuration. Editing lands in Phase 2b — for now changes go through the TUI or <code>~/.config/strivo/config.toml</code>.</p>
    <div class="stg-shell">
      <nav class="stg-rail" aria-label="Settings sections">${rail}</nav>
      <div class="stg-pane" id="stg-pane">${pane}</div>
    </div>
  `);
  setupChromeHandlers();
}

// Build the right-pane HTML for a section. Each section is a sequence of
// sub-headed groups, then a flat list of rows: label · value · hint.
function renderSettingsPane(slug, s) {
  const rec = s.recording || {};
  const arc = s.archiver || {};
  const ui = s.ui || {};
  const yesno = (b) => (b ? "Yes" : "No");
  const badge = (ok, okText, noText) =>
    `<span class="cfg-badge ${ok ? "ok" : "warn"}">${ok ? okText : noText}</span>`;
  const code = (v) => `<code>${escape(v || "—")}</code>`;
  // Row helper. `hint` is rendered as a tooltip on a ⓘ glyph so the
  // layout stays clean; long-form text only appears on hover.
  const row = (label, value, hint) => `
    <div class="stg-row">
      <div class="stg-row-label">
        ${escape(label)}
        ${hint ? `<span class="stg-hint" title="${escape(hint)}" aria-label="${escape(hint)}">ⓘ</span>` : ""}
      </div>
      <div class="stg-row-value">${value}</div>
    </div>`;
  const group = (title, rows) => `
    <section class="stg-group">
      <h2 class="stg-group-title">${escape(title)}</h2>
      <div class="stg-rows">${rows}</div>
    </section>`;

  switch (slug) {
    case "general":
      return [
        group("Polling", [
          row(
            "Channel poll interval",
            `${s.poll_interval_secs ?? "?"} s`,
            "How often StriVo checks each tracked channel for a live-state change. Twitch EventSub + YouTube WebSub push live signals in real time; this poll is the fallback.",
          ),
          row(
            "Auto-record channels",
            `${(s.auto_record_channels || []).length}`,
            "Channels whose new live broadcasts are recorded automatically. Managed from the Library page.",
          ),
        ].join("")),
        group("Storage", [
          row(
            "Recording directory",
            code(s.recording_dir),
            "Root directory for all recordings. Each platform/channel gets its own subdirectory.",
          ),
        ].join("")),
      ].join("");

    case "recording":
      return [
        group("Output", [
          row("Filename template", code(rec.filename_template),
            "Tokens like {channel}, {title}, {date} expand at record-start time."),
          row("Container", code(rec.container || "matroska (default)"),
            "Output muxer. Matroska is the browser-friendliest default; switch only if you have a downstream pipeline that needs MP4 or TS."),
          row("Transcode", yesno(rec.transcode),
            "Re-encode on the fly via h264_nvenc. Off = stream-copy (zero CPU, original bitrate)."),
        ].join("")),
        group("Twitch", [
          row("Record from start", yesno(rec.twitch_live_from_start),
            "Pull from the first available HLS segment (~5 min back) instead of the live edge. Sub-only channels reject this and StriVo silently falls back to live edge."),
        ].join("")),
        group("YouTube / VOD", [
          row("Auto VOD backfill", yesno(rec.auto_vod_backfill),
            "When a stream ends, automatically queue the resulting VOD for download via yt-dlp."),
          row("Auto-trim ads", yesno(rec.auto_trim_ads),
            "Run sponsorblock-style ad-segment trimming on completed Twitch VODs."),
        ].join("")),
      ].join("");

    case "platforms":
      return [
        group("Twitch", [
          row("Status", badge(s.twitch_configured, "configured", "not configured"),
            "Client-id + secret + user-token. Configure via the TUI's platform wizard (Phase 2c will add an in-app wizard)."),
        ].join("")),
        group("YouTube", [
          row("Status", badge(s.youtube_configured, "configured", "not configured"),
            "OAuth2 client + refresh token. Required for live-state detection and VOD downloads on channels you subscribe to."),
        ].join("")),
        group("Patreon", [
          row("Status", badge(s.patreon_configured, "configured", "not configured"),
            "Optional. Enables Patreon-locked VOD pulls from creators you support."),
        ].join("")),
      ].join("");

    case "plugins":
      return [
        group("Archiver", [
          row("Enabled", badge(arc.enabled, "enabled", "disabled"),
            "Back-catalog VOD archiver. Walks each tracked channel's history and downloads anything missing."),
          row("Archive directory", code(arc.archive_dir),
            "Where archived VODs land. Defaults under the main recording dir."),
          row("Format", code(arc.format),
            "yt-dlp format selector. Default targets bestvideo+bestaudio with a sensible cap."),
          row("Concurrent fragments", `${arc.concurrent_fragments ?? "—"}`,
            "yt-dlp -N flag. Higher = faster downloads, more bandwidth/CPU."),
        ].join("")),
        group("Other plugins", [
          row("Pro plugins", `<a href="#/plugins" class="stg-linkbtn">Open Plugins page →</a>`,
            "Crunchr, Viewguard, Insights. Activate Strivo Pro from the Plugins hub."),
        ].join("")),
      ].join("");

    case "interface":
      return [
        group("Accessibility", [
          row("Reduce motion", yesno(ui.reduce_motion),
            "Disables non-essential transitions across the UI. Mirrors the OS-level prefers-reduced-motion."),
          row("Verbose status", yesno(ui.verbose_status),
            "Adds extra status text to long-running operations. Useful on screen readers."),
        ].join("")),
        group("Scheduling", [
          row("Scheduled recordings", `${(s.schedule || []).length}`,
            "Cron-style fixed-time recordings. Edit via TUI."),
        ].join("")),
      ].join("");

    case "advanced":
      return [
        group("Daemon", [
          row("IPC socket", code("~/.local/share/strivo/strivo.sock"),
            "Unix socket the web UI uses to talk to the daemon. Path is fixed."),
          row("Persist DB", code("~/.local/share/strivo/jobs.db"),
            "Recording history + retry queue. SQLite."),
          row("Log file", code("~/.local/share/strivo/strivo.<date>.log"),
            "Rolling daily log. See the Logs page for live tail."),
        ].join("")),
        group("Developer", [
          row("Dev unlock", code(envOrDefault("STRIVO_DEV_UNLOCK_ALL", "off")),
            "Set STRIVO_DEV_UNLOCK_ALL=1 in the daemon's environment to bypass all Strivo Pro gating. Use during plugin development; never in shipped builds."),
        ].join("")),
      ].join("");

    case "about":
    default:
      return [
        group("Build", [
          row("Application", "StriVo",
            "Live-stream PVR for Twitch and YouTube."),
          row("Source", `<a href="https://github.com/Chorosyne/strivo" class="stg-linkbtn">github.com/Chorosyne/strivo →</a>`),
          row("Plugins", `<a href="#/plugins" class="stg-linkbtn">Plugin hub →</a>`),
        ].join("")),
        group("Licence", [
          row("Strivo Pro", `<a href="#/plugins" class="stg-linkbtn">Manage entitlement →</a>`,
            "One-time $25 unlock for every shipped plugin. Activate or start a 3-day trial from the Plugins hub."),
        ].join("")),
      ].join("");
  }
}

// envOrDefault is a UI-side helper: the daemon doesn't expose env vars
// to the client (it shouldn't — it's behind auth on the local box, but
// minimising attack surface anyway). Until we add a /api/v1/env route
// in Phase 3, surface the placeholder.
function envOrDefault(_name, dflt) {
  return `<span class="muted">${escape(dflt)}</span>`;
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
            <span class="task-cadence">every
              <input id="poll-interval" type="number" min="15" step="5"
                     value="${settings ? settings.poll_interval_secs : 60}"
                     aria-label="Poll interval seconds" /> s
              <button id="poll-interval-save" class="sm" title="Apply poll interval">Save</button>
            </span>
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
    await withBusy(e.currentTarget, "Polling…", async () => {
      await API.pollNow();
      Toast.success("Channel poll triggered");
    }).catch((err) => Toast.error(`Poll failed: ${err.message}`));
  });
  // Live-editable poll interval (item 14b) + inline field validation (item 25).
  document.getElementById("poll-interval-save")?.addEventListener("click", async (e) => {
    const input = document.getElementById("poll-interval");
    const raw = parseInt(input?.value, 10);
    if (!Number.isFinite(raw) || raw < 15) {
      input?.setAttribute("aria-invalid", "true");
      Toast.error("Poll interval must be at least 15 seconds");
      return;
    }
    input?.removeAttribute("aria-invalid");
    await withBusy(e.currentTarget, "Saving…", async () => {
      const r = await API.setPollInterval(raw);
      Toast.success(`Poll interval set to ${r.poll_interval_secs}s`);
    }).catch((err) => Toast.error(`Failed: ${err.message}`));
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
          <span class="task-cadence">${formatBytes(b.bytes || 0)} · ${(b.files || []).map(escape).join(", ")}</span>
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
    ? rows.map(recordingPillHtml).join("")
    : '<div class="empty">No recording history yet.</div>';
  root.innerHTML = chrome(`
    <h1 class="page-title">History</h1>
    <p class="page-subtitle">Durable record of every capture (survives restarts) · ${rows.length} entries</p>
    <div class="media-list">${body}</div>
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
// Compact relative age for the rail "last live" slot. Progression:
//   <1h  → "Xm ago"   (sub-hour, kept for usability on a freshly-offline row)
//   <1d  → "Xh ago"
//   <1mo → "Xd ago"
//   <1y  → "Xm ago"   or "Xm Yd ago" when there's a calendar-day remainder
//   ≥1y  → "Xy ago"   or "Xy Ym ago" when there's a calendar-month remainder
// Months and years are calendar-aware so leap years and short months don't lie.
function relTime(iso) {
  const past = new Date(iso);
  const t = past.getTime();
  if (!t) return "";
  const now = new Date();
  const secs = Math.max(0, Math.floor((now.getTime() - t) / 1000));
  if (secs < 60) return "just now";
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;

  // Calendar diff (UTC) — match by year / month / day fields so DST and
  // leap years don't shift the answer by a day.
  let years = now.getUTCFullYear() - past.getUTCFullYear();
  let months = now.getUTCMonth() - past.getUTCMonth();
  let days = now.getUTCDate() - past.getUTCDate();
  if (days < 0) {
    months -= 1;
    // Borrow days from the previous calendar month.
    const prev = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), 0));
    days += prev.getUTCDate();
  }
  if (months < 0) {
    years -= 1;
    months += 12;
  }

  if (years >= 1) return months > 0 ? `${years}y ${months}m ago` : `${years}y ago`;
  if (months >= 1) return days > 0 ? `${months}m ${days}d ago` : `${months}m ago`;
  return `${days}d ago`;
}

// Tooltip companion — absolute local timestamp, since the rail label already
// carries the relative form.
function lastLiveLong(iso) {
  const d = new Date(iso);
  if (!d.getTime()) return "unknown";
  return d.toLocaleString(undefined, {
    weekday: "short",
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
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
      // VOD downloads carry yt-dlp progress; stash on the cached job so
      // a re-render of the channel detail picks the latest values.
      if (p.download_pct != null) j.download_pct = p.download_pct;
      if (p.download_eta_secs != null) j.download_eta_secs = p.download_eta_secs;
      if (p.download_rate_bps != null) j.download_rate_bps = p.download_rate_bps;
      // Surgical DOM update for the matching VOD pill (avoids repainting
      // the whole channel detail every 2s).
      updateVodProgressDom(j);
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
        seedVodDownloadStateFromRecCache();
        updateLiveCount(recCache.filter((r) => isInProgress(r.state)).length);
        if (currentRoute() === "recordings") renderRecordings().catch(() => {});
        else {
          paintDashboard();
          paintChannelList();
        }
        // If a channel detail is open, refresh its VOD pills so any
        // newly-Finished source_url flips the button to Downloaded
        // (and any newly-Started one to Downloading).
        if (selectedChannelKey) {
          const [platform, id] = selectedChannelKey.split(":");
          if (id) paintChannelVods(id, platform);
        }
      })
      .catch(() => {});
  }

  // Explicit prune event from delete-recording / clear-errored — the daemon
  // tells us exactly which job_ids it dropped from jobs.db. Surgically
  // remove them from recCache + repaint, without an extra refetch.
  if (event.RecordingsPruned) {
    const ids = new Set(event.RecordingsPruned.job_ids || []);
    if (ids.size) {
      recCache = recCache.filter((r) => !ids.has(r.id));
      dashRecordings = recCache;
      updateLiveCount(recCache.filter((r) => isInProgress(r.state)).length);
      if (currentRoute() === "recordings") renderRecordings().catch(() => {});
      else { paintDashboard(); paintChannelList(); }
    }
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
