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
    if (res.status === 402) {
      // Pro gate — extract the plugin name + detail so callers can
      // render a polished upsell card instead of the raw JSON. Detail
      // shape from problem.rs: { detail, instance, status, title, type }.
      let detail = "Strivo Pro plugin — activate or start a 3-day trial.";
      let plugin = null;
      try {
        const j = await res.json();
        if (j && j.detail) {
          detail = j.detail;
          const m = /^([a-z0-9_-]+) is a Strivo Pro plugin/i.exec(j.detail);
          if (m) plugin = m[1];
        }
      } catch (_) { /* keep defaults */ }
      const err = new Error(detail);
      err.code = 402;
      err.plugin = plugin;
      throw err;
    }
    if (!res.ok) {
      // Try to extract problem+json's `detail` for a clean message; fall
      // back to the raw body when the response isn't JSON.
      const text = await res.text();
      let detail = text;
      try {
        const j = JSON.parse(text);
        if (j && typeof j.detail === "string") detail = j.detail;
      } catch (_) { /* not json */ }
      throw new Error(`HTTP ${res.status}: ${detail}`);
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
  scheduleAdd: (body) => API._fetch("/schedule", { method: "POST", body }),
  scheduleDelete: (index) =>
    API._fetch(`/schedule/${encodeURIComponent(index)}`, { method: "DELETE" }),
  // Monitor (record-when-live + auto-download new uploads).
  monitor: () => API._fetch("/monitor"),
  setArchiverTandem: (key, enabled) =>
    API._fetch(`/channels/${encodeURIComponent(key)}/archiver_tandem`, {
      method: "PUT",
      body: { enabled },
    }),
  setArchiverPlaylists: (key, playlists) =>
    API._fetch(`/channels/${encodeURIComponent(key)}/archiver_playlists`, {
      method: "PUT",
      body: { playlists },
    }),
  // DAW-vision capability matrix.
  pluginCapabilities: () => API._fetch("/plugins/capabilities"),
  chaptersGenerate: (recordingId) =>
    API._fetch(`/plugins/chapters/${encodeURIComponent(recordingId)}`, { method: "POST" }),
  cuepointsGenerate: (recordingId) =>
    API._fetch(`/plugins/cuepoints/${encodeURIComponent(recordingId)}`, { method: "POST" }),
  clipperAnalyze: (recordingId) =>
    API._fetch(`/plugins/clipper/${encodeURIComponent(recordingId)}/analyze`, { method: "POST" }),
  clipperExtract: (recordingId, body) =>
    API._fetch(`/plugins/clipper/${encodeURIComponent(recordingId)}/extract`, {
      method: "POST",
      body,
    }),
  clipperListClips: (recordingId) =>
    API._fetch(`/plugins/clipper/${encodeURIComponent(recordingId)}/clips`),
  thumbnailsGenerate: (recordingId, body) =>
    API._fetch(`/plugins/thumbnails/${encodeURIComponent(recordingId)}`, { method: "POST", body }),
  thumbnailsList: (recordingId, stem = "candidate") =>
    API._fetch(`/plugins/thumbnails/${encodeURIComponent(recordingId)}/${encodeURIComponent(stem)}`),
  thumbnailFileUrl: (absPath) =>
    `/api/v1/plugins/thumbnails/file?p=${encodeURIComponent(absPath)}`,
  insightsCompare: (recordingA, recordingB) =>
    API._fetch(`/plugins/insights/compare?recs=${encodeURIComponent(recordingA + "," + recordingB)}`),
  insightsRetention: (recordingId, bucketSecs = 30) =>
    API._fetch(`/plugins/insights/retention/${encodeURIComponent(recordingId)}?bucket_secs=${bucketSecs}`),
  captionsExportUrl: (recordingId, fmt = "srt", lang = "en") =>
    `/api/v1/plugins/captions/${encodeURIComponent(recordingId)}?fmt=${encodeURIComponent(fmt)}&lang=${encodeURIComponent(lang)}`,
  multitrackList: (recordingId) =>
    API._fetch(`/plugins/multitrack/${encodeURIComponent(recordingId)}`),
  multitrackExtract: (recordingId, body) =>
    API._fetch(`/plugins/multitrack/${encodeURIComponent(recordingId)}/extract`, { method: "POST", body }),
  brandsafeScan: (recordingId) =>
    API._fetch(`/plugins/brandsafe/${encodeURIComponent(recordingId)}`),
  reuseGenerate: (recordingId) =>
    API._fetch(`/plugins/reuse/${encodeURIComponent(recordingId)}/generate`, { method: "POST" }),
  reuseList: (recordingId) =>
    API._fetch(`/plugins/reuse/${encodeURIComponent(recordingId)}`),
  casebookFetch: (recordingId) =>
    API._fetch(`/plugins/casebook/${encodeURIComponent(recordingId)}?fmt=json`),
  casebookMarkdownUrl: (recordingId) =>
    `/api/v1/plugins/casebook/${encodeURIComponent(recordingId)}?fmt=markdown`,
  heatmapCompute: (recordingId, bucketSecs = 30) =>
    API._fetch(`/plugins/heatmap/${encodeURIComponent(recordingId)}?bucket_secs=${bucketSecs}`),
  editorLoad: (recordingId) =>
    API._fetch(`/plugins/editor/${encodeURIComponent(recordingId)}`),
  editorSave: (recordingId, edl, label) => {
    const qs = label ? `?label=${encodeURIComponent(label)}` : "";
    return API._fetch(`/plugins/editor/${encodeURIComponent(recordingId)}${qs}`, { method: "POST", body: edl });
  },
  editorRevisions: (recordingId) =>
    API._fetch(`/plugins/editor/${encodeURIComponent(recordingId)}/revisions`),
  editorRevisionRestore: (recordingId, revId) =>
    API._fetch(`/plugins/editor/${encodeURIComponent(recordingId)}/revisions/${encodeURIComponent(revId)}/restore`, { method: "POST" }),
  editorRender: (recordingId) =>
    API._fetch(`/plugins/editor/${encodeURIComponent(recordingId)}/render`, { method: "POST" }),
  scheduleOptimizerRun: (recordingId, body) =>
    API._fetch(`/plugins/schedule-optimizer/${encodeURIComponent(recordingId)}`, {
      method: "POST",
      body,
    }),
  scenesList: (recordingId) =>
    API._fetch(`/plugins/scenes/${encodeURIComponent(recordingId)}`),
  scenesCapture: (recordingId, name, thumbnailDataUrl) =>
    API._fetch(`/plugins/scenes/${encodeURIComponent(recordingId)}`, {
      method: "POST",
      body: { name, thumbnail_data_url: thumbnailDataUrl || null },
    }),
  scenesRestore: (recordingId, sceneId) =>
    API._fetch(
      `/plugins/scenes/${encodeURIComponent(recordingId)}/${encodeURIComponent(sceneId)}/restore`,
      { method: "POST" },
    ),
  scenesDelete: (recordingId, sceneId) =>
    API._fetch(
      `/plugins/scenes/${encodeURIComponent(recordingId)}/${encodeURIComponent(sceneId)}`,
      { method: "DELETE" },
    ),
  sidechainBuild: (recordingId, body) =>
    API._fetch(`/plugins/sidechain/${encodeURIComponent(recordingId)}`, {
      method: "POST",
      body,
    }),
  vadAnalyze: (recordingId, opts = {}) => {
    const p = new URLSearchParams();
    if (opts.window_sec != null) p.set("window_sec", opts.window_sec);
    if (opts.open_db != null) p.set("open_db", opts.open_db);
    if (opts.close_db != null) p.set("close_db", opts.close_db);
    if (opts.min_keep_sec != null) p.set("min_keep_sec", opts.min_keep_sec);
    const qs = p.toString() ? `?${p.toString()}` : "";
    return API._fetch(`/plugins/vad/${encodeURIComponent(recordingId)}${qs}`, { method: "POST" });
  },
  deadairDetect: (recordingId, opts = {}) => {
    const p = new URLSearchParams();
    if (opts.noise_db != null) p.set("noise_db", opts.noise_db);
    if (opts.min_span_secs != null) p.set("min_span_secs", opts.min_span_secs);
    if (opts.trim_threshold_secs != null) p.set("trim_threshold_secs", opts.trim_threshold_secs);
    const qs = p.toString() ? `?${p.toString()}` : "";
    return API._fetch(`/plugins/deadair/${encodeURIComponent(recordingId)}${qs}`, { method: "POST" });
  },
  chatRooms: () => API._fetch("/plugins/chat/rooms"),
  chatParseBatch: (lines) =>
    API._fetch("/plugins/chat/parse", { method: "POST", body: { lines: lines.join("\n") } }),
  structureClassify: (recordingId, body) =>
    API._fetch(`/plugins/structure/${encodeURIComponent(recordingId)}`, { method: "POST", body }),
  loudnessMeasure: (recordingId, platform) => {
    const qs = platform ? `?platform=${encodeURIComponent(platform)}` : "";
    return API._fetch(`/plugins/loudness/${encodeURIComponent(recordingId)}${qs}`, { method: "POST" });
  },
  multistreamTiles: (containerW, containerH, mode, host) => {
    const p = new URLSearchParams({ container_w: containerW, container_h: containerH, host });
    if (mode) p.set("mode", JSON.stringify(mode));
    return API._fetch(`/plugins/multistream/tiles?${p.toString()}`);
  },
  brandingLoad: (recordingId) =>
    API._fetch(`/plugins/branding/${encodeURIComponent(recordingId)}`),
  brandingSave: (recordingId, spec) =>
    API._fetch(`/plugins/branding/${encodeURIComponent(recordingId)}`, { method: "POST", body: spec }),
  viewguardTrend: () => API._fetch("/plugins/viewguard/trend"),
  pipelinesDag: () => API._fetch("/pipelines/dag"),
  marketplaceCatalog: () => API._fetch("/marketplace/catalog"),
  patreonPull: (body) =>
    API._fetch("/patreon/pull", { method: "POST", body }),
  vodDownload: (body) =>
    API._fetch("/vods/download", { method: "POST", body }),
  remuxRecording: (id) =>
    API._fetch(`/recordings/${encodeURIComponent(id)}/remux`, { method: "POST" }),
  login: (apiKey) =>
    API._fetch("/auth/login", { method: "POST", body: { api_key: apiKey } }),
  logout: () => API._fetch("/auth/logout", { method: "POST" }),
  // ── Strivo Pro licensing (Phase 1: status only; activate/trial 501) ──
  updateSetting: (path, value) =>
    API._fetch("/settings/update", { method: "POST", body: { path, value } }),
  setPlatform: (name, body) =>
    API._fetch(`/settings/platform/${encodeURIComponent(name)}`, {
      method: "POST",
      body,
    }),
  licenceStatus: () => API._fetch("/licence/status"),
  licenceTrial: () => API._fetch("/licence/trial", { method: "POST" }),
  licenceActivate: (key) =>
    API._fetch("/licence/activate", { method: "POST", body: { key } }),
  licenceTrial: () => API._fetch("/licence/trial", { method: "POST" }),
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
// State chip filter — Set of state classnames the user has whitelisted
// ("finished", "recording", "downloading", "failed", "file-error"…).
// Empty = no filter (show everything). Persisted across page reloads.
let recStateFilter = new Set(
  (localStorage.getItem("strivo-rec-state-filter") || "")
    .split(",").filter(Boolean),
);
// Group-by toggle — "none" or "channel". Persisted.
let recGroupBy = localStorage.getItem("strivo-rec-groupby") || "none";
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
  "watch",
  "chat",
  "settings",
  "system",
  "logs",
  "history",
  "login",
];

function currentRoute() {
  // Strip any query string ("#/recordings?channel=foo") so the route
  // matcher only sees the path segment.
  const raw = window.location.hash.replace(/^#\/?/, "").split("?")[0];
  const hash = raw || "library";
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
  // Clear any prior per-page hint before the new route paints; it'll be
  // re-mounted (if applicable) by maybeMountPageHint after the route
  // renderer finishes. Avoids stale Library copy bleeding onto Chat etc.
  document.getElementById("page-hint")?.remove();
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
    case "watch":
      await renderWatch();
      break;
    case "chat":
      await renderChat();
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
  // After whichever route renderer finishes, mount its per-page hint
  // unconditionally. Renderers that already call setupChromeHandlers()
  // (most of them) mounted earlier; this is a belt for the few that
  // bypass it (renderChat, renderWatch). maybeMountPageHint short-
  // circuits when a hint is already present, so the double call is
  // safe + idempotent.
  maybeMountPageHint(r);
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
  ["schedule", "📅", "Monitor", "s", "/assets/icons/candy/schedule.svg"],
  // Pipelines now ships the DAW-vision cross-plugin DAG; restored to
  // the topnav (was hidden in the audit when the page was empty).
  ["pipelines", "🔁", "Pipelines", "d", "/assets/icons/candy/pipelines.svg"],
  ["watch", "▦", "Watch", "w", "/assets/icons/candy/watch.svg"],
  ["chat", "💬", "Chat", "t", "/assets/icons/candy/chat.svg"],
  ["plugins", "🧩", "Plugins", "g", "/assets/icons/candy/plugins.svg"],
  ["settings", "⚙", "Settings", "c", "/assets/icons/candy/settings.svg"],
  ["system", "🛠", "System", "y", "/assets/icons/candy/system.svg"],
  ["logs", "📜", "Logs", "o", "/assets/icons/candy/logs.svg"],
  ["history", "🗂", "History", "h", "/assets/icons/candy/history.svg"],
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
  document.getElementById("logout")?.addEventListener("click", () => {
    // Quick confirm — one misclick on the topbar shouldn't sign you out.
    if (!confirm("Sign out? You'll need to re-enter the API key to come back.")) return;
    API.logout().catch(() => {}).then(() => route("login"));
  });
  // Health pill — amber/red when any check is degraded (roadmap item 13).
  refreshHealthPill();
  // Channel list lives in the left rail on every page.
  paintChannelList();
  // Per-page first-visit hint banner. No-op when this route's hint has
  // already been dismissed or no hint copy exists for the route.
  maybeMountPageHint(currentRoute());
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
    // Filter Recordings + History by this channel when clicked. Live
    // channels link to the recording dashboard so you can spot the
    // active capture quickly; offline rows go straight to the filtered
    // Recordings page (audit B7/M2).
    const href = c.is_live
      ? "#/library"
      : `#/recordings?channel=${encodeURIComponent(c.display_name || c.name || "")}`;
    return `
      <a class="ch-row ${c.is_live ? "live" : ""} ${isPatreon ? "patreon" : ""} ${sel}"
         data-channel-key="${key}" data-channel-id="${c.id}"
         data-platform="${c.platform}" href="${href}">
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
  // "Remember me" pre-fills the API key from localStorage on a returning
  // visit. The session cookie itself already persists across reloads via
  // the server; this just spares typing after a browser restart or a
  // dropped cookie (audit M18). Stored under a distinct key per host so
  // sharing a browser across StriVo instances stays clean.
  const remembered = (() => {
    try { return localStorage.getItem("strivo:remembered-api-key") || ""; }
    catch (_) { return ""; }
  })();
  root.innerHTML = `
    <div class="login-screen">
      <form class="login-card" id="login-form">
        <h1>StriVo</h1>
        <p class="subtitle">Sign in to the web console</p>
        <label for="api-key">API Key</label>
        <input type="password" id="api-key" autocomplete="current-password"
               value="${escape(remembered)}" autofocus />
        <label class="login-remember">
          <input type="checkbox" id="api-remember" ${remembered ? "checked" : ""} />
          <span>Remember on this browser</span>
        </label>
        <button type="submit" class="primary">Sign in</button>
        ${errorMsg ? `<div class="error">${escape(errorMsg)}</div>` : ""}
        <div class="hint">
          API key lives in <code>~/.config/strivo/config.toml</code> under
          <code>[web]</code>. <br />
          Or run: <code>strivo config get web.api_key</code><br />
          <span class="login-recovery">Lost it? Stop the daemon, edit
          <code>~/.config/strivo/config.toml</code>, replace the
          <code>api_key</code> with anything random, and restart.</span>
        </div>
      </form>
    </div>
  `;
  document.getElementById("login-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const key = document.getElementById("api-key").value.trim();
    if (!key) return;
    const remember = document.getElementById("api-remember").checked;
    try {
      await API.login(key);
      try {
        if (remember) localStorage.setItem("strivo:remembered-api-key", key);
        else localStorage.removeItem("strivo:remembered-api-key");
      } catch (_) {}
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
  //
  // Use Promise.allSettled so a transient failure on one side (e.g. the
  // daemon socket bouncing) doesn't drop the OTHER side's data into the
  // empty-rail state. Previously Promise.all rejected atomically and we
  // caught at the outer try/catch, leaving both caches stale — visually
  // that surfaced as "rail vanished" because the unauth check at the top
  // already returned for genuine 401s.
  const [chRes, recRes] = await Promise.allSettled([API.channels(), API.recordings()]);
  if (chRes.status === "fulfilled") {
    channelCache = chRes.value.channels || [];
  } else if (chRes.reason && chRes.reason.message && chRes.reason.message.includes("unauthorized")) {
    return;
  }
  if (recRes.status === "fulfilled") {
    recCache = recRes.value.recordings || [];
    dashRecordings = recCache;
    seedVodDownloadStateFromRecCache();
  } else if (recRes.reason && recRes.reason.message && recRes.reason.message.includes("unauthorized")) {
    return;
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
  // FILE MISSING overlay on the thumbnail mirrors the Recordings page
  // treatment so the Library dashboard doesn't quietly hide broken
  // rows (audit U2).
  const missingOverlay = j.file_exists === false
    ? '<span class="mp-missing">FILE MISSING</span>'
    : "";
  // Twitch live-pull + auto-VOD-backfill produces two rows per
  // broadcast — surface a small chip when the source is the
  // backfill path so the user can tell them apart at a glance
  // (audit B5). source_url is set when the recording was created
  // via DownloadVod (the backfill path).
  const sourceBadge = j.source_url
    ? '<span class="mp-source" title="From Twitch/YouTube VOD backfill">VOD</span>'
    : "";
  return `
    <div class="media-pill${j.file_exists === false ? " mp-broken" : ""}">
      <div class="mp-thumb">${missingOverlay}<img class="mp-thumb-img" loading="lazy" alt=""
        src="/api/v1/recordings/${encodeURIComponent(j.id)}/thumb" onerror="this.remove()"></div>
      <div class="mp-info">
        <div class="mp-title">${escape(niceTitle(j.stream_title) || j.channel_name || "(recording)")} ${sourceBadge}</div>
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
          <div class="mp-title">${escape(niceTitle(v.title))}</div>
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
    modal.className = "app-modal";
    document.body.appendChild(modal);
    modal.addEventListener("click", (e) => {
      if (e.target === modal) closeAppModal(modal);
    });
  }
  paintAddWizardSearch(modal);
  modal.classList.add("open");
  document.body.classList.add("modal-open");
}

// One owner for the click-outside / ESC / route-change dismissal of all
// .app-modal dialogs. Built so the keyboard-help (kbd-help) overlay
// stays separate — it has its own toggle and shouldn't be auto-closed
// on navigation.
function closeAppModal(modal) {
  if (!modal) return;
  modal.classList.remove("open");
  // Clear body lock only when no other modal is still open.
  if (!document.querySelector(".app-modal.open")) {
    document.body.classList.remove("modal-open");
  }
}
function closeAllAppModals() {
  document.querySelectorAll(".app-modal.open").forEach(closeAppModal);
}
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") closeAllAppModals();
  // Global jump-to-recording / channel — Ctrl/Cmd+K or "/". The "/"
  // shortcut is ignored when the user is typing in an existing field
  // (matches GitHub/Linear/Slack conventions). (audit M14)
  const inField =
    document.activeElement &&
    /^(INPUT|TEXTAREA|SELECT)$/i.test(document.activeElement.tagName);
  const isSlash = e.key === "/" && !inField;
  const isCmdK = (e.key === "k" || e.key === "K") && (e.metaKey || e.ctrlKey);
  if (isSlash || isCmdK) {
    e.preventDefault();
    openCommandPalette();
  }
});

// Lightweight command palette — list every recording title + every
// channel name and route the user there on pick. Single-pass filter.
async function openCommandPalette() {
  if (document.getElementById("cmd-palette")) return;
  const dlg = document.createElement("div");
  dlg.id = "cmd-palette";
  dlg.className = "app-modal open";
  dlg.innerHTML = `
    <form class="card cmd-card" role="dialog" aria-label="Quick jump">
      <input id="cmd-q" type="search" placeholder="Search recordings, channels, settings…" autofocus />
      <div id="cmd-results" class="cmd-results">Loading…</div>
      <p class="cmd-hint">↑↓ to navigate · Enter to open · Esc to close</p>
    </form>`;
  document.body.appendChild(dlg);
  document.body.classList.add("modal-open");
  dlg.addEventListener("click", (e) => { if (e.target === dlg) closeAppModal(dlg); });

  const [recs, chans] = await Promise.all([
    API.recordings().then((r) => r.recordings || []).catch(() => []),
    API.channels().then((r) => r.channels || []).catch(() => []),
  ]);
  const items = [
    ...recs.map((r) => ({
      label: niceTitle(r.stream_title) || "(no title)",
      sub: `${r.channel_name || ""} · recording`,
      href: `#/recordings`,
      hay: `${niceTitle(r.stream_title)} ${r.channel_name}`.toLowerCase(),
    })),
    ...chans.map((c) => ({
      label: c.display_name || c.name,
      sub: `${c.platform} · channel`,
      href: c.is_live ? "#/library" : `#/recordings?channel=${encodeURIComponent(c.display_name || c.name)}`,
      hay: `${c.display_name} ${c.name} ${c.platform}`.toLowerCase(),
    })),
    ...["library", "recordings", "schedule", "plugins", "settings", "system", "logs", "history"].map((r) => ({
      label: r[0].toUpperCase() + r.slice(1),
      sub: "page",
      href: `#/${r}`,
      hay: r,
    })),
  ];
  const out = dlg.querySelector("#cmd-results");
  const q = dlg.querySelector("#cmd-q");
  let active = 0;
  const paint = () => {
    const term = q.value.trim().toLowerCase();
    const hits = term
      ? items.filter((it) => it.hay.includes(term)).slice(0, 25)
      : items.slice(0, 25);
    if (!hits.length) {
      out.innerHTML = '<div class="empty sm">No matches.</div>';
      return;
    }
    active = Math.min(active, hits.length - 1);
    out.innerHTML = hits
      .map(
        (it, i) =>
          `<a class="cmd-item${i === active ? " is-active" : ""}" href="${escape(it.href)}" data-i="${i}">
            <span class="cmd-label">${escape(it.label)}</span>
            <span class="cmd-sub">${escape(it.sub)}</span>
          </a>`,
      )
      .join("");
    out.querySelectorAll(".cmd-item").forEach((el, i) => {
      el.addEventListener("click", (e) => {
        e.preventDefault();
        location.hash = hits[i].href;
        closeAppModal(dlg);
      });
    });
  };
  q.addEventListener("input", paint);
  q.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const visible = [...out.querySelectorAll(".cmd-item")];
      if (!visible.length) return;
      visible[active]?.classList.remove("is-active");
      active = (active + (e.key === "ArrowDown" ? 1 : visible.length - 1)) % visible.length;
      visible[active]?.classList.add("is-active");
      visible[active]?.scrollIntoView({ block: "nearest" });
    } else if (e.key === "Enter") {
      e.preventDefault();
      out.querySelectorAll(".cmd-item")[active]?.click();
    }
  });
  paint();
}
window.addEventListener("hashchange", closeAllAppModals);

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
    modal.className = "app-modal";
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
  // Allow sidebar links / external bookmarks to seed the search box
  // via #/recordings?channel=NAME (audit M2).
  const hash = window.location.hash || "";
  const qIdx = hash.indexOf("?");
  if (qIdx !== -1) {
    try {
      const params = new URLSearchParams(hash.slice(qIdx + 1));
      const ch = params.get("channel");
      if (ch != null) recFilter = ch;
    } catch (_) {}
  }
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
      <button id="rec-groupby" class="sm" title="Group rows by channel">
        ${recGroupBy === "channel" ? "▼ Grouped by channel" : "≣ Group by channel"}
      </button>
      <button id="rec-density" class="sm" title="Toggle row density">
        ${recDensity === "compact" ? "≡ Comfortable rows" : "═ Compact rows"}
      </button>
      ${(() => {
        const errored = recCache.filter((r) => stateClassName(r.state) === "failed" || stateLabel(r.state).toLowerCase().includes("interrupt")).length;
        return errored > 0
          ? `<button id="rec-clear-errored" class="danger sm" title="Trash all failed/interrupted recordings">✕ Clear errored (${errored})</button>`
          : "";
      })()}
    </div>
    <div id="rec-state-chips" class="rec-state-chips" role="group" aria-label="Filter by state"></div>
    <p class="page-subtitle" id="rec-count"></p>
    <div id="rec-massbar" class="massbar"></div>
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
  document.getElementById("rec-groupby")?.addEventListener("click", () => {
    recGroupBy = recGroupBy === "channel" ? "none" : "channel";
    localStorage.setItem("strivo-rec-groupby", recGroupBy);
    renderRecordings().catch((e) => Toast.error(e.message));
  });
  // Build state chips from the unique states currently in the cache, so
  // we don't paint chips for states that have zero rows. Each chip is a
  // toggle that AND-narrows the visible rows (empty filter = show all).
  paintRecStateChips();
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

function paintRecStateChips() {
  const host = document.getElementById("rec-state-chips");
  if (!host) return;
  const counts = new Map();
  for (const r of recCache) {
    const key = stateClassName(r.state);
    counts.set(key, (counts.get(key) || 0) + 1);
  }
  if (counts.size <= 1) {
    // Single state in cache → chips add no value; skip the row entirely.
    host.innerHTML = "";
    return;
  }
  const sorted = Array.from(counts.entries()).sort((a, b) => b[1] - a[1]);
  const chips = sorted
    .map(([state, n]) => {
      const active = recStateFilter.size === 0 || recStateFilter.has(state);
      return `<button class="rec-state-chip state-${escape(state)} ${active ? "active" : ""}"
                data-state="${escape(state)}" type="button">
        <span class="rec-state-chip-dot"></span>
        ${escape(stateChipLabel(state))}
        <span class="rec-state-chip-count">${n}</span>
      </button>`;
    })
    .join("");
  const allActive = recStateFilter.size === 0;
  host.innerHTML = `
    <button class="rec-state-chip rec-state-chip-all ${allActive ? "active" : ""}"
            type="button" title="Show every state">
      <span class="rec-state-chip-dot"></span>All <span class="rec-state-chip-count">${recCache.length}</span>
    </button>
    ${chips}`;
  host.querySelector(".rec-state-chip-all")?.addEventListener("click", () => {
    recStateFilter.clear();
    localStorage.setItem("strivo-rec-state-filter", "");
    paintRecStateChips();
    paintRecordings();
  });
  host.querySelectorAll("[data-state]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const s = btn.dataset.state;
      // Click pattern: starting from "all visible", a click selects ONLY
      // that state. Subsequent clicks toggle additional states (AND-narrow
      // turns into OR-additive — matches gmail's chip behaviour).
      if (recStateFilter.size === 0) {
        recStateFilter = new Set([s]);
      } else if (recStateFilter.has(s)) {
        recStateFilter.delete(s);
      } else {
        recStateFilter.add(s);
      }
      localStorage.setItem(
        "strivo-rec-state-filter",
        Array.from(recStateFilter).join(","),
      );
      paintRecStateChips();
      paintRecordings();
    });
  });
}

// Human-friendly label for a state classname. Falls back to title-case.
function stateChipLabel(cls) {
  switch (cls) {
    case "finished": return "Finished";
    case "recording": return "Recording";
    case "downloading": return "Downloading";
    case "failed": return "Failed";
    case "file-error": return "File missing";
    case "scheduled": return "Scheduled";
    default: return cls.replace(/[-_]/g, " ").replace(/\b\w/g, c => c.toUpperCase());
  }
}

function recHeader(key, label) {
  // Active column shows the direction arrow; inactive sortable columns
  // get a faint ↕ so the affordance is discoverable (R6 audit fix).
  const arrow =
    recSort.col === key
      ? (recSort.dir === "asc" ? " ▲" : " ▼")
      : ' <span class="rec-th-sort-hint" aria-hidden="true">↕</span>';
  return `<th data-sort="${key}" class="rec-th-sortable">${label}${arrow}</th>`;
}

