"use strict";

// ---------------------------------------------------------------------------
// Bridge to the Rust side (or an in-browser demo when opened without Tauri).
// ---------------------------------------------------------------------------

const api = window.__TAURI__
  ? {
      invoke: (cmd, args) => window.__TAURI__.core.invoke(cmd, args),
      listen: (event, cb) => window.__TAURI__.event.listen(event, cb),
    }
  : demoApi();

const OS_LABEL = { windows: "Windows", macos: "macOS", linux: "Linux", other: "Other" };
const EDGE_LABEL = { left: "on the left", right: "on the right", top: "above", bottom: "below" };
const EDGES = ["top", "left", "right", "bottom"];

let state = null;
let info = { backend: "", config_path: "", log_dir: "", version: "" };
let dialogKey = null;
const openRows = new Set();
// An available update ({ version, current, notes }), and how far installing it got:
// null (not started), a percentage, or "working" (size unknown / installing).
let update = null;
let updateProgress = null;
let checkingUpdate = false;
// Windows Firewall: "ok", "blocked", "missing", "unknown" or "n/a".
let firewall = "n/a";
let firewallCheckedAt = 0;

const $ = (sel) => document.querySelector(sel);
const esc = (s) =>
  String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

const screenIcon = (cls = "screen-icon") =>
  `<svg class="${cls}" viewBox="0 0 36 28" aria-hidden="true"><rect class="bezel" x="1.5" y="1.5" width="33" height="20" rx="4"/><rect class="glass" x="5" y="5" width="26" height="13" rx="1.5"/><path class="stand" d="M13 26h10"/></svg>`;

// Replace an element's contents only when they changed. State arrives often
// (every connection change, every screen update from another computer), and
// rebuilding unchanged HTML closes open dropdowns and swallows clicks.
const shownHTML = new WeakMap();
function setHTML(el, html) {
  if (shownHTML.get(el) === html) return;
  shownHTML.set(el, html);
  el.innerHTML = html;
}

function peerName(id) {
  return state.peers.find((p) => p.id === id)?.name ?? "another computer";
}

