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
let info = { backend: "", config_path: "" };
let dialogKey = null;
const openRows = new Set();

const $ = (sel) => document.querySelector(sel);
const esc = (s) =>
  String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

const screenIcon = (cls = "screen-icon") =>
  `<svg class="${cls}" viewBox="0 0 34 26" aria-hidden="true"><rect x="1.5" y="1.5" width="31" height="19" rx="3" fill="none" stroke="currentColor" stroke-width="2.4"/><path d="M12 24.5h10" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"/></svg>`;

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
  const action =
    status.state === "needs_permission" && state.me.os === "macos"
      ? `<button class="btn small secondary" data-action="open-permissions">Open Privacy settings</button>`
      : "";
  return `<div class="banner ${kind}"><div class="text"><strong>${esc(title)}</strong>${esc(status.detail ?? "")}</div>${action}</div>`;
}

function renderBanners() {
  let html = "";
  html += statusBanner(state.capture, "share its mouse and keyboard");
  if (JSON.stringify(state.emulation) !== JSON.stringify(state.capture)) {
    html += statusBanner(state.emulation, "be controlled by other computers");
  }
  if (state.listen_error) {
    html += `<div class="banner error"><div class="text"><strong>Other computers can't connect</strong>${esc(state.listen_error)}</div></div>`;
  }
  $("#banners").innerHTML = html;
}

function tile(peer, { removable = false, active = false } = {}) {
  const online = peer.online;
  return `<div class="tile ${online ? "" : "offline"} ${active ? "active" : ""}" draggable="true" data-id="${esc(peer.id)}">
      ${removable ? `<button class="remove" data-action="unplace" data-id="${esc(peer.id)}" title="Remove from this side" aria-label="Remove ${esc(peer.name)} from this side">×</button>` : ""}
      ${screenIcon()}
      <div class="tile-name">${esc(peer.name)}</div>
      <div class="tile-meta"><span class="status-dot ${online ? "on" : ""}"></span>${online ? (peer.available ? "Ready" : "Paused") : "Offline"} · ${esc(OS_LABEL[peer.os] ?? "")}</div>
    </div>`;
}

function renderLayout() {
  const f = state.focus;
  const activeId = f.kind === "controlling" ? f.peer : null;
  $("#this-computer").innerHTML = `<div class="tile ${f.kind === "local" ? "" : ""}">
      <span class="badge-here">This computer</span>
      ${screenIcon()}
      <div class="tile-name">${esc(state.me.name)}</div>
      <div class="tile-meta">${esc(OS_LABEL[state.me.os] ?? "")}</div>
    </div>`;

  const paired = state.peers.filter((p) => p.paired);
  const unplaced = paired.filter((p) => !p.edge);
  for (const edge of EDGES) {
    const slot = document.querySelector(`.slot[data-edge="${edge}"]`);
    const id = state.layout[edge];
    const peer = id && paired.find((p) => p.id === id);
    slot.classList.toggle("filled", !!peer);
    if (peer) {
      slot.innerHTML = tile(peer, { removable: true, active: peer.id === activeId });
    } else {
      const options = unplaced.map((p) => `<option value="${esc(p.id)}">${esc(p.name)}</option>`).join("");
      slot.innerHTML = `<div class="slot-empty">
          <span>${paired.length ? "Drop a computer here" : "Pair a computer first"}</span>
          ${unplaced.length ? `<select data-edge="${edge}" aria-label="Place a computer ${EDGE_LABEL[edge]}"><option value="">Place…</option>${options}</select>` : ""}
        </div>`;
    }
  }
}