// Apply the live filter + sort to recCache and repaint the table body.
function paintRecordings() {
  const body = document.getElementById("rec-body");
  if (!body) return;
  const q = recFilter.trim().toLowerCase();
  let rows = recCache.filter((r) => {
    if (recStateFilter.size > 0 && !recStateFilter.has(stateClassName(r.state))) return false;
    if (!q) return true;
    return (
      (r.channel_name || "").toLowerCase().includes(q) ||
      niceTitle(r.stream_title).toLowerCase().includes(q)
    );
  });
  const dir = recSort.dir === "asc" ? 1 : -1;
  const key = (r) => {
    switch (recSort.col) {
      case "state": return stateLabel(r.state).toLowerCase();
      case "channel": return (r.channel_name || "").toLowerCase();
      case "title": return niceTitle(r.stream_title).toLowerCase();
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
  if (recGroupBy === "channel") {
    // Cluster rows by channel_name while preserving the active sort order
    // within each cluster. Each cluster gets a heading row spanning every
    // column — sticky-styled via CSS — so the table reads like a grouped
    // ledger without needing a separate render pass per group.
    const order = [];
    const byChannel = new Map();
    for (const r of rows) {
      const k = r.channel_name || "(unknown)";
      if (!byChannel.has(k)) { byChannel.set(k, []); order.push(k); }
      byChannel.get(k).push(r);
    }
    const html = order.map((ch) => {
      const list = byChannel.get(ch);
      const totalBytes = list.reduce((a, b) => a + (b.bytes_written || 0), 0);
      return `<tr class="rec-group-head"><td colspan="7">
        <span class="rec-group-name">${escape(ch)}</span>
        <span class="rec-group-meta">${list.length} recording${list.length === 1 ? "" : "s"} · ${formatBytes(totalBytes)}</span>
      </td></tr>${list.map(recordingRow).join("")}`;
    }).join("");
    body.innerHTML = html;
  } else {
    body.innerHTML = rows.map(recordingRow).join("");
  }
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
  body.querySelectorAll("[data-action=rec-rescan]").forEach((btn) => {
    btn.addEventListener("click", (e) => { e.stopPropagation(); reScanRecording(btn); });
  });
  body.querySelectorAll("[data-action=rec-locate]").forEach((btn) => {
    btn.addEventListener("click", (e) => { e.stopPropagation(); showRecordingPath(btn.dataset.path); });
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
    // Audit fix: persistent toolbar so the bulk affordances are
    // discoverable BEFORE selection. Disabled buttons show what's
    // possible; selecting any row enables them.
    bar.hidden = false;
    bar.classList.add("massbar-empty");
    bar.innerHTML = `
      <span class="massbar-count muted">No rows selected — tick a checkbox to enable bulk actions</span>
      <button class="sm" disabled>Stop active</button>
      <button class="sm" disabled>Re-record</button>
      <button class="sm" disabled>Remux</button>
      <button class="danger sm" disabled>Delete</button>`;
    return;
  }
  bar.classList.remove("massbar-empty");
  const active = sel.filter((r) => stateClassName(r.state) === "recording");
  bar.hidden = false;
  // Pre-compute which selected rows are finished + look browser-broken,
  // so the Remux button is only offered when it could actually help.
  const remuxable = sel.filter((r) => stateClassName(r.state) === "finished" && r.file_exists !== false);
  const deletable = sel.filter((r) => r.file_exists !== false || stateClassName(r.state) !== "recording");
  bar.innerHTML = `
    <span class="massbar-count">${sel.length} selected</span>
    ${active.length ? `<button id="mass-stop" class="danger sm">Stop ${active.length} active</button>` : ""}
    <button id="mass-rerecord" class="sm">Re-record ${sel.length}</button>
    ${remuxable.length ? `<button id="mass-remux" class="sm" title="Remux for browser playback (matroska + aac_adtstoasc)">Remux ${remuxable.length}</button>` : ""}
    ${deletable.length ? `<button id="mass-delete" class="danger sm">Delete ${deletable.length}</button>` : ""}
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
  document.getElementById("mass-remux")?.addEventListener("click", async () => {
    if (!(await confirmDialog(`Remux ${remuxable.length} recording(s) for browser playback? Originals are kept as <name>.orig until success.`, { ok: "Remux" })))
      return;
    let ok = 0;
    for (const r of remuxable) {
      try {
        await API.remuxRecording(r.id);
        ok++;
      } catch (_) {}
    }
    Toast.success(`Remuxed ${ok}/${remuxable.length}`);
    recSelected.clear();
    setTimeout(() => render().catch(() => {}), 500);
  });
  document.getElementById("mass-delete")?.addEventListener("click", async () => {
    if (!(await confirmDialog(`Delete ${deletable.length} recording(s)? Files move to the 7-day trash.`, { ok: "Delete", danger: true })))
      return;
    let ok = 0;
    for (const r of deletable) {
      try {
        await API.deleteRecordingFile(r.id);
        ok++;
      } catch (_) {}
    }
    Toast.success(`Deleted ${ok}/${deletable.length}`);
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
  // Action set per state. Play sits in slot 1 across every row; when the
  // recording isn't playable yet we render a disabled placeholder so the
  // button columns stay vertically aligned (in-flight downloads + failed
  // captures previously dropped slot 1 and the remaining buttons hopped
  // left).
  const playBtn = isFinished
    ? `<button class="primary sm" data-action="rec-play" data-job-id="${r.id}" title="Open player (Enter)">▶ Play</button>`
    : `<button class="primary sm rec-play-disabled" disabled aria-disabled="true" title="${isActive ? "Playable when capture finishes" : "Recording unavailable"}">▶ Play</button>`;
  const tailBtns = isActive
    ? `<button class="danger sm" data-action="stop" data-job-id="${r.id}">Stop</button>`
    : `<button class="sm" data-action="rec-info" data-job-id="${r.id}" title="Recording details (I)">ⓘ Info</button>
       <button class="danger sm" data-action="rec-delete" data-job-id="${r.id}" title="Delete (Del)">✕</button>`;
  // File-error remediation: re-scan (re-check file_exists, in case the
  // user remounted a drive or restored from backup) + locate (show the
  // absolute path with a copy gesture). Distinct from Failed which is
  // a process error — file-error means the journal-vs-disk drifted.
  const fileErrorBtns = stateClass === "file-error"
    ? `<button class="sm" data-action="rec-rescan" data-job-id="${r.id}" title="Re-check whether the file exists">↻ Re-scan</button>
       <button class="sm" data-action="rec-locate" data-job-id="${r.id}" data-path="${escape(r.output_path || "")}" title="Show the expected file path">📂 Show path</button>`
    : "";
  const actions = `${playBtn}${fileErrorBtns}${tailBtns}`;
  return `
    <tr class="${recSelected.has(r.id) ? "rec-sel" : ""}" data-rec-row="${escape(r.id)}">
      <td class="rec-check"><input type="checkbox" class="rec-row-check" data-job-id="${escape(r.id)}" ${recSelected.has(r.id) ? "checked" : ""} aria-label="Select recording"></td>
      <td><span class="state-pill ${stateClass}">${state}</span></td>
      <td>${escape(r.channel_name)}</td>
      <td><div class="rec-title-cell">${recThumb(r)}<span>${escape(niceTitle(r.stream_title) || "(no title)")}</span></div></td>
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
// File-error remediation: refetch /recordings so the backend re-runs
// augment_recording's file_exists probe on the current row. When the
// flag flips back to true (file restored / drive remounted) the next
// render shows it as plain Finished again.
async function reScanRecording(btn) {
  const id = btn.dataset.jobId;
  await withBusy(btn, "Scanning…", async () => {
    try {
      const r = await API.recordingOne(id);
      if (r && r.file_exists !== false) {
        Toast.success("File found — refreshing");
      } else {
        Toast.error("Still missing — file not present at the recorded path");
      }
      // Whichever way it went, repaint the current route so the badge updates.
      render().catch(() => {});
    } catch (err) {
      Toast.error(`Re-scan failed: ${err.message}`);
    }
  });
}

// Pop a tiny copy-friendly modal showing the recording's intended file
// path. Doesn't try to open a native file manager (the SPA can't reach
// the desktop) — instead lets the user copy the path with one click so
// they can paste it into their own shell / finder.
function showRecordingPath(path) {
  if (!path) {
    Toast.error("No path recorded for this row");
    return;
  }
  const overlay = ensureModalContainer("rec-locate-modal");
  overlay.innerHTML = `
    <div class="modal-card rec-locate-card">
      <header class="rec-locate-head">
        <h2>Recording file path</h2>
        <button class="modal-close" data-action="modal-close" aria-label="Close">✕</button>
      </header>
      <p class="pg-cap-hint">The recording was written here. The SPA can't open your file manager directly — copy the path and open it yourself.</p>
      <div class="rec-locate-row">
        <code class="rec-locate-path">${escape(path)}</code>
        <button class="primary sm rec-locate-copy">Copy path</button>
      </div>
    </div>`;
  document.body.classList.add("modal-open");
  overlay.addEventListener("click", (e) => { if (e.target === overlay) closeRecLocate(); });
  overlay.querySelector("[data-action=modal-close]").addEventListener("click", closeRecLocate);
  overlay.querySelector(".rec-locate-copy").addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(path);
      Toast.success("Path copied to clipboard");
      closeRecLocate();
    } catch (err) {
      Toast.error(`Copy failed: ${err.message}`);
    }
  });
}
function closeRecLocate() {
  document.getElementById("rec-locate-modal")?.remove();
  document.body.classList.remove("modal-open");
}

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
// Plugins that have a dedicated SPA sub-route. Clicking a node routes
// there; everything else goes to the plugin hub so users land on the
// catalog entry.
const PIPELINE_NODE_ROUTES = new Set([
  "crunchr", "archiver", "viewguard", "insights",
  "schedule-optimizer",
]);

async function renderPipelines() {
  let payload = { pipelines: [] };
  let recs = { recordings: [] };
  try {
    [payload, recs] = await Promise.all([
      API.pipelinesDag(),
      API.recordings().catch(() => ({ recordings: [] })),
    ]);
  } catch (_) {}
  root.removeAttribute("aria-busy");
  const pipelines = payload.pipelines || [];
  // Cache finished recordings so the Run-on-… picker can list them.
  const finishedRecs = (recs.recordings || [])
    .filter((r) => stateClassName(r.state) === "finished" && r.file_exists !== false)
    .sort((a, b) => new Date(b.started_at) - new Date(a.started_at));

  const flow = (pipe) => {
    // Layout the nodes left→right by the topological order the server
    // shipped. Edges are encoded as " → " arrows between consecutive
    // nodes that actually connect, with a tag chip.
    const order = pipe.order && pipe.order.length ? pipe.order : pipe.nodes.map((n) => n.id);
    const nodeById = new Map(pipe.nodes.map((n) => [n.id, n]));
    const edges = pipe.edges || [];
    const edgeBetween = (a, b) => edges.find((e) => e.from === a && e.to === b);
    const cells = [];
    for (let i = 0; i < order.length; i++) {
      const node = nodeById.get(order[i]);
      if (!node) continue;
      const statusClass = node.status === "available" ? "is-avail" : "is-roadmap";
      const produces = (node.produces || [])
        .map((c) => `<span class="pl-cap pl-cap-produces">${escape(c.replace(/_/g, " "))}</span>`)
        .join("");
      const consumes = (node.consumes || [])
        .map((c) => `<span class="pl-cap pl-cap-consumes">${escape(c.replace(/_/g, " "))}</span>`)
        .join("");
      // Every node is a clickable anchor — routes to the plugin's own
      // sub-page when one exists, else to the plugin-hub catalog. The
      // hub upsell card (iter 26) handles the Pro-gate UX without us
      // having to know entitlement here.
      const href = PIPELINE_NODE_ROUTES.has(node.id)
        ? `#/plugins/${node.id}`
        : `#/plugins`;
      cells.push(`<a class="pl-node ${statusClass}" href="${escape(href)}"
          title="${escape(node.blurb)} · click to open ${escape(node.label)}"
          data-plugin="${escape(node.id)}">
          <div class="pl-node-head">
            <span class="pl-node-label">${escape(node.label)}</span>
            <span class="pl-node-status">${escape(node.status)}</span>
          </div>
          <div class="pl-node-caps">${consumes}${produces}</div>
        </a>`);
      const next = order[i + 1];
      if (next) {
        const eRec = edgeBetween(node.id, next);
        const viaLabel = eRec ? eRec.via.replace(/_/g, " ") : "";
        cells.push(`<div class="pl-arrow${eRec ? "" : " pl-arrow-loose"}" title="${escape(viaLabel)}">
          <span class="pl-arrow-line"></span>
          ${eRec ? `<span class="pl-arrow-via">${escape(viaLabel)}</span>` : ""}
          <span class="pl-arrow-tip">▸</span>
        </div>`);
      }
    }
    return cells.join("");
  };

  const cards = pipelines
    .map((p, idx) => {
      const totalNodes = (p.nodes || []).length;
      const availNodes = (p.nodes || []).filter((n) => n.status === "available").length;
      const pct = totalNodes === 0 ? 0 : Math.round((availNodes / totalNodes) * 100);
      return `
    <section class="cfg-card pl-pipe-card">
      <header class="pl-pipe-head">
        <h2 class="cfg-title">${escape(p.name)} <span class="pg-cap-hint">${escape(p.description)}</span></h2>
        <div class="pl-pipe-actions">
          <span class="pl-pipe-readiness ${pct === 100 ? "complete" : "partial"}"
                title="${availNodes} of ${totalNodes} stages available">
            ${availNodes}/${totalNodes} ready
          </span>
          <button class="sm pl-run-btn" data-pipe="${idx}"
                  ${finishedRecs.length === 0 ? "disabled title=\"No finished recordings available yet\"" : ""}>
            ▶ Run on…
          </button>
        </div>
      </header>
      <div class="pl-pipe-bar"><span style="width:${pct}%"></span></div>
      <div class="pl-flow">${flow(p)}</div>
    </section>`;
    })
    .join("");

  root.innerHTML = chrome(`
    <h1 class="page-title">Pipelines</h1>
    <p class="page-subtitle">
      Cross-plugin pipelines. Every artefact the DAW-vision toolkit ships rides one of these chains.
      Click any node to open the plugin · "Run on…" picks a recording and opens it in the appropriate view.
    </p>
    ${cards || '<div class="empty">No pipelines defined.</div>'}
  `);
  setupChromeHandlers();

  // Run-on-… picker: small overlay listing the 12 most recent finished
  // recordings. On pick we open the Info modal — that surface already
  // mounts every per-capability run button (Generate subtitles,
  // Detect cuepoints, Generate chapters, Render EDL, …), so each
  // pipeline-card's CTA reaches the right surface without us having to
  // model 'run pipeline' as a single server call.
  document.querySelectorAll(".pl-run-btn[data-pipe]").forEach((btn) => {
    btn.addEventListener("click", () => {
      if (btn.disabled) return;
      openRecordingPickerForPipeline(pipelines[parseInt(btn.dataset.pipe, 10)], finishedRecs);
    });
  });
}

function openRecordingPickerForPipeline(pipe, recs) {
  if (!recs.length) return;
  const overlay = ensureModalContainer("pl-run-picker");
  overlay.innerHTML = `
    <div class="modal-card pl-picker-card">
      <header class="pl-picker-head">
        <h2>Run "${escape(pipe.name)}" on a recording</h2>
        <button class="modal-close" data-action="modal-close" aria-label="Close">✕</button>
      </header>
      <p class="pg-cap-hint">Pick a recording. Its Info panel surfaces a button for every stage's plugin — we open straight to it so you can fire the chain.</p>
      <div class="pl-picker-list">
        ${recs.slice(0, 12).map((r) => `
          <button class="pl-picker-row" data-job-id="${escape(r.id)}" type="button">
            <span class="pl-picker-channel">${escape(r.channel_name || "(channel)")}</span>
            <span class="pl-picker-title">${escape(niceTitle(r.stream_title) || "(no title)")}</span>
            <span class="pl-picker-meta">${escape(new Date(r.started_at).toLocaleDateString())} · ${formatBytes(r.bytes_written || 0)}</span>
          </button>`).join("")}
      </div>
    </div>`;
  document.body.classList.add("modal-open");
  const close = () => {
    overlay.remove();
    document.body.classList.remove("modal-open");
  };
  overlay.addEventListener("click", (e) => { if (e.target === overlay) close(); });
  overlay.querySelector("[data-action=modal-close]").addEventListener("click", close);
  overlay.querySelectorAll(".pl-picker-row").forEach((row) => {
    row.addEventListener("click", () => {
      const id = row.dataset.jobId;
      close();
      openRecordingInfo(id);
    });
  });
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
// Chat client route — Twitch IRC over anonymous WSS, multi-tab Chatterino-
// style layout. The room list comes from the backend (followed Twitch
// channels live first); each active tab opens its own WS that auto-
// reconnects on close. Filter chips run client-side via the same logic
// shape the host's strivo-chat crate uses.
const chatState = {
  rooms: [],
  active: null,           // active room name (Twitch login)
  buffers: {},            // room → { messages: [], unread, mentions, watched_user }
  sockets: {},            // room → WebSocket
  filters: [],            // [{ kind, needle?, user? }]
  watched_user: null,     // your own twitch login if known (mention highlight)
  paint_timer: null,
};
const CHAT_TWITCH_WS = "wss://irc-ws.chat.twitch.tv:443";
const CHAT_ANON_NICK = () => `justinfan${Math.floor(10000 + Math.random() * 89999)}`;
const CHAT_BUFFER_CAP = 500;

function chatPushMsg(room, msg) {
  const buf = chatState.buffers[room] ||= { messages: [], unread: 0, mentions: 0 };
  if (buf.messages.length >= CHAT_BUFFER_CAP) buf.messages.shift();
  buf.messages.push(msg);
  buf.unread += 1;
  if (chatState.watched_user && msgMentionsUser(msg.text, chatState.watched_user)) {
    buf.mentions += 1;
  }
  schedulePaintChat();
}
function msgMentionsUser(text, user) {
  const target = user.replace(/^@/, "").toLowerCase();
  return text.split(/\s+/).some((w) => {
    const cleaned = w.replace(/[.,!?]+$/, "");
    return cleaned.startsWith("@") && cleaned.slice(1).toLowerCase() === target;
  });
}
function schedulePaintChat() {
  if (chatState.paint_timer) return;
  chatState.paint_timer = setTimeout(() => {
    chatState.paint_timer = null;
    paintChatBody();
  }, 50);
}
// Minimal client-side mirror of strivo-chat's parse_twitch_irc. We could
// round-trip through /plugins/chat/parse to use the host parser, but the
// WS firehose is high-rate and adding network latency per line would lag
// the live feed. Keep parity by reusing the host parser in batched
// previews (e.g. filter test on the recent buffer).
function parseTwitchIrc(line) {
  let rest = line.replace(/[\r\n]+$/, "");
  let tags = {};
  if (rest.startsWith("@")) {
    const sp = rest.indexOf(" ");
    if (sp < 0) return null;
    const raw = rest.slice(1, sp);
    for (const pair of raw.split(";")) {
      const eq = pair.indexOf("=");
      if (eq < 0) continue;
      tags[pair.slice(0, eq)] = pair.slice(eq + 1);
    }
    rest = rest.slice(sp + 1);
  }
  if (!rest.startsWith(":")) return null;
  const sp1 = rest.indexOf(" ");
  if (sp1 < 0) return null;
  const prefix = rest.slice(1, sp1);
  const sender = prefix.split("!")[0];
  rest = rest.slice(sp1 + 1);
  const sp2 = rest.indexOf(" ");
  if (sp2 < 0) return null;
  const verb = rest.slice(0, sp2);
  if (verb !== "PRIVMSG") return null;
  rest = rest.slice(sp2 + 1);
  const colon = rest.indexOf(" :");
  if (colon < 0) return null;
  const channel = rest.slice(0, colon).replace(/^#/, "");
  let text = rest.slice(colon + 2);
  let is_action = false;
  // Twitch wraps /me as CTCP \x01ACTION text\x01.
  const CTCP = String.fromCharCode(1);
  if (text.startsWith(CTCP) && text.endsWith(CTCP)) text = text.slice(1, -1);
  if (text.startsWith("ACTION ")) {
    text = text.slice("ACTION ".length);
    is_action = true;
  }
  // Badges with versions: 'subscriber/12,vip/1' → [{id:'subscriber',v:'12'},…]
  const badges = (tags["badges"] || "")
    .split(",")
    .filter(Boolean)
    .map((b) => {
      const [id, v] = b.split("/");
      return { id, version: v || "1" };
    });
  // Twitch native emote ranges: 'emote_id:start-end,start-end/emote_id:…'.
  // Parsed client-side mirroring strivo-chat::parse_twitch_emotes; the SPA
  // can't round-trip through the host parser cheaply on the live firehose.
  const emote_ranges = parseTwitchEmotes(tags["emotes"] || "");
  return {
    id: tags["id"] || `${channel}-${tags["tmi-sent-ts"] || Date.now()}`,
    room: channel,
    sender: tags["display-name"]?.replace(/\\s/g, " ") || sender,
    sender_color: tags["color"] || null,
    text,
    timestamp_ms: parseInt(tags["tmi-sent-ts"] || "0", 10),
    badges,
    emote_ranges,
    is_action,
    is_system: false,
    deleted: false,
  };
}

function parseTwitchEmotes(raw) {
  if (!raw) return [];
  const out = [];
  for (const group of raw.split("/")) {
    const colon = group.indexOf(":");
    if (colon < 0) continue;
    const id = group.slice(0, colon);
    for (const run of group.slice(colon + 1).split(",")) {
      const dash = run.indexOf("-");
      if (dash < 0) continue;
      const s = parseInt(run.slice(0, dash), 10);
      const e = parseInt(run.slice(dash + 1), 10);
      if (!isFinite(s) || !isFinite(e) || e < s) continue;
      out.push({ id, start: s, end: e });
    }
  }
  out.sort((a, b) => a.start - b.start);
  return out;
}

// BTTV global emotes — fetched once per session, keyed by emote code so
// the per-message tokenizer can substitute them inline. We don't pull
// channel-scoped BTTV/FFZ here (that needs the Twitch user id resolved
// at chat-join time; a future iter).
const bttvCache = { ready: false, map: new Map() };
async function ensureBttvGlobal() {
  if (bttvCache.ready) return bttvCache.map;
  try {
    const r = await fetch("https://api.betterttv.net/3/cached/emotes/global");
    if (!r.ok) throw new Error("bttv fetch failed");
    const list = await r.json();
    for (const e of list) {
      bttvCache.map.set(e.code, `https://cdn.betterttv.net/emote/${e.id}/1x`);
    }
  } catch (_) { /* graceful: chat works without BTTV */ }
  bttvCache.ready = true;
  return bttvCache.map;
}

function connectChatRoom(room) {
  if (chatState.sockets[room]) return;
  const ws = new WebSocket(CHAT_TWITCH_WS);
  chatState.sockets[room] = ws;
  ws.onopen = () => {
    ws.send("CAP REQ :twitch.tv/tags twitch.tv/commands");
    ws.send(`NICK ${CHAT_ANON_NICK()}`);
    ws.send(`JOIN #${room.toLowerCase()}`);
  };
  ws.onmessage = (ev) => {
    for (const line of ev.data.split(/\r?\n/)) {
      if (!line) continue;
      if (line.startsWith("PING ")) {
        try { ws.send(line.replace("PING", "PONG")); } catch (_) {}
        continue;
      }
      const m = parseTwitchIrc(line);
      if (m) chatPushMsg(room, m);
    }
  };
  ws.onclose = () => {
    delete chatState.sockets[room];
    // Auto-reconnect with backoff if room is still active.
    if (chatState.active === room) {
      setTimeout(() => connectChatRoom(room), 2500);
    }
  };
  ws.onerror = () => {
    try { ws.close(); } catch (_) {}
  };
}
function disconnectChatRoom(room) {
  const ws = chatState.sockets[room];
  if (ws) try { ws.close(); } catch (_) {}
  delete chatState.sockets[room];
}

function chatRoomMatchesFilters(msg) {
  for (const f of chatState.filters) {
    switch (f.kind) {
      case "keyword_in":
        if (!msg.text.toLowerCase().includes((f.needle || "").toLowerCase())) return false;
        break;
      case "keyword_out":
        if (msg.text.toLowerCase().includes((f.needle || "").toLowerCase())) return false;
        break;
      case "from_user":
        if (msg.sender.toLowerCase() !== (f.user || "").toLowerCase()) return false;
        break;
      case "no_links":
        if (msg.text.includes("http://") || msg.text.includes("https://")) return false;
        break;
      case "no_actions":
        if (msg.is_action) return false;
        break;
      case "mentions_user":
        if (!msgMentionsUser(msg.text, f.user || "")) return false;
        break;
    }
  }
  return true;
}

function paintChatBody() {
  const body = document.getElementById("chat-body");
  if (!body) return;
  const room = chatState.active;
  if (!room) return;
  const buf = chatState.buffers[room] || { messages: [] };
  const wasAtBottom = body.scrollHeight - body.scrollTop - body.clientHeight < 50;
  const visible = buf.messages.filter(chatRoomMatchesFilters);
  body.innerHTML = visible
    .slice(-200)
    .map((m) => {
      const cls = `chat-msg${m.deleted ? " deleted" : ""}${m.is_action ? " action" : ""}`;
      const senderCol = m.sender_color ? `style="color:${escape(m.sender_color)}"` : "";
      const badges = renderChatBadges(m.badges || []);
      const tokens = renderChatTokens(m.text, m.emote_ranges || []);
      const mentioned = chatState.watched_user && msgMentionsUser(m.text, chatState.watched_user)
        ? " mentioned" : "";
      return `<div class="${cls}${mentioned}">
        ${badges}<span class="chat-sender" ${senderCol}>${escape(m.sender)}</span><span class="chat-sep">:</span> <span class="chat-text">${tokens}</span>
      </div>`;
    })
    .join("");
  if (wasAtBottom) body.scrollTop = body.scrollHeight;
  // Repaint tabs (unread + mention counters drifted).
  paintChatTabs();
}

// Render badges as small images served from twitch's CDN. Falls back to
// the text chip for non-standard ids the SPA doesn't know URLs for.
const BADGE_CDN = (id, version) =>
  `https://static-cdn.jtvnw.net/badges/v1/${BADGE_UUIDS[id] || id}/${version}`;
// Mapping from semantic badge id → twitch CDN uuid. The handful of
// global badges have stable uuids; channel-scoped sub badges resolve
// at fetch time (a future iter — they need the broadcaster's id).
const BADGE_UUIDS = {
  // Empty by default — CDN URL pattern works for global badges by id
  // (subscriber/12, moderator/1, vip/1 etc.) so the fallback is fine
  // for the live firehose. Channel-scoped uuids land in a future iter.
};
function renderChatBadges(badges) {
  return badges.map((b) => {
    const url = BADGE_CDN(b.id, b.version);
    return `<img class="chat-badge-img" alt="${escape(b.id)}" title="${escape(b.id)}/${escape(b.version)}" src="${escape(url)}" onerror="this.outerHTML='<span class=&quot;chat-badge&quot;>${escape(b.id)}</span>'">`;
  }).join("");
}

// Token renderer with Twitch emote-range overlay + BTTV global emote
// substitution. Mirrors strivo-chat::tokenize_text_with_ranges so a
// future host parser switch keeps the same shape.
function renderChatTokens(text, ranges = []) {
  // Helper that classifies a single whitespace-split run.
  const classifyRun = (run) => {
    if (!run) return "";
    if (run.startsWith("@")) {
      const user = run.replace(/[.,!?]+$/, "").slice(1);
      if (/^[A-Za-z0-9_]+$/.test(user)) {
        return `<span class="chat-mention">@${escape(user)}</span>`;
      }
    }
    if (/^https?:\/\//.test(run)) {
      return `<a class="chat-link" href="${escape(run)}" target="_blank" rel="noopener noreferrer">${escape(run)}</a>`;
    }
    const bttv = bttvCache.map.get(run);
    if (bttv) {
      return `<img class="chat-emote" loading="lazy" alt="${escape(run)}" title="${escape(run)}" src="${escape(bttv)}">`;
    }
    return escape(run);
  };
  // Plain text path when there are no Twitch emote ranges.
  const renderPlain = (s) =>
    s.split(/(\s+)/).map((p) => /^\s+$/.test(p) ? p : classifyRun(p)).join("");
  if (!ranges.length) return renderPlain(text);
  // Twitch ranges are in CODE-POINT indices, not byte offsets. Walk by
  // chars so multi-byte codepoints (emoji-prefixed messages) stay aligned.
  const chars = Array.from(text);
  const out = [];
  let cursor = 0;
  for (const r of ranges) {
    if (r.start >= chars.length) continue;
    if (r.start > cursor) out.push(renderPlain(chars.slice(cursor, r.start).join("")));
    const end = Math.min(r.end + 1, chars.length);
    const name = chars.slice(r.start, end).join("");
    const url = `https://static-cdn.jtvnw.net/emoticons/v2/${r.id}/default/dark/1.0`;
    out.push(`<img class="chat-emote" loading="lazy" alt="${escape(name)}" title="${escape(name)}" src="${escape(url)}">`);
    cursor = end;
  }
  if (cursor < chars.length) out.push(renderPlain(chars.slice(cursor).join("")));
  return out.join("");
}

function paintChatTabs() {
  const tabs = document.getElementById("chat-tabs");
  if (!tabs) return;
  tabs.innerHTML = chatState.rooms.map((r) => {
    const buf = chatState.buffers[r.room] || { unread: 0, mentions: 0 };
    const active = r.room === chatState.active ? "active" : "";
    const mentionPill = buf.mentions > 0 ? `<span class="chat-tab-mentions">${buf.mentions}</span>` : "";
    const unreadPill = (buf.unread > 0 && r.room !== chatState.active)
      ? `<span class="chat-tab-unread">${buf.unread}</span>` : "";
    const liveDot = r.is_live ? `<span class="chat-tab-live" title="live">◉</span>` : "";
    const offline = r.connectable === false ? " offline" : "";
    return `<button class="chat-tab ${active}${offline}" data-room="${escape(r.room)}" ${!r.connectable ? "disabled" : ""}>
      ${liveDot}<span class="chat-tab-name">${escape(r.display_name)}</span>${mentionPill}${unreadPill}
    </button>`;
  }).join("");
  tabs.querySelectorAll(".chat-tab").forEach((t) => {
    t.addEventListener("click", () => {
      const room = t.dataset.room;
      switchChatRoom(room);
    });
  });
}

function switchChatRoom(room) {
  if (room === chatState.active) return;
  chatState.active = room;
  // Mark prior room as read (snapshot unread counter is preserved in buf,
  // but the badge clears on switch).
  if (chatState.buffers[room]) {
    chatState.buffers[room].unread = 0;
    chatState.buffers[room].mentions = 0;
  }
  connectChatRoom(room);
  paintChatTabs();
  paintChatBody();
}

async function renderChat() {
  root.innerHTML = chrome(`
    <div id="chat-root" class="chat-root">
      <aside id="chat-tabs" class="chat-tabs" role="tablist"></aside>
      <main class="chat-main">
        <div class="chat-filters">
          <input id="chat-filter-kw" type="text" placeholder="filter: contains…" />
          <input id="chat-filter-out" type="text" placeholder="filter: hide…" />
          <label class="chat-filter-tog"><input type="checkbox" id="chat-no-links"> no links</label>
          <label class="chat-filter-tog"><input type="checkbox" id="chat-no-actions"> no /me</label>
        </div>
        <div id="chat-body" class="chat-body" role="log" aria-live="polite"></div>
      </main>
    </div>
  `);
  // Kick off the BTTV global emote fetch in the background; we don't
  // await it because the chat firehose should never block on a third
  // party. ensureBttvGlobal() populates a module-level cache the
  // tokenizer consults on each repaint.
  ensureBttvGlobal().then(() => schedulePaintChat());
  let rooms;
  try {
    rooms = (await API.chatRooms()).rooms || [];
  } catch (e) {
    document.getElementById("chat-root").innerHTML =
      `<div class="empty"><div class="glyph">⚠</div>${escape(e.message)}</div>`;
    return;
  }
  // Live first, then alpha.
  rooms.sort((a, b) => {
    if (a.is_live !== b.is_live) return a.is_live ? -1 : 1;
    return a.display_name.localeCompare(b.display_name);
  });
  chatState.rooms = rooms;
  // Default to the first connectable room.
  const first = rooms.find((r) => r.connectable);
  if (first) chatState.active = first.room;
  paintChatTabs();
  if (chatState.active) {
    connectChatRoom(chatState.active);
    paintChatBody();
  } else {
    document.getElementById("chat-body").innerHTML =
      `<div class="empty"><div class="glyph">💬</div>
        <p>No Twitch channels followed yet. Add some in Settings → Channels.</p>
        <p class="pg-cap-hint">YouTube live chat needs an OAuth flow — coming soon.</p></div>`;
  }
  // Filter inputs.
  const applyFilters = () => {
    const kw = document.getElementById("chat-filter-kw").value.trim();
    const out = document.getElementById("chat-filter-out").value.trim();
    const noLinks = document.getElementById("chat-no-links").checked;
    const noActions = document.getElementById("chat-no-actions").checked;
    chatState.filters = [];
    if (kw) chatState.filters.push({ kind: "keyword_in", needle: kw });
    if (out) chatState.filters.push({ kind: "keyword_out", needle: out });
    if (noLinks) chatState.filters.push({ kind: "no_links" });
    if (noActions) chatState.filters.push({ kind: "no_actions" });
    paintChatBody();
  };
  document.getElementById("chat-filter-kw").addEventListener("input", applyFilters);
  document.getElementById("chat-filter-out").addEventListener("input", applyFilters);
  document.getElementById("chat-no-links").addEventListener("change", applyFilters);
  document.getElementById("chat-no-actions").addEventListener("change", applyFilters);
}

// Multi-stream viewer route. Server returns tiles already laid out for
// the requested container size + mode, plus each stream's ready-to-mount
// embed URL. Mode is kept in URL params so refresh / share preserves the
// view.
// Background poll handle so route switches can cancel the previous
// timer before mounting a new one.
let _watchRefreshTimer = null;

// Append the muted-state parameter Twitch / YouTube embeds use. Sticky-
// muted by default keeps every tile silent on mount (browsers block
// autoplay-with-audio anyway), and the per-tile Unmute button forces
// the solo stream to audible via a src reload.
function withMuted(url, muted) {
  if (!url) return url;
  const param = url.includes("youtube.com") ? `mute=${muted ? 1 : 0}` : `muted=${muted}`;
  return url + (url.includes("?") ? "&" : "?") + param;
}

async function renderWatch() {
  // Stop any prior refresh poll before we mount a new stage.
  if (_watchRefreshTimer) { clearInterval(_watchRefreshTimer); _watchRefreshTimer = null; }

  const params = new URLSearchParams(window.location.hash.split("?")[1] || "");
  const modeName = params.get("mode") || "auto";
  const focusId = params.get("focus") || "";
  const sideId = params.get("side") || "";
  const soloId = params.get("solo") || "";
  let mode;
  if (modeName === "focus" && focusId) mode = { mode: "focus", stream_id: focusId };
  else if (modeName === "pip" && focusId) mode = { mode: "pip", main: focusId, side: sideId };
  else mode = { mode: "auto" };
  root.innerHTML = chrome(`<div id="watch" class="watch-root" role="main"><div class="empty">Loading live streams…</div></div>`);
  const watch = document.getElementById("watch");
  const cw = Math.max(320, Math.floor(watch.clientWidth));
  const ch = Math.max(180, Math.floor(window.innerHeight - watch.getBoundingClientRect().top - 32));
  let resp;
  try {
    resp = await API.multistreamTiles(cw, ch, mode, window.location.host);
  } catch (e) {
    watch.innerHTML = `<div class="empty"><div class="glyph">⚠</div>${escape(e.message)}</div>`;
    return;
  }
  const streams = resp.streams || [];
  if (!streams.length) {
    watch.innerHTML = `<div class="empty watch-empty"><div class="glyph">▦</div>
      <p>No followed channels are live right now.</p>
      <p class="pg-cap-hint">Follow Twitch / YouTube channels in Settings — they'll auto-appear here when they go live.</p></div>`;
    return;
  }
  const tiles = resp.tiles || [];
  const modeBtn = (m, label, target) =>
    `<button class="sm watch-mode-btn ${modeName === m ? "active" : ""}" data-mode="${m}" data-target="${target || ""}">${label}</button>`;
  // Resolve the solo stream — defaults to none, but if the URL says
  // 'solo=<id>' the matching tile is the only one audible.
  const effectiveSolo = soloId && streams.find((s) => s.stream_id === soloId) ? soloId : "";
  const muteAllPressed = effectiveSolo ? "" : "active";
  const toolbar = `
    <div class="watch-toolbar">
      <span class="watch-count pg-cap-hint">${streams.length} live</span>
      ${modeBtn("auto", "▦ Auto")}
      ${streams.map((s) => modeBtn("focus", `◉ ${escape(s.channel_name)}`, s.stream_id)).join("")}
      ${streams.length >= 2 ? modeBtn("pip", "⧉ PiP", `${streams[0].stream_id}|${streams[1].stream_id}`) : ""}
      <span class="watch-tb-sep" aria-hidden="true">·</span>
      <button class="sm watch-mute-all ${muteAllPressed}" id="watch-mute-all" title="Mute every tile">🔇 Mute all</button>
    </div>`;
  const stage = document.createElement("div");
  stage.className = "watch-stage";
  stage.style.position = "relative";
  stage.style.width = `${cw}px`;
  stage.style.height = `${ch}px`;
  for (const t of tiles) {
    const s = streams.find((x) => x.stream_id === t.stream_id);
    if (!s) continue;
    const tile = document.createElement("div");
    tile.className = "watch-tile";
    tile.dataset.streamId = s.stream_id;
    Object.assign(tile.style, {
      position: "absolute", left: `${t.x}px`, top: `${t.y}px`,
      width: `${t.w}px`, height: `${t.h}px`, zIndex: t.z,
    });
    const muted = effectiveSolo ? s.stream_id !== effectiveSolo : true;
    const audioBtn = muted
      ? `<button class="watch-tile-btn watch-tile-unmute" title="Unmute this stream (mutes all others)" data-stream="${escape(s.stream_id)}">🔇</button>`
      : `<button class="watch-tile-btn watch-tile-mute" title="Mute this stream" data-stream="${escape(s.stream_id)}">🔊</button>`;
    tile.innerHTML = `
      <div class="watch-tile-head">
        <span class="watch-tile-name">${escape(s.channel_name)}</span>
        <span class="watch-tile-meta">
          <span class="watch-tile-plat pg-cap-hint" data-watch-meta="plat">${escape(s.platform)}${s.viewer_count != null ? ` · <span data-watch-meta="viewers">${formatCount(s.viewer_count)}</span>` : ""}</span>
          ${audioBtn}
          <button class="watch-tile-btn watch-tile-fs" title="Fullscreen this tile" data-stream="${escape(s.stream_id)}">⛶</button>
        </span>
      </div>
      <iframe class="watch-tile-iframe" loading="lazy" allow="autoplay; fullscreen"
              src="${escape(withMuted(s.embed_url, muted))}" allowfullscreen frameborder="0"></iframe>`;
    stage.appendChild(tile);
  }
  watch.innerHTML = "";
  watch.insertAdjacentHTML("beforeend", toolbar);
  watch.appendChild(stage);

  // Helper: rewrite the hash with a new param key/value, preserving
  // every other key. Re-renders the route automatically via hashchange.
  const setHashParam = (key, value) => {
    const p = new URLSearchParams(window.location.hash.split("?")[1] || "");
    if (value == null || value === "") p.delete(key);
    else p.set(key, value);
    window.location.hash = `#/watch${p.toString() ? "?" + p.toString() : ""}`;
  };

  watch.querySelectorAll(".watch-mode-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      const m = btn.dataset.mode;
      const target = btn.dataset.target || "";
      const p = new URLSearchParams();
      p.set("mode", m);
      if (m === "focus") p.set("focus", target);
      if (m === "pip") {
        const [main, side] = target.split("|");
        p.set("focus", main || "");
        p.set("side", side || "");
      }
      // Preserve solo across mode changes.
      if (effectiveSolo) p.set("solo", effectiveSolo);
      window.location.hash = `#/watch?${p.toString()}`;
    });
  });

  // Unmute → solo this stream (mutes all others). Pure URL-state
  // transition; re-render swaps each iframe's muted param.
  watch.querySelectorAll(".watch-tile-unmute").forEach((btn) => {
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      setHashParam("solo", btn.dataset.stream);
    });
  });
  watch.querySelectorAll(".watch-tile-mute").forEach((btn) => {
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      // Mute the currently-soloed stream → mute-all state.
      setHashParam("solo", "");
    });
  });
  document.getElementById("watch-mute-all")?.addEventListener("click", () => {
    setHashParam("solo", "");
  });
  // Per-tile fullscreen. Use the tile container rather than the iframe so
  // the head row stays visible inside the fullscreen overlay.
  watch.querySelectorAll(".watch-tile-fs").forEach((btn) => {
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      const tile = btn.closest(".watch-tile");
      if (!tile) return;
      if (document.fullscreenElement) {
        document.exitFullscreen?.();
      } else {
        tile.requestFullscreen?.();
      }
    });
  });

  // Background refresh: poll the tiles endpoint every 30s and patch the
  // per-tile viewer counts in place. Avoids tearing the iframes (the
  // streams keep playing) but keeps the meta-line fresh.
  const liveRefresh = async () => {
    try {
      const r = await API.multistreamTiles(cw, ch, mode, window.location.host);
      const byId = new Map((r.streams || []).map((s) => [s.stream_id, s]));
      // If the live-set changed (someone went offline / live), trigger a
      // full re-render so the toolbar + tile grid match reality.
      const have = new Set(streams.map((s) => s.stream_id));
      const got = new Set([...byId.keys()]);
      const sameSet = have.size === got.size && [...have].every((x) => got.has(x));
      if (!sameSet) {
        renderWatch().catch(() => {});
        return;
      }
      // Same streams, just patch viewer counts.
      watch.querySelectorAll(".watch-tile").forEach((tile) => {
        const s = byId.get(tile.dataset.streamId);
        if (!s) return;
        const meta = tile.querySelector('[data-watch-meta="viewers"]');
        if (meta && s.viewer_count != null) {
          meta.textContent = formatCount(s.viewer_count);
        }
      });
    } catch (_) { /* one bad poll shouldn't tear the page */ }
  };
  _watchRefreshTimer = setInterval(liveRefresh, 30000);
}