function call(cmd, args) {
  return api.invoke(cmd, args).catch((e) => toast(String(e), "error"));
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

function render() {
  if (!state) return;
  renderTop();
  renderBanners();
  renderLayout();
  renderDevices();
  renderSettings();
  renderDialog();
}

function renderTop() {
  const name = $("#my-name");
  if (document.activeElement !== name) name.value = state.me.name;
  $("#my-os").textContent = OS_LABEL[state.me.os] ?? state.me.os;
  $("#enabled").checked = state.settings.enabled;

  const fs = $("#focus-status");
  const f = state.focus;
  fs.classList.toggle("remote", f.kind !== "local");
  if (!state.settings.enabled) fs.innerHTML = `Paused`;
  else if (f.kind === "controlling") fs.innerHTML = `<span class="dot"></span>Controlling ${esc(peerName(f.peer))}`;
  else if (f.kind === "controlled_by") fs.innerHTML = `<span class="dot"></span>Controlled from ${esc(peerName(f.peer))}`;
  else fs.innerHTML = `<span class="dot"></span>Mouse is here`;
}

function statusBanner(status, what) {
  if (!status || status.state === "ok") return "";
  const kind = status.state === "error" ? "error" : "warn";
  const title = {
    needs_permission: `Permission needed to ${what}`,
    unsupported: `This computer can't ${what}`,
    error: `Skerry can't ${what}`,
  }[status.state];
  const needsInputMonitoring = (status.detail ?? "").includes("Input Monitoring");
  const action =
    status.state === "needs_permission" && state.me.os === "macos"
      ? `<div class="actions">
           <button class="btn small secondary" data-action="open-permissions" data-pane="accessibility">Accessibility</button>
           ${needsInputMonitoring ? `<button class="btn small secondary" data-action="open-permissions" data-pane="input_monitoring">Input Monitoring</button>` : ""}
           <button class="btn small" data-action="reset-permissions">Reset permissions</button>
         </div>`
      : "";
  return `<div class="banner ${kind}"><div class="text"><strong>${esc(title)}</strong>${esc(status.detail ?? "")}</div>${action}</div>`;
}

function updateBanner() {
  if (!update) return "";
  let progress = "";
  if (updateProgress === "working") progress = `<progress aria-label="Updating"></progress>`;
  else if (updateProgress !== null) progress = `<progress max="100" value="${updateProgress}" aria-label="Downloading the update"></progress>`;
  const mac = state.me.os === "macos" ? " If macOS asks for Skerry's permissions again afterwards, allow Skerry again." : "";
  return `<div class="banner info"><div class="text"><strong>Skerry ${esc(update.version)} is available</strong>You have ${esc(state.me.version)}. Updating keeps your pairings and settings, and Skerry restarts by itself.${esc(mac)}</div>
    <div class="actions">${progress}<button class="btn small" data-action="install-update" ${updateProgress !== null ? "disabled" : ""}>${updateProgress !== null ? "Updating…" : "Update and restart"}</button></div></div>`;
}

function renderBanners() {
  let html = updateBanner();
  html += statusBanner(state.capture, "share its mouse and keyboard");
  if (JSON.stringify(state.emulation) !== JSON.stringify(state.capture)) {
    html += statusBanner(state.emulation, "be controlled by other computers");
  }
  if (state.listen_error) {
    html += `<div class="banner error"><div class="text"><strong>Other computers can't connect</strong>${esc(state.listen_error)}</div></div>`;
  }
  if (firewall === "blocked" || firewall === "missing") {
    const why =
      firewall === "blocked"
        ? "A Windows Firewall rule blocks Skerry, so other computers can't connect to this one (this computer can still connect to them)."
        : "No Windows Firewall rule allows Skerry yet, so other computers may not be able to connect to this one.";
    html += `<div class="banner warn"><div class="text"><strong>Windows Firewall is blocking Skerry</strong>${esc(why)}</div>
      <div class="actions"><button class="btn small" data-action="fix-firewall">Allow Skerry</button></div></div>`;
  }
  if (state.me.os === "macos" && state.peers.some((p) => (p.last_error ?? "").includes("Local Network"))) {
    html += `<div class="banner warn"><div class="text"><strong>macOS may be blocking Skerry's connections</strong>Switch Skerry on in System Settings → Privacy & Security → Local Network. If it's already on, switch it off and on again, then click Scan network. Other computers can still connect to this Mac in the meantime.</div>
      <div class="actions"><button class="btn small secondary" data-action="open-permissions" data-pane="local_network">Local Network settings</button></div></div>`;
  }
  $("#banners").innerHTML = html;
}

function tile(peer, { removable = false, active = false } = {}) {
  const online = peer.online;
  return `<div class="tile ${online ? "" : "offline"} ${active ? "active" : ""}" draggable="true" data-id="${esc(peer.id)}">
      ${removable ? `<button class="remove" data-action="unplace" data-id="${esc(peer.id)}" title="Remove from this side" aria-label="Remove ${esc(peer.name)} from this side">×</button>` : ""}
      ${active ? `<span class="tile-tag">In control</span>` : ""}
      ${screenIcon()}
      <div class="tile-name">${esc(peer.name)}</div>
      <div class="tile-meta"><span class="status-dot ${online ? "on" : ""}"></span>${online ? (peer.available ? "Ready" : "Paused") : "Offline"} · ${esc(OS_LABEL[peer.os] ?? "")}</div>
    </div>`;
}

function renderLayout() {
  const f = state.focus;
  const activeId = f.kind === "controlling" ? f.peer : null;
  const ips = state.me.ips ?? [];
  const ip = ips.length
    ? `<button class="tile-ip" data-action="copy-ip" data-ip="${esc(ips[0])}" title="${esc(ips.length > 1 ? `Also: ${ips.slice(1).join(", ")}. ` : "")}Port ${esc(state.me.port)}. Click to copy.">Your IP is ${esc(ips[0])}</button>`
    : "";
  setHTML(
    $("#this-computer"),
    `<div class="tile">
      <span class="badge-here">This computer</span>
      ${screenIcon()}
      <div class="tile-name">${esc(state.me.name)}</div>
      <div class="tile-meta">${esc(OS_LABEL[state.me.os] ?? "")}</div>
      ${ip}
    </div>`,
  );

  $("#this-computer").classList.toggle("here", state.settings.enabled && f.kind === "local");

  const paired = state.peers.filter((p) => p.paired);
  const unplaced = paired.filter((p) => !p.edge);
  for (const edge of EDGES) {
    const slot = document.querySelector(`.slot[data-edge="${edge}"]`);
    const id = state.layout[edge];
    const peer = id && paired.find((p) => p.id === id);
    // Leave a side alone while its "Place…" menu is open.
    if (slot.contains(document.activeElement) && document.activeElement.tagName === "SELECT") continue;
    slot.classList.toggle("filled", !!peer);
    slot.classList.toggle("online", !!peer?.online);
    slot.classList.toggle("active", !!peer && peer.id === activeId);
    if (peer) {
      setHTML(slot, tile(peer, { removable: true, active: peer.id === activeId }));
    } else {
      const options = unplaced.map((p) => `<option value="${esc(p.id)}">${esc(p.name)}</option>`).join("");
      setHTML(slot, `<div class="slot-empty">
          <span>${paired.length ? "Drop a computer here" : "Pair a computer first"}</span>
          ${unplaced.length ? `<select data-edge="${edge}" aria-label="Place a computer ${EDGE_LABEL[edge]}"><option value="">Place…</option>${options}</select>` : ""}
        </div>`);
    }
  }
}

function renderDevices() {
  const paired = state.peers.filter((p) => p.paired);
  const nearby = state.peers.filter((p) => !p.paired);

  setHTML($("#paired-list"), paired.length
    ? paired
        .map((p) => {
          const open = openRows.has(p.id);
          const where = p.edge ? ` · ${EDGE_LABEL[p.edge]}` : "";
          return `<div class="device ${open ? "open" : ""} ${p.online ? "online" : ""}" draggable="true" data-id="${esc(p.id)}">
            ${screenIcon()}
            <div class="name">${esc(p.name)}</div>
            <div class="meta"><span class="status-dot ${p.online ? "on" : ""}"></span>${p.online ? "Online" : "Offline"} · ${esc(OS_LABEL[p.os] ?? "")}${esc(where)}</div>
            <div class="actions"><button class="icon-btn" data-action="toggle-row" data-id="${esc(p.id)}" aria-expanded="${open}" aria-label="Details for ${esc(p.name)}">${open ? "▴" : "▾"}</button></div>
            ${!p.online && p.last_error ? `<div class="problem">${esc(p.last_error)}</div>` : ""}
            <div class="extra">
              ${p.addr ? `<span>Address <code>${esc(p.addr)}</code></span>` : ""}
              ${p.fingerprint ? `<span>Key <code>${esc(p.fingerprint)}</code></span>` : ""}
              <label class="speed">Pointer speed <input type="range" min="0.25" max="3" step="0.05" value="${p.speed}" data-action="speed" data-id="${esc(p.id)}"><span>${p.speed.toFixed(2)}×</span></label>
              <span class="spacer"></span>
              <button class="btn danger small" data-action="forget" data-id="${esc(p.id)}">Forget</button>
            </div>
          </div>`;
        })
        .join("")
    : `<div class="empty">No paired computers yet. Install Skerry on your other computers and connect them to the same network. They'll appear under Nearby.</div>`);

  setHTML($("#nearby-list"), nearby.length
    ? nearby
        .map(
          (p) => `<div class="device">
            ${screenIcon()}
            <div class="name">${esc(p.name)}</div>
            <div class="meta">${esc(OS_LABEL[p.os] ?? "")}${p.addr ? ` · ${esc(p.addr)}` : ""}</div>
            <div class="actions"><button class="btn small" data-action="pair-device" data-id="${esc(p.id)}">Pair</button></div>
          </div>`,
        )
        .join("")
    : state.scanning
      ? `<div class="empty">Scanning this network for computers running Skerry…</div>`
      : `<div class="empty">Looking for other computers running Skerry on this network. If one doesn't show up, make sure Skerry is running on it and press <strong>Scan network</strong>, or pair by its address below.</div>`);

  const scan = $("#scan");
  scan.disabled = state.scanning;
  scan.innerHTML = state.scanning ? `<span class="mini-spinner" aria-hidden="true"></span>Scanning…` : "Scan network";
}

function renderSettings() {
  for (const input of document.querySelectorAll("[data-setting]")) {
    input.checked = !!state.settings[input.dataset.setting];
  }
  const actionName = (a) =>
    ({ left: "Go to the computer on the left", right: "Go to the computer on the right", top: "Go to the computer above", bottom: "Go to the computer below", home: "Bring the mouse back here" })[a] ?? a;
  $("#hotkeys").innerHTML = state.hotkeys
    .map((h) => `<li><span>${esc(actionName(h.action))}</span><kbd>${esc(h.keys.split("+").map((k) => k[0].toUpperCase() + k.slice(1)).join(" + "))}</kbd></li>`)
    .join("");
  $("#manual-list").innerHTML = state.manual_peers
    .map((a) => `<li><code>${esc(a)}</code><button class="icon-btn" data-action="remove-manual" data-addr="${esc(a)}" aria-label="Remove ${esc(a)}">Remove</button></li>`)
    .join("");
  $("#about").innerHTML = `
    <dt>Key</dt><dd><code>${esc(state.me.fingerprint)}</code></dd>
    <dt>Device id</dt><dd><code>${esc(state.me.id)}</code></dd>
    <dt>IP address</dt><dd>${(state.me.ips ?? []).length ? state.me.ips.map((ip) => `<code>${esc(ip)}</code>`).join(" ") : "Not connected to a network"}</dd>
    <dt>Port</dt><dd>${esc(state.me.port)}</dd>
    <dt>Input</dt><dd>${esc(info.backend)}</dd>
    <dt>Version</dt><dd>${esc(state.me.version)}</dd>
    <dt>Settings file</dt><dd><code>${esc(info.config_path)}</code></dd>
    ${info.log_dir ? `<dt>Logs</dt><dd><code>${esc(info.log_dir)}</code></dd>` : ""}`;
}

function renderDialog() {
  const dlg = $("#pair-dialog");
  const p = state.pairings[0];
  if (!p) {
    dialogKey = null;
    if (dlg.open) dlg.close();
    return;
  }
  const key = `${p.session}:${p.stage}`;
  if (key === dialogKey && dlg.open) return;
  dialogKey = key;

  const name = esc(p.peer_name);
  const fp = p.peer_fingerprint ? `<p class="fingerprint">${name}'s key: <code>${esc(p.peer_fingerprint)}</code></p>` : "";
  const cancel = `<button type="button" class="btn secondary" data-action="cancel-pairing" data-session="${p.session}">Cancel</button>`;
  let title, body, actions;
  switch (p.stage) {
    case "show_code":
      title = `Pair with ${name}?`;
      body = `<p>Enter this code on <strong>${name}</strong> to finish pairing:</p>
              <div class="code" aria-label="Pairing code">${esc(p.code.slice(0, 3))} ${esc(p.code.slice(3))}</div>${fp}
              <p class="small">If you didn't start this, press Cancel.</p>`;
      actions = cancel;
      break;
    case "enter_code":
      title = "Enter the pairing code";
      body = `<p>Type the 6-digit code shown on <strong>${name}</strong>.</p>
              <input class="code-input" id="code-input" inputmode="numeric" autocomplete="one-time-code" maxlength="7" placeholder="000 000" aria-label="Pairing code">${fp}`;
      actions = `${cancel}<button type="submit" class="btn" data-session="${p.session}">Pair</button>`;
      break;
    case "verifying":
      title = "Checking the code…";
      body = `<div class="spinner"></div>`;
      actions = cancel;
      break;
    default:
      title = `Connecting to ${name}…`;
      body = `<div class="spinner"></div><p class="small">Make sure Skerry is running on the other computer.</p>`;
      actions = cancel;
  }
  $("#pair-title").innerHTML = title;
  $("#pair-body").innerHTML = body;
  $("#pair-actions").innerHTML = actions;
  $("#pair-form").dataset.session = p.session;
  if (!dlg.open) dlg.showModal();
  $("#code-input")?.focus();
}

// ---------------------------------------------------------------------------
// Updates, firewall and diagnostics
// ---------------------------------------------------------------------------

async function checkForUpdates(manual) {
  if (checkingUpdate) {
    if (manual) toast("Already checking for updates…", "ok");
    return;
  }
  checkingUpdate = true;
  try {
    const found = await api.invoke("check_update");
    if (found) {
      update = found;
      renderBanners();
      if (manual) toast(`Skerry ${found.version} is available.`, "ok");
    } else {
      if (update && updateProgress === null) {
        update = null;
        renderBanners();
      }
      if (manual) toast(`No updates: you have the latest version of Skerry (${state.me.version}).`, "ok");
    }
  } catch (e) {
    if (manual) toast(String(e), "error");
  } finally {
    checkingUpdate = false;
  }
}

async function installUpdate() {
  updateProgress = 0;
  renderBanners();
  try {
    // Skerry restarts when this succeeds.
    await api.invoke("install_update");
  } catch (e) {
    updateProgress = null;
    renderBanners();
    toast(String(e), "error");
  }
}

function onUpdateProgress({ downloaded, total }) {
  const next = total && downloaded < total ? Math.floor((downloaded * 100) / total) : "working";
  if (next !== updateProgress) {
    updateProgress = next;
    renderBanners();
  }
}

async function resetPermissions() {
  try {
    await api.invoke("reset_permissions");
    toast("Permissions reset. Allow Skerry when macOS asks (or switch it on under Accessibility). If this message stays afterwards, quit and reopen Skerry.", "ok");
  } catch (e) {
    toast(String(e), "error");
  }
}

// The id the Forget dialog is asking about. (The webview's built-in confirm()
// box doesn't appear on macOS, so Skerry asks with its own dialog.)
let forgetId = null;

function showForget(id) {
  forgetId = id;
  $("#forget-title").textContent = `Forget ${peerName(id)}?`;
  $("#forget-dialog").showModal();
}

async function copyText(text, done) {
  try {
    await navigator.clipboard.writeText(text);
    toast(done, "ok");
  } catch {
    toast(`Couldn't copy. The address is ${text}.`, "error");
  }
}

async function showUninstall() {
  $("#uninstall-items").innerHTML = "<li>Checking what to remove…</li>";
  $("#uninstall-paths").innerHTML = "";
  $("#uninstall-notes").innerHTML = "";
  const confirmBtn = $('[data-action="confirm-uninstall"]');
  confirmBtn.disabled = true;
  $("#uninstall-dialog").showModal();
  try {
    const s = await api.invoke("uninstall_plan");
    $("#uninstall-items").innerHTML = s.items.map((i) => `<li>${esc(i)}</li>`).join("");
    $("#uninstall-paths").innerHTML = s.paths.map((p) => `<li><code>${esc(p)}</code></li>`).join("");
    $("#uninstall-notes").innerHTML = s.notes.map((n) => `<p class="small">${esc(n)}</p>`).join("");
    confirmBtn.disabled = false;
  } catch (e) {
    $("#uninstall-dialog").close();
    toast(String(e), "error");
  }
}

async function confirmUninstall(btn) {
  btn.disabled = true;
  btn.textContent = "Uninstalling…";
  try {
    // Skerry quits once this succeeds.
    await api.invoke("uninstall");
  } catch (e) {
    btn.disabled = false;
    btn.textContent = "Uninstall Skerry";
    toast(String(e), "error");
  }
}

async function refreshFirewall() {
  if (state?.me.os !== "windows" || Date.now() - firewallCheckedAt < 60_000) return;
  firewallCheckedAt = Date.now();
  try {
    firewall = await api.invoke("firewall_status");
  } catch {
    firewall = "unknown";
  }
  renderBanners();
}

async function fixFirewall() {
  try {
    firewall = await api.invoke("fix_firewall");
    toast(firewall === "ok" ? "Windows Firewall now lets other computers connect to Skerry." : "The firewall rule was added, but Windows still reports a problem.", firewall === "ok" ? "ok" : "error");
  } catch (e) {
    toast(String(e), "error");
  }
  firewallCheckedAt = Date.now();
  renderBanners();
}

async function showDiagnostics() {
  const text = $("#diag-text");
  text.value = "Collecting…";
  $("#diag-dialog").showModal();
  try {
    text.value = await api.invoke("get_diagnostics");
  } catch (e) {
    text.value = String(e);
  }
}

async function copyDiagnostics() {
  const text = $("#diag-text");
  try {
    await navigator.clipboard.writeText(text.value);
  } catch {
    text.focus();
    text.select();
    document.execCommand("copy");
  }
  toast("Copied. Paste it into your bug report or message.", "ok");
}

function toast(message, kind = "") {
  const el = document.createElement("div");
  el.className = `toast ${kind}`;
  el.textContent = message;
  $("#toasts").appendChild(el);
  setTimeout(() => el.remove(), kind === "error" ? 7000 : 4500);
}

// ---------------------------------------------------------------------------
// Sections and theme
// ---------------------------------------------------------------------------

const VIEWS = {
  desk: ["Desk", "Drag a computer to the side of this screen where it sits on your desk. Then move the mouse off that edge."],
  settings: ["Settings", ""],
  help: ["Help", "Hotkeys, addresses and what to do when something isn't working."],
};

function showView(view) {
  if (!VIEWS[view]) view = "desk";
  for (const el of document.querySelectorAll(".view")) el.hidden = el.dataset.view !== view;
  for (const el of document.querySelectorAll(".nav-item")) {
    if (el.dataset.view === view) el.setAttribute("aria-current", "page");
    else el.removeAttribute("aria-current");
  }
  $("#page-title").textContent = VIEWS[view][0];
  $("#page-sub").textContent = VIEWS[view][1];
  $(".sheet").scrollTop = 0;
}

// "system", "light" or "dark". theme.js applies the saved one before the page
// paints; this also tells the window, so its title bar matches.
function setTheme(theme, save) {
  document.documentElement.dataset.theme = theme;
  for (const r of document.querySelectorAll('input[name="theme"]')) r.checked = r.value === theme;
  if (save) {
    try {
      localStorage.setItem("skerry-theme", theme);
    } catch {
      // Not saved; it still applies until Skerry restarts.
    }
  }
  try {
    window.__TAURI__?.window?.getCurrentWindow().setTheme(theme === "system" ? null : theme).catch(() => {});
  } catch {
    // Older webviews: the page still follows the setting.
  }
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

function onEngineEvent(ev) {
  if (ev.type === "state") {
    const { type, ...snapshot } = ev;
    state = snapshot;
    render();
  } else if (ev.type === "pairing_finished") {
    toast(ev.message, ev.ok ? "ok" : "error");
  } else if (ev.type === "notice") {
    toast(ev.message);
  }
}

function wire() {
  for (const el of document.querySelectorAll(".nav-item")) el.addEventListener("click", () => showView(el.dataset.view));
  for (const r of document.querySelectorAll('input[name="theme"]')) r.addEventListener("change", () => setTheme(r.value, true));
  setTheme(document.documentElement.dataset.theme || "system", false);

  $("#my-name").addEventListener("change", (e) => {
    const name = e.target.value.trim();
    if (name) call("update_settings", { settings: { name } });
  });
  $("#my-name").addEventListener("keydown", (e) => e.key === "Enter" && e.target.blur());
  $("#enabled").addEventListener("change", (e) => call("update_settings", { settings: { enabled: e.target.checked } }));

  for (const input of document.querySelectorAll("[data-setting]")) {
    input.addEventListener("change", () => call("update_settings", { settings: { [input.dataset.setting]: input.checked } }));
  }
  $("#autostart").addEventListener("change", (e) => call("set_autostart", { enabled: e.target.checked }));

  $("#pair-address").addEventListener("submit", async (e) => {
    e.preventDefault();
    const value = $("#address").value.trim();
    if (!value) return;
    const ok = await call("pair", { target: { kind: "address", value } });
    if (ok !== undefined) $("#address").value = "";
  });

  $("#manual-form").addEventListener("submit", (e) => {
    e.preventDefault();
    const addr = $("#manual-addr").value.trim();
    if (addr) call("add_manual_peer", { addr });
    $("#manual-addr").value = "";
  });

  $("#pair-form").addEventListener("submit", (e) => {
    e.preventDefault();
    const input = $("#code-input");
    if (!input) return;
    const code = input.value.replace(/\D/g, "");
    if (code.length !== 6) {
      toast("The code has 6 digits.", "error");
      return;
    }
    call("submit_code", { session: Number(e.target.dataset.session), code });
  });
  $("#pair-dialog").addEventListener("cancel", (e) => {
    e.preventDefault();
    const s = Number($("#pair-form").dataset.session);
    if (s) call("cancel_pairing", { session: s });
  });

  document.addEventListener("click", (e) => {
    const el = e.target.closest("[data-action]");
    if (!el) return;
    const { action, id } = el.dataset;
    if (action === "pair-device") call("pair", { target: { kind: "device", value: id } });
    else if (action === "forget") showForget(id);
    else if (action === "confirm-forget") {
      $("#forget-dialog").close();
      call("forget", { id: forgetId });
    } else if (action === "copy-ip") copyText(el.dataset.ip, "IP address copied.");
    else if (action === "toggle-row") {
      openRows.has(id) ? openRows.delete(id) : openRows.add(id);
      renderDevices();
    } else if (action === "unplace") {
      const edge = EDGES.find((e) => state.layout[e] === id);
      if (edge) call("set_layout", { edge, peer: null });
    } else if (action === "cancel-pairing") call("cancel_pairing", { session: Number(el.dataset.session) });
    else if (action === "remove-manual") call("remove_manual_peer", { addr: el.dataset.addr });
    else if (action === "open-permissions") call("open_permission_settings", { pane: el.dataset.pane ?? null });
    else if (action === "install-update") installUpdate();
    else if (action === "check-update") checkForUpdates(true);
    else if (action === "fix-firewall") fixFirewall();
    else if (action === "reset-permissions") resetPermissions();
    else if (action === "uninstall") showUninstall();
    else if (action === "confirm-uninstall") confirmUninstall(el);
    else if (action === "diagnostics") showDiagnostics();
    else if (action === "copy-diagnostics") copyDiagnostics();
    else if (action === "open-logs") call("open_logs");
  });
  $("#scan").addEventListener("click", () => call("rescan"));
  window.addEventListener("focus", refreshFirewall);

  document.addEventListener("change", (e) => {
    const el = e.target;
    if (el.matches("select[data-edge]") && el.value) call("set_layout", { edge: el.dataset.edge, peer: el.value });
    if (el.matches('input[data-action="speed"]')) call("set_speed", { id: el.dataset.id, speed: Number(el.value) });
  });
  document.addEventListener("input", (e) => {
    const el = e.target;
    if (el.matches('input[data-action="speed"]')) el.nextElementSibling.textContent = `${Number(el.value).toFixed(2)}×`;
  });

  // Drag a computer (from the list or a tile) onto a side of this screen.
  document.addEventListener("dragstart", (e) => {
    const el = e.target.closest("[draggable][data-id]");
    if (el) {
      e.dataTransfer.setData("text/plain", el.dataset.id);
      e.dataTransfer.effectAllowed = "move";
    }
  });
  for (const slot of document.querySelectorAll(".slot")) {
    slot.addEventListener("dragover", (e) => {
      e.preventDefault();
      slot.classList.add("drop-target");
    });
    slot.addEventListener("dragleave", () => slot.classList.remove("drop-target"));
    slot.addEventListener("drop", (e) => {
      e.preventDefault();
      slot.classList.remove("drop-target");
      const id = e.dataTransfer.getData("text/plain");
      if (id && state.peers.some((p) => p.id === id && p.paired)) call("set_layout", { edge: slot.dataset.edge, peer: id });
    });
  }
}

async function main() {
  wire();
  [state, info] = await Promise.all([api.invoke("get_state"), api.invoke("get_info")]);
  render();
  await api.listen("engine", (e) => onEngineEvent(e.payload));
  await api.listen("update", (e) => {
    update = e.payload;
    renderBanners();
  });
  await api.listen("update-progress", (e) => onUpdateProgress(e.payload));
  await api.listen("menu-check-update", () => checkForUpdates(true));
  api.invoke("get_autostart").then((v) => ($("#autostart").checked = !!v));
  refreshFirewall();
}

main();

// ---------------------------------------------------------------------------
// Demo mode: lets the UI run in a plain browser for development.
// ---------------------------------------------------------------------------

function demoApi() {
  const listeners = [];
  const demo = {
    me: { id: "1c843bbbadba346f", name: "Studio Desktop", os: new URLSearchParams(location.search).has("macperm") ? "macos" : "linux", fingerprint: "1c84-3bbb-adba-346f-1f47", port: 24870, version: "1.1.0", ips: ["192.168.1.20"] },
    settings: { enabled: true, clipboard_sync: true, swap_cmd_ctrl: true, edge_switching: true, block_switch_while_dragging: true, check_updates: true },
    layout: { left: "a1", right: "b2", top: null, bottom: null },
    peers: [
      { id: "a1", name: "MacBook Air", os: "macos", paired: true, online: true, available: true, discovered: true, addr: "192.168.1.31:24870", fingerprint: "9f3a-77c1-02be-d5e0-11aa", edge: "left", speed: 1.4, version: "1.0.0" },
      { id: "b2", name: "Gaming PC", os: "windows", paired: true, online: true, available: true, discovered: true, addr: "192.168.1.40:24870", fingerprint: "3b10-6d2e-8a44-c901-7f2b", edge: "right", speed: 1, version: "1.0.0" },
      { id: "c3", name: "Office Laptop", os: "windows", paired: true, online: false, available: false, discovered: false, addr: "192.168.1.52:24870", fingerprint: "77de-a012-55c3-09ab-e4f1", edge: null, speed: 1, version: null, last_error: "Couldn't reach it (no answer). Make sure Skerry is running on that computer and that its firewall allows Skerry (Windows: Settings → Windows Security → Firewall → Allow an app)." },
      { id: "d4", name: "living-room-nuc", os: "linux", paired: false, online: false, available: false, discovered: true, addr: "192.168.1.60:24870", fingerprint: null, edge: null, speed: 1, version: "1.0.0" },
    ],
    focus: new URLSearchParams(location.search).has("controlling") ? { kind: "controlling", peer: "b2" } : { kind: "local" },
    capture: new URLSearchParams(location.search).has("macperm")
      ? { state: "needs_permission", detail: "Switch Skerry on in System Settings → Privacy & Security → Accessibility. If it's already switched on there, macOS is remembering an older copy of Skerry: click Reset permissions, then allow Skerry again when macOS asks." }
      : { state: "ok" },
    emulation: { state: "ok" },
    pairings: [],
    hotkeys: [
      { keys: "ctrl+alt+shift+left", action: "left" },
      { keys: "ctrl+alt+shift+right", action: "right" },
      { keys: "ctrl+alt+shift+up", action: "top" },
      { keys: "ctrl+alt+shift+down", action: "bottom" },
      { keys: "ctrl+alt+shift+escape", action: "home" },
    ],
    manual_peers: [],
    listen_error: null,
    scanning: false,
  };
  if (new URLSearchParams(location.search).has("pairing")) {
    demo.pairings = [{ session: 7, peer_name: "living-room-nuc", peer_fingerprint: "5e21-0c9d-4b7a-a3f0-6612", stage: "show_code", code: "684858" }];
  }
  const emit = () => listeners.forEach((cb) => cb({ payload: { type: "state", ...structuredClone(demo) } }));
  return {
    async invoke(cmd, args = {}) {
      switch (cmd) {
        case "get_state": return structuredClone(demo);
        case "get_info": return { backend: "Demo", config_path: "~/.config/skerry/config.toml", log_dir: "~/.config/skerry/logs", version: "1.1.0" };
        case "rescan":
          demo.scanning = true;
          setTimeout(() => { demo.scanning = false; emit(); }, 2500);
          break;
        case "check_update": return new URLSearchParams(location.search).has("update") ? { version: "1.2.0", current: "1.1.0", notes: null } : null;
        case "install_update": throw "Updates can't be installed in the demo.";
        case "firewall_status": return "n/a";
        case "get_diagnostics": return "Skerry 1.1.0 on linux x86_64 (input: Demo)\nThis computer: Studio Desktop\n";
        case "open_logs": case "open_permission_settings": case "fix_firewall": case "reset_permissions": return null;
        case "uninstall_plan":
          return {
            items: ["The Skerry package “skerry” (deb); Linux asks for your password", "Your Skerry settings, pairings and this computer's Skerry key", "Skerry's logs, saved window data and caches", "Skerry's “Start at login” entry"],
            paths: ["~/.config/skerry", "~/.local/share/org.skerry.app", "~/.cache/org.skerry.app", "~/.config/autostart/Skerry.desktop"],
            notes: ["Your other computers keep this one in their list until you click Forget on them."],
          };
        case "uninstall": throw "Uninstalling isn't possible in the demo.";
        case "get_autostart": return true;
        case "update_settings":
          for (const [k, v] of Object.entries(args.settings)) k === "name" ? (demo.me.name = v) : (demo.settings[k] = v);
          break;
        case "set_layout": {
          for (const e of EDGES) if (demo.layout[e] === args.peer) demo.layout[e] = null;
          demo.layout[args.edge] = args.peer;
          demo.peers.forEach((p) => (p.edge = EDGES.find((e) => demo.layout[e] === p.id) ?? null));
          break;
        }
        case "pair": {
          const p = demo.peers.find((x) => x.id === args.target.value);
          demo.pairings = [{ session: 9, peer_name: p ? p.name : args.target.value, peer_fingerprint: null, stage: "enter_code", code: null }];
          break;
        }
        case "submit_code": {
          demo.pairings = [];
          const p = demo.peers.find((x) => !x.paired);
          if (p) Object.assign(p, { paired: true, online: true, available: true, fingerprint: "5e21-0c9d-4b7a-a3f0-6612" });
          setTimeout(() => listeners.forEach((cb) => cb({ payload: { type: "pairing_finished", session: 9, ok: true, message: `Paired with ${p ? p.name : "computer"}.` } })), 0);
          break;
        }
        case "cancel_pairing": demo.pairings = []; break;
        case "forget": demo.peers = demo.peers.filter((p) => p.id !== args.id); break;
        case "set_speed": demo.peers.find((p) => p.id === args.id).speed = args.speed; break;
        case "add_manual_peer": demo.manual_peers.push(args.addr); break;
        case "remove_manual_peer": demo.manual_peers = demo.manual_peers.filter((a) => a !== args.addr); break;
      }
      setTimeout(emit, 0);
    },
    async listen(event, cb) {
      if (event === "engine") listeners.push(cb);
    },
  };
}