function renderDevices() {
  const paired = state.peers.filter((p) => p.paired);
  const nearby = state.peers.filter((p) => !p.paired);

  $("#paired-list").innerHTML = paired.length
    ? paired
        .map((p) => {
          const open = openRows.has(p.id);
          const where = p.edge ? ` · ${EDGE_LABEL[p.edge]}` : "";
          return `<div class="device ${open ? "open" : ""}" draggable="true" data-id="${esc(p.id)}">
            ${screenIcon()}
            <div class="name">${esc(p.name)}</div>
            <div class="meta"><span class="status-dot ${p.online ? "on" : ""}"></span>${p.online ? "Online" : "Offline"} · ${esc(OS_LABEL[p.os] ?? "")}${esc(where)}</div>
            <div class="actions"><button class="icon-btn" data-action="toggle-row" data-id="${esc(p.id)}" aria-expanded="${open}" aria-label="Details for ${esc(p.name)}">${open ? "▴" : "▾"}</button></div>
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
    : `<div class="empty">No paired computers yet. Install Skerry on your other computers and connect them to the same network. They'll appear under Nearby.</div>`;

  $("#nearby-list").innerHTML = nearby.length
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
    : `<div class="empty">Looking for other computers running Skerry on this network…</div>`;
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
    <dt>Port</dt><dd>${esc(state.me.port)}</dd>
    <dt>Input</dt><dd>${esc(info.backend)}</dd>
    <dt>Version</dt><dd>${esc(state.me.version)}</dd>
    <dt>Settings file</dt><dd><code>${esc(info.config_path)}</code></dd>`;
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

function toast(message, kind = "") {
  const el = document.createElement("div");
  el.className = `toast ${kind}`;
  el.textContent = message;
  $("#toasts").appendChild(el);
  setTimeout(() => el.remove(), kind === "error" ? 7000 : 4500);
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
    else if (action === "forget") {
      if (confirm(`Forget ${peerName(id)}? You'll need to pair again to use it.`)) call("forget", { id });
    } else if (action === "toggle-row") {
      openRows.has(id) ? openRows.delete(id) : openRows.add(id);
      renderDevices();
    } else if (action === "unplace") {
      const edge = EDGES.find((e) => state.layout[e] === id);
      if (edge) call("set_layout", { edge, peer: null });
    } else if (action === "cancel-pairing") call("cancel_pairing", { session: Number(el.dataset.session) });
    else if (action === "remove-manual") call("remove_manual_peer", { addr: el.dataset.addr });
    else if (action === "open-permissions") call("open_permission_settings");
  });

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
  api.invoke("get_autostart").then((v) => ($("#autostart").checked = !!v));
}

main();

// ---------------------------------------------------------------------------
// Demo mode: lets the UI run in a plain browser for development.
// ---------------------------------------------------------------------------

function demoApi() {
  const listeners = [];
  const demo = {
    me: { id: "1c843bbbadba346f", name: "Studio Desktop", os: "linux", fingerprint: "1c84-3bbb-adba-346f-1f47", port: 24870, version: "1.0.0" },
    settings: { enabled: true, clipboard_sync: true, swap_cmd_ctrl: true, edge_switching: true, block_switch_while_dragging: true },
    layout: { left: "a1", right: "b2", top: null, bottom: null },
    peers: [
      { id: "a1", name: "MacBook Air", os: "macos", paired: true, online: true, available: true, discovered: true, addr: "192.168.1.31:24870", fingerprint: "9f3a-77c1-02be-d5e0-11aa", edge: "left", speed: 1.4, version: "1.0.0" },
      { id: "b2", name: "Gaming PC", os: "windows", paired: true, online: true, available: true, discovered: true, addr: "192.168.1.40:24870", fingerprint: "3b10-6d2e-8a44-c901-7f2b", edge: "right", speed: 1, version: "1.0.0" },
      { id: "c3", name: "Office Laptop", os: "windows", paired: true, online: false, available: false, discovered: false, addr: "192.168.1.52:24870", fingerprint: "77de-a012-55c3-09ab-e4f1", edge: null, speed: 1, version: null },
      { id: "d4", name: "living-room-nuc", os: "linux", paired: false, online: false, available: false, discovered: true, addr: "192.168.1.60:24870", fingerprint: null, edge: null, speed: 1, version: "1.0.0" },
    ],
    focus: new URLSearchParams(location.search).has("controlling") ? { kind: "controlling", peer: "b2" } : { kind: "local" },
    capture: { state: "ok" },
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
  };
  if (new URLSearchParams(location.search).has("pairing")) {
    demo.pairings = [{ session: 7, peer_name: "living-room-nuc", peer_fingerprint: "5e21-0c9d-4b7a-a3f0-6612", stage: "show_code", code: "684858" }];
  }
  const emit = () => listeners.forEach((cb) => cb({ payload: { type: "state", ...structuredClone(demo) } }));
  return {
    async invoke(cmd, args = {}) {
      switch (cmd) {
        case "get_state": return structuredClone(demo);
        case "get_info": return { backend: "Demo", config_path: "~/.config/skerry/config.toml" };
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
    async listen(_event, cb) { listeners.push(cb); },
  };
}