function toTitleCase(slug) {
  return slug.split(/[-_]/).map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join(" ");
}

// Single source of truth for the plugin set across the Settings →
// Plugins manager AND the hub. Every shipped plugin appears once with
// the metadata the SPA needs to render its enable toggle + open link.
const PLUGIN_REGISTRY = [
  // First-party Pro plugins with dedicated SPA sub-routes.
  { name: "crunchr",   label: "Crunchr",   category: "Transcription", route: "#/plugins/crunchr",   proGated: true,  description: "Transcribe every recording — speaker timeline, quote search, exportable subtitles." },
  { name: "archiver",  label: "Archiver",  category: "Archive",       route: "#/plugins/archiver",  proGated: true,  description: "Auto-catalog the full back-catalog of any followed channel." },
  { name: "insights",  label: "Insights",  category: "Analytics",     route: "#/plugins/insights",  proGated: true,  description: "Cross-stream analytics, word frequency, topic shifts, retention proxy." },
  { name: "viewguard", label: "Viewguard", category: "Analytics",     route: "#/plugins/viewguard", proGated: true,  description: "Live fraud-signal scoring during captures + cross-stream trend dashboard." },
  // Editor stack.
  { name: "editor",     label: "EDL editor",   category: "Editor", proGated: true, description: "Non-destructive EDL with split / ripple-delete / dead-air trim / branding overlay + revision history." },
  { name: "deadair",    label: "Dead-air trim", category: "Editor", proGated: true, description: "Silence detection + one-click EDL trim from inside the editor." },
  { name: "branding",   label: "Branding",      category: "Editor", proGated: true, description: "Watermark + intro/outro banner overlay spec, applied via ffmpeg filter_complex on render." },
  { name: "broll",      label: "B-roll finder", category: "Editor", proGated: true, description: "Suggest B-roll cuts from a tagged local library based on transcript topics." },
  { name: "loudness",   label: "Loudness",      category: "Editor", proGated: true, description: "EBU R128 master-bus loudness check with per-platform presets (YouTube/Spotify/Apple/EBU/Twitch)." },
  { name: "structure",  label: "Structure",     category: "Editor", proGated: true, description: "DAW-style section labeller — intro / gameplay / break / outro tiling from chapters + chat density + scene cuepoints." },
  { name: "automation", label: "Automation",    category: "Editor", proGated: true, description: "Volume automation curves — time-keyed gain with linear/cosine/step interpolation, baked via ffmpeg asendcmd." },
  { name: "scenes",     label: "Scene snapshots", category: "Editor", proGated: true, description: "DAW-style session save/recall — bundle every per-recording plugin state as a named scene." },
  { name: "schedule-optimizer", label: "Schedule optimizer", category: "Publish", proGated: true, description: "Publish-slot recommender — engagement samples → top weekly publish times with confidence + plateau coverage." },
  { name: "beat-detect",        label: "Beat detection",     category: "Editor", proGated: true, description: "DAW-style tempo grid — onset detector + BPM autocorrelation for music-sync montage cuts." },
  { name: "vad",                label: "Voice gate",         category: "Editor", proGated: true, description: "DAW-style noise gate — hysteresis VAD that surfaces auto-tighten ripple-deletes for podcast/commentary recordings." },
  { name: "sidechain",          label: "Sidechain compressor", category: "Editor", proGated: true, description: "DAW sidechain — VAD voice intervals → ducking automation curve baked via the existing volume-automation render path." },
  // Asset / analytics / publishing.
  { name: "chapters",         label: "Chapters",         category: "Analytics", proGated: true, description: "Heuristic chapter markers extracted from pacing." },
  { name: "cuepoints",        label: "Cuepoints",        category: "Analytics", proGated: true, description: "Scene-change detection from ffmpeg's select filter." },
  { name: "thumbnails",       label: "Thumbnails",       category: "Analytics", proGated: true, description: "Frame ranking + facecam crop candidates." },
  { name: "clipper",          label: "Clipper",          category: "Editor",    proGated: true, description: "Highlight detection + one-click clip extraction." },
  { name: "captions",         label: "Captions",         category: "Transcription", proGated: true, description: "SRT / VTT / TXT export with translator-trait pluggable backend." },
  { name: "multitrack",       label: "Multitrack",       category: "Editor",    proGated: true, description: "Audio track enumeration + extraction." },
  { name: "brandsafe",        label: "Brand safety",     category: "Publish",   proGated: true, description: "Pre-publish content classifier." },
  { name: "reuse",            label: "Reuse",            category: "Publish",   proGated: true, description: "Cross-format publish-queue drafter." },
  { name: "casebook",         label: "Casebook",         category: "Reports",   proGated: true, description: "Post-stream markdown briefing." },
  { name: "heatmap",          label: "Heatmap",          category: "Analytics", proGated: true, description: "Multi-signal retention overlay." },
  { name: "insights-compare", label: "Compare", category: "Analytics", proGated: true, description: "Stream-vs-stream side-by-side." },
  { name: "viewguard-trend",  label: "Viewguard trend",  category: "Analytics", proGated: true, description: "Cross-stream fraud trend dashboard." },
  { name: "chat-density",     label: "Chat density",     category: "Analytics", proGated: true, description: "Audience-retention proxy from chat rate." },
  // Viewer layer.
  { name: "multistream", label: "Multistream viewer", category: "Viewer", route: "#/watch", proGated: true, description: "Auto-tile any subset of currently-live followed channels." },
  { name: "chat",        label: "Chat client",       category: "Viewer", route: "#/chat",  proGated: true, description: "Chatterino-class IRC + tokenizer + filter pipeline + ring buffer." },
  // Cross-cutting.
  { name: "pipelines-dag", label: "Pipelines DAG", category: "Reports", route: "#/pipelines", proGated: true, description: "Cross-plugin pipeline graph." },
  { name: "marketplace",   label: "Marketplace",   category: "Reports", route: "#/plugins",   proGated: true, description: "Third-party plugin catalog stub." },
];

// Per-plugin pitch lines for the upsell card. Keyed by plugin name so the
// CTA copy stays specific instead of generic. Defaults to the plugin's
// description fetched from the marketplace catalog when present.
const PRO_UPSELL_PITCH = {
  crunchr: "Transcribe every recording, jump-to-quote search, speaker timeline, exportable subtitles.",
  archiver: "Auto-catalog the full back-catalog of any followed channel, dedup VODs, search by title or game.",
  insights: "Cross-stream analytics: word frequency, topic shifts, retention proxy, side-by-side compares.",
  viewguard: "Live fraud-signal scoring during captures; cross-stream trend dashboard.",
  editor: "Non-destructive EDL editor with split / ripple-delete / dead-air trim / branding overlay + revision history.",
  chapters: "Heuristic chapter markers extracted from your stream's pacing.",
  clipper: "Highlight detection + one-click clip extraction from the timeline.",
  captions: "Export SRT / VTT / TXT with a translator-trait pluggable backend.",
};

function renderProUpsell(plugin, licence) {
  const pitch = PRO_UPSELL_PITCH[plugin] || "Unlock this plugin's analytics, automation, and editor features.";
  const trial = licence && licence.trial;
  const hasTrialUsed = trial && trial.used;
  const trialNote = hasTrialUsed
    ? "Your 3-day trial has already been used on this machine."
    : "Start a free 3-day trial — no card needed.";
  const trialBtn = hasTrialUsed
    ? `<button class="btn-primary" disabled title="trial already used">Trial used</button>`
    : `<button class="btn-primary pg-upsell-trial">▶ Start 3-day trial</button>`;
  return `
    <div class="pg-upsell-card">
      <div class="pg-upsell-icon">★</div>
      <div class="pg-upsell-body">
        <h2 class="pg-upsell-title">${escape(toTitleCase(plugin))} is a Strivo Pro plugin</h2>
        <p class="pg-upsell-pitch">${escape(pitch)}</p>
        <p class="pg-upsell-trial-note pg-cap-hint">${escape(trialNote)}</p>
        <div class="pg-upsell-actions">
          ${trialBtn}
          <span class="pg-upsell-sep">or</span>
          <input type="text" class="pg-upsell-key" placeholder="paste licence key…" aria-label="licence key"/>
          <button class="sm pg-upsell-activate">Activate</button>
        </div>
        <p class="pg-upsell-foot pg-cap-hint">
          Already a subscriber? Find your key in your Strivo account.
        </p>
      </div>
    </div>`;
}

function wireProUpsell(host, plugin) {
  host.querySelector(".pg-upsell-trial")?.addEventListener("click", async (e) => {
    const btn = e.currentTarget;
    await withBusy(btn, "Starting…", async () => {
      try {
        await API.licenceTrial();
        Toast.success(`Trial active — ${toTitleCase(plugin)} unlocked. Refreshing…`);
        setTimeout(() => location.reload(), 800);
      } catch (err) {
        Toast.error(`Trial failed: ${err.message}`);
      }
    });
  });
  host.querySelector(".pg-upsell-activate")?.addEventListener("click", async (e) => {
    const btn = e.currentTarget;
    const key = host.querySelector(".pg-upsell-key").value.trim();
    if (!key) { Toast.error("Paste a licence key first."); return; }
    await withBusy(btn, "Activating…", async () => {
      try {
        await API.licenceActivate(key);
        Toast.success(`Activated — ${toTitleCase(plugin)} unlocked. Refreshing…`);
        setTimeout(() => location.reload(), 800);
      } catch (err) {
        Toast.error(`Activate failed: ${err.message}`);
      }
    });
  });
}

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
      case "schedule-optimizer":
        return await renderScheduleOptimizer();
      default:
        return await renderPluginHub();
    }
  } catch (e) {
    if (e.message && e.message.includes("unauthorized")) return;
    root.removeAttribute("aria-busy");
    if (e.code === 402) {
      const plugin = e.plugin || slug || "this plugin";
      root.innerHTML = chrome(
        `${pluginHeader(toTitleCase(plugin), "Strivo Pro")}<div id="pg-upsell-host"></div>`,
      );
      setupChromeHandlers();
      const host = document.getElementById("pg-upsell-host");
      const licence = await API.licenceStatus().catch(() => null);
      host.innerHTML = renderProUpsell(plugin, licence);
      wireProUpsell(host, plugin);
      return;
    }
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
  // Plugin first-action hints. Keyed by plugin name; shown when the
  // plugin reports zero data (audit U12). Long-form copy lives here
  // so non-engineers can iterate on it without touching render code.
  const PLUGIN_GETSTARTED = {
    crunchr: "Open a recording's ⓘ Info → Generate subtitles to transcribe it.",
    archiver: "Enable Archiver tandem on a channel from the channel row to start backfilling.",
    insights: "Insights aggregate Crunchr output — transcribe at least one recording.",
    viewguard: "Viewguard scores Twitch viewer signals during live captures.",
  };
  const cards = plugins
    .map((p) => {
      const stats = p.stats || {};
      const totalStats = Object.values(stats).reduce((a, b) => a + (Number(b) || 0), 0);
      const statBits = Object.entries(stats)
        .map(
          ([k, v]) =>
            `<span class="pg-stat"><strong>${formatCount(v)}</strong> ${escape(k.replace(/_/g, " "))}</span>`,
        )
        .join("");
      // Locked Pro plugins never reach the SPA — the server filters
      // them out of /api/v1/plugins when the gate denies. So this
      // only sees entitled or free plugins.
      const status = p.available
        ? `<span class="cfg-badge ok">ready</span>`
        : `<span class="cfg-badge">idle</span>`;
      const href = p.available ? `#/plugins/${p.name}` : null;
      // Get-started guidance fills the stats footprint while there's
      // nothing to count yet — replaces the bland "no data yet" stub.
      const statsHtml = statBits
        ? `<div class="pg-stats">${statBits}</div>`
        : totalStats === 0 && PLUGIN_GETSTARTED[p.name]
          ? `<div class="pg-getstarted"><strong>Get started:</strong> ${escape(PLUGIN_GETSTARTED[p.name])}</div>`
          : '<div class="pg-stats"><span class="pg-stat muted">no data yet</span></div>';
      const verbs = Array.isArray(p.verbs) && p.verbs.length
        ? `<div class="pg-verbs">${p.verbs
            .map(
              (v) =>
                `<span class="pg-verb-chip" title="${escape(v.scope ? `Scope: ${v.scope}` : "")}">${escape(v.label || v.verb)}</span>`,
            )
            .join("")}</div>`
        : "";
      const dataDir = p.data_dir
        ? `<div class="pg-meta"><code title="Plugin data folder">${escape(p.data_dir)}</code></div>`
        : "";
      const body = `
        <div class="pg-card-head">
          <span class="pg-icon pg-icon-${p.name}" aria-hidden="true">${escape((p.display || p.name)[0])}</span>
          <span class="pg-card-name">${escape(p.display || p.name)}</span>
          ${status}
          <a class="pg-card-gear" href="#/settings/plugins"
             title="Open plugin manager"
             onclick="event.stopPropagation()">⚙</a>
        </div>
        <p class="pg-card-desc">${escape(p.description || "")}</p>
        ${statsHtml}
        ${verbs}
        ${dataDir}`;
      return href
        ? `<a class="pg-card" href="${href}" data-plugin="${p.name}">${body}</a>`
        : `<div class="pg-card pg-card-idle" data-plugin="${p.name}">${body}</div>`;
    })
    .join("");
  // Capability matrix + marketplace both render lazily so the plugin
  // grid paints first.
  API.pluginCapabilities().then(renderCapabilityMatrix).catch(() => {});
  API.marketplaceCatalog().then(renderMarketplaceSection).catch(() => {});
  root.innerHTML = chrome(`
    ${pluginHeader("Plugins", "First-party plugins. Pick one to browse what it has produced.")}
    ${upgrade}
    <div id="pg-capability-matrix"></div>
    <div id="pg-marketplace"></div>
    <div class="pg-grid">${
      cards ||
      (upgrade
        ? '<div class="empty">Activate Strivo Pro above to populate this grid.</div>'
        : '<div class="empty">No plugins loaded.</div>')
    }</div>
  `);
  setupChromeHandlers();
  wireUpgradeCard();
}

// Render the DAW-vision capability matrix into #pg-capability-matrix.
// Built lazily so the plugin grid paints first. Groups roadmap vs.
// available providers so the user can see the trajectory at a glance.
function renderCapabilityMatrix(matrix) {
  const host = document.getElementById("pg-capability-matrix");
  if (!host || !Array.isArray(matrix)) return;
  const rows = matrix
    .map((row) => {
      const chips = (row.providers || [])
        .map(
          (p) =>
            // Two visible spans so CSS can give the state badge a pill of
            // its own — without the explicit element, `name+status` ran
            // together visually ("crunchravailable" / "chaptersroadmap").
            `<a class="pg-cap-chip pg-cap-${escape(p.status)}" href="#/plugins/${escape(p.plugin)}" title="${escape(p.plugin)} · ${escape(p.status)}">
              <span class="pg-cap-name">${escape(p.plugin)}</span>
              <span class="pg-cap-state pg-cap-state-${escape(p.status)}">${escape(p.status)}</span>
            </a>`,
        )
        .join("");
      const label = row.capability.replace(/_/g, " ");
      return `<div class="pg-cap-row">
        <span class="pg-cap-label">${escape(label)}</span>
        <span class="pg-cap-providers">${chips}</span>
      </div>`;
    })
    .join("");
  host.innerHTML = `
    <details class="pg-cap-matrix" open>
      <summary><strong>Capability matrix</strong> <span class="pg-cap-hint">— what each plugin contributes toward the DAW-for-streaming vision</span></summary>
      <div class="pg-cap-grid">${rows}</div>
    </details>`;
}

// Render the marketplace catalog into #pg-marketplace. Renders each
// plugin as a card with status badge (installed / available / coming
// soon), price chip, capability tags, and a primary action (Install
// when entry_point is real, "Watchlist" when roadmap).
function renderMarketplaceSection(payload) {
  const host = document.getElementById("pg-marketplace");
  if (!host || !payload || !payload.catalog || !payload.catalog.entries) return;
  const entries = payload.catalog.entries;
  const sourceColour = {
    first_party: "hsl(280, 60%, 65%)",
    verified: "hsl(140, 60%, 60%)",
    community: "hsl(35, 70%, 60%)",
  };
  const fmtPrice = (cents) => {
    if (cents == null) return '<span class="mk-free">free</span>';
    return `<span class="mk-price">$${(cents / 100).toFixed(2)}</span>`;
  };
  const entryStatus = (ep) => {
    const kind = (ep && ep.kind) || "roadmap";
    if (kind === "roadmap") return { label: "Coming soon", action: "Watchlist" };
    return { label: "Available", action: "Install" };
  };
  const cards = entries
    .map((e) => {
      const m = e.manifest;
      const sColour = sourceColour[e.source] || sourceColour.community;
      const status = entryStatus(m.entry_point);
      const caps = (m.capabilities || [])
        .slice(0, 6)
        .map((c) => `<span class="pl-cap pl-cap-produces" title="provides">${escape(c.replace(/_/g, " "))}</span>`)
        .join("");
      const consumes = (m.consumes || [])
        .slice(0, 4)
        .map((c) => `<span class="pl-cap pl-cap-consumes" title="needs">${escape(c.replace(/_/g, " "))}</span>`)
        .join("");
      return `<div class="mk-card" style="--mk-c:${sColour}">
        <div class="mk-card-head">
          <span class="mk-card-name">${escape(m.name)}</span>
          <span class="mk-source">${escape(e.source)}</span>
        </div>
        <div class="mk-card-meta">
          <span class="mk-version">v${escape(m.version)}</span>
          <span class="mk-author">${escape(m.author)}</span>
          ${fmtPrice(m.price_cents)}
        </div>
        <p class="mk-desc">${escape(m.description)}</p>
        <div class="mk-caps">${caps}${consumes}</div>
        <div class="mk-card-foot">
          <span class="mk-status">${escape(status.label)}</span>
          ${m.repository ? `<a class="pg-linkbtn" href="${escape(m.repository)}" target="_blank" rel="noopener">repository →</a>` : ""}
          <button class="sm" type="button" disabled title="Install endpoint lands in a follow-up">${escape(status.action)}</button>
        </div>
      </div>`;
    })
    .join("");
  host.innerHTML = `
    <details class="pg-cap-matrix mk-section" open>
      <summary><strong>Marketplace</strong> <span class="pg-cap-hint">third-party plugins · host v${escape(payload.host_version)}</span></summary>
      <div class="mk-grid">${cards}</div>
    </details>`;
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

// ── Schedule optimizer ────────────────────────────────────────────────
// 7×24 heatmap + top-slot recommender driven by the iter-44 backend.
// The iter ships with a synthetic dataset baked in so users can see the
// renderer work without first plumbing chat-density / Insights output;
// the textarea lets them paste real samples too.
const DAYS_OF_WEEK = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const SAMPLE_DATASET = [
  // Friday afternoon plateau.
  { day_of_week: 4, hour_of_day: 14, score: 70 },
  { day_of_week: 4, hour_of_day: 14, score: 72 },
  { day_of_week: 4, hour_of_day: 14, score: 68 },
  { day_of_week: 4, hour_of_day: 15, score: 75 },
  { day_of_week: 4, hour_of_day: 15, score: 72 },
  { day_of_week: 4, hour_of_day: 16, score: 70 },
  // Tuesday early-hour spike (high score, isolated → low coverage).
  { day_of_week: 1, hour_of_day: 3, score: 80 },
  { day_of_week: 1, hour_of_day: 3, score: 78 },
  // Thursday evening cluster.
  { day_of_week: 3, hour_of_day: 20, score: 65 },
  { day_of_week: 3, hour_of_day: 20, score: 63 },
  { day_of_week: 3, hour_of_day: 21, score: 68 },
  // Sunday singleton.
  { day_of_week: 6, hour_of_day: 18, score: 55 },
];

function heatmapColor(mean, lo, hi) {
  // Cool → warm map. Empty cells stay neutral; we only call this when
  // count > 0 so the gradient endpoints are real numbers.
  if (!isFinite(mean) || hi <= lo) return "rgba(255,255,255,0.04)";
  const t = ((mean - lo) / (hi - lo)).max?.(0)?.min?.(1) ?? Math.max(0, Math.min(1, (mean - lo) / (hi - lo)));
  // Lerp from cyan (low) → amber (mid) → red (high).
  const stops = [
    [0.0, [76, 201, 240]],
    [0.5, [251, 191, 36]],
    [1.0, [239, 68, 68]],
  ];
  let lo_stop = stops[0], hi_stop = stops[stops.length - 1];
  for (let i = 0; i < stops.length - 1; i++) {
    if (t >= stops[i][0] && t <= stops[i + 1][0]) {
      lo_stop = stops[i]; hi_stop = stops[i + 1];
      break;
    }
  }
  const span = (hi_stop[0] - lo_stop[0]) || 1;
  const u = (t - lo_stop[0]) / span;
  const rgb = lo_stop[1].map((c, i) => Math.round(c + (hi_stop[1][i] - c) * u));
  return `rgb(${rgb[0]}, ${rgb[1]}, ${rgb[2]})`;
}

let schedOptState = {
  samplesText: JSON.stringify(SAMPLE_DATASET, null, 2),
  topN: 3,
  mode: "spread",
  minGap: 4,
  lastResp: null,
};

async function renderScheduleOptimizer() {
  root.innerHTML = chrome(`
    ${pluginHeader("Schedule optimizer",
      "DAW launch-quantize for publish slots — engagement samples → 7×24 grid → top weekly publish times."
    )}
    <div class="sopt-grid">
      <section class="cfg-card sopt-input">
        <h2 class="cfg-title">Engagement samples</h2>
        <p class="pg-cap-hint">JSON list of <code>{day_of_week (0–6), hour_of_day (0–23), score}</code>. The seeded dataset shows the canonical plateau-vs-spike scenario; paste your own from chat-density or VOD-views data.</p>
        <textarea id="sopt-samples" class="sopt-samples" spellcheck="false"></textarea>
        <div class="sopt-controls">
          <label><span>Top N</span>
            <input id="sopt-topn" type="number" min="1" max="14" value="${schedOptState.topN}"/>
          </label>
          <label><span>Mode</span>
            <select id="sopt-mode">
              <option value="spread" ${schedOptState.mode === "spread" ? "selected" : ""}>Spread (min-gap)</option>
              <option value="greedy" ${schedOptState.mode === "greedy" ? "selected" : ""}>Greedy</option>
            </select>
          </label>
          <label><span>Min gap (h)</span>
            <input id="sopt-mingap" type="number" min="0" max="23" value="${schedOptState.minGap}"/>
          </label>
          <button id="sopt-run" class="btn-primary sm" type="button">▶ Run optimizer</button>
        </div>
      </section>
      <section class="cfg-card sopt-output" id="sopt-output">
        <h2 class="cfg-title">Recommendations</h2>
        <div class="pg-cap-hint">Run the optimizer to see top publish slots + the weekly heatmap.</div>
      </section>
    </div>
  `);
  setupChromeHandlers();
  document.getElementById("sopt-samples").value = schedOptState.samplesText;
  document.getElementById("sopt-run").addEventListener("click", () => runScheduleOptimizer());
  // Auto-run on mount if we never have — gives users an instant view.
  if (!schedOptState.lastResp) {
    runScheduleOptimizer().catch(() => {});
  } else {
    paintScheduleOptimizer();
  }
}

async function runScheduleOptimizer() {
  const samplesText = document.getElementById("sopt-samples").value.trim();
  let samples;
  try { samples = JSON.parse(samplesText); }
  catch (err) { Toast.error(`Samples JSON invalid: ${err.message}`); return; }
  if (!Array.isArray(samples)) { Toast.error("Samples must be a JSON array"); return; }
  const topN = parseInt(document.getElementById("sopt-topn").value, 10) || 3;
  const mode = document.getElementById("sopt-mode").value;
  const minGap = parseInt(document.getElementById("sopt-mingap").value, 10) || 4;
  schedOptState.samplesText = samplesText;
  schedOptState.topN = topN;
  schedOptState.mode = mode;
  schedOptState.minGap = minGap;
  const out = document.getElementById("sopt-output");
  out.innerHTML = `<h2 class="cfg-title">Recommendations</h2><div class="empty sm">Running…</div>`;
  try {
    const resp = await API.scheduleOptimizerRun("interactive", {
      samples,
      top_n: topN,
      mode,
      min_gap_hours: minGap,
    });
    schedOptState.lastResp = resp;
    paintScheduleOptimizer();
  } catch (err) {
    out.innerHTML = `<h2 class="cfg-title">Recommendations</h2><div class="empty"><div class="glyph">⚠</div>${escape(err.message)}</div>`;
  }
}

function paintScheduleOptimizer() {
  const out = document.getElementById("sopt-output");
  if (!out) return;
  const resp = schedOptState.lastResp;
  if (!resp) return;
  const picks = resp.recommendations || [];
  // Pull min/max across non-empty cells for the heatmap colour scale.
  let lo = Infinity, hi = -Infinity;
  const buckets = resp.grid?.buckets || [];
  for (const row of buckets) {
    for (const b of row) {
      if (b.count > 0) { lo = Math.min(lo, b.mean); hi = Math.max(hi, b.mean); }
    }
  }
  if (!isFinite(lo)) { lo = 0; hi = 1; }
  // Header row + day rows.
  const hourCells = [];
  for (let h = 0; h < 24; h++) hourCells.push(`<div class="sopt-hour-label">${h}</div>`);
  const dayRows = DAYS_OF_WEEK.map((day, dIdx) => {
    const cells = [];
    for (let h = 0; h < 24; h++) {
      const b = buckets[dIdx]?.[h] || { mean: 0, count: 0 };
      if (b.count === 0) {
        cells.push(`<div class="sopt-cell sopt-cell-empty" title="${day} ${h}:00 · no data"></div>`);
      } else {
        const color = heatmapColor(b.mean, lo, hi);
        const isPick = picks.some(p => p.day_of_week === dIdx && p.hour_of_day === h);
        cells.push(`<div class="sopt-cell ${isPick ? "sopt-cell-pick" : ""}"
          title="${day} ${h}:00 · mean ${b.mean.toFixed(1)} · n=${b.count}"
          style="background:${color}"></div>`);
      }
    }
    return `<div class="sopt-day-label">${day}</div>${cells.join("")}`;
  }).join("");
  const picksHtml = picks.map((p, i) => `
    <div class="sopt-pick">
      <div class="sopt-pick-rank">#${i + 1}</div>
      <div class="sopt-pick-when"><strong>${DAYS_OF_WEEK[p.day_of_week]}</strong> ${String(p.hour_of_day).padStart(2, "0")}:00</div>
      <div class="sopt-pick-mean">mean <strong>${p.mean_score.toFixed(1)}</strong></div>
      <div class="sopt-pick-bars">
        <div class="sopt-pick-bar" title="confidence ${(p.confidence*100).toFixed(0)}%"><span style="width:${(p.confidence*100).toFixed(1)}%"></span></div>
        <div class="sopt-pick-bar coverage" title="coverage ${(p.window_coverage*100).toFixed(0)}%"><span style="width:${(p.window_coverage*100).toFixed(1)}%"></span></div>
      </div>
      <div class="sopt-pick-meta pg-cap-hint">n=${p.sample_count} · conf ${(p.confidence*100).toFixed(0)}% · coverage ${(p.window_coverage*100).toFixed(0)}%</div>
    </div>`).join("");
  out.innerHTML = `
    <h2 class="cfg-title">Recommendations <span class="pg-cap-hint">${resp.sample_count} sample${resp.sample_count===1?"":"s"} · ${picks.length} pick${picks.length===1?"":"s"}</span></h2>
    <div class="sopt-picks">${picksHtml || '<div class="empty sm">No picks — try a wider range or check your sample data.</div>'}</div>
    <h3 class="sopt-heatmap-h">Weekly heatmap</h3>
    <div class="sopt-heatmap">
      <div class="sopt-corner"></div>
      ${hourCells.join("")}
      ${dayRows}
    </div>
    <div class="sopt-legend">
      <span>${lo.toFixed(1)}</span>
      <div class="sopt-legend-bar"></div>
      <span>${hi.toFixed(1)}</span>
    </div>
  `;
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
            <span class="pg-row-title">${escape(niceTitle(r.title) || "(untitled)")}</span>
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
    <div class="pg-list">${rows || `<div class="empty">Nothing transcribed yet.</div>
      <div class="pg-getstarted"><strong>Get started:</strong> open a finished recording's ⓘ Info on the Recordings page and click <em>Generate subtitles</em>. Transcripts land here after the run completes.</div>`}</div>
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

  const segments = d.segments || [];
  // Build the set of distinct speakers for the filter chip row.
  const speakers = [...new Set(segments.map((s) => s.speaker).filter(Boolean))].sort();
  // Group consecutive same-speaker lines into a single block (Descript-
  // style readability). Each block keeps the seek timestamp of its
  // first line; its `lines` keep their own timestamps for line-level
  // click-to-seek inside the block.
  const blocks = [];
  for (const seg of segments) {
    const top = blocks[blocks.length - 1];
    if (top && top.speaker === seg.speaker) {
      top.lines.push(seg);
    } else {
      blocks.push({ speaker: seg.speaker, lines: [seg] });
    }
  }
  // Speaker chip colours — deterministic per speaker name so the same
  // person stays the same colour across reloads / recordings.
  const speakerColour = (name) => {
    if (!name) return "#888";
    let h = 0;
    for (const ch of name) h = (h * 31 + ch.charCodeAt(0)) | 0;
    return `hsl(${Math.abs(h) % 360}, 55%, 65%)`;
  };

  const chipsRow = speakers.length
    ? `<div class="cr-chips" id="cr-chips">
        <button class="cr-chip is-active" data-spk="" type="button">all</button>
        ${speakers
          .map(
            (s) =>
              `<button class="cr-chip is-active" data-spk="${escape(s)}" style="--cr-spk:${speakerColour(s)}" type="button"><span class="cr-chip-dot"></span>${escape(s)}</button>`,
          )
          .join("")}
      </div>`
    : "";

  const blockHtml = blocks
    .map((b) => {
      const colour = speakerColour(b.speaker);
      const firstStart = b.lines[0]?.start_sec ?? 0;
      const linesHtml = b.lines
        .map(
          (line) =>
            `<span class="cr-line" data-seek="${line.start_sec ?? 0}" title="Open player at ${fmtClock(line.start_sec)}">${escape(line.text)}</span>`,
        )
        .join(" ");
      return `<div class="cr-block" data-spk="${escape(b.speaker || "")}">
        <div class="cr-block-meta">
          <button class="cr-block-jump" data-seek="${firstStart}" title="Jump to ${fmtClock(firstStart)}">${fmtClock(firstStart)}</button>
          ${b.speaker ? `<span class="cr-block-spk" style="--cr-spk:${colour}"><span class="cr-spk-dot"></span>${escape(b.speaker)}</span>` : ""}
        </div>
        <div class="cr-block-body">${linesHtml}</div>
      </div>`;
    })
    .join("");

  root.innerHTML = chrome(`
    ${pluginHeader(d.title || "Transcript", `${escape(d.channel_name)} · ${escape(d.status)}`, "#/plugins/crunchr")}
    <div class="pg-verbs">
      <button id="retranscribe" data-rec="${escape(d.recording_id)}">↻ Re-transcribe</button>
      <a class="pg-linkbtn" href="#/plugins/insights/rec/${encodeURIComponent(d.recording_id)}">View insights →</a>
      <button id="cr-export-vtt" class="pg-linkbtn" type="button">Export .vtt</button>
      <button id="cr-export-md" class="pg-linkbtn" type="button">Copy as markdown</button>
      <button id="cr-chapters" class="pg-linkbtn" type="button" title="Generate YouTube/Twitch chapter markers from the transcript">Generate chapters</button>
      <button id="cr-brandsafe" class="pg-linkbtn" type="button" title="Pre-publish brand-safety scan (slurs / profanity / restricted games / music mentions)">⚠ Brand-safety scan</button>
      <div class="cr-caption-export">
        <span class="cr-caption-label">Captions:</span>
        <a class="pg-linkbtn" download href="${escape(API.captionsExportUrl(d.recording_id, "srt", "en"))}">.srt</a>
        <a class="pg-linkbtn" download href="${escape(API.captionsExportUrl(d.recording_id, "vtt", "en"))}">.vtt</a>
        <a class="pg-linkbtn" download href="${escape(API.captionsExportUrl(d.recording_id, "txt", "en"))}">.txt</a>
        <select id="cr-caption-lang" title="Target language (translation backend ships in a follow-up; today returns identity)">
          <option value="en">en (identity)</option>
          <option value="es">es</option>
          <option value="pt">pt</option>
          <option value="ja">ja</option>
          <option value="de">de</option>
          <option value="fr">fr</option>
        </select>
      </div>
    </div>
    <section class="cfg-card" id="cr-chapters-card" hidden>
      <h2 class="cfg-title">Chapters</h2>
      <p class="pg-cap-hint">Heuristic chapter markers derived from the transcript topic-shift. Paste straight into a YouTube/Twitch description.</p>
      <div class="cr-chapters-list" id="cr-chapters-list"></div>
      <details class="cr-chapters-block"><summary>Description block</summary><pre id="cr-chapters-pre"></pre></details>
      <div class="cr-chapters-actions">
        <button id="cr-chapters-copy" class="pg-linkbtn" type="button">Copy</button>
      </div>
    </section>
    ${analysis}
    <section class="cfg-card" id="cr-heatmap-card" hidden>
      <h2 class="cfg-title">Heatmap <span class="pg-cap-hint">talk · action · highlight · brand-safety (anti-signal)</span></h2>
      <div id="cr-heatmap-strip"></div>
      <div id="cr-heatmap-top"></div>
    </section>
    <section class="cfg-card" id="cr-brandsafe-card" hidden>
      <h2 class="cfg-title">Brand-safety verdicts <span id="cr-brandsafe-count"></span></h2>
      <div id="cr-brandsafe-list"></div>
    </section>
    <section class="cfg-card">
      <h2 class="cfg-title">Transcript <span class="pg-cap-hint">${speakers.length} speaker${speakers.length === 1 ? "" : "s"} · ${blocks.length} block${blocks.length === 1 ? "" : "s"}</span></h2>
      <div class="cr-retention" id="cr-retention" hidden></div>
      ${chipsRow}
      <div class="pg-transcript cr-transcript">${blockHtml || '<div class="empty sm">No segments — transcription may still be running.</div>'}</div>
    </section>
  `);
  // Lazy multi-signal heatmap — surfaces alongside the existing
  // retention curve below. Pulls cuepoints/highlights/brandsafe from
  // their caches; no second ffmpeg pass required.
  if (d.recording_id) {
    API.heatmapCompute(d.recording_id, 30).then((resp) => {
      const card = document.getElementById("cr-heatmap-card");
      const strip = document.getElementById("cr-heatmap-strip");
      const topHost = document.getElementById("cr-heatmap-top");
      if (!card || !strip || !topHost) return;
      const buckets = resp.buckets || [];
      if (!buckets.length) return;
      const dur = resp.duration_sec || 1;
      const bandRow = (key, label, colour) => {
        const bars = buckets
          .map((b) => `<span class="cr-hm-cell" style="--cr-hm-h:${Math.round(b[key] * 100)}%;--cr-hm-c:${colour};" title="${fmtClock(b.bucket_start)} · ${label} ${(b[key] * 100).toFixed(0)}%"></span>`)
          .join("");
        return `<div class="cr-hm-row"><span class="cr-hm-label">${escape(label)}</span><div class="cr-hm-bars">${bars}</div></div>`;
      };
      const fusedRow = `<div class="cr-hm-row cr-hm-row-fused"><span class="cr-hm-label"><strong>fused</strong></span><div class="cr-hm-bars">${buckets
        .map((b) => `<a class="cr-hm-cell cr-hm-fused-bar" data-seek="${b.bucket_start}" href="#" style="--cr-hm-h:${Math.round(b.fused * 100)}%;--cr-hm-hue:${200 - Math.round((b.highlight - b.brandsafe) * 60)};" title="${fmtClock(b.bucket_start)} · retention ${(b.fused * 100).toFixed(0)}%"></a>`)
        .join("")}</div></div>`;
      strip.innerHTML = `
        ${bandRow("talk", "talk", "hsl(200, 70%, 55%)")}
        ${bandRow("action", "action", "hsl(40, 80%, 60%)")}
        ${bandRow("highlight", "highlight", "hsl(120, 60%, 55%)")}
        ${bandRow("brandsafe", "brandsafe", "hsl(0, 70%, 60%)")}
        ${fusedRow}
        <div class="rec-cp-axis"><span>0:00</span><span>${fmtClock(dur)}</span></div>`;
      const top = (resp.top_k || []).map(
        (b) => `<a class="cr-hm-top" href="#" data-seek="${b.bucket_start}">${fmtClock(b.bucket_start)} <span>${(b.fused * 100).toFixed(0)}%</span></a>`,
      );
      topHost.innerHTML = top.length
        ? `<h5 class="ins-cmp-h">Top moments</h5><div class="cr-hm-top-row">${top.join("")}</div>`
        : "";
      strip.querySelectorAll(".cr-hm-fused-bar, .cr-hm-top").forEach((el) => {
        el.addEventListener("click", (e) => {
          e.preventDefault();
          seek(parseFloat(el.dataset.seek || "0"));
        });
      });
      card.hidden = false;
    }).catch(() => {});
  }

  // Lazy retention curve — async so the transcript paints first.
  if (d.recording_id) {
    API.insightsRetention(d.recording_id, 30).then((retention) => {
      const host = document.getElementById("cr-retention");
      if (!host || !retention || !retention.points || !retention.points.length) return;
      const dur = retention.duration_sec || 1;
      // Compose a sparkline-ish strip: each bucket is a vertical bar
      // whose height encodes retention and whose hue carries the
      // talk/action mix (cyan-ish for talk-heavy, magenta-ish for
      // action-heavy). Click any bar → seek the (future) player.
      const bars = retention.points
        .map((p) => {
          const pct = Math.max(0, Math.min(1, p.retention || 0));
          const hue = 200 - Math.round((p.action_density - p.talk_density) * 60);
          return `<a class="cr-ret-bar" href="#" data-seek="${p.bucket_start}" title="${fmtClock(p.bucket_start)} · retention ${(pct * 100).toFixed(0)}%" style="--ret-h:${(pct * 100).toFixed(0)}%; --ret-hue:${hue}"></a>`;
        })
        .join("");
      host.hidden = false;
      host.innerHTML = `
        <div class="cr-ret-head">
          <span>Retention proxy</span>
          <span class="pg-cap-hint">${retention.points.length} buckets · ${retention.bucket_secs}s each · talk + cuepoint density</span>
        </div>
        <div class="cr-ret-strip" role="img" aria-label="Retention curve">${bars}</div>
        <div class="rec-cp-axis"><span>0:00</span><span>${fmtClock(dur)}</span></div>`;
      host.querySelectorAll(".cr-ret-bar").forEach((el) => {
        el.addEventListener("click", (e) => {
          e.preventDefault();
          seek(parseFloat(el.dataset.seek || "0"));
        });
      });
    }).catch(() => {});
  }
  setupChromeHandlers();

  const btn = document.getElementById("retranscribe");
  if (btn) {
    btn.addEventListener("click", () =>
      dispatchVerb("crunchr", "Re-transcribe", [btn.dataset.rec], btn),
    );
  }

  // Click any line/jump → open the recording player at that timestamp.
  // openRecordingPlayer reads a `seekTo` argument and the player binds
  // it in the next iteration when we extend the player.
  const seek = (sec) => {
    if (!d.recording_id) return;
    openRecordingPlayer(d.recording_id, { seekTo: sec });
  };
  document.querySelectorAll(".cr-line, .cr-block-jump").forEach((el) => {
    el.addEventListener("click", (e) => {
      e.preventDefault();
      seek(parseFloat(el.dataset.seek || "0"));
    });
  });

  // Speaker filter — toggling a chip hides blocks not matching the
  // active speaker set. "all" is the reset.
  document.getElementById("cr-chips")?.addEventListener("click", (e) => {
    const btn = e.target.closest(".cr-chip");
    if (!btn) return;
    const chips = [...document.querySelectorAll(".cr-chip")];
    if (btn.dataset.spk === "") {
      chips.forEach((c) => c.classList.add("is-active"));
    } else {
      btn.classList.toggle("is-active");
      const allChip = chips.find((c) => c.dataset.spk === "");
      if (allChip) allChip.classList.toggle("is-active", false);
    }
    const active = new Set(
      chips
        .filter((c) => c.classList.contains("is-active") && c.dataset.spk)
        .map((c) => c.dataset.spk),
    );
    const showAll = chips.find((c) => c.dataset.spk === "" && c.classList.contains("is-active"));
    document.querySelectorAll(".cr-block").forEach((blk) => {
      const visible = showAll || active.size === 0 || active.has(blk.dataset.spk);
      blk.classList.toggle("cr-block-hidden", !visible);
    });
  });

  // .vtt export — bake segments into a WebVTT file and trigger download.
  document.getElementById("cr-export-vtt")?.addEventListener("click", () => {
    const lines = ["WEBVTT", ""];
    const fmtVtt = (sec) => {
      const ms = Math.max(0, Math.round((sec ?? 0) * 1000));
      const h = String(Math.floor(ms / 3_600_000)).padStart(2, "0");
      const m = String(Math.floor((ms / 60_000) % 60)).padStart(2, "0");
      const s = String(Math.floor((ms / 1000) % 60)).padStart(2, "0");
      const f = String(ms % 1000).padStart(3, "0");
      return `${h}:${m}:${s}.${f}`;
    };
    segments.forEach((s, i) => {
      const start = fmtVtt(s.start_sec);
      const end = fmtVtt(s.end_sec ?? (s.start_sec ?? 0) + 5);
      lines.push(String(i + 1), `${start} --> ${end}`);
      lines.push(s.speaker ? `<v ${s.speaker}>${s.text}` : s.text, "");
    });
    const blob = new Blob([lines.join("\n")], { type: "text/vtt" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${(d.title || "transcript").replace(/[\\/:*?"<>|]/g, "_")}.vtt`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    Toast.success("WebVTT exported");
  });

  // Caption language selector — rewrites the .srt/.vtt/.txt URLs so a
  // change reflects in all three download links. Identity-only today
  // (Pro plugin backend ships in a follow-up).
  document.getElementById("cr-caption-lang")?.addEventListener("change", (e) => {
    const lang = e.target.value;
    document
      .querySelectorAll(".cr-caption-export a[download]")
      .forEach((a) => {
        const fmt = a.textContent.replace(".", "");
        a.href = API.captionsExportUrl(d.recording_id, fmt, lang);
      });
  });

  // Brandsafe scan — runs the scanners, renders verdicts.
  document.getElementById("cr-brandsafe")?.addEventListener("click", async (e) => {
    const btn = e.currentTarget;
    await withBusy(btn, "Scanning…", async () => {
      const resp = await API.brandsafeScan(d.recording_id);
      const verdicts = resp.verdicts || [];
      const card = document.getElementById("cr-brandsafe-card");
      const list = document.getElementById("cr-brandsafe-list");
      const count = document.getElementById("cr-brandsafe-count");
      if (!card || !list || !count) return;
      card.hidden = false;
      count.innerHTML = verdicts.length
        ? `<span class="pg-cap-hint">${verdicts.length} verdict${verdicts.length === 1 ? "" : "s"} · category "${escape(resp.category)}"</span>`
        : '<span class="cfg-badge ok">all clear</span>';
      if (!verdicts.length) {
        list.innerHTML = '<div class="empty sm">No content-safety risks detected. Scan covers slurs, profanity, restricted game categories, and music mentions.</div>';
        return;
      }
      const sevColour = {
        critical: "hsl(0, 80%, 60%)",
        high: "hsl(20, 80%, 60%)",
        medium: "hsl(40, 80%, 60%)",
        low: "hsl(200, 60%, 60%)",
      };
      list.innerHTML = verdicts
        .map(
          (v) => `
        <div class="cr-bs-row sev-${escape(v.severity)}" style="--bs-c:${sevColour[v.severity] || sevColour.low}">
          <span class="cr-bs-sev">${escape(v.severity)}</span>
          <div class="cr-bs-body">
            <div class="cr-bs-head">
              <span class="cr-bs-kind">${escape(v.kind.replace(/_/g, " "))}</span>
              ${v.platform ? `<span class="mon-plat plat-${escape(v.platform.toLowerCase())}">${escape(v.platform)}</span>` : ""}
              ${typeof v.at_sec === "number" ? `<button class="cr-bs-jump" data-seek="${v.at_sec}">${fmtClock(v.at_sec)}</button>` : ""}
            </div>
            <div class="cr-bs-snippet">${escape(v.snippet)}</div>
            <div class="cr-bs-fix">${escape(v.fix_hint)}</div>
          </div>
        </div>`,
        )
        .join("");
      list.querySelectorAll(".cr-bs-jump").forEach((el) => {
        el.addEventListener("click", () => seek(parseFloat(el.dataset.seek || "0")));
      });
      Toast.success(`Scan complete: ${verdicts.length} verdict(s)`);
    }).catch((err) => Toast.error(`Brand-safety scan failed: ${err.message}`));
  });

  // Chapters — POST to /api/v1/plugins/chapters/<id>, render the result.
  document.getElementById("cr-chapters")?.addEventListener("click", async (e) => {
    const btn = e.currentTarget;
    await withBusy(btn, "Generating…", async () => {
      const resp = await API.chaptersGenerate(d.recording_id);
      const card = document.getElementById("cr-chapters-card");
      const list = document.getElementById("cr-chapters-list");
      const pre = document.getElementById("cr-chapters-pre");
      if (!card || !list || !pre) return;
      list.innerHTML = (resp.chapters || [])
        .map(
          (c) =>
            `<a class="cr-chapter" href="#" data-seek="${c.start_sec}"><span class="cr-chapter-time">${fmtClock(c.start_sec)}</span><span class="cr-chapter-title">${escape(c.title)}</span></a>`,
        )
        .join("") || '<div class="empty sm">No chapter boundaries detected.</div>';
      pre.textContent = resp.description || "";
      card.hidden = false;
      list.querySelectorAll(".cr-chapter").forEach((el) => {
        el.addEventListener("click", (e) => {
          e.preventDefault();
          seek(parseFloat(el.dataset.seek || "0"));
        });
      });
      document.getElementById("cr-chapters-copy")?.addEventListener("click", async () => {
        try {
          await navigator.clipboard.writeText(resp.description || "");
          Toast.success("Description copied");
        } catch (_) {
          Toast.error("Couldn't copy");
        }
      });
      Toast.success(`Generated ${(resp.chapters || []).length} chapter(s)`);
    }).catch((err) => Toast.error(`Chapters failed: ${err.message}`));
  });

  // Markdown export — copy to clipboard, ready to paste into a notes
  // app / show notes draft / Casebook plugin (iter 12).
  document.getElementById("cr-export-md")?.addEventListener("click", async () => {
    const md = blocks
      .map((b) => {
        const head = `**[${fmtClock(b.lines[0].start_sec)}] ${b.speaker || "—"}**`;
        const body = b.lines.map((l) => l.text).join(" ");
        return `${head}\n\n${body}`;
      })
      .join("\n\n");
    try {
      await navigator.clipboard.writeText(md);
      Toast.success("Markdown copied to clipboard");
    } catch (e) {
      Toast.error("Couldn't copy to clipboard");
    }
  });
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
    <div class="pg-list">${rows || `<div class="empty">No channels archived yet.</div>
      <div class="pg-getstarted"><strong>Get started:</strong> add a channel and enable Archiver tandem from its row — Archiver back-fills the channel's existing VODs in priority order.</div>`}</div>
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
          <span class="pg-row-title">${escape(niceTitle(v.title))}</span>
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
    <div id="vg-trend-summary"></div>
    <div class="cfg-grid">${cards || `<div class="empty">No verdicts yet — viewers are sampled while channels are live.</div>
      <div class="pg-getstarted"><strong>Get started:</strong> Viewguard runs automatically during live Twitch captures. Verdicts appear here after a stream ends and samples are scored.</div>`}</div>
  `);
  setupChromeHandlers();
  // Lazy-load the trend dashboard so the per-channel cards paint
  // first. Pure render; the summary inserts above the grid when ready.
  API.viewguardTrend().then(renderViewguardTrend).catch(() => {});
}

// Render the cross-stream trend dashboard above the per-channel grid.
// Shows banded watchlists (Critical / Warning / Watch) — Clear is
// hidden by default since there's nothing actionable there.
function renderViewguardTrend(resp) {
  const host = document.getElementById("vg-trend-summary");
  if (!host || !resp || !resp.watchlist) return;
  const wl = resp.watchlist;
  const bandSpec = [
    ["critical", "Critical", "hsl(0, 80%, 60%)"],
    ["warning", "Warning", "hsl(20, 80%, 60%)"],
    ["watch", "Watch", "hsl(40, 80%, 60%)"],
  ];
  const actionLabel = {
    no_action: "No action",
    keep_monitoring: "Keep monitoring",
    manual_review: "Manual review",
    escalate_and_report: "Escalate + report",
  };
  const directionGlyph = {
    improving: "↓",
    stable: "→",
    worsening: "↑",
  };
  const bands = bandSpec
    .map(([key, label, colour]) => {
      const list = wl[key] || [];
      if (!list.length) return "";
      const rows = list
        .map(
          (t) => `
        <div class="vg-trend-row" style="--vg-c:${colour}">
          <span class="vg-trend-name">${escape(t.channel_name)}</span>
          <span class="vg-trend-score">${(t.latest_score * 100).toFixed(0)}%</span>
          <span class="vg-trend-dir" title="latest ${t.latest_score.toFixed(2)} vs rolling mean ${t.rolling_mean.toFixed(2)} (Δ ${t.delta >= 0 ? "+" : ""}${(t.delta * 100).toFixed(0)}pp)">
            ${escape(directionGlyph[t.direction] || "→")} ${escape(t.direction)}
          </span>
          ${t.anomaly ? '<span class="vg-trend-anomaly" title="latest deviates from rolling mean by >20pp">anomaly</span>' : ""}
          <span class="vg-trend-samples">${t.samples} sample${t.samples === 1 ? "" : "s"}</span>
          <span class="vg-trend-action">${escape(actionLabel[t.suggested_action] || t.suggested_action)}</span>
        </div>`,
        )
        .join("");
      return `<details class="vg-trend-band" open data-band="${escape(key)}" style="--vg-c:${colour}">
        <summary><strong>${escape(label)}</strong> <span class="pg-cap-hint">${list.length} channel${list.length === 1 ? "" : "s"}</span></summary>
        <div class="vg-trend-list">${rows}</div>
      </details>`;
    })
    .filter(Boolean)
    .join("");
  const clearCount = (wl.clear || []).length;
  host.innerHTML = `
    <section class="cfg-card vg-trend-card">
      <h2 class="cfg-title">Cross-stream trend <span class="pg-cap-hint">${resp.samples} verdict sample${resp.samples === 1 ? "" : "s"} · ${clearCount} clear channel${clearCount === 1 ? "" : "s"} hidden</span></h2>
      ${bands || '<div class="empty sm">No actionable trends right now — every channel is in the Clear band.</div>'}
    </section>`;
}

// ── Insights ─────────────────────────────────────────────────────────
let insightsState = { stopwords: false };
async function renderInsights() {
  const parts = routeParts();
  const recId = parts[2] === "rec" ? parts[3] : null;
  const [wordsResp, topicsResp, crunchrResp] = await Promise.all([
    API.insightsWords({ stopwords: insightsState.stopwords, limit: 40 }),
    API.insightsTopics(),
    // Pull the list of transcribed recordings so the comparison
    // picker has options. Failure is fine — the picker just stays
    // empty.
    API.crunchrRecordings().catch(() => ({ recordings: [] })),
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

  // Comparison picker: pick any two transcribed recordings.
  const allRecs = (crunchrResp && crunchrResp.recordings) || [];
  const recOptions = allRecs
    .map(
      (r) =>
        `<option value="${escape(r.recording_id)}">${escape(niceTitle(r.title) || r.recording_id)} · ${escape(r.channel_name)}</option>`,
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
      <section class="cfg-card" id="ins-compare-card">
        <h2 class="cfg-title">Compare two streams <span class="pg-cap-hint">word overlap · Jaccard · what's new vs gone</span></h2>
        <form id="ins-compare-form" class="mon-add">
          <select id="ins-compare-a">${recOptions}</select>
          <select id="ins-compare-b">${recOptions}</select>
          <button class="btn-primary" type="submit">Compare</button>
        </form>
        <div id="ins-compare-result"></div>
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

  // Comparison submit — POSTs nothing (idempotent GET).
  document.getElementById("ins-compare-form")?.addEventListener("submit", async (e) => {
    e.preventDefault();
    const a = document.getElementById("ins-compare-a").value;
    const b = document.getElementById("ins-compare-b").value;
    if (!a || !b || a === b) {
      Toast.error("Pick two different recordings");
      return;
    }
    const host = document.getElementById("ins-compare-result");
    host.innerHTML = '<div class="empty sm">Comparing…</div>';
    try {
      const r = await API.insightsCompare(a, b);
      const c = r.comparison;
      const sharedRows = (c.shared || [])
        .slice(0, 30)
        .map(
          (s) =>
            `<tr><td>${escape(s.word)}</td><td>${s.count_a}</td><td>${s.count_b}</td><td>${isFinite(s.a_over_b) ? s.a_over_b.toFixed(2) : "∞"}</td></tr>`,
        )
        .join("");
      const onlyA = (c.only_a || []).slice(0, 20).map((w) => `<li>${escape(w.word)} <em>${w.count}</em></li>`).join("");
      const onlyB = (c.only_b || []).slice(0, 20).map((w) => `<li>${escape(w.word)} <em>${w.count}</em></li>`).join("");
      host.innerHTML = `
        <div class="ins-cmp-summary">
          <span class="cfg-badge">Jaccard ${(c.jaccard * 100).toFixed(0)}%</span>
          <span class="pg-cap-hint">${c.shared.length} shared · ${c.only_a.length} only-A · ${c.only_b.length} only-B</span>
        </div>
        <div class="ins-cmp-grid">
          <div class="ins-cmp-table-wrap">
            <h3 class="ins-cmp-h">Shared words</h3>
            <table class="ins-cmp-table">
              <thead><tr><th>word</th><th>A</th><th>B</th><th>A÷B</th></tr></thead>
              <tbody>${sharedRows || '<tr><td colspan="4" class="empty sm">No shared words</td></tr>'}</tbody>
            </table>
          </div>
          <div>
            <h3 class="ins-cmp-h">Only in A</h3>
            <ul class="ins-cmp-list">${onlyA || '<li class="empty sm">None</li>'}</ul>
          </div>
          <div>
            <h3 class="ins-cmp-h">Only in B</h3>
            <ul class="ins-cmp-list">${onlyB || '<li class="empty sm">None</li>'}</ul>
          </div>
        </div>`;
    } catch (err) {
      host.innerHTML = `<div class="empty sm">Compare failed: ${escape(err.message)}</div>`;
    }
  });
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

// Parse a clock-shaped input ("1:30:00", "1:30", "90", "90.5s") into
// seconds. Falls back to the raw numeric value when the format is
// loose. Used by the EDL editor prompts.
function parseTimeInput(raw, max) {
  const s = String(raw || "").trim().replace(/s$/i, "");
  if (!s) return NaN;
  if (s.includes(":")) {
    const parts = s.split(":").map((x) => parseFloat(x));
    if (parts.some((p) => !isFinite(p))) return NaN;
    let n = 0;
    for (const p of parts) n = n * 60 + p;
    return Math.min(Math.max(n, 0), max ?? n);
  }
  const n = parseFloat(s);
  if (!isFinite(n)) return NaN;
  return Math.min(Math.max(n, 0), max ?? n);
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
      <h2>${escape(niceTitle(rec.stream_title) || "(no title)")}</h2>
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
        ${rec.output_path ? meta("File", `<span class="rec-info-pathwrap"><code class="rec-info-path">${escape(rec.output_path)}</code><button class="rec-copy" data-copy="${escape(rec.output_path)}" title="Copy path">⧉</button></span>`) : ""}
        ${rec.error ? meta("Error", `<span class="cfg-badge err">${escape(rec.error)}</span>`) : ""}
      </dl>
    </div>
    ${probeSectionHtml(probe)}
    <section class="rec-info-actions">
      <h3>Plugin actions</h3>
      <div class="rec-info-verbs">${actionsHtml}</div>
      ${isFinished ? `<button class="sm rec-info-cuepoints-btn" data-action="rec-info-cuepoints" title="Scene-change cuepoints (ffmpeg full pass)">⌶ Detect scene changes</button>` : ""}
      ${isFinished ? `<button class="sm rec-info-clipper-btn" data-action="rec-info-clipper" title="Mine highlight candidates (uses cuepoints; runs ffmpeg pass if needed)">★ Find highlights</button>` : ""}
      ${isFinished ? `<button class="sm rec-info-thumbs-btn" data-action="rec-info-thumbs" title="Sample candidate thumbnail frames at cuepoints / highlights">▥ Pick thumbnail</button>` : ""}
      ${isFinished ? `<button class="sm rec-info-tracks-btn" data-action="rec-info-tracks" title="List audio tracks (OBS multi-track captures) + extract individual stems">♪ Audio tracks</button>` : ""}
      ${isFinished ? `<button class="sm rec-info-reuse-btn" data-action="rec-info-reuse" title="Build cross-format publish drafts (YT long / Shorts / TikTok / Patreon / podcast / blog)">⇪ Publish drafts</button>` : ""}
      ${isFinished ? `<button class="sm rec-info-casebook-btn" data-action="rec-info-casebook" title="Post-stream Casebook report (markdown briefing)">📓 Casebook</button>` : ""}
      ${isFinished ? `<button class="sm rec-info-editor-btn" data-action="rec-info-editor" title="Open the EDL editor — cut, ripple-delete, render">✄ EDL editor</button>` : ""}
      <div class="rec-cuepoints" id="rec-cuepoints" hidden></div>
      <div class="rec-clipper" id="rec-clipper" hidden></div>
      <div class="rec-thumbs" id="rec-thumbs" hidden></div>
      <div class="rec-tracks" id="rec-tracks" hidden></div>
      <div class="rec-reuse" id="rec-reuse" hidden></div>
      <div class="rec-casebook" id="rec-casebook" hidden></div>
      <div class="rec-editor" id="rec-editor" hidden></div>
    </section>
    <footer class="rec-info-foot">
      ${isFinished ? `<button class="primary" data-action="rec-info-play">▶ Open in player</button>` : ""}
      ${isFinished ? `<button class="sm" data-action="rec-info-remux" title="Remux to matroska + aac_adtstoasc so the in-browser player can decode it. Keeps the original as .orig.">⟳ Remux for browser</button>` : ""}
      <button class="danger" data-action="rec-info-delete">✕ Delete</button>
    </footer>`;

  overlay.querySelectorAll("[data-action=modal-close]").forEach((b) =>
    b.addEventListener("click", closeRecordingModals));
  overlay.querySelector("[data-action=rec-info-play]")?.addEventListener("click", () => {
    closeRecordingModals();
    openRecordingPlayer(jobId);
  });
  overlay.querySelector("[data-action=rec-info-cuepoints]")?.addEventListener("click", async (e) => {
    const btn = e.currentTarget;
    await withBusy(btn, "Detecting…", async () => {
      const resp = await API.cuepointsGenerate(jobId);
      const host = document.getElementById("rec-cuepoints");
      if (!host) return;
      // Compute duration as a max-time + 5% pad so the timeline isn't
      // clipped if the last cuepoint is near the end of the file.
      const points = resp.points || [];
      const maxTime = points.length ? Math.max(...points.map((p) => p.time_sec)) : 0;
      const duration = Math.max(maxTime * 1.05, 60);
      if (!points.length) {
        host.innerHTML = `<div class="empty sm">No scene changes detected at threshold ${resp.threshold}.</div>`;
        host.hidden = false;
        return;
      }
      host.innerHTML = `
        <h4 class="rec-cp-title">${points.length} scene change${points.length === 1 ? "" : "s"} <span class="pg-cap-hint">${resp.cached ? "(cached)" : "(fresh extraction)"}</span></h4>
        <div class="rec-cp-strip" style="--rec-cp-dur:${duration}">
          ${points
            .map(
              (p) =>
                `<a class="rec-cp-tick" style="--rec-cp-pct:${((p.time_sec / duration) * 100).toFixed(2)}%" data-seek="${p.time_sec}" title="${fmtClock(p.time_sec)}" href="#"></a>`,
            )
            .join("")}
        </div>
        <div class="rec-cp-axis">
          <span>0:00</span>
          <span>${fmtClock(duration)}</span>
        </div>`;
      host.hidden = false;
      host.querySelectorAll(".rec-cp-tick").forEach((el) => {
        el.addEventListener("click", (e) => {
          e.preventDefault();
          closeRecordingModals();
          openRecordingPlayer(jobId, { seekTo: parseFloat(el.dataset.seek || "0") });
        });
      });
      Toast.success(`Detected ${points.length} scene change(s)`);
    }).catch((err) => Toast.error(`Cuepoints failed: ${err.message}`));
  });
  overlay.querySelector("[data-action=rec-info-clipper]")?.addEventListener("click", async (e) => {
    const btn = e.currentTarget;
    await withBusy(btn, "Mining…", async () => {
      const [analysis, existing] = await Promise.all([
        API.clipperAnalyze(jobId),
        API.clipperListClips(jobId).catch(() => ({ clips: [] })),
      ]);
      const host = document.getElementById("rec-clipper");
      if (!host) return;
      host.hidden = false;
      const highlights = analysis.highlights || [];
      const cutByStart = new Map(
        (existing.clips || []).map((c) => [Math.round(c.start_sec), c]),
      );
      if (!highlights.length) {
        host.innerHTML = `<div class="empty sm">No highlight candidates found (transcript or cuepoints empty?).</div>`;
        return;
      }
      host.innerHTML = `
        <h4 class="rec-cp-title">${highlights.length} highlight candidate${highlights.length === 1 ? "" : "s"} <span class="pg-cap-hint">window ${analysis.window_secs}s</span></h4>
        <div class="rec-hl-list">
          ${highlights
            .map((h, i) => {
              const cut = cutByStart.get(Math.round(h.time_sec));
              return `<div class="rec-hl-row" data-i="${i}">
                <button class="rec-hl-jump" data-seek="${h.time_sec}" title="Jump to ${fmtClock(h.time_sec)}">${fmtClock(h.time_sec)}</button>
                <span class="rec-hl-score" title="Score ${h.score.toFixed(2)} · density ${h.density}">
                  <span class="rec-hl-bar" style="--rec-hl-pct:${(h.score * 100).toFixed(0)}%"></span>
                  <span>${Math.round(h.score * 100)}%</span>
                </span>
                <span class="rec-hl-meta">${h.density} cuepoint${h.density === 1 ? "" : "s"} · ${h.suggested_duration}s</span>
                ${cut
                  ? `<span class="cfg-badge ok" title="${escape(cut.clip_path)}">✓ cut · ${formatBytes(cut.bytes)}</span>`
                  : `<button class="sm rec-hl-cut" data-start="${h.time_sec}" data-dur="${h.suggested_duration}">Cut clip</button>`}
              </div>`;
            })
            .join("")}
        </div>`;
      host.querySelectorAll(".rec-hl-jump").forEach((el) => {
        el.addEventListener("click", (e) => {
          e.preventDefault();
          closeRecordingModals();
          openRecordingPlayer(jobId, { seekTo: parseFloat(el.dataset.seek || "0") });
        });
      });
      host.querySelectorAll(".rec-hl-cut").forEach((btn) => {
        btn.addEventListener("click", async (e) => {
          const cb = e.currentTarget;
          await withBusy(cb, "Cutting…", async () => {
            const res = await API.clipperExtract(jobId, {
              start_sec: parseFloat(cb.dataset.start),
              duration_sec: parseFloat(cb.dataset.dur),
              stem: `${niceTitle(rec.stream_title).replace(/[^a-zA-Z0-9_-]+/g, "_").slice(0, 60)}_${Math.round(parseFloat(cb.dataset.start))}`,
            });
            Toast.success(`Cut ${formatBytes(res.bytes)} → ${res.clip_path}`);
            cb.outerHTML = `<span class="cfg-badge ok" title="${escape(res.clip_path)}">✓ cut · ${formatBytes(res.bytes)}</span>`;
          }).catch((err) => Toast.error(`Cut failed: ${err.message}`));
        });
      });
      Toast.success(`Found ${highlights.length} highlight candidate(s)`);
    }).catch((err) => Toast.error(`Highlights failed: ${err.message}`));
  });

  overlay.querySelector("[data-action=rec-info-thumbs]")?.addEventListener("click", async (e) => {
    const btn = e.currentTarget;
    await withBusy(btn, "Sampling…", async () => {
      const resp = await API.thumbnailsGenerate(jobId, {
        source: "cuepoints",
        facecam: "top_right",
      });
      const host = document.getElementById("rec-thumbs");
      if (!host) return;
      host.hidden = false;
      const candidates = resp.candidates || [];
      if (!candidates.length) {
        host.innerHTML = '<div class="empty sm">No thumbnail candidates generated.</div>';
        return;
      }
      host.innerHTML = `
        <h4 class="rec-cp-title">${candidates.length} thumbnail candidate${candidates.length === 1 ? "" : "s"} <span class="pg-cap-hint">ranked by saliency</span></h4>
        <div class="rec-thumbs-grid">
          ${candidates
            .map(
              (c, i) =>
                `<figure class="rec-thumb-card" data-i="${i}">
                  <a class="rec-thumb-img" href="${escape(API.thumbnailFileUrl(c.path))}" target="_blank" rel="noopener">
                    <img loading="lazy" alt="" src="${escape(API.thumbnailFileUrl(c.path))}" />
                    <span class="rec-thumb-time">${fmtClock(c.time_sec)}</span>
                  </a>
                  <figcaption>
                    <span class="rec-thumb-score" title="Score ${c.score.toFixed(2)} · ${formatBytes(c.bytes)}">
                      <span class="rec-hl-bar" style="--rec-hl-pct:${(c.score * 100).toFixed(0)}%"></span>
                      <span>${Math.round(c.score * 100)}%</span>
                    </span>
                    ${c.crop_path ? `<a class="pg-linkbtn" href="${escape(API.thumbnailFileUrl(c.crop_path))}" target="_blank" rel="noopener" title="9:16 facecam crop">9:16 crop</a>` : ""}
                  </figcaption>
                </figure>`,
            )
            .join("")}
        </div>`;
      Toast.success(`Generated ${candidates.length} thumbnail candidate(s)`);
    }).catch((err) => Toast.error(`Thumbnails failed: ${err.message}`));
  });

  overlay.querySelector("[data-action=rec-info-editor]")?.addEventListener("click", async (e) => {
    const btn = e.currentTarget;
    await withBusy(btn, "Loading EDL…", async () => {
      const host = document.getElementById("rec-editor");
      if (!host) return;
      let { edl, total_duration } = await API.editorLoad(jobId);
      host.hidden = false;

      const paint = () => {
        const dur = edl.cuts.reduce((a, c) => a + Math.max(0, c.end_sec - c.start_sec), 0);
        const sourceDur = total_duration || dur || 1;
        host.innerHTML = `
          <h4 class="rec-cp-title">EDL editor <span class="pg-cap-hint">${edl.cuts.length} cut${edl.cuts.length === 1 ? "" : "s"} · output ${fmtClock(dur)}</span></h4>
          <div class="rec-ed-actions">
            <button class="sm rec-ed-add-split" type="button">Split at time…</button>
            <button class="sm rec-ed-delete" type="button">Ripple-delete range…</button>
            <button class="sm rec-ed-deadair" type="button" title="Detect dead air (silencedetect) and trim spans longer than 6s">▢ Trim dead air…</button>
            <button class="sm rec-ed-vad" type="button" title="DAW-style voice gate — hysteresis VAD finds speech runs and ripple-deletes the natural breath gaps">▢ Voice gate…</button>
            <button class="sm rec-ed-sidechain" type="button" title="DAW sidechain — VAD voice intervals → ducking automation curve baked at render. Composes VAD + sidechain + automation in one click.">🦆 Sidechain duck…</button>
            <button class="sm rec-ed-branding" type="button" title="Watermark + intro/outro banner overlay applied at render">★ Branding…</button>
            <button class="sm rec-ed-loudness" type="button" title="EBU R128 loudness check + per-platform normalisation target">♪ Loudness…</button>
            <button class="sm rec-ed-history" type="button" title="Revision history — revert across saves (DAW-style undo)">↺ History…</button>
            <button class="sm rec-ed-scenes" type="button" title="Scenes — Ableton-style session save/recall bundling EDL + branding + automation + loudness + captions style">🎬 Scenes…</button>
            <button class="btn-primary rec-ed-render" type="button">⚡ Render to MKV</button>
          </div>
          <div class="rec-branding" hidden></div>
          <div class="rec-loudness" hidden></div>
          <div class="rec-history" hidden></div>
          <div class="rec-scenes" hidden></div>
          <div class="rec-ed-list">
            ${edl.cuts
              .map(
                (c, i) => `
              <div class="rec-ed-row" data-i="${i}">
                <span class="rec-ed-idx">${i + 1}</span>
                <span class="rec-ed-kind ${escape(c.kind.kind || "source")}">${escape(c.kind.kind || "source")}</span>
                <span class="rec-ed-src">${escape((c.kind.source_path || c.kind.broll_path || "").split("/").slice(-1)[0])}</span>
                <span class="rec-ed-time">${fmtClock(c.start_sec)} → ${fmtClock(c.end_sec)} · ${fmtClock(c.end_sec - c.start_sec)}</span>
                <button class="sm rec-ed-trim" data-i="${i}" type="button" title="Trim this cut">trim</button>
                <button class="sm danger rec-ed-rm" data-i="${i}" type="button" title="Remove this cut">✕</button>
              </div>`,
              )
              .join("")}
          </div>
          <p class="pg-cap-hint">All edits are non-destructive — original recording stays intact. Render writes &lt;recording_parent&gt;/edl/&lt;id&gt;.mkv.</p>`;

        const persist = async (label) => {
          try {
            await API.editorSave(jobId, edl, label);
          } catch (err) {
            Toast.error(`Save failed: ${err.message}`);
          }
        };
        host.querySelector(".rec-ed-add-split")?.addEventListener("click", async () => {
          const s = prompt("Split at output time (HH:MM:SS or seconds):");
          if (!s) return;
          const t = parseTimeInput(s, sourceDur);
          if (!isFinite(t)) {
            Toast.error("Could not parse time");
            return;
          }
          // Local split — same algorithm as server. Walk and split.
          let elapsed = 0;
          for (let i = 0; i < edl.cuts.length; i++) {
            const c = edl.cuts[i];
            const cd = c.end_sec - c.start_sec;
            const out_hi = elapsed + cd;
            if (t > elapsed + 0.001 && t < out_hi - 0.001) {
              const offset = t - elapsed;
              const right = JSON.parse(JSON.stringify(c));
              const newSplit = c.start_sec + offset;
              right.start_sec = newSplit;
              c.end_sec = newSplit;
              edl.cuts.splice(i + 1, 0, right);
              break;
            }
            elapsed = out_hi;
          }
          await persist("split");
          paint();
          Toast.success("Split");
        });
        host.querySelector(".rec-ed-delete")?.addEventListener("click", async () => {
          const range = prompt("Range to delete (e.g. 1:30-2:45 or 90-165):");
          if (!range) return;
          const m = range.match(/(.+?)\s*[-–]\s*(.+)/);
          if (!m) { Toast.error("Use lo-hi format"); return; }
          const lo = parseTimeInput(m[1], sourceDur);
          const hi = parseTimeInput(m[2], sourceDur);
          if (!isFinite(lo) || !isFinite(hi) || hi <= lo) {
            Toast.error("Invalid range");
            return;
          }
          // Mirror server-side delete_range — walk and trim.
          let elapsed = 0;
          const next = [];
          for (const cut of edl.cuts) {
            const cd = cut.end_sec - cut.start_sec;
            const out_lo = elapsed;
            const out_hi = elapsed + cd;
            elapsed = out_hi;
            if (out_hi <= lo || out_lo >= hi) { next.push(cut); continue; }
            if (out_lo >= lo && out_hi <= hi) { continue; }
            if (out_lo < lo && out_hi <= hi) {
              const trim = JSON.parse(JSON.stringify(cut));
              trim.end_sec = cut.start_sec + (lo - out_lo);
              next.push(trim);
              continue;
            }
            if (out_lo >= lo && out_hi > hi) {
              const trim = JSON.parse(JSON.stringify(cut));
              trim.start_sec = cut.start_sec + (hi - out_lo);
              next.push(trim);
              continue;
            }
            const left = JSON.parse(JSON.stringify(cut));
            left.end_sec = cut.start_sec + (lo - out_lo);
            const right = JSON.parse(JSON.stringify(cut));
            right.start_sec = cut.start_sec + (hi - out_lo);
            next.push(left, right);
          }
          edl.cuts = next.filter((c) => c.end_sec - c.start_sec > 0.001);
          await persist("ripple-delete");
          paint();
          Toast.success("Range deleted");
        });
        host.querySelectorAll(".rec-ed-rm").forEach((b) => {
          b.addEventListener("click", async () => {
            const i = +b.dataset.i;
            edl.cuts.splice(i, 1);
            await persist("remove cut");
            paint();
          });
        });
        host.querySelectorAll(".rec-ed-trim").forEach((b) => {
          b.addEventListener("click", async () => {
            const i = +b.dataset.i;
            const c = edl.cuts[i];
            const s = prompt(`Trim cut ${i + 1} — start..end (seconds or HH:MM:SS), e.g. "10-60"`, `${c.start_sec}-${c.end_sec}`);
            if (!s) return;
            const m = s.match(/(.+?)\s*[-–]\s*(.+)/);
            if (!m) { Toast.error("Use start-end format"); return; }
            const lo = parseTimeInput(m[1], sourceDur);
            const hi = parseTimeInput(m[2], sourceDur);
            if (!isFinite(lo) || !isFinite(hi) || hi <= lo) { Toast.error("Invalid"); return; }
            c.start_sec = lo;
            c.end_sec = hi;
            await persist("trim cut");
            paint();
          });
        });
        host.querySelector(".rec-ed-deadair")?.addEventListener("click", async (e2) => {
          const dabtn = e2.currentTarget;
          await withBusy(dabtn, "Scanning silence…", async () => {
            const r = await API.deadairDetect(jobId);
            const cuts = (r.result && r.result.recommended_cuts) || [];
            const totalTrim = (r.result && r.result.total_trim_secs) || 0;
            if (!cuts.length) {
              Toast.success(`No dead-air spans above the trim threshold detected.`);
              return;
            }
            if (!confirm(`Found ${cuts.length} dead-air span(s) totalling ${fmtClock(totalTrim)}.\n\nApply all as ripple-deletes? Edits are non-destructive — only the EDL changes.`)) return;
            // Apply each cut in DESCENDING order so prior deletes
            // don't shift later coordinates. The cut times are in
            // source-file coordinates, but our EDL initially mirrors
            // the source 1:1, so for the first apply they're
            // equivalent. After the first cut everything shifts; we
            // re-fetch the EDL before each subsequent cut to stay
            // honest about output-time coords.
            const sorted = [...cuts].sort((a, b) => b.start_sec - a.start_sec);
            for (const cut of sorted) {
              const lo = cut.start_sec;
              const hi = cut.end_sec;
              let elapsed = 0;
              const next = [];
              for (const c of edl.cuts) {
                const cd = c.end_sec - c.start_sec;
                const out_lo = elapsed;
                const out_hi = elapsed + cd;
                elapsed = out_hi;
                if (out_hi <= lo || out_lo >= hi) { next.push(c); continue; }
                if (out_lo >= lo && out_hi <= hi) { continue; }
                if (out_lo < lo && out_hi <= hi) {
                  const trim = JSON.parse(JSON.stringify(c));
                  trim.end_sec = c.start_sec + (lo - out_lo);
                  next.push(trim);
                  continue;
                }
                if (out_lo >= lo && out_hi > hi) {
                  const trim = JSON.parse(JSON.stringify(c));
                  trim.start_sec = c.start_sec + (hi - out_lo);
                  next.push(trim);
                  continue;
                }
                const left = JSON.parse(JSON.stringify(c));
                left.end_sec = c.start_sec + (lo - out_lo);
                const right = JSON.parse(JSON.stringify(c));
                right.start_sec = c.start_sec + (hi - out_lo);
                next.push(left, right);
              }
              edl.cuts = next.filter((c) => c.end_sec - c.start_sec > 0.001);
            }
            await persist("trim dead air");
            paint();
            Toast.success(`Trimmed ${cuts.length} dead-air span(s) · saved ${fmtClock(totalTrim)}.`);
          }).catch((err) => Toast.error(`Dead-air scan failed: ${err.message}`));
        });
        host.querySelector(".rec-ed-vad")?.addEventListener("click", async (e2) => {
          const vbtn = e2.currentTarget;
          const promptMin = prompt(
            "Minimum pause to KEEP between speech (sec) — gaps below this become ripple-deletes.\n" +
              "Default 1.0; lower = tighter; try 0.3 for podcast pacing.",
            "1.0",
          );
          if (promptMin == null) return;
          const minKeep = parseFloat(promptMin);
          if (!isFinite(minKeep) || minKeep < 0) { Toast.error("Invalid min_keep value"); return; }
          await withBusy(vbtn, "Scanning voice…", async () => {
            // Cap the window at 1h so a 4h archive doesn't melt the
            // host; the editor only cares about the section the user is
            // currently working on.
            const r = await API.vadAnalyze(jobId, { min_keep_sec: minKeep, window_sec: 3600 });
            const gaps = r.recommended_gaps || [];
            const savings = r.total_savings_sec || 0;
            const intervalCount = (r.voice_intervals || []).length;
            if (!gaps.length) {
              Toast.success(`Found ${intervalCount} voice run(s); no gaps above the ${minKeep}s keep threshold to tighten.`);
              return;
            }
            if (!confirm(
              `Voice gate found ${intervalCount} voice run(s) and ${gaps.length} ripple-delete candidate(s) ` +
                `(${fmtClock(savings)} of natural silence to remove).\n\n` +
                `Apply all? Edits are non-destructive — only the EDL changes.`,
            )) return;
            // Same descending-order ripple-delete loop the dead-air
            // path uses — keeps coordinate drift honest.
            const sorted = [...gaps].sort((a, b) => b.start_sec - a.start_sec);
            for (const gap of sorted) {
              const lo = gap.start_sec;
              const hi = gap.end_sec;
              let elapsed = 0;
              const next = [];
              for (const c of edl.cuts) {
                const cd = c.end_sec - c.start_sec;
                const out_lo = elapsed;
                const out_hi = elapsed + cd;
                elapsed = out_hi;
                if (out_hi <= lo || out_lo >= hi) { next.push(c); continue; }
                if (out_lo >= lo && out_hi <= hi) { continue; }
                if (out_lo < lo && out_hi <= hi) {
                  const trim = JSON.parse(JSON.stringify(c));
                  trim.end_sec = c.start_sec + (lo - out_lo);
                  next.push(trim);
                  continue;
                }
                if (out_lo >= lo && out_hi > hi) {
                  const trim = JSON.parse(JSON.stringify(c));
                  trim.start_sec = c.start_sec + (hi - out_lo);
                  next.push(trim);
                  continue;
                }
                const left = JSON.parse(JSON.stringify(c));
                left.end_sec = c.start_sec + (lo - out_lo);
                const right = JSON.parse(JSON.stringify(c));
                right.start_sec = c.start_sec + (hi - out_lo);
                next.push(left, right);
              }
              edl.cuts = next.filter((c) => c.end_sec - c.start_sec > 0.001);
            }
            await persist("voice gate");
            paint();
            Toast.success(`Voice gate · trimmed ${gaps.length} gap(s) · saved ${fmtClock(savings)}.`);
          }).catch((err) => Toast.error(`Voice-gate scan failed: ${err.message}`));
        });
        host.querySelector(".rec-ed-sidechain")?.addEventListener("click", async (e2) => {
          // One-click sidechain compressor: VAD → sidechain → automation.
          // Demonstrates the iter-46 + iter-50 + iter-41 plugin chain
          // composing in a single user gesture. The result is persisted
          // to the volume-automation store so the next ⚡ Render bakes
          // the ducking curve via the existing asendcmd pipeline.
          const sbtn = e2.currentTarget;
          const promptDuck = prompt(
            "Sidechain ducking — how many dB to drop the audio bus while voice is active?\n" +
              "Default -12 dB (podcast-natural). Try -6 dB for a gentler duck or -20 dB for voice-over.",
            "-12",
          );
          if (promptDuck == null) return;
          const duckDb = parseFloat(promptDuck);
          if (!isFinite(duckDb) || duckDb >= 0) { Toast.error("Duck depth must be < 0 dB"); return; }
          await withBusy(sbtn, "Building VAD…", async () => {
            const vad = await API.vadAnalyze(jobId, { window_sec: 3600 });
            const intervals = vad.voice_intervals || [];
            const envelopeDur = (vad.voice_intervals?.length
              ? Math.max(vad.envelope_frames * 0.05, intervals[intervals.length - 1].end_sec + 1)
              : (total_duration || dur || 0));
            if (!intervals.length) {
              Toast.success("VAD found no voice activity — nothing to duck. Try lowering open_db or raising window_sec.");
              return;
            }
            sbtn.textContent = "Sidechain…";
            const sc = await API.sidechainBuild(jobId, {
              voice_intervals: intervals,
              total_duration_sec: envelopeDur,
              knobs: { duck_db: duckDb, attack_sec: 0.05, release_sec: 0.3, hold_sec: 0.1, step_sec: 0.05 },
              persist: true,
            });
            if (!sc.persisted_to_automation_store) {
              Toast.error("Sidechain built but persistence failed — check daemon logs");
              return;
            }
            Toast.success(
              `Sidechain ready · ${intervals.length} voice run(s) → ${sc.point_count} automation point(s) ducking to ${duckDb} dB. Hit ⚡ Render to bake.`,
            );
          }).catch((err) => Toast.error(`Sidechain failed: ${err.message}`));
        });
        host.querySelector(".rec-ed-branding")?.addEventListener("click", async (e2) => {
          const bbtn = e2.currentTarget;
          await withBusy(bbtn, "Loading…", async () => {
            const r = await API.brandingLoad(jobId);
            const panel = host.querySelector(".rec-branding");
            if (!panel) return;
            const spec = r.spec || { watermark: null, banners: [] };
            const wm = spec.watermark || { source: { kind: "text", text: "", font_size: 32, color_rgba: "white" }, anchor: "bottom_right", inset_px: 24, opacity: 0.7 };
            const ANCHORS = [
              "top_left","top_center","top_right",
              "middle_left","middle_center","middle_right",
              "bottom_left","bottom_center","bottom_right",
            ];
            const anchorOpts = (sel) => ANCHORS.map((a) => `<option value="${a}"${a === sel ? " selected" : ""}>${a.replace(/_/g, " ")}</option>`).join("");
            const renderBanners = () => (spec.banners || []).map((b, i) => `
              <div class="rec-br-banner" data-i="${i}">
                <select class="rec-br-slot"><option value="intro"${b.slot==="intro"?" selected":""}>intro</option><option value="outro"${b.slot==="outro"?" selected":""}>outro</option></select>
                <input class="rec-br-text" type="text" value="${escape(b.text||"")}" placeholder="Banner text"/>
                <select class="rec-br-anchor">${anchorOpts(b.anchor)}</select>
                <input class="rec-br-dur" type="number" step="0.5" min="0.5" max="60" value="${b.duration_secs||3}" title="Visible duration (sec)"/>
                <button class="sm danger rec-br-rmb" type="button" title="Remove banner">✕</button>
              </div>`).join("");
            panel.hidden = false;
            panel.innerHTML = `
              <h5>Branding overlay</h5>
              <div class="rec-br-wm">
                <label class="rec-br-on"><input type="checkbox" class="rec-br-enabled" ${spec.watermark ? "checked" : ""}/> Watermark</label>
                <input class="rec-br-wtext" type="text" value="${escape(wm.source?.text||"@channel")}" placeholder="Watermark text"/>
                <select class="rec-br-wanchor">${anchorOpts(wm.anchor)}</select>
                <input class="rec-br-wop" type="number" step="0.05" min="0" max="1" value="${wm.opacity ?? 0.7}" title="Opacity (0–1)"/>
              </div>
              <div class="rec-br-banners">${renderBanners()}</div>
              <div class="rec-br-actions">
                <button class="sm rec-br-addb" type="button">+ Banner</button>
                <button class="btn-primary rec-br-save" type="button">Save</button>
                <span class="rec-br-preview pg-cap-hint"></span>
              </div>
              <pre class="rec-br-filter" title="filter_complex this spec produces">${escape(r.filter_complex||"")}</pre>
            `;
            const collect = () => {
              const enabled = panel.querySelector(".rec-br-enabled").checked;
              const newSpec = {
                watermark: enabled ? {
                  source: { kind: "text", text: panel.querySelector(".rec-br-wtext").value || "@channel", font_size: 32, color_rgba: "white" },
                  anchor: panel.querySelector(".rec-br-wanchor").value,
                  inset_px: 24,
                  opacity: parseFloat(panel.querySelector(".rec-br-wop").value) || 0.7,
                } : null,
                banners: Array.from(panel.querySelectorAll(".rec-br-banner")).map((row) => ({
                  slot: row.querySelector(".rec-br-slot").value,
                  text: row.querySelector(".rec-br-text").value || "",
                  font_size: 48,
                  color_rgba: "white",
                  anchor: row.querySelector(".rec-br-anchor").value,
                  inset_px: 40,
                  duration_secs: parseFloat(row.querySelector(".rec-br-dur").value) || 3,
                })),
              };
              return newSpec;
            };
            panel.querySelector(".rec-br-addb").addEventListener("click", () => {
              spec.banners = collect().banners;
              spec.banners.push({ slot: "intro", text: "Welcome", font_size: 48, color_rgba: "white", anchor: "top_center", inset_px: 40, duration_secs: 3.0 });
              panel.querySelector(".rec-br-banners").innerHTML = renderBanners();
            });
            panel.addEventListener("click", (ev) => {
              const t = ev.target;
              if (t && t.classList && t.classList.contains("rec-br-rmb")) {
                const idx = parseInt(t.closest(".rec-br-banner").dataset.i, 10);
                const next = collect();
                next.banners.splice(idx, 1);
                spec.banners = next.banners;
                spec.watermark = next.watermark;
                panel.querySelector(".rec-br-banners").innerHTML = renderBanners();
              }
            });
            panel.querySelector(".rec-br-save").addEventListener("click", async (ev) => {
              const sb = ev.currentTarget;
              const newSpec = collect();
              await withBusy(sb, "Saving…", async () => {
                const saved = await API.brandingSave(jobId, newSpec);
                panel.querySelector(".rec-br-filter").textContent = saved.filter_complex || "";
                Toast.success("Branding saved · applied at next render");
              }).catch((err) => Toast.error(`Save failed: ${err.message}`));
            });
          }).catch((err) => Toast.error(`Branding failed: ${err.message}`));
        });
        host.querySelector(".rec-ed-loudness")?.addEventListener("click", async (e2) => {
          const lbtn = e2.currentTarget;
          const panel = host.querySelector(".rec-loudness");
          if (!panel) return;
          panel.hidden = false;
          panel.innerHTML = `
            <h5>EBU R128 loudness</h5>
            <div class="rec-loud-bar">
              <label>
                <span>Target platform</span>
                <select class="rec-loud-platform">
                  <option value="youtube">YouTube · -14 LUFS</option>
                  <option value="spotify">Spotify · -14 LUFS / 7 LU</option>
                  <option value="apple_music">Apple Music · -16 LUFS</option>
                  <option value="ebu_r128">EBU R128 · -23 LUFS</option>
                  <option value="twitch">Twitch · -14 LUFS</option>
                </select>
              </label>
              <button class="btn-primary sm rec-loud-measure">▶ Measure now</button>
            </div>
            <div class="rec-loud-result"></div>
          `;
          panel.querySelector(".rec-loud-measure").addEventListener("click", async (ev) => {
            const mb = ev.currentTarget;
            const platform = panel.querySelector(".rec-loud-platform").value;
            const out = panel.querySelector(".rec-loud-result");
            out.innerHTML = `<div class="empty sm">Running ffmpeg pass-1 — this can take a minute on long captures…</div>`;
            await withBusy(mb, "Measuring…", async () => {
              try {
                const r = await API.loudnessMeasure(jobId, platform);
                const m = r.measurement;
                const d = r.delta;
                const dRow = (label, value, target, delta, unit) => `
                  <div class="rec-loud-row">
                    <span class="rec-loud-label">${escape(label)}</span>
                    <span class="rec-loud-meas">${value.toFixed(2)} ${unit}</span>
                    <span class="rec-loud-target">target ${target.toFixed(2)} ${unit}</span>
                    <span class="rec-loud-delta ${delta >= 0 ? "over" : "under"}">${delta >= 0 ? "+" : ""}${delta.toFixed(2)} ${unit}</span>
                  </div>`;
                out.innerHTML = `
                  <p class="pg-cap-hint">Pass-1 measurement complete. Toggle 'Apply normalisation' on the next render to bake the pass-2 filter into the EDL output.</p>
                  ${dRow("Integrated (I)",       m.input_i,    r.target.i,   d.i_delta,   "LUFS")}
                  ${dRow("True peak (TP)",      m.input_tp,   r.target.tp,  d.tp_delta,  "dBTP")}
                  ${dRow("Loudness range (LRA)", m.input_lra,  r.target.lra, d.lra_delta, "LU")}
                  <details class="rec-loud-filter">
                    <summary>Pass-2 ffmpeg filter</summary>
                    <pre>${escape(r.pass2_filter)}</pre>
                  </details>`;
                Toast.success(`Measured · I=${m.input_i.toFixed(2)} LUFS (Δ ${d.i_delta >= 0 ? "+" : ""}${d.i_delta.toFixed(2)})`);
              } catch (err) {
                out.innerHTML = `<div class="empty sm">⚠ ${escape(err.message)}</div>`;
              }
            });
          });
        });
        host.querySelector(".rec-ed-history")?.addEventListener("click", async (e2) => {
          const hbtn = e2.currentTarget;
          await withBusy(hbtn, "Loading…", async () => {
            const r = await API.editorRevisions(jobId);
            const panel = host.querySelector(".rec-history");
            if (!panel) return;
            const revs = r.revisions || [];
            panel.hidden = false;
            if (!revs.length) {
              panel.innerHTML = `<p class="pg-cap-hint">No revisions yet. Edits get logged here as you go.</p>`;
              return;
            }
            panel.innerHTML = `
              <h5>Revision history <span class="pg-cap-hint">${revs.length} saved</span></h5>
              <div class="rec-hist-list">
                ${revs.map((v, i) => `
                  <div class="rec-hist-row" data-rev="${v.revision_id}">
                    <span class="rec-hist-idx">v${revs.length - i}</span>
                    <span class="rec-hist-label">${escape(v.label)}</span>
                    <span class="rec-hist-meta pg-cap-hint">${v.cut_count} cut${v.cut_count===1?"":"s"} · ${fmtClock(v.total_duration_sec)} · ${escape(v.created_at.replace("T"," ").split(".")[0])}</span>
                    <button class="sm rec-hist-restore" type="button" title="Restore this revision as the current EDL">Restore</button>
                  </div>`).join("")}
              </div>
              <p class="pg-cap-hint">Restoring appends a new revision tagged "revert to vN" so restores are themselves undoable.</p>`;
            panel.querySelectorAll(".rec-hist-restore").forEach((rb) => {
              rb.addEventListener("click", async () => {
                const revId = rb.closest(".rec-hist-row").dataset.rev;
                if (!confirm(`Restore revision v${revId}? Current EDL will become the prior state; a new revision will be appended so this restore is itself undoable.`)) return;
                await withBusy(rb, "Restoring…", async () => {
                  const res = await API.editorRevisionRestore(jobId, revId);
                  edl = res.edl;
                  paint();
                  Toast.success(`Restored · ${res.label}`);
                  // refresh the panel
                  host.querySelector(".rec-ed-history")?.click();
                }).catch((err) => Toast.error(`Restore failed: ${err.message}`));
              });
            });
          }).catch((err) => Toast.error(`History failed: ${err.message}`));
        });
        host.querySelector(".rec-ed-scenes")?.addEventListener("click", async (e2) => {
          const sbtn = e2.currentTarget;
          await withBusy(sbtn, "Loading…", async () => {
            const r = await API.scenesList(jobId);
            const panel = host.querySelector(".rec-scenes");
            if (!panel) return;
            const scenes = r.scenes || [];
            panel.hidden = false;
            const sceneRows = scenes.map((s) => `
              <div class="rec-scene-row" data-scene-id="${escape(s.id)}">
                <div class="rec-scene-head">
                  <span class="rec-scene-name">${escape(s.name)}</span>
                  <span class="rec-scene-meta pg-cap-hint">${(s.component_keys || []).length} component${(s.component_keys||[]).length===1?"":"s"} · ${formatBytes(s.size_bytes || 0)} · ${escape(s.created_at.replace("T"," ").split(".")[0])}</span>
                </div>
                <div class="rec-scene-tags">
                  ${(s.component_keys || []).map(k => `<span class="rec-scene-tag">${escape(k)}</span>`).join("")}
                </div>
                <div class="rec-scene-actions">
                  <button class="sm rec-scene-restore" type="button" title="Restore this scene as the current state">Restore</button>
                  <button class="sm danger rec-scene-delete" type="button" title="Delete this scene (irreversible)">✕</button>
                </div>
              </div>`).join("");
            panel.innerHTML = `
              <h5>Scene snapshots <span class="pg-cap-hint">${scenes.length} saved</span></h5>
              <form class="rec-scene-capture" onsubmit="return false;">
                <input class="rec-scene-name-input" type="text" placeholder="Scene name (e.g. 'v1 — main mix')" required />
                <button class="btn-primary sm rec-scene-capture-btn" type="submit">+ Capture current state</button>
              </form>
              ${sceneRows ? `<div class="rec-scene-list">${sceneRows}</div>`
                          : `<p class="pg-cap-hint">No scenes yet. Capture the current state to save EDL + branding + automation + loudness + captions style as a named bundle.</p>`}
              <p class="pg-cap-hint">Restoring writes every captured component back to its plugin's store; the EDL restore goes through the editor's revision history so it's itself undoable.</p>`;
            // Wire capture form
            const form = panel.querySelector(".rec-scene-capture");
            form.addEventListener("submit", async (ev) => {
              ev.preventDefault();
              const input = panel.querySelector(".rec-scene-name-input");
              const name = input.value.trim();
              if (!name) { Toast.error("Scene name required"); return; }
              const captureBtn = panel.querySelector(".rec-scene-capture-btn");
              await withBusy(captureBtn, "Capturing…", async () => {
                const res = await API.scenesCapture(jobId, name);
                Toast.success(`Captured · ${res.component_keys.length} component(s) · ${formatBytes(res.size_bytes || 0)}`);
                // Re-open to refresh
                host.querySelector(".rec-ed-scenes")?.click();
              }).catch((err) => Toast.error(`Capture failed: ${err.message}`));
            });
            // Wire restore per row
            panel.querySelectorAll(".rec-scene-restore").forEach((rb) => {
              rb.addEventListener("click", async () => {
                const id = rb.closest(".rec-scene-row").dataset.sceneId;
                if (!confirm("Restore this scene? Every component (EDL, branding, automation, loudness, captions style) will be overwritten with the captured state. The EDL restore is itself undoable via the History panel.")) return;
                await withBusy(rb, "Restoring…", async () => {
                  const res = await API.scenesRestore(jobId, id);
                  // Re-fetch the EDL since the restore touched the
                  // editor store; rebuild the in-memory copy so the
                  // toolbar reflects the new cut list.
                  const reloaded = await API.editorLoad(jobId);
                  edl = reloaded.edl;
                  paint();
                  Toast.success(`Restored · ${res.restored.length} component(s)${res.skipped.length ? ` · ${res.skipped.length} skipped` : ""}.`);
                }).catch((err) => Toast.error(`Restore failed: ${err.message}`));
              });
            });
            // Wire delete per row
            panel.querySelectorAll(".rec-scene-delete").forEach((db) => {
              db.addEventListener("click", async () => {
                const row = db.closest(".rec-scene-row");
                const id = row.dataset.sceneId;
                if (!confirm("Delete this scene? Irreversible.")) return;
                await withBusy(db, "Deleting…", async () => {
                  await API.scenesDelete(jobId, id);
                  row.remove();
                  Toast.success("Scene deleted");
                }).catch((err) => Toast.error(`Delete failed: ${err.message}`));
              });
            });
          }).catch((err) => Toast.error(`Scenes failed: ${err.message}`));
        });
        host.querySelector(".rec-ed-render")?.addEventListener("click", async (e2) => {
          const btnR = e2.currentTarget;
          if (!confirm(`Render EDL to MKV? ${edl.cuts.length} cut(s), total ${fmtClock(dur)}. ffmpeg pass per cut + concat.`)) return;
          await withBusy(btnR, "Rendering…", async () => {
            const res = await API.editorRender(jobId);
            Toast.success(`Rendered ${formatBytes(res.bytes)} → ${res.output_path}`);
          }).catch((err) => Toast.error(`Render failed: ${err.message}`));
        });
      };
      paint();
      Toast.success("EDL loaded");
    }).catch((err) => Toast.error(`Editor failed: ${err.message}`));
  });

  overlay.querySelector("[data-action=rec-info-casebook]")?.addEventListener("click", async (e) => {
    const btn = e.currentTarget;
    await withBusy(btn, "Composing…", async () => {
      const resp = await API.casebookFetch(jobId);
      const host = document.getElementById("rec-casebook");
      if (!host) return;
      const report = resp.report;
      const md = resp.markdown || "";
      host.hidden = false;
      const sectionHtml = (report.sections || [])
        .map(
          (s) => `<details class="rec-cb-section" open>
            <summary><span class="rec-cb-h">${escape(s.heading)}</span></summary>
            <div class="rec-cb-body">${md_to_html(s.body)}</div>
          </details>`,
        )
        .join("");
      const titlesHtml = (report.suggested_titles || [])
        .map((t) => `<li>${escape(t)}</li>`)
        .join("");
      host.innerHTML = `
        <h4 class="rec-cp-title">Casebook · ${escape(report.title || "")} <span class="pg-cap-hint">${report.sections.length} sections · ${report.suggested_titles.length} title ideas</span></h4>
        <div class="rec-cb-actions">
          <a class="pg-linkbtn" href="${escape(API.casebookMarkdownUrl(jobId))}" download>Download .md</a>
          <button class="sm rec-cb-copy" type="button">Copy markdown</button>
        </div>
        ${titlesHtml ? `<details class="rec-cb-section"><summary><span class="rec-cb-h">Suggested titles</span></summary><ul class="rec-cb-titles">${titlesHtml}</ul></details>` : ""}
        ${sectionHtml}`;
      host.querySelector(".rec-cb-copy")?.addEventListener("click", async () => {
        try {
          await navigator.clipboard.writeText(md);
          Toast.success("Markdown copied");
        } catch (_) {
          Toast.error("Couldn't copy");
        }
      });
      Toast.success(`Casebook composed (${report.sections.length} sections)`);
    }).catch((err) => Toast.error(`Casebook failed: ${err.message}`));
  });

  overlay.querySelector("[data-action=rec-info-reuse]")?.addEventListener("click", async (e) => {
    const btn = e.currentTarget;
    await withBusy(btn, "Drafting…", async () => {
      const resp = await API.reuseGenerate(jobId);
      const host = document.getElementById("rec-reuse");
      if (!host) return;
      host.hidden = false;
      const drafts = resp.drafts || [];
      if (!drafts.length) {
        host.innerHTML = '<div class="empty sm">No drafts generated.</div>';
        return;
      }
      const fmtColour = {
        youtube_long: "hsl(0, 70%, 55%)",
        youtube_short: "hsl(15, 80%, 60%)",
        tiktok: "hsl(0, 0%, 90%)",
        patreon: "hsl(20, 70%, 60%)",
        podcast: "hsl(265, 60%, 70%)",
        blog: "hsl(170, 50%, 55%)",
      };
      const fmtLabel = {
        youtube_long: "YouTube (long)",
        youtube_short: "YouTube Shorts",
        tiktok: "TikTok",
        patreon: "Patreon",
        podcast: "Podcast",
        blog: "Blog draft",
      };
      host.innerHTML = `
        <h4 class="rec-cp-title">${drafts.length} publish drafts <span class="pg-cap-hint">queued · re-run regenerates</span></h4>
        <div class="rec-ru-grid">
          ${drafts
            .map(
              (d, i) => `
            <details class="rec-ru-card" style="--ru-c:${fmtColour[d.format] || fmtColour.blog}" data-i="${i}">
              <summary>
                <span class="rec-ru-fmt">${escape(fmtLabel[d.format] || d.format)}</span>
                <span class="rec-ru-meta">${escape(d.aspect)} · ${d.duration_sec > 0 ? fmtClock(d.duration_sec) : "—"}${d.clip_starts.length ? ` · ${d.clip_starts.length} clips` : ""}</span>
              </summary>
              <div class="rec-ru-body">
                <h5>Title</h5>
                <div class="rec-ru-title">${escape(d.title)}</div>
                <h5>Description</h5>
                <pre class="rec-ru-desc">${escape(d.description)}</pre>
                ${d.hashtags.length ? `<h5>Hashtags</h5><div class="rec-ru-tags">${d.hashtags.map((t) => `<span class="cfg-badge">${escape(t)}</span>`).join("")}</div>` : ""}
                <div class="rec-ru-actions">
                  <button class="sm rec-ru-copy-title" data-i="${i}">Copy title</button>
                  <button class="sm rec-ru-copy-desc" data-i="${i}">Copy description</button>
                  ${d.hashtags.length ? `<button class="sm rec-ru-copy-tags" data-i="${i}">Copy hashtags</button>` : ""}
                </div>
              </div>
            </details>`,
            )
            .join("")}
        </div>`;
      const cp = async (text) => {
        try {
          await navigator.clipboard.writeText(text || "");
          Toast.success("Copied");
        } catch (_) {
          Toast.error("Couldn't copy");
        }
      };
      host.querySelectorAll(".rec-ru-copy-title").forEach((b) =>
        b.addEventListener("click", () => cp(drafts[+b.dataset.i].title)));
      host.querySelectorAll(".rec-ru-copy-desc").forEach((b) =>
        b.addEventListener("click", () => cp(drafts[+b.dataset.i].description)));
      host.querySelectorAll(".rec-ru-copy-tags").forEach((b) =>
        b.addEventListener("click", () => cp(drafts[+b.dataset.i].hashtags.join(" "))));
      Toast.success(`Generated ${drafts.length} draft(s)`);
    }).catch((err) => Toast.error(`Draft failed: ${err.message}`));
  });

  overlay.querySelector("[data-action=rec-info-tracks]")?.addEventListener("click", async (e) => {
    const btn = e.currentTarget;
    await withBusy(btn, "Probing…", async () => {
      const resp = await API.multitrackList(jobId);
      const host = document.getElementById("rec-tracks");
      if (!host) return;
      const tracks = resp.tracks || [];
      host.hidden = false;
      if (!tracks.length) {
        host.innerHTML = '<div class="empty sm">No audio tracks detected.</div>';
        return;
      }
      const KIND_COLOUR = {
        mic: "hsl(140, 60%, 60%)",
        game: "hsl(210, 70%, 60%)",
        discord: "hsl(265, 60%, 65%)",
        music: "hsl(35, 80%, 60%)",
        browser: "hsl(195, 60%, 60%)",
        other: "hsl(0, 0%, 65%)",
      };
      host.innerHTML = `
        <h4 class="rec-cp-title">${tracks.length} audio track${tracks.length === 1 ? "" : "s"} <span class="pg-cap-hint">${tracks.length > 1 ? "OBS-style multi-track capture" : "single mixed track"}</span></h4>
        <div class="rec-tk-list">
          ${tracks
            .map(
              (t) => `
            <div class="rec-tk-row" data-idx="${t.index}">
              <span class="rec-tk-kind" style="--rec-tk-c:${KIND_COLOUR[t.inferred_kind] || KIND_COLOUR.other}">${escape(t.inferred_kind)}</span>
              <span class="rec-tk-label">${escape(t.title || `track ${t.index}`)}</span>
              <span class="rec-tk-meta">${t.codec} · ${t.channels}ch · ${t.sample_rate ? t.sample_rate + " Hz" : "?"}</span>
              <button class="sm rec-tk-extract" data-idx="${t.index}" data-stem="${escape((t.title || `track_${t.index}`).replace(/[^A-Za-z0-9_-]+/g, "_"))}">Extract</button>
            </div>`,
            )
            .join("")}
        </div>`;
      host.querySelectorAll(".rec-tk-extract").forEach((btn) => {
        btn.addEventListener("click", async (e) => {
          const b = e.currentTarget;
          await withBusy(b, "Cutting…", async () => {
            const res = await API.multitrackExtract(jobId, {
              track_index: parseInt(b.dataset.idx, 10),
              stem: b.dataset.stem,
            });
            Toast.success(`Cut ${formatBytes(res.bytes)} → ${res.output_path}`);
            b.outerHTML = `<span class="cfg-badge ok" title="${escape(res.output_path)}">✓ ${formatBytes(res.bytes)}</span>`;
          }).catch((err) => Toast.error(`Extract failed: ${err.message}`));
        });
      });
      Toast.success(`Probed ${tracks.length} track(s)`);
    }).catch((err) => Toast.error(`Probe failed: ${err.message}`));
  });

  overlay.querySelector("[data-action=rec-info-remux]")?.addEventListener("click", async (e) => {
    if (!(await confirmDialog(
      "Remux this recording into a matroska container with the aac_adtstoasc filter? The original is kept as <name>.orig.<ext> until success.",
      { ok: "Remux" },
    )))
      return;
    const btn = e.currentTarget;
    await withBusy(btn, "Remuxing…", async () => {
      await API.remuxRecording(jobId);
      Toast.success("Remuxed — try Play again");
    }).catch((err) => Toast.error(`Remux failed: ${err.message}`));
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
  overlay.querySelectorAll(".rec-copy").forEach((b) =>
    b.addEventListener("click", () => {
      const v = b.dataset.copy || "";
      navigator.clipboard?.writeText(v).then(
        () => Toast.success("Path copied"),
        () => Toast.error("Couldn't copy to clipboard"),
      );
    }));
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

async function openRecordingPlayer(jobId, opts = {}) {
  closeRecordingModals();
  // Defensive: if the keymap was left open from an earlier session it
  // shouldn't obscure the player. Belt to the Shift+I binding above.
  document.getElementById("kbd-help")?.classList.remove("open");
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
      <h2 class="rec-player-title">${escape(niceTitle(rec.stream_title) || rec.channel_name || "Recording")}</h2>
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
    // Honour opts.seekTo from openRecordingPlayer callers — e.g. the
    // Crunchr transcript line-click. Once metadata loads we know the
    // duration is valid, so clamping is safe. Auto-play makes the
    // jump feel snappy without the user pressing space.
    if (typeof opts.seekTo === "number" && opts.seekTo >= 0) {
      v.currentTime = Math.min(opts.seekTo, v.duration || opts.seekTo);
      v.play().catch(() => {});
    }
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
  { slug: "notifications", label: "Notifications", icon: "🔔" },
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
    <p class="page-subtitle">Live daemon configuration. Toggles and numeric knobs persist to <code>~/.config/strivo/config.toml</code> on change.</p>
    <div class="stg-shell">
      <nav class="stg-rail" aria-label="Settings sections">
        <div class="stg-search-wrap">
          <input id="stg-search" class="stg-search" type="search"
                 placeholder="Filter settings…" aria-label="Filter settings" />
        </div>
        ${rail}
      </nav>
      <div class="stg-pane" id="stg-pane">${pane}</div>
    </div>
  `);
  setupChromeHandlers();
  wireSettingsControls();
  wireSettingsSearch();
}

// Filter rows in the right pane and rail items by typed query (audit M10).
function wireSettingsSearch() {
  const input = document.getElementById("stg-search");
  if (!input) return;
  input.addEventListener("input", () => {
    const q = input.value.trim().toLowerCase();
    document.querySelectorAll(".stg-row").forEach((r) => {
      const txt = r.textContent.toLowerCase();
      r.classList.toggle("stg-row-hidden", q.length > 0 && !txt.includes(q));
    });
    // Hide group headings whose rows all collapsed.
    document.querySelectorAll(".stg-group").forEach((g) => {
      const anyVisible = g.querySelector(".stg-row:not(.stg-row-hidden)");
      g.style.display = q && !anyVisible ? "none" : "";
    });
  });
}

// Wire every editable control on the right pane. Each control declares
// its dotted config path via `data-stg-path` and its type via the input
// itself (checkbox / number). On change we POST to /settings/update;
// failure rolls the control back to its previous value and toasts.
function wireSettingsControls() {
  const pane = document.getElementById("stg-pane");
  if (!pane) return;
  // Configure / Reconfigure buttons on the Platforms section open a
  // wizard modal per platform.
  pane.querySelectorAll(".stg-cfg-btn").forEach((btn) => {
    btn.addEventListener("click", () => openPlatformWizard(btn.dataset.platform));
  });
  // Master toggle on Notifications dims the dependent Events group when
  // off. We do this in JS rather than re-rendering so users see immediate
  // visual feedback during the save round-trip.
  const masterEl = pane.querySelector('[data-stg-path="notifications.desktop_enabled"]');
  const condEl = pane.querySelector(".stg-subgroup-conditional");
  const syncMaster = () => {
    if (!masterEl || !condEl) return;
    if (masterEl.checked) {
      condEl.style.opacity = "";
      condEl.style.pointerEvents = "";
    } else {
      condEl.style.opacity = "0.55";
      condEl.style.pointerEvents = "none";
    }
  };
  if (masterEl) masterEl.addEventListener("change", syncMaster);
  // Onboarding controls — replay the welcome tour / reset per-page hints.
  pane.querySelector("#stg-replay-tour")?.addEventListener("click", () => {
    localStorage.removeItem("strivo-tour-done");
    startOnboardingTour();
  });
  pane.querySelector("#stg-reset-hints")?.addEventListener("click", () => {
    for (const k of Object.keys(localStorage)) {
      if (k.startsWith("strivo-hint-")) localStorage.removeItem(k);
    }
    Toast.success("Per-page hints reset · will reappear next visit");
    render().catch(() => {});
  });
  pane.querySelectorAll("[data-stg-path]").forEach((el) => {
    el.addEventListener("change", async (e) => {
      const path = el.getAttribute("data-stg-path");
      let value;
      if (el.type === "checkbox") value = el.checked;
      else if (el.type === "number") value = parseInt(el.value, 10);
      else value = (el.value || "").trim();
      const previous = el.type === "checkbox"
        ? !el.checked
        : el.getAttribute("data-prev") || "";
      try {
        await API.updateSetting(path, value);
        if (el.type !== "checkbox") el.setAttribute("data-prev", String(value));
        Toast.success(`Saved · ${path}`);
      } catch (err) {
        if (el.type === "checkbox") el.checked = previous;
        else el.value = previous;
        Toast.error(`Couldn't save ${path}: ${err.message}`);
      }
    });
  });
}

// Build the right-pane HTML for a section. Each section is a sequence of
// sub-headed groups, then a flat list of rows: label · value · hint.
function renderSettingsPane(slug, s) {
  const rec = s.recording || {};
  const arc = s.archiver || {};
  const ui = s.ui || {};
  const badge = (ok, okText, noText) =>
    `<span class="cfg-badge ${ok ? "ok" : "warn"}">${ok ? okText : noText}</span>`;
  const code = (v) => `<code>${escape(v || "—")}</code>`;
  // Editable controls: rendered as live inputs bound to a config path.
  // wireSettingsControls() picks them up via [data-stg-path].
  const toggle = (path, checked) => `
    <label class="stg-toggle">
      <input type="checkbox" data-stg-path="${escape(path)}" ${checked ? "checked" : ""} />
      <span class="stg-toggle-track"><span class="stg-toggle-knob"></span></span>
    </label>`;
  const numInput = (path, value, min, max) => `
    <input class="stg-num" type="number" data-stg-path="${escape(path)}"
           data-prev="${value ?? ""}" value="${value ?? ""}"
           min="${min}" max="${max}" step="1" />`;
  const textInput = (path, value, placeholder = "") => `
    <input class="stg-text" type="text" data-stg-path="${escape(path)}"
           data-prev="${escape(value ?? "")}" value="${escape(value ?? "")}"
           placeholder="${escape(placeholder)}" spellcheck="false" />`;
  const selectInput = (path, value, opts) => {
    const options = opts
      .map((o) => `<option value="${escape(o)}"${o === value ? " selected" : ""}>${escape(o)}</option>`)
      .join("");
    return `<select class="stg-select" data-stg-path="${escape(path)}" data-prev="${escape(value ?? "")}">${options}</select>`;
  };
  // Filename template token reference shown via the ⓘ hover hint.
  const TEMPLATE_TOKENS_HINT =
    "Tokens: {channel} channel name · {title} stream title · {date} YYYY-MM-DD · {time} HHMMSS · {platform} twitch/youtube/patreon · {id} broadcast id. Path-safe at write-time.";
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
      <h3 class="stg-group-title">${escape(title)}</h3>
      <div class="stg-rows">${rows}</div>
    </section>`;

  switch (slug) {
    case "general":
      return [
        group("At a glance", [
          row(
            "Tracked channels",
            `<a href="#/library" class="stg-linkbtn">${channelCache.length} channel${channelCache.length === 1 ? "" : "s"} →</a>`,
            "Click to manage channels in Library.",
          ),
          row(
            "Active recordings",
            `<a href="#/recordings" class="stg-linkbtn">${recCache.filter((r) => isInProgress(r.state)).length} in progress →</a>`,
            "Live captures + VOD pulls in flight.",
          ),
          row(
            "Patreon creators",
            `${(patreonState.creators || []).length}`,
            "Followed Patreon creators (read-only here; manage via the rail).",
          ),
        ].join("")),
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

    case "notifications": {
      const n = s.notifications || {};
      const masterOn = n.desktop_enabled !== false;
      const noteAttr = masterOn ? "" : ' style="opacity:0.55;pointer-events:none"';
      return [
        group("Desktop notifications", [
          row(
            "Master switch",
            toggle("notifications.desktop_enabled", n.desktop_enabled !== false),
            "When off, the daemon skips every notify-rust banner regardless of the toggles below. Useful for headless / kiosk setups.",
          ),
        ].join("")),
        `<div class="stg-subgroup-conditional"${noteAttr}>${[
          group("Events", [
            row(
              "Channel goes live",
              toggle("notifications.on_go_live", n.on_go_live !== false),
              "Banner when a tracked channel transitions offline → live.",
            ),
            row(
              "Recording finished",
              toggle("notifications.on_recording_finished", n.on_recording_finished !== false),
              "Banner when a live capture or VOD pull completes successfully.",
            ),
            row(
              "Recording failed",
              toggle("notifications.on_recording_failed", n.on_recording_failed !== false),
              "Always recommended: silent failures are the worst class of PVR bug.",
            ),
            row(
              "VOD backfill ready",
              toggle("notifications.on_vod_ready", n.on_vod_ready === true),
              "Banner when the Twitch auto-VOD-backfill pull lands. Off by default — most users don't track it manually.",
            ),
          ].join("")),
        ].join("")}</div>`,
      ].join("");
    }

    case "recording":
      return [
        group("Output", [
          row("Filename template",
            textInput("recording.filename_template", rec.filename_template, "{channel}_{date}_{title}.mkv"),
            TEMPLATE_TOKENS_HINT),
          row("Container",
            selectInput("recording.container",
              (rec.container || "matroska").toLowerCase(),
              ["matroska", "mp4", "webm"]),
            "Output muxer. Matroska is the browser-friendliest default; switch only if you have a downstream pipeline that needs MP4 or WebM."),
          row("Transcode", toggle("recording.transcode", rec.transcode),
            "Re-encode on the fly via h264_nvenc. Off = stream-copy (zero CPU, original bitrate)."),
        ].join("")),
        group("Twitch", [
          row("Record from start", toggle("recording.twitch_live_from_start", rec.twitch_live_from_start),
            "Pull from the first available HLS segment (~5 min back) instead of the live edge. Sub-only channels reject this and StriVo silently falls back to live edge."),
        ].join("")),
        group("YouTube / VOD", [
          row("Auto VOD backfill", toggle("recording.auto_vod_backfill", rec.auto_vod_backfill),
            "When a stream ends, automatically queue the resulting VOD for download via yt-dlp."),
          row("Auto-trim ads", toggle("recording.auto_trim_ads", rec.auto_trim_ads),
            "Run sponsorblock-style ad-segment trimming on completed Twitch VODs."),
        ].join("")),
      ].join("");

    case "platforms": {
      const platformRow = (key, statusOk) => `
        <div class="stg-row">
          <div class="stg-row-label">Status</div>
          <div class="stg-row-value">
            ${badge(statusOk, "configured", "not configured")}
            <button class="stg-linkbtn stg-cfg-btn" data-platform="${escape(key)}" type="button">
              ${statusOk ? "Reconfigure" : "Configure"} →
            </button>
          </div>
        </div>`;
      return [
        group("Twitch",
          platformRow("twitch", s.twitch_configured) +
          `<div class="stg-row"><div class="stg-row-label">Setup
            <span class="stg-hint" title="Create at dev.twitch.tv/console/apps — type=Other, OAuth Redirect URL http://localhost:8181/oauth/twitch">ⓘ</span>
          </div><div class="stg-row-value muted">Twitch Developer Console → Register Your Application → Client ID + Secret.</div></div>`),
        group("YouTube",
          platformRow("youtube", s.youtube_configured) +
          `<div class="stg-row"><div class="stg-row-label">Setup
            <span class="stg-hint" title="Google Cloud Console → APIs &amp; Services → Credentials → OAuth client ID. Use Desktop type.">ⓘ</span>
          </div><div class="stg-row-value muted">OAuth client (Desktop type). Optional Netscape cookies.txt for member-only / age-gated VODs.</div></div>`),
        group("Patreon",
          platformRow("patreon", s.patreon_configured) +
          `<div class="stg-row"><div class="stg-row-label">Setup
            <span class="stg-hint" title="patreon.com/portal/registration/register-clients">ⓘ</span>
          </div><div class="stg-row-value muted">Optional. Enables Patreon-locked VOD pulls from creators you support.</div></div>`),
      ].join("");
    }

    case "plugins": {
      // Plugin manager. Lists every shipped plugin with a per-plugin
      // enable toggle bound to plugins.<name>.enabled, plus an 'Open'
      // CTA that deep-links into the plugin's own page (when one
      // exists) or the marketplace catalog card otherwise. Pre-existing
      // Archiver per-knob settings stay in their own group below.
      const toggles = s.plugin_toggles || {};
      // PLUGIN_REGISTRY is the same set the Plugins hub + marketplace
      // share — lives at the bottom of spa.js. category drives the
      // sub-group heading.
      const groups = {};
      for (const meta of PLUGIN_REGISTRY) {
        (groups[meta.category] ||= []).push(meta);
      }
      const enabledFor = (name) => {
        const t = toggles[name];
        return t == null ? true : t.enabled !== false;
      };
      const pluginRow = (meta) => {
        const open = meta.route
          ? `<a href="${escape(meta.route)}" class="stg-linkbtn">Open →</a>`
          : `<a href="#/plugins" class="stg-linkbtn">View in hub →</a>`;
        return `
          <div class="stg-row stg-plugin-row" data-plugin-name="${escape(meta.name)}">
            <div class="stg-row-label">
              <span class="stg-plugin-name">${escape(meta.label)}</span>
              <span class="stg-hint" title="${escape(meta.description)}">ⓘ</span>
              <span class="stg-plugin-tags">
                ${meta.proGated ? '<span class="cfg-badge ok" title="Strivo Pro plugin">Pro</span>' : ""}
                ${meta.installed === false ? '<span class="cfg-badge warn">not installed</span>' : ""}
              </span>
            </div>
            <div class="stg-row-value stg-plugin-actions">
              ${toggle(`plugins.${meta.name}.enabled`, enabledFor(meta.name))}
              ${open}
            </div>
          </div>`;
      };
      const archiverExtras = `
        <details class="stg-plugin-details">
          <summary>Archiver advanced</summary>
          ${row("Archive directory",
            textInput("archiver.archive_dir", arc.archive_dir, "/path/to/archives"),
            "Where archived VODs land. Defaults under the main recording dir.")}
          ${row("Format",
            textInput("archiver.format", arc.format, "best"),
            "yt-dlp format selector. Default targets bestvideo+bestaudio with a sensible cap.")}
          ${row("Concurrent fragments", numInput("archiver.concurrent_fragments", arc.concurrent_fragments ?? 4, 1, 16),
            "yt-dlp -N flag. 1–16; higher = faster but more rate-limit pressure.")}
        </details>`;
      const sections = Object.keys(groups).sort().map((cat) =>
        group(cat, groups[cat].map(pluginRow).join("") + (cat === "Archive" ? archiverExtras : ""))
      ).join("");
      return sections;
    }

    case "interface":
      return [
        group("Onboarding", [
          row("Welcome tour",
            `<button class="sm" id="stg-replay-tour" type="button">Replay tour</button>`,
            "Walk through the topbar one stop at a time. Useful after a major UI change."),
          row("Per-page hints",
            `<button class="sm" id="stg-reset-hints" type="button">Reset dismissed hints</button>`,
            "Make every per-page hint banner show up again on the next visit."),
        ].join("")),
        group("Accessibility", [
          row("Reduce motion", toggle("ui.reduce_motion", ui.reduce_motion),
            "Disables non-essential transitions across the UI. Mirrors the OS-level prefers-reduced-motion."),
          row("Verbose status", toggle("ui.verbose_status", ui.verbose_status),
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
          // Source link points at the home docs site to survive the
          // private-repo flip (audit U19). chorosyne.com → strivo will
          // 404 today but won't link to a 404'd github repo after the
          // visibility flip.
          row("Project", `<a href="https://chorosyne.com" class="stg-linkbtn" target="_blank" rel="noopener">chorosyne.com →</a>`),
          row("Plugins", `<a href="#/plugins" class="stg-linkbtn">Plugin hub →</a>`),
        ].join("")),
        group("Licence", [
          row("Strivo Pro", `<a href="#/plugins" class="stg-linkbtn">Manage entitlement →</a>`,
            "One-time $25 unlock for every shipped plugin. Activate or start a 3-day trial from the Plugins hub."),
        ].join("")),
      ].join("");
  }
}

// Each platform's wizard form spec: the fields it needs + a docs link
// the modal renders below the inputs. Kept tiny so it's obvious what
// each platform asks for; if it grows we lift it to its own module.
const PLATFORM_SPECS = {
  twitch: {
    title: "Configure Twitch",
    docsLabel: "Twitch Developer Console",
    docsUrl: "https://dev.twitch.tv/console/apps",
    fields: [
      { name: "client_id", label: "Client ID", type: "text", required: true },
      { name: "client_secret", label: "Client Secret", type: "password", required: true },
    ],
    notes: "Register Your Application → type 'Other', OAuth Redirect URL <code>http://localhost:8181/oauth/twitch</code>.",
  },
  youtube: {
    title: "Configure YouTube",
    docsLabel: "Google Cloud Console",
    docsUrl: "https://console.cloud.google.com/apis/credentials",
    fields: [
      { name: "client_id", label: "OAuth Client ID", type: "text", required: true },
      { name: "client_secret", label: "OAuth Client Secret", type: "password", required: true },
      { name: "cookies_path", label: "Cookies file (optional)", type: "text", placeholder: "/path/to/cookies.txt" },
      { name: "websub_callback_url", label: "WebSub callback URL (optional)", type: "url", placeholder: "https://your.tld/yt-websub" },
    ],
    notes: "Create OAuth 2.0 client ID, application type <em>Desktop app</em>. Cookies file enables age-restricted + member-only VODs.",
  },
  patreon: {
    title: "Configure Patreon",
    docsLabel: "Patreon Platform",
    docsUrl: "https://www.patreon.com/portal/registration/register-clients",
    fields: [
      { name: "client_id", label: "Client ID", type: "text", required: true },
      { name: "client_secret", label: "Client Secret", type: "password", required: true },
      { name: "cookies_path", label: "Cookies file (optional)", type: "text", placeholder: "/path/to/cookies.txt" },
    ],
    notes: "Cookies file is your logged-in patreon.com session — required to download VOD posts.",
  },
};

function openPlatformWizard(platform) {
  const spec = PLATFORM_SPECS[platform];
  if (!spec) return;
  const fieldHtml = spec.fields
    .map(
      (f) => `
        <label class="modal-field">
          <span class="modal-field-label">${escape(f.label)}${f.required ? " *" : ""}</span>
          <input class="modal-input" name="${escape(f.name)}" type="${escape(f.type)}"
            ${f.required ? "required" : ""}
            ${f.placeholder ? `placeholder="${escape(f.placeholder)}"` : ""} />
        </label>`,
    )
    .join("");
  const dlg = document.createElement("div");
  dlg.className = "modal-backdrop";
  dlg.innerHTML = `
    <form class="modal" role="dialog" aria-labelledby="pf-title">
      <header class="modal-head">
        <h2 id="pf-title">${escape(spec.title)}</h2>
        <button type="button" class="modal-close" aria-label="Close">×</button>
      </header>
      <div class="modal-body">
        ${fieldHtml}
        <p class="modal-notes">${spec.notes}
          <a href="${escape(spec.docsUrl)}" target="_blank" rel="noopener">${escape(spec.docsLabel)} →</a>
        </p>
      </div>
      <footer class="modal-foot">
        <button type="button" class="btn-ghost modal-cancel">Cancel</button>
        <button type="submit" class="btn-primary">Save</button>
      </footer>
    </form>`;
  document.body.appendChild(dlg);
  const close = () => dlg.remove();
  dlg.querySelector(".modal-close").addEventListener("click", close);
  dlg.querySelector(".modal-cancel").addEventListener("click", close);
  dlg.addEventListener("click", (e) => { if (e.target === dlg) close(); });
  dlg.querySelector(".modal").addEventListener("submit", async (e) => {
    e.preventDefault();
    const body = {};
    spec.fields.forEach((f) => {
      body[f.name] = e.target.elements[f.name].value.trim();
    });
    try {
      await API.setPlatform(platform, body);
      Toast.success(`${spec.title.replace("Configure ", "")} saved`);
      close();
      // Re-render the Settings page so the status badge flips green.
      render();
    } catch (err) {
      Toast.error(`Couldn't save: ${err.message}`);
    }
  });
  // Autofocus the first field for a keyboard-driven flow.
  dlg.querySelector(".modal-input")?.focus();
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
          (c) => {
            // Surface a Re-authenticate link on platform-auth checks
            // (audit M9). When the token's healthy, the link is hidden;
            // either way the Settings wizard is one click away.
            const lc = c.name.toLowerCase();
            const reauth =
              c.domain === "Platform Auth" && ["twitch", "youtube", "patreon"].includes(lc)
                ? ` <a class="sys-reauth" href="#/settings/platforms" title="Open the ${lc} setup wizard">Re-authenticate →</a>`
                : "";
            return `
    <div class="sys-check ${c.severity}">
      <span class="sys-sev">${sevGlyph[c.severity] || "•"}</span>
      <span class="sys-label">${escape(c.name)}</span>
      <span class="sys-msg">${escape(c.message)}${c.fix ? ` <span class="sys-fix">— ${escape(c.fix)}</span>` : ""}${reauth}</span>
    </div>`;
          },
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
              <input id="poll-interval" type="number" min="15" max="86400" step="5"
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
          <a class="sm" href="#/library">View</a>
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
        <a class="sm" href="/api/v1/backups/${encodeURIComponent(b.name)}/download"
           download="strivo-backup-${escape(b.name)}.tar.gz"
           title="Download backup as tarball">Download</a>
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

// ── Logs viewer ──────────────────────────────────────────────────────
// Tails the rolling log file. Level dropdown + per-source filter chips +
// free-text search + multi-line entry collapse (audit U5, M7, R5).
let logsLevel = "info";
let logsQuery = "";
let logsSourceFilter = ""; // crate/module substring; "" = all sources
let logsFollow = localStorage.getItem("strivo-logs-follow") === "1";
let logsRegex = localStorage.getItem("strivo-logs-regex") === "1";
let logsFollowTimer = null;

// A "log entry" is a starting line (parsable timestamp + level) plus any
// following indented/JSON-blob continuation lines. We collapse those
// continuation lines into a single click-to-expand block so YouTube
// quota 403s stop dominating the viewport.
function parseLogEntries(lines) {
  const entries = [];
  const startRe = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/;
  for (const raw of lines) {
    if (startRe.test(raw)) {
      entries.push({ head: raw, tail: [] });
    } else if (entries.length) {
      entries[entries.length - 1].tail.push(raw);
    } else {
      entries.push({ head: raw, tail: [] });
    }
  }
  return entries;
}

// Pull a coarse "source" tag (crate::module) out of a log line head.
function logSource(line) {
  const m = line.match(/\s+(strivo_[a-zA-Z_]+(::[a-zA-Z_]+)*)/);
  return m ? m[1] : "";
}

async function renderLogs() {
  const levels = ["error", "warn", "info", "debug", "trace"];
  const options = levels
    .map((l) => `<option value="${l}"${l === logsLevel ? " selected" : ""}>${l.toUpperCase()}</option>`)
    .join("");
  // Stop any prior tail-follow timer before mounting the page (route
  // navigation, theme change, hot reload, etc.).
  if (logsFollowTimer) { clearInterval(logsFollowTimer); logsFollowTimer = null; }
  root.innerHTML = chrome(`
    <h1 class="page-title">Logs</h1>
    <div class="logs-toolbar">
      <label>Min level <select id="logs-level">${options}</select></label>
      <input id="logs-search" class="logs-search" type="search"
             placeholder="${logsRegex ? "Regex (case-insensitive)…" : "Search log text…"}"
             value="${escape(logsQuery)}" />
      <label class="logs-toggle" title="Search as case-insensitive regex">
        <input type="checkbox" id="logs-regex" ${logsRegex ? "checked" : ""}/> regex
      </label>
      <label class="logs-toggle" title="Auto-refresh every 4s and pin scroll to bottom">
        <input type="checkbox" id="logs-follow" ${logsFollow ? "checked" : ""}/> follow
      </label>
      <span id="logs-sources" class="logs-sources"></span>
      <button id="logs-refresh" class="sm" title="Reload now">↻ Refresh</button>
      <button id="logs-copy" class="sm" title="Copy filtered log lines to clipboard">⧉ Copy</button>
      <button id="logs-download" class="sm" title="Save filtered log lines as a .log file">⬇ Download</button>
      <span id="logs-file" class="logs-file"></span>
    </div>
    <div id="logs-output" class="logs-output" aria-live="polite">Loading…</div>
  `);
  setupChromeHandlers();

  async function load() {
    const out = document.getElementById("logs-output");
    const fileEl = document.getElementById("logs-file");
    try {
      const r = await API.logs(logsLevel, 500);
      const allLines = r.lines || [];
      const allEntries = parseLogEntries(allLines);
      // Build the source-filter chip set from what's currently in view.
      const sources = [...new Set(allEntries.map((e) => logSource(e.head)).filter(Boolean))].sort();
      const chips = document.getElementById("logs-sources");
      if (chips) {
        chips.innerHTML = ['<button class="logs-chip" data-src="">all</button>']
          .concat(
            sources.map(
              (s) =>
                `<button class="logs-chip${s === logsSourceFilter ? " is-active" : ""}" data-src="${escape(s)}">${escape(s.replace(/^strivo_/, ""))}</button>`,
            ),
          )
          .join("");
        chips.querySelectorAll(".logs-chip").forEach((b) => {
          b.addEventListener("click", () => {
            logsSourceFilter = b.dataset.src || "";
            load();
          });
        });
      }
      const q = logsQuery.trim();
      // Regex compile once per load. Invalid pattern → tooltip via input
      // border colour + skip the filter (don't silently exclude
      // everything when the user mistypes).
      let pattern = null;
      let patternBad = false;
      if (q && logsRegex) {
        try { pattern = new RegExp(q, "i"); }
        catch (_) { patternBad = true; }
      }
      const searchInput = document.getElementById("logs-search");
      if (searchInput) searchInput.classList.toggle("logs-search-bad", patternBad);
      const qLower = q.toLowerCase();
      const filtered = allEntries.filter((e) => {
        if (logsSourceFilter && !e.head.includes(logsSourceFilter)) return false;
        if (!q || patternBad) return true;
        const hay = e.head + "\n" + e.tail.join("\n");
        if (pattern) return pattern.test(hay);
        return hay.toLowerCase().includes(qLower);
      });
      out.innerHTML = filtered.length
        ? filtered
            .map((e) => {
              const head = escape(e.head);
              if (!e.tail.length) return `<div class="log-line">${head}</div>`;
              const tail = escape(e.tail.join("\n"));
              return `<details class="log-line log-multi"><summary>${head} <span class="log-more">+${e.tail.length}</span></summary><pre>${tail}</pre></details>`;
            })
            .join("")
        : "<div class='empty sm'>No log lines match the current filters.</div>";
      if (fileEl) fileEl.textContent = r.file ? `· ${r.file} · ${filtered.length}/${allEntries.length} entries` : "";
      // Pin scroll to bottom in follow mode UNLESS the user has
      // intentionally scrolled up (we treat "within 80px of bottom"
      // as still-following so the auto-pin doesn't fight a fast scroll
      // recovery).
      const userPaused = out.scrollHeight - out.scrollTop - out.clientHeight > 80;
      if (!logsFollow || !userPaused) out.scrollTop = out.scrollHeight;
      // Stash for Copy/Download handlers.
      logsLastFilteredText = filtered
        .map((e) => e.tail.length ? `${e.head}\n${e.tail.join("\n")}` : e.head)
        .join("\n");
      logsLastFile = r.file || "strivo.log";
    } catch (e) {
      out.textContent = `Failed to load logs: ${e.message}`;
    }
  }
  document.getElementById("logs-level")?.addEventListener("change", (e) => {
    logsLevel = e.target.value;
    load();
  });
  document.getElementById("logs-search")?.addEventListener("input", (e) => {
    logsQuery = e.target.value;
    load();
  });
  document.getElementById("logs-regex")?.addEventListener("change", (e) => {
    logsRegex = e.target.checked;
    localStorage.setItem("strivo-logs-regex", logsRegex ? "1" : "0");
    // Re-render so the placeholder copy updates; load() also re-runs to
    // apply the new pattern interpretation against the cached entries.
    renderLogs().catch(() => {});
  });
  document.getElementById("logs-follow")?.addEventListener("change", (e) => {
    logsFollow = e.target.checked;
    localStorage.setItem("strivo-logs-follow", logsFollow ? "1" : "0");
    if (logsFollow) {
      if (logsFollowTimer) clearInterval(logsFollowTimer);
      logsFollowTimer = setInterval(load, 4000);
    } else if (logsFollowTimer) {
      clearInterval(logsFollowTimer);
      logsFollowTimer = null;
    }
  });
  document.getElementById("logs-refresh")?.addEventListener("click", load);
  document.getElementById("logs-copy")?.addEventListener("click", async (e) => {
    const btn = e.currentTarget;
    try {
      await navigator.clipboard.writeText(logsLastFilteredText || "");
      Toast.success("Logs copied to clipboard");
    } catch (err) {
      Toast.error(`Copy failed: ${err.message}`);
    }
  });
  document.getElementById("logs-download")?.addEventListener("click", () => {
    const blob = new Blob([logsLastFilteredText || ""], { type: "text/plain;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = logsLastFile || "strivo.log";
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  });
  await load();
  // Auto-arm follow if it was previously enabled.
  if (logsFollow) {
    logsFollowTimer = setInterval(load, 4000);
  }
}

// Cache the last-rendered filtered text so Copy/Download don't have to
// re-walk the DOM. Updated inside renderLogs.load().
let logsLastFilteredText = "";
let logsLastFile = "strivo.log";

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
  // Page lives at #/schedule for back-compat with bookmarks, but is now
  // the Monitor page — record-when-live and auto-download new uploads
  // replace the cron form that 95% of users found foreign. (Power users
  // can still add cron entries via config.toml's [[schedule]] table;
  // they show up in the "Cron schedule" group below when present.)
  let monitor = { auto_record: [], auto_download: [] };
  let channels = [];
  let cronEntries = [];
  let settings = {};
  let health = {};
  try {
    const [m, c, s, st, h] = await Promise.all([
      API.monitor().catch(() => ({ auto_record: [], auto_download: [] })),
      API.channels().then((r) => r.channels || []).catch(() => []),
      API.schedule().then((r) => r.schedule || []).catch(() => []),
      API.settings().catch(() => ({})),
      API.health().catch(() => ({})),
    ]);
    monitor = m;
    channels = c;
    cronEntries = s;
    settings = st;
    health = h;
  } catch (_) {}
  root.removeAttribute("aria-busy");

  // Build a channel lookup so we can show display_name + platform.
  const channelByKey = new Map(
    channels.map((c) => [`${c.platform}:${c.id}`, c]),
  );
  const channelByName = new Map(
    channels.map((c) => [(c.display_name || c.name || "").toLowerCase(), c]),
  );
  const channelsAvailableForDownload = channels.filter(
    (c) => c.platform === "YouTube",
  );

  // Section 1 — record when live (existing auto-record list).
  const recordRows = monitor.auto_record
    .map(
      (e) => `
    <div class="task-row">
      <div class="task-info">
        <span class="task-name">${escape(e.channel_name || e.channel_id)} <span class="mon-plat plat-${escape(e.platform.toLowerCase())}">${escape(e.platform)}</span></span>
        <span class="task-cadence">${escape(e.key)}</span>
      </div>
      <button class="sm mon-rec-rm" data-key="${escape(e.key)}" title="Stop auto-recording this channel">✕</button>
    </div>`,
    )
    .join("");

  // Section 2 — auto-download new uploads (YouTube only).
  const downloadRows = monitor.auto_download
    .map((e) => {
      const ch = channelByKey.get(e.key);
      const name = ch ? (ch.display_name || ch.name) : e.channel_id;
      const playlistsValue = (e.playlists || []).join(", ");
      return `
      <div class="task-row mon-dl-row">
        <div class="task-info">
          <span class="task-name">${escape(name)} <span class="mon-plat plat-${escape(e.platform.toLowerCase())}">${escape(e.platform)}</span></span>
          <span class="task-cadence">
            <label class="mon-scope">
              <span>Limit to playlists (optional, comma-separated)</span>
              <input class="mon-playlists" type="text" data-key="${escape(e.key)}"
                     placeholder="PLxxx, PLyyy — leave empty for whole channel"
                     value="${escape(playlistsValue)}" />
            </label>
          </span>
        </div>
        <button class="sm mon-dl-rm" data-key="${escape(e.key)}" title="Stop auto-downloading uploads from this channel">✕</button>
      </div>`;
    })
    .join("");

  // Cron schedule section — kept for power users who already use it,
  // collapsed by default. Empty unless config.toml has entries.
  const cronGroup = cronEntries.length
    ? `<details class="mon-cron"><summary>Advanced cron schedule (${cronEntries.length})</summary>
        ${cronEntries
          .map(
            (e, i) => `
          <div class="task-row">
            <div class="task-info">
              <span class="task-name">${escape(e.channel || "scheduled")}</span>
              <span class="task-cadence"><code>${escape(e.cron || "")}</code>${e.duration ? ` · ${escape(e.duration)}` : ""}${e.next_fire ? ` · next: ${escape(new Date(e.next_fire).toLocaleString())}` : ""}</span>
            </div>
            <button class="sm sch-del" data-i="${i}" title="Delete this cron entry">✕</button>
          </div>`,
          )
          .join("")}
        <p class="mon-cron-hint">Cron entries are added via <code>~/.config/strivo/config.toml</code> under <code>[[schedule]]</code>. They fire at the cron expression's next match regardless of live state — useful for predictable shows on platforms without a live API. Most users want the simpler primitives above.</p>
      </details>`
    : "";

  // Get-channel-name helper for the Add forms — match by case-insensitive
  // display name, fall back to "Platform:id" parsing.
  const resolveChannelKey = (raw) => {
    const t = raw.trim();
    if (!t) return null;
    if (t.includes(":")) return t;
    const c = channelByName.get(t.toLowerCase());
    return c ? `${c.platform}:${c.id}` : null;
  };

  // Live capture status: active recordings + disk free + current limits.
  // Recordings cache is shared across the SPA so we can read in-progress
  // count without a separate fetch.
  const activeCount = recCache.filter((r) => isInProgress(r.state)).length;
  const limits = settings.monitor_limits || {};
  const maxConcurrent = limits.max_concurrent_recordings || 0;
  const diskBudgetGb = limits.disk_budget_reserved_gb || 0;
  const diskAvailBytes = (health.disk && health.disk.filesystem_avail_bytes) || 0;
  const diskTotalBytes = (health.disk && health.disk.filesystem_total_bytes) || 0;
  const availPct = diskTotalBytes > 0 ? (diskAvailBytes / diskTotalBytes) * 100 : 0;
  // Reserve budget vs free: warn when free < reserved + 5 GB headroom.
  const reservedBytes = diskBudgetGb * 1024 * 1024 * 1024;
  const diskOverBudget = reservedBytes > 0 && diskAvailBytes < reservedBytes;
  const concurrentSaturated = maxConcurrent > 0 && activeCount >= maxConcurrent;
  const statusBanner = (concurrentSaturated || diskOverBudget)
    ? `<div class="mon-status-banner warn">
         ${concurrentSaturated ? `<span>⚠ Concurrent cap hit: ${activeCount}/${maxConcurrent} recordings in flight — new live captures will queue.</span>` : ""}
         ${diskOverBudget ? `<span>⚠ Disk free (${formatBytes(diskAvailBytes)}) is below the reserved ${diskBudgetGb} GB — new captures will defer.</span>` : ""}
       </div>`
    : `<div class="mon-status-banner ok">
         <span>✓ ${activeCount} recording${activeCount === 1 ? "" : "s"} in flight${maxConcurrent ? ` / ${maxConcurrent}` : ""} · ${formatBytes(diskAvailBytes)} free</span>
       </div>`;

  root.innerHTML = chrome(`
    <h1 class="page-title">Monitor</h1>
    <p class="page-subtitle">Channels StriVo is watching. Record live broadcasts as they happen, or auto-download new YouTube uploads.</p>

    ${statusBanner}

    <section class="cfg-card">
      <h2 class="cfg-title">Capture limits <a href="#/settings/notifications" class="stg-linkbtn" style="margin-left:auto;font-size:0.78em">Configure go-live banners →</a></h2>
      <p class="mon-help">Safety knobs that defer new captures when StriVo is already busy or disk is tight. Zero in either field disables that cap.</p>
      <div class="mon-limits-grid">
        <label class="mon-limit">
          <span class="mon-limit-label">Max concurrent recordings</span>
          <input class="mon-limit-input" type="number" min="0" max="64" step="1"
                 id="mon-limit-concurrent" value="${maxConcurrent}" />
          <span class="mon-limit-hint">${maxConcurrent === 0 ? "unlimited" : `${activeCount} of ${maxConcurrent} in use`}</span>
        </label>
        <label class="mon-limit">
          <span class="mon-limit-label">Reserved disk budget (GB)</span>
          <input class="mon-limit-input" type="number" min="0" max="100000" step="1"
                 id="mon-limit-disk" value="${diskBudgetGb}" />
          <span class="mon-limit-hint">${diskBudgetGb === 0 ? "no circuit breaker" : diskOverBudget ? "ENGAGED" : "armed"}</span>
        </label>
        <div class="mon-disk-gauge" title="Recording filesystem usage">
          <span class="mon-disk-label">Free disk</span>
          <div class="mon-disk-bar"><div class="mon-disk-fill" style="width:${(100 - availPct).toFixed(1)}%"></div></div>
          <span class="mon-disk-meta">${formatBytes(diskAvailBytes)} free of ${formatBytes(diskTotalBytes)}</span>
        </div>
      </div>
    </section>

    <section class="cfg-card">
      <h2 class="cfg-title">Record when live</h2>
      <p class="mon-help">Twitch and YouTube live broadcasts capture automatically. Add channels from the topbar's <em>+ Add channel</em>, then enable Auto-record on the channel card.</p>
      ${recordRows || '<div class="empty sm">No channels are set to record-when-live yet.</div>'}
    </section>

    <section class="cfg-card">
      <h2 class="cfg-title">Auto-download new uploads</h2>
      <p class="mon-help">Pulls new uploads from a YouTube channel as the monitor sees them. Leave the playlist field empty for the whole channel, or paste one or more playlist IDs to limit scope.</p>
      ${downloadRows || '<div class="empty sm">No channels are set to auto-download yet.</div>'}
      <form id="mon-dl-add" class="mon-add">
        <select id="mon-dl-channel">
          <option value="">Pick a YouTube channel…</option>
          ${channelsAvailableForDownload
            .map((c) => `<option value="${escape(`${c.platform}:${c.id}`)}">${escape(c.display_name || c.name)}</option>`)
            .join("")}
        </select>
        <button class="btn-primary" type="submit">Enable</button>
      </form>
    </section>

    ${cronGroup}
  `);
  setupChromeHandlers();

  // Capture-limit inputs — debounced save to /settings/update so each
  // keystroke doesn't fire a round-trip. Repaint on save so the gauge
  // and banner reflect the new state.
  const wireLimit = (id, path, max) => {
    const el = document.getElementById(id);
    if (!el) return;
    let timer;
    el.addEventListener("input", () => {
      clearTimeout(timer);
      timer = setTimeout(async () => {
        const v = Math.max(0, Math.min(max, parseInt(el.value, 10) || 0));
        if (v !== parseInt(el.value, 10)) el.value = v;
        try {
          await API.updateSetting(path, v);
          Toast.success(`Saved · ${path}`);
          renderSchedule().catch(() => {});
        } catch (err) {
          Toast.error(`Save failed: ${err.message}`);
        }
      }, 600);
    });
  };
  wireLimit("mon-limit-concurrent", "monitor_limits.max_concurrent_recordings", 64);
  wireLimit("mon-limit-disk", "monitor_limits.disk_budget_reserved_gb", 100000);

  // Record-when-live row delete.
  document.querySelectorAll(".mon-rec-rm").forEach((btn) => {
    btn.addEventListener("click", async () => {
      if (!confirm("Stop auto-recording this channel?")) return;
      try {
        await API.toggleAutoRecord(btn.dataset.key, false);
        Toast.success("Stopped");
        renderSchedule();
      } catch (e) {
        Toast.error(`Couldn't stop: ${e.message}`);
      }
    });
  });

  // Auto-download row delete + playlist edits.
  document.querySelectorAll(".mon-dl-rm").forEach((btn) => {
    btn.addEventListener("click", async () => {
      if (!confirm("Stop auto-downloading new uploads from this channel?")) return;
      try {
        await API.setArchiverTandem(btn.dataset.key, false);
        Toast.success("Stopped");
        renderSchedule();
      } catch (e) {
        Toast.error(`Couldn't stop: ${e.message}`);
      }
    });
  });
  // Debounced save on playlist field changes — split on comma/space.
  document.querySelectorAll(".mon-playlists").forEach((inp) => {
    let timer;
    inp.addEventListener("input", () => {
      clearTimeout(timer);
      timer = setTimeout(async () => {
        const key = inp.dataset.key;
        const playlists = inp.value
          .split(/[\s,]+/)
          .map((s) => s.trim())
          .filter(Boolean);
        try {
          await API.setArchiverPlaylists(key, playlists);
          Toast.success("Playlists saved");
        } catch (e) {
          Toast.error(`Couldn't save: ${e.message}`);
        }
      }, 600);
    });
  });

  // Add new auto-download channel.
  document.getElementById("mon-dl-add")?.addEventListener("submit", async (e) => {
    e.preventDefault();
    const key = document.getElementById("mon-dl-channel").value;
    if (!key) return;
    try {
      await API.setArchiverTandem(key, true);
      Toast.success("Enabled");
      renderSchedule();
    } catch (err) {
      Toast.error(`Couldn't enable: ${err.message}`);
    }
  });
  // Cron entry delete (still works for power users).
  document.querySelectorAll(".sch-del").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const i = parseInt(btn.dataset.i, 10);
      if (!confirm("Delete this cron entry?")) return;
      try {
        await API.scheduleDelete(i);
        Toast.success("Removed");
        renderSchedule();
      } catch (err) {
        Toast.error(`Couldn't delete: ${err.message}`);
      }
    });
  });
  // Silence the unused channel-lookup helper warning when no channels
  // happen to be queried — kept for future quick-add by name.
  void resolveChannelKey;
}

// Legacy cron-form renderer retained for ref but unused — kept as a
// no-op to avoid breaking any externally-cached bookmarks of the old
// shape. Power users still add cron entries via config.toml.
// eslint-disable-next-line no-unused-vars
async function _renderSchedule_legacy_cron_unused() {
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
    ? '<div class="empty">No scheduled recordings yet. Add one below.</div>'
    : "";

  const listHtml = entries
    .map(
      (e, i) => `
    <div class="task-row">
      <div class="task-info">
        <span class="task-name">${escape(e.channel || "scheduled")}</span>
        <span class="task-cadence"><code>${escape(e.cron || "")}</code>${e.duration ? ` · ${escape(e.duration)}` : ""}${e.next_fire ? ` · next: ${escape(new Date(e.next_fire).toLocaleString())}` : ""}</span>
      </div>
      <button class="sm sch-del" data-i="${i}" title="Delete this schedule entry">✕</button>
    </div>`,
    )
    .join("");

  root.innerHTML = chrome(`
    <h1 class="page-title">Schedule</h1>
    <p class="page-subtitle">Upcoming scheduled recordings · ${dated.length} upcoming</p>
    ${empty}
    <section class="cfg-card">
      <h2 class="cfg-title">Add scheduled recording</h2>
      <form id="sch-add" class="sch-form">
        <label class="sch-field">
          <span>Channel</span>
          <input name="channel" type="text" placeholder="Platform:channel_id (e.g. Twitch:12345)" required />
        </label>
        <label class="sch-field">
          <span>Cron <span class="stg-hint" title="5-field cron: minute hour day-of-month month day-of-week. Example: 0 9 * * 1-5 = 9am weekdays.">ⓘ</span></span>
          <input name="cron" type="text" placeholder="0 9 * * 1-5" required />
        </label>
        <label class="sch-field">
          <span>Duration</span>
          <input name="duration" type="text" placeholder="4h" />
        </label>
        <button class="btn-primary" type="submit">Add</button>
      </form>
    </section>
    <div class="cfg-grid">${groupsHtml}${undatedHtml}</div>
    ${entries.length ? `<section class="cfg-card"><h2 class="cfg-title">All schedule entries</h2>${listHtml}</section>` : ""}
  `);
  setupChromeHandlers();
  document.getElementById("sch-add")?.addEventListener("submit", async (e) => {
    e.preventDefault();
    const fd = new FormData(e.target);
    try {
      await API.scheduleAdd({
        channel: fd.get("channel"),
        cron: fd.get("cron"),
        duration: fd.get("duration"),
      });
      Toast.success("Schedule entry added");
      renderSchedule();
    } catch (err) {
      Toast.error(`Add failed: ${err.message}`);
    }
  });
  document.querySelectorAll(".sch-del").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const i = parseInt(btn.dataset.i, 10);
      if (!confirm("Delete this schedule entry?")) return;
      try {
        await API.scheduleDelete(i);
        Toast.success("Schedule entry removed");
        renderSchedule();
      } catch (err) {
        Toast.error(`Delete failed: ${err.message}`);
      }
    });
  });
}

// ── Durable History (item 17) — completed/failed audit from the jobs DB,
// survives restarts (unlike the in-memory /recordings snapshot). ──
// History page filter / group state — persisted like the Recordings ones.
let histFilter = "";
let histGroupBy = localStorage.getItem("strivo-hist-groupby") || "none"; // "none" | "channel" | "date"
let histStateFilter = new Set(
  (localStorage.getItem("strivo-hist-state-filter") || "")
    .split(",").filter(Boolean),
);
let histCache = [];

async function renderHistory() {
  // Fetch history alongside the live /recordings snapshot so we can
  // overlay file_exists state (audit B4). Without this, History happily
  // reports 'Finished, 9 GB' for files the Recordings page knows are
  // long gone.
  let [hist, recs] = [[], []];
  try {
    const [h, r] = await Promise.all([
      API.history().catch(() => ({ history: [] })),
      API.recordings().catch(() => ({ recordings: [] })),
    ]);
    hist = h.history || [];
    recs = r.recordings || [];
  } catch (_) {}
  const liveById = new Map(recs.map((r) => [r.id, r]));
  histCache = hist.map((row) => {
    const live = liveById.get(row.id);
    if (live && live.file_exists === false) {
      return { ...row, file_exists: false, state: "Failed" };
    }
    return row;
  });
  root.removeAttribute("aria-busy");

  if (histCache.length === 0) {
    root.innerHTML = chrome(`
      <h1 class="page-title">History</h1>
      <div class="empty">
        <div class="glyph">🗂</div>
        No recording history yet. Captures land here automatically.
      </div>
    `);
    setupChromeHandlers();
    return;
  }

  root.innerHTML = chrome(`
    <h1 class="page-title">History</h1>
    <p class="page-subtitle" id="hist-count"></p>
    <div class="rec-toolbar">
      <input id="hist-filter" class="grid-filter" type="search"
             placeholder="Filter by channel or title…"
             aria-label="Filter history" value="${escape(histFilter)}">
      <button id="hist-groupby" class="sm" title="Group rows">
        ${histGroupBy === "channel" ? "▼ Grouped by channel"
          : histGroupBy === "date" ? "▼ Grouped by month"
          : "≣ Group by…"}
      </button>
    </div>
    <div id="hist-state-chips" class="rec-state-chips" role="group" aria-label="Filter by state"></div>
    <div id="hist-list" class="media-list"></div>
  `);
  setupChromeHandlers();
  paintHistChips();
  paintHistory();

  document.getElementById("hist-filter")?.addEventListener("input", (e) => {
    histFilter = e.target.value;
    paintHistory();
  });
  document.getElementById("hist-groupby")?.addEventListener("click", () => {
    histGroupBy = histGroupBy === "none"
      ? "channel"
      : histGroupBy === "channel" ? "date" : "none";
    localStorage.setItem("strivo-hist-groupby", histGroupBy);
    renderHistory().catch((e) => Toast.error(e.message));
  });
}

function paintHistChips() {
  const host = document.getElementById("hist-state-chips");
  if (!host) return;
  const counts = new Map();
  for (const r of histCache) {
    const key = stateClassName(r.state);
    counts.set(key, (counts.get(key) || 0) + 1);
  }
  if (counts.size <= 1) { host.innerHTML = ""; return; }
  const sorted = Array.from(counts.entries()).sort((a, b) => b[1] - a[1]);
  const allActive = histStateFilter.size === 0;
  host.innerHTML = `
    <button class="rec-state-chip rec-state-chip-all ${allActive ? "active" : ""}" type="button">
      <span class="rec-state-chip-dot"></span>All <span class="rec-state-chip-count">${histCache.length}</span>
    </button>
    ${sorted.map(([state, n]) => {
      const active = histStateFilter.size === 0 || histStateFilter.has(state);
      return `<button class="rec-state-chip state-${escape(state)} ${active ? "active" : ""}"
                data-state="${escape(state)}" type="button">
        <span class="rec-state-chip-dot"></span>
        ${escape(stateChipLabel(state))}
        <span class="rec-state-chip-count">${n}</span>
      </button>`;
    }).join("")}`;
  host.querySelector(".rec-state-chip-all")?.addEventListener("click", () => {
    histStateFilter.clear();
    localStorage.setItem("strivo-hist-state-filter", "");
    paintHistChips();
    paintHistory();
  });
  host.querySelectorAll("[data-state]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const s = btn.dataset.state;
      if (histStateFilter.size === 0) histStateFilter = new Set([s]);
      else if (histStateFilter.has(s)) histStateFilter.delete(s);
      else histStateFilter.add(s);
      localStorage.setItem("strivo-hist-state-filter",
        Array.from(histStateFilter).join(","));
      paintHistChips();
      paintHistory();
    });
  });
}

function paintHistory() {
  const host = document.getElementById("hist-list");
  if (!host) return;
  const q = histFilter.trim().toLowerCase();
  const rows = histCache.filter((r) => {
    if (histStateFilter.size > 0 && !histStateFilter.has(stateClassName(r.state))) return false;
    if (!q) return true;
    return (r.channel_name || "").toLowerCase().includes(q)
        || niceTitle(r.stream_title).toLowerCase().includes(q);
  });
  // Newest-first inside each cluster + as the default flat order.
  rows.sort((a, b) => new Date(b.started_at) - new Date(a.started_at));
  const countEl = document.getElementById("hist-count");
  if (countEl) {
    countEl.textContent = (q || histStateFilter.size > 0 || rows.length !== histCache.length)
      ? `${rows.length} of ${histCache.length} entries`
      : `${histCache.length} entries · durable record of every capture (survives restarts)`;
  }

  if (rows.length === 0) {
    host.innerHTML = `<div class="empty"><div class="glyph">🗂</div>No history rows match the current filter.</div>`;
    return;
  }
  let html;
  if (histGroupBy === "channel") {
    const order = [];
    const groups = new Map();
    for (const r of rows) {
      const k = r.channel_name || "(unknown)";
      if (!groups.has(k)) { groups.set(k, []); order.push(k); }
      groups.get(k).push(r);
    }
    html = order.map((ch) => {
      const list = groups.get(ch);
      const totalBytes = list.reduce((a, b) => a + (b.bytes_written || 0), 0);
      return `<div class="hist-group">
        <div class="hist-group-head">
          <span class="rec-group-name">${escape(ch)}</span>
          <span class="rec-group-meta">${list.length} entr${list.length === 1 ? "y" : "ies"} · ${formatBytes(totalBytes)}</span>
        </div>
        ${list.map(historyPillHtml).join("")}
      </div>`;
    }).join("");
  } else if (histGroupBy === "date") {
    const order = [];
    const groups = new Map();
    for (const r of rows) {
      const d = new Date(r.started_at);
      const k = isNaN(d.getTime()) ? "(unknown)"
        : `${d.getFullYear()}-${String(d.getMonth()+1).padStart(2,"0")}`;
      if (!groups.has(k)) { groups.set(k, []); order.push(k); }
      groups.get(k).push(r);
    }
    html = order.map((mo) => {
      const list = groups.get(mo);
      const totalBytes = list.reduce((a, b) => a + (b.bytes_written || 0), 0);
      const niceMonth = mo === "(unknown)" ? mo
        : new Date(mo + "-01").toLocaleString(undefined, { year: "numeric", month: "long" });
      return `<div class="hist-group">
        <div class="hist-group-head">
          <span class="rec-group-name">${escape(niceMonth)}</span>
          <span class="rec-group-meta">${list.length} entr${list.length === 1 ? "y" : "ies"} · ${formatBytes(totalBytes)}</span>
        </div>
        ${list.map(historyPillHtml).join("")}
      </div>`;
    }).join("");
  } else {
    html = rows.map(historyPillHtml).join("");
  }
  host.innerHTML = html;

  // Wire per-row buttons. Reuses the same handlers Recordings table
  // mounts so behaviours stay consistent.
  host.querySelectorAll("[data-action=rec-play]").forEach((btn) => {
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      openRecordingPlayer(btn.dataset.jobId);
    });
  });
  host.querySelectorAll("[data-action=rec-info]").forEach((btn) => {
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      openRecordingInfo(btn.dataset.jobId);
    });
  });
  host.querySelectorAll("[data-action=rec-rescan]").forEach((btn) => {
    btn.addEventListener("click", (e) => { e.stopPropagation(); reScanRecording(btn); });
  });
  host.querySelectorAll("[data-action=rec-locate]").forEach((btn) => {
    btn.addEventListener("click", (e) => { e.stopPropagation(); showRecordingPath(btn.dataset.path); });
  });
  host.querySelectorAll("[data-action=rec-delete]").forEach((btn) => {
    btn.addEventListener("click", async (e) => {
      e.stopPropagation();
      if (!(await confirmDialog("Delete this recording? The file moves to the 7-day trash.", { ok: "Delete", danger: true })))
        return;
      await withBusy(btn, "Deleting…", async () => {
        await API.deleteRecordingFile(btn.dataset.jobId);
        Toast.success("Deleted");
        histCache = histCache.filter((r) => r.id !== btn.dataset.jobId);
        renderHistory().catch(() => {});
      }).catch((err) => Toast.error(`Delete failed: ${err.message}`));
    });
  });
  // Clicking anywhere on the pill (outside buttons) opens the Info
  // modal — same convention as the Recordings table.
  host.querySelectorAll(".media-pill").forEach((pill) => {
    pill.addEventListener("click", (e) => {
      if (e.target.closest("button, input, a")) return;
      const id = pill.dataset.jobId;
      if (id) openRecordingInfo(id);
    });
  });
}

// Action-rich pill used on the History page. Mirrors recordingPillHtml's
// layout but adds the Recordings-page Play/Info/Delete affordance set.
function historyPillHtml(j) {
  const when = j.started_at ? new Date(j.started_at).toLocaleString() : "—";
  const missingOverlay = j.file_exists === false
    ? '<span class="mp-missing">FILE MISSING</span>' : "";
  const sourceBadge = j.source_url
    ? '<span class="mp-source" title="From Twitch/YouTube VOD backfill">VOD</span>' : "";
  const isFinished = stateClassName(j.state) === "finished" && j.file_exists !== false;
  const isFileError = j.file_exists === false;
  const playBtn = isFinished
    ? `<button class="primary sm" data-action="rec-play" data-job-id="${escape(j.id)}" title="Open player">▶ Play</button>`
    : `<button class="primary sm rec-play-disabled" disabled aria-disabled="true" title="${isFileError ? "File missing" : "Not finished"}">▶ Play</button>`;
  const fileErrorBtns = isFileError
    ? `<button class="sm" data-action="rec-rescan" data-job-id="${escape(j.id)}" title="Re-check whether the file exists">↻ Re-scan</button>
       <button class="sm" data-action="rec-locate" data-job-id="${escape(j.id)}" data-path="${escape(j.output_path || "")}" title="Show the expected file path">📂 Show path</button>`
    : "";
  return `
    <div class="media-pill hist-pill${j.file_exists === false ? " mp-broken" : ""}"
         data-job-id="${escape(j.id)}">
      <div class="mp-thumb">${missingOverlay}<img class="mp-thumb-img" loading="lazy" alt=""
        src="/api/v1/recordings/${encodeURIComponent(j.id)}/thumb" onerror="this.remove()"></div>
      <div class="mp-info">
        <div class="mp-title">${escape(niceTitle(j.stream_title) || j.channel_name || "(recording)")} ${sourceBadge}</div>
        <div class="mp-sub">${escape(j.channel_name || "")} · ${escape(when)}</div>
      </div>
      <div class="mp-meta">
        ${(() => { const d = recordingDisplayState(j); return `<span class="state-pill ${d.className}">${escape(d.label)}</span>`; })()}
        <span class="mp-size">${formatBytes(j.bytes_written || 0)}</span>
      </div>
      <div class="hist-actions">
        ${playBtn}
        ${fileErrorBtns}
        <button class="sm" data-action="rec-info" data-job-id="${escape(j.id)}" title="Recording details">ⓘ Info</button>
        <button class="danger sm" data-action="rec-delete" data-job-id="${escape(j.id)}" title="Delete (moves file to 7-day trash)">✕</button>
      </div>
    </div>`;
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
// md_to_html — tiny subset of markdown for Casebook section bodies.
// Handles **bold**, `code`, leading-dash unordered lists, and newlines.
// Not a full markdown parser — Casebook only emits a tight subset.
function md_to_html(text) {
  if (!text) return "";
  const escaped = escape(text);
  // Lists first: turn lines starting with "- " into <ul><li>.
  const lines = escaped.split("\n");
  const out = [];
  let inUl = false;
  for (const raw of lines) {
    if (raw.startsWith("- ")) {
      if (!inUl) {
        out.push("<ul>");
        inUl = true;
      }
      out.push(`<li>${raw.slice(2)}</li>`);
    } else {
      if (inUl) {
        out.push("</ul>");
        inUl = false;
      }
      out.push(raw + "<br/>");
    }
  }
  if (inUl) out.push("</ul>");
  return out
    .join("")
    .replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>")
    .replace(/`([^`]+)`/g, "<code>$1</code>");
}

// niceTitle — strip filename-derived noise from a recording title so the
// UI shows the semantic title only. The on-disk filename is untouched.
//
// Strips:
//   - leading HHMMSS_ timestamp prefix from ffmpeg filename templates
//   - trailing API/source decorations like "_Video_", "[Video]", "_AUDIO_"
//   - underscores standing in for spaces (filesystem-safe substitution)
//   - editorial appendations Patreon/YouTube auto-tag (BONUS Video, etc.)
//   - bracketed/parens descriptors that are non-semantic
// Then collapses double-spaces and trims.
const TITLE_TRAILING_TAGS = [
  // Order matters: most specific (multi-word) first.
  "BONUS Video", "BONUS Audio", "BONUS [Video]", "BONUS [Audio]",
  "Full Episode", "Patreon Exclusive", "Patreon Only", "Members Only",
  "BONUS", "FREE", "EXCLUSIVE", "VOD",
  "_Video_", "[Video]", "_VIDEO_", "[VIDEO]",
  "_Audio_", "[Audio]", "_AUDIO_", "[AUDIO]",
];
function niceTitle(t) {
  if (t == null) return "";
  let s = String(t);
  // 4-6 digit timestamp prefix produced by {date}/{time} in the template.
  s = s.replace(/^\d{4,6}_+/, "");
  // Underscore → space (filename-safe substitution).
  s = s.replace(/_+/g, " ");
  // Strip each known trailing tag, repeatedly, with surrounding punctuation.
  for (let i = 0; i < 4; i++) {
    let before = s;
    for (const tag of TITLE_TRAILING_TAGS) {
      const re = new RegExp(
        "[\\s\\-\\u2013\\u2014:,\\(\\[]*" +
          tag.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") +
          "[\\s\\)\\]]*$",
        "i",
      );
      s = s.replace(re, "");
    }
    if (s === before) break;
  }
  // Collapse double-spaces; tidy stray punctuation tails like " - " " — ".
  s = s.replace(/\s+/g, " ")
       .replace(/[\s\-–—:,]+$/g, "")
       .trim();
  return s;
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

// Refetch caches when the tab regains focus and the rail looks emptied
// out (e.g. after the daemon socket bounced while the tab was idle). This
// is belt-and-braces alongside the Promise.allSettled fan-out above: the
// SSE reconnect handles the live-update channel, but a one-shot fetch is
// the cheapest way to reconcile a partial-fetch render that's already on
// screen. Cheap — the route render itself is idempotent.
document.addEventListener("visibilitychange", () => {
  if (document.hidden) return;
  if (channelCache.length === 0 || recCache.length === 0) {
    render();
  }
});

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

  // Keyboard help: Shift+I (capital I). The earlier `?` binding was
  // collateral-fired by video-element native shortcuts inside the player
  // modal, leaving the help stuck visible behind it.
  if (e.shiftKey && (e.key === "I" || e.key === "i")) {
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

// ── Onboarding tour + per-page hint banners ──────────────────────────
// LocalStorage keys:
//   strivo-tour-done                → seen the welcome walkthrough
//   strivo-hint-<route>             → dismissed the per-page hint
// Hint copy is intentionally one line each — the goal is "I know what
// this surface is for", not full docs.
const PAGE_HINTS = {
  library:    "Live channels in the rail + the current capture dashboard. Click any rail row to see channel detail.",
  recordings: "Every recording past + present. Tick rows to enable bulk actions, click headers to sort, chips filter by state.",
  schedule:   "Per-channel record-when-live + auto-download switches. Capture limits + disk gauge live up top.",
  pipelines:  "Cross-plugin pipelines as DAGs. Click any node to open the plugin, or Run on a recording to fire the chain.",
  watch:      "Tile any subset of currently live channels. Unmute one tile at a time; Shift+I shows shortcuts.",
  chat:       "Twitch IRC over WSS. Tab strip picks a room; filter chips narrow live. BTTV globals + Twitch emotes render as images.",
  plugins:    "Plugin hub. Each card opens the plugin; ⚙ deep-links to the per-plugin Settings panel.",
  settings:   "Live daemon config. Toggles persist to ~/.config/strivo/config.toml on change.",
  system:     "Health checks + storage gauge + platform-auth status + Backup/Restore.",
  logs:       "Rolling daemon log. Toggle Follow for tail mode; Copy / Download exports the filtered view.",
  history:    "Durable per-recording journal that survives daemon restarts.",
};

// Top-bar slots the tour walks. Order matches the natural left-to-right
// flow; each step pins to the corresponding .topnav-link by data-route.
const TOUR_STEPS = [
  { route: "library",    title: "Library",    body: "Your home. Live channel rail on the left; current captures + recent recordings in the centre." },
  { route: "recordings", title: "Recordings", body: "Every past + active recording in a sortable / filterable / groupable table. Bulk actions on selection." },
  { route: "schedule",   title: "Monitor",    body: "Tell StriVo which channels to auto-record + auto-download. Capture limits + disk-budget circuit breaker live here." },
  { route: "watch",      title: "Watch",      body: "Multi-stream viewer with auto-tile, focus, and PiP modes. One tile unmuted at a time." },
  { route: "chat",       title: "Chat",       body: "Twitch IRC client with filter chips, mention highlighting, BTTV global emotes." },
  { route: "pipelines",  title: "Pipelines",  body: "Cross-plugin DAGs. Click a node to open it; 'Run on…' picks a recording + opens the right plugin." },
  { route: "plugins",    title: "Plugins",    body: "The shipped plugin set + marketplace catalog. Click any card to open; gear icon → per-plugin Settings." },
  { route: "settings",   title: "Settings",   body: "All daemon config: Notifications, Platforms, plugin enable/disable, theme, advanced paths." },
];

function tourDone() { return localStorage.getItem("strivo-tour-done") === "1"; }
function markTourDone() { localStorage.setItem("strivo-tour-done", "1"); }
function hintDismissed(route) { return localStorage.getItem(`strivo-hint-${route}`) === "1"; }
function dismissHint(route) { localStorage.setItem(`strivo-hint-${route}`, "1"); }

function startOnboardingTour() {
  if (tourDone()) return;
  let idx = 0;
  const overlay = document.createElement("div");
  overlay.id = "tour-overlay";
  overlay.className = "tour-overlay";
  document.body.appendChild(overlay);

  const paint = () => {
    const step = TOUR_STEPS[idx];
    const target = document.querySelector(`.topnav-link[data-route="${step.route}"]`);
    const rect = target?.getBoundingClientRect();
    const cardLeft = rect ? Math.max(12, Math.min(window.innerWidth - 380, rect.left + rect.width / 2 - 180)) : 24;
    const cardTop = rect ? rect.bottom + 12 : 80;
    overlay.innerHTML = `
      <div class="tour-spotlight" style="${rect ? `left:${rect.left - 6}px;top:${rect.top - 6}px;width:${rect.width + 12}px;height:${rect.height + 12}px;` : "display:none"}"></div>
      <div class="tour-card" style="left:${cardLeft}px;top:${cardTop}px;">
        <div class="tour-step-meta">Step ${idx + 1} of ${TOUR_STEPS.length}</div>
        <h3 class="tour-title">${escape(step.title)}</h3>
        <p class="tour-body">${escape(step.body)}</p>
        <div class="tour-actions">
          <button class="sm tour-skip" type="button">Skip tour</button>
          <span class="spacer"></span>
          ${idx > 0 ? `<button class="sm tour-prev" type="button">← Back</button>` : ""}
          <button class="btn-primary sm tour-next" type="button">
            ${idx === TOUR_STEPS.length - 1 ? "Finish" : "Next →"}
          </button>
        </div>
      </div>`;
    overlay.querySelector(".tour-skip").addEventListener("click", finish);
    overlay.querySelector(".tour-prev")?.addEventListener("click", () => { idx = Math.max(0, idx - 1); paint(); });
    overlay.querySelector(".tour-next").addEventListener("click", () => {
      if (idx >= TOUR_STEPS.length - 1) { finish(); return; }
      idx += 1;
      paint();
    });
  };
  const finish = () => {
    markTourDone();
    overlay.remove();
  };
  paint();
}

// Mount a per-page hint banner above the current route's main content
// IFF the user hasn't dismissed this route's hint yet. Idempotent —
// called after each render() and short-circuits when already mounted.
function maybeMountPageHint(route) {
  if (!route || hintDismissed(route) || !PAGE_HINTS[route]) return;
  if (document.getElementById("page-hint")) return;
  const banner = document.createElement("div");
  banner.id = "page-hint";
  banner.className = "page-hint";
  banner.innerHTML = `
    <span class="page-hint-icon" aria-hidden="true">💡</span>
    <span class="page-hint-text">${escape(PAGE_HINTS[route])}</span>
    <button class="page-hint-dismiss sm" type="button" aria-label="Dismiss this hint">✕</button>`;
  // Insert as the first child of the main chrome region so it sits
  // above any page-specific page-title / subtitle.
  const chrome = document.querySelector(".chrome");
  if (!chrome) return;
  chrome.insertBefore(banner, chrome.children[1] || null);
  banner.querySelector(".page-hint-dismiss").addEventListener("click", () => {
    dismissHint(route);
    banner.remove();
  });
}

function injectKeyboardHelp() {
  if (document.getElementById("kbd-help")) return;
  const div = document.createElement("div");
  div.id = "kbd-help";
  div.className = "kbd-help";
  div.setAttribute("role", "dialog");
  div.setAttribute("aria-label", "Keyboard shortcuts");
  // Click-outside dismiss. Without this the only escape route was Escape,
  // and any video-element shortcut inside the player modal could pop it
  // back open via the legacy `?` binding (now Shift+I).
  div.addEventListener("click", (e) => {
    if (e.target === div) div.classList.remove("open");
  });
  div.innerHTML = `
    <div class="card">
      <h2>Keyboard shortcuts</h2>
      <dl>
        <dt>Shift+I</dt><dd>This help</dd>
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
seedPatreon()
  .finally(render)
  .finally(() => {
    // Fire the welcome tour once per machine — runs after the first
    // paint settles so the topnav slots have their bounding rects.
    setTimeout(startOnboardingTour, 600);
  });
