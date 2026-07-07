const { listen } = window.__TAURI__.event;
const { invoke } = window.__TAURI__.core;

const GB = 1024 ** 3;
const MB = 1024 ** 2;
const gb = (b) => (b / GB).toFixed(1);
// Adaptive: GB when >= 1 GB, MB when >= 1 MB, otherwise KB.
const fmt = (b) =>
  b >= GB ? `${gb(b)} GB` : b >= MB ? `${(b / MB).toFixed(0)} MB` : `${(b / 1024).toFixed(0)} KB`;
const $ = (id) => document.getElementById(id);

// Thresholds come from settings; refreshed after every save.
let memWarnPct = 85;
async function refreshThresholds() {
  try {
    memWarnPct = (await invoke("get_config")).mem_warn_pct;
  } catch {}
}
refreshThresholds();

// ---------- tabs ----------
// Popovers size to their content, like native macOS menus: measure the
// active pane and fit the window to it (clamped to sane bounds).
function resizeForTab() {
  requestAnimationFrame(() => {
    const pane = document.querySelector(".tab-pane.active");
    if (!pane) return;
    let h = 88; // panel padding + header + tabs + outer gaps
    for (const child of pane.children) {
      if (child.hidden) continue;
      h += child.classList.contains("scroll")
        ? Math.min(child.scrollHeight, 460)
        : child.offsetHeight;
      h += 9; // flex gap
    }
    h = Math.max(240, Math.min(640, h));
    invoke("resize_popover", { height: h }).catch(() => {});
  });
}

const tabButtons = [...document.querySelectorAll(".tab")];
function moveTabIndicator(btn) {
  const ind = $("tab-indicator");
  ind.style.left = `${btn.offsetLeft}px`;
  ind.style.width = `${btn.offsetWidth}px`;
}
let activeTabIndex = 0;
tabButtons.forEach((btn, idx) => {
  btn.addEventListener("click", () => {
    const dir = idx > activeTabIndex ? "enter-right" : "enter-left";
    activeTabIndex = idx;
    tabButtons.forEach((b) => b.classList.toggle("active", b === btn));
    moveTabIndicator(btn);
    document.querySelectorAll(".tab-pane").forEach((p) => {
      const on = p.id === `pane-${btn.dataset.tab}`;
      p.classList.toggle("active", on);
      p.classList.remove("enter-left", "enter-right");
      if (on) {
        void p.offsetWidth; // restart animation
        p.classList.add(dir);
      }
    });
    resizeForTab();
    if (btn.dataset.tab === "settings") loadSettings();
    if (btn.dataset.tab === "docker" && !dockerLoaded) refreshDocker();
  });
});
requestAnimationFrame(() => moveTabIndicator(tabButtons[0]));

// Replay the panel entrance spring every time the popover opens.
listen("popover-shown", () => {
  const panel = document.querySelector(".panel");
  panel.classList.remove("opening");
  void panel.offsetWidth;
  panel.classList.add("opening");
  moveTabIndicator(tabButtons[activeTabIndex]);
  resizeForTab();
});

// ---------- toast & modal ----------
let toastTimer;
function toast(msg, ms = 3500) {
  const t = $("toast");
  t.textContent = msg;
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.hidden = true), ms);
}

function confirmModal(title, bodyNodes, confirmLabel = "Confirm") {
  return new Promise((resolve) => {
    $("modal-title").textContent = title;
    const body = $("modal-body");
    body.replaceChildren(...bodyNodes);
    $("modal-confirm").textContent = confirmLabel;
    $("modal").hidden = false;
    const done = (ok) => {
      $("modal").hidden = true;
      $("modal-confirm").onclick = $("modal-cancel").onclick = null;
      resolve(ok);
    };
    $("modal-confirm").onclick = () => done(true);
    $("modal-cancel").onclick = () => done(false);
  });
}

const pathLine = (p) => {
  const d = document.createElement("div");
  d.className = "path-line";
  d.textContent = p;
  return d;
};
const textDiv = (t, cls = "") => {
  const d = document.createElement("div");
  if (cls) d.className = cls;
  d.textContent = t;
  return d;
};

// ---------- monitor tab ----------
// Click the memory section to expand from top 5 to the full breakdown.
let memExpanded = false;
let lastStats = null;
$("mem-section").addEventListener("click", () => {
  memExpanded = !memExpanded;
  $("mem-chevron").classList.toggle("open", memExpanded);
  $("proc-list").classList.toggle("expanded", memExpanded);
  if (lastStats) renderStats(lastStats);
  resizeForTab();
});

listen("stats", (event) => renderStats(event.payload));

function renderStats(s) {
  lastStats = s;
  $("status-dot").classList.add("live");

  const memPct = s.memory_total_bytes ? (s.memory_used_bytes / s.memory_total_bytes) * 100 : 0;
  const crit = memPct >= memWarnPct;
  const warn = memPct >= memWarnPct - 15 && !crit;
  $("mem-text").textContent =
    `${fmt(s.memory_used_bytes)} of ${fmt(s.memory_total_bytes)} · ${memPct.toFixed(0)}%`;
  const memBar = $("mem-bar");
  memBar.style.width = `${memPct.toFixed(1)}%`;
  memBar.classList.toggle("crit", crit);
  memBar.classList.toggle("warn", warn);
  $("mem-warning").textContent = crit
    ? `⚠ Memory above your ${memWarnPct}% warning threshold`
    : "";

  $("proc-list").replaceChildren(
    ...s.top_processes.slice(0, memExpanded ? 20 : 5).map((p) => {
      const li = document.createElement("li");
      li.className = "proc-row";
      const info = document.createElement("div");
      info.className = "proc-info";
      info.append(textDiv(p.name, "proc-name"));
      const sub = [];
      if (p.process_count > 1) sub.push(`${p.process_count} processes`);
      if (!p.is_app) {
        sub.push("CLI/background");
        li.title = "Not an app — could be a live terminal session. Quit it from wherever it runs.";
      }
      if (sub.length) info.append(textDiv(sub.join(" · "), "proc-sub"));
      const ramPct = s.memory_total_bytes
        ? ((p.memory_bytes / s.memory_total_bytes) * 100).toFixed(0)
        : 0;
      const mem = textDiv(`${fmt(p.memory_bytes)} · ${ramPct}%`, "proc-mem");
      li.append(info, mem);
      if (p.is_app) {
        const quit = document.createElement("button");
        quit.className = "btn mini quit";
        quit.textContent = "Quit";
        quit.addEventListener("click", async () => {
          const ok = await confirmModal(
            `Quit ${p.name}?`,
            [textDiv(`Asks ${p.name} to quit like ⌘Q — you can save unsaved work first. Frees ~${fmt(p.memory_bytes)}.`)],
            "Quit app"
          );
          if (!ok) return;
          try {
            await invoke("quit_app", { name: p.name, pids: p.pids });
            toast(`Asked ${p.name} to quit`);
          } catch (e) {
            toast(String(e), 5000);
          }
        });
        li.append(quit);
      }
      return li;
    })
  );

  const usedPct = s.disk_total_bytes
    ? ((s.disk_total_bytes - s.disk_free_bytes) / s.disk_total_bytes) * 100
    : 0;
  $("disk-text").textContent = `${fmt(s.disk_free_bytes)} free of ${fmt(s.disk_total_bytes)}`;
  $("disk-bar").style.width = `${usedPct.toFixed(1)}%`;

  if (s.total_saved_bytes > 0) {
    $("total-saved").textContent = `Total saved with DevSpace: ${fmt(s.total_saved_bytes)}`;
  }
}

async function refreshForecast() {
  try {
    const fc = await invoke("get_forecast");
    const el = $("forecast-text");
    if (fc.days_left != null) {
      el.textContent = `At the current rate you'll hit the low-disk threshold in ~${Math.round(fc.days_left)} days`;
    } else if (fc.samples < 24) {
      el.textContent = `Storage forecast: collecting data (${fc.samples}/24 samples)`;
    } else {
      el.textContent = "Storage trend: stable";
    }
  } catch {}
}
refreshForecast();
setInterval(refreshForecast, 10 * 60 * 1000);

// ---------- disk tab ----------
let lastScan = null;
const selected = new Set();

$("scan-btn").addEventListener("click", async () => {
  try {
    await invoke("start_scan");
    $("scan-btn").disabled = true;
    $("scan-status").textContent = "scanning…";
  } catch (e) {
    toast(String(e));
  }
});

listen("scan-progress", (event) => {
  const p = event.payload;
  const short = p.path.split("/").slice(-2).join("/");
  $("scan-status").textContent = `scanning… ${short}`;
  $("models-status").textContent = "scanning…";
});

listen("scan-done", (event) => {
  lastScan = event.payload;
  selected.clear();
  $("scan-btn").disabled = false;
  $("scan-status").textContent = `${lastScan.findings.length} findings`;
  renderScan();
  renderModels();
});

// ---------- ML models tab ----------
$("models-scan").addEventListener("click", () => $("scan-btn").click());

function renderModels() {
  const box = $("models-list");
  if (!lastScan) return;
  const checkpoints = lastScan.findings.filter(
    (f) => f.bucket === "precious" && f.kind === "checkpoint"
  );
  box.replaceChildren();
  const totalB = checkpoints.reduce((a, f) => a + f.size_bytes, 0);
  $("models-status").textContent = checkpoints.length
    ? `${checkpoints.length} checkpoint${checkpoints.length === 1 ? "" : "s"} · ${fmt(totalB)}`
    : "";
  if (!checkpoints.length) {
    box.append(
      textDiv("No checkpoints found. Run a scan — .pt, .pth, .ckpt, .safetensors, .gguf, .h5, .onnx and large .bin files show up here, grouped by project.", "hint")
    );
    resizeForTab();
    return;
  }

  // Group by project folder, biggest group first.
  const groups = new Map();
  for (const f of checkpoints) {
    const key = f.project ?? f.path.split("/").slice(0, -1).join("/");
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(f);
  }
  const sorted = [...groups.entries()]
    .map(([root, items]) => ({
      root,
      name: root.split("/").pop(),
      items: items.sort((a, b) => b.size_bytes - a.size_bytes),
      total: items.reduce((a, f) => a + f.size_bytes, 0),
    }))
    .sort((a, b) => b.total - a.total);

  for (const g of sorted) {
    box.append(textDiv(`${g.name} · ${g.items.length} file${g.items.length === 1 ? "" : "s"} · ${fmt(g.total)}`, "group-title precious"));
    g.items.forEach((f, i) => {
      const row = findingRow(f, false);
      if (i < 12) {
        row.classList.add("pop");
        row.style.animationDelay = `${i * 22}ms`;
      }
      box.append(row);
    });
  }
  resizeForTab();
}

listen("scan-error", (event) => {
  $("scan-btn").disabled = false;
  $("scan-status").textContent = "";
  toast(String(event.payload), 6000);
});

// Thin stroke glyphs, SF-Symbols outline style.
const svg = (path) =>
  `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linejoin="round" stroke-linecap="round">${path}</svg>`;
const BUCKET_ICONS = {
  "regen-low": svg(
    '<path d="M2.5 5.2A1.4 1.4 0 0 1 3.9 3.8h2.4c.35 0 .68.15.9.4l.8.9h4.1a1.4 1.4 0 0 1 1.4 1.4v4.7a1.4 1.4 0 0 1-1.4 1.4H3.9a1.4 1.4 0 0 1-1.4-1.4V5.2z"/>'
  ),
  "regen-medium": svg(
    '<path d="M8 2.2l5.3 2v5.4c0 .5-.3 1-.7 1.2L8 13.8l-4.6-3c-.4-.2-.7-.7-.7-1.2V4.2l5.3-2z"/><path d="M8 2.2v11.6M3.4 4.4L8 6.5l4.6-2.1"/>'
  ),
  precious: svg(
    '<path d="M8 2l1.6 3.6 3.9.4-2.9 2.6.8 3.9L8 10.5l-3.4 2 .8-3.9-2.9-2.6 3.9-.4L8 2z"/>'
  ),
  "large-file": svg(
    '<path d="M4.2 2h4.6l3 3v8.5a1.3 1.3 0 0 1-1.3 1.3H4.2a1.3 1.3 0 0 1-1.3-1.3V3.3A1.3 1.3 0 0 1 4.2 2z"/><path d="M8.8 2v3h3"/>'
  ),
};

function findingRow(f, checkable) {
  const row = document.createElement("div");
  row.className = "finding";
  if (checkable) {
    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.checked = selected.has(f.path);
    const sync = () => {
      cb.checked ? selected.add(f.path) : selected.delete(f.path);
      row.classList.toggle("selected", cb.checked);
      updateCleanBtn();
    };
    row.classList.toggle("selected", cb.checked);
    cb.addEventListener("change", sync);
    // The whole row toggles the checkbox — no fiddly click target.
    row.style.cursor = "pointer";
    row.addEventListener("click", (e) => {
      if (e.target === cb || e.target.tagName === "BUTTON") return;
      cb.checked = !cb.checked;
      sync();
    });
  }
  const icon = document.createElement("span");
  icon.className = "f-icon";
  icon.innerHTML = BUCKET_ICONS[f.bucket] ?? BUCKET_ICONS["large-file"];
  row.append(icon);
  const info = document.createElement("div");
  info.className = "f-info";
  const name = textDiv(
    f.kind + (f.rebuild_risk ? "  ⚠ no lockfile found" : ""),
    "f-name" + (f.rebuild_risk ? " risk" : "")
  );
  info.append(name, textDiv(f.path, "f-path"));
  row.append(info, textDiv(fmt(f.size_bytes), "f-size"));
  if (f.bucket === "precious") {
    const btn = document.createElement("button");
    btn.className = "btn mini";
    btn.textContent = "Archive";
    btn.addEventListener("click", () => archiveFlow(f));
    row.append(btn);
  }
  return row;
}

function updateCleanBtn() {
  const total = [...selected].reduce((acc, p) => {
    const f = lastScan?.findings.find((x) => x.path === p);
    return acc + (f ? f.size_bytes : 0);
  }, 0);
  $("clean-actions").hidden = selected.size === 0;
  $("clean-btn").textContent = `Move ${selected.size} item${selected.size === 1 ? "" : "s"} (${fmt(total)}) to Trash`;
}

async function renderScan() {
  const box = $("scan-results");
  box.replaceChildren();
  if (!lastScan) return;

  const groups = [
    ["regen-low", "Safe to clean", "low", true],
    ["regen-medium", "Rebuildable — check twice", "medium", true],
    ["large-file", "Large files (≥500 MB, not dev artifacts)", "large", true],
    ["precious", "Precious — never auto-deleted", "precious", false],
  ];
  for (const [bucket, title, cls, checkable] of groups) {
    const items = lastScan.findings.filter((f) => f.bucket === bucket);
    if (!items.length) continue;
    const totalB = items.reduce((a, f) => a + f.size_bytes, 0);
    const header = document.createElement("div");
    header.className = "row";
    header.append(textDiv(`${title} · ${fmt(totalB)}`, `group-title ${cls}`));
    if (checkable) {
      const allBtn = document.createElement("button");
      allBtn.className = "btn mini";
      const allSelected = () => items.every((f) => selected.has(f.path));
      allBtn.textContent = allSelected() ? "Clear all" : "Select all";
      allBtn.addEventListener("click", () => {
        const clear = allSelected();
        items.forEach((f) => (clear ? selected.delete(f.path) : selected.add(f.path)));
        renderScan();
      });
      header.append(allBtn);
    }
    box.append(header);
    items.slice(0, 60).forEach((f, i) => {
      const row = findingRow(f, checkable);
      if (i < 16) {
        row.classList.add("pop");
        row.style.animationDelay = `${i * 22}ms`;
      }
      box.append(row);
    });
  }

  // Hibernation candidates.
  const cfg = await invoke("get_config").catch(() => null);
  const ageDays = cfg?.hibernation_age_days ?? 60;
  const candidates = lastScan.projects.filter(
    (p) => p.hibernated || (p.days_idle >= ageDays && p.regen_low_bytes > 50 * 1024 ** 2)
  );
  if (candidates.length) {
    box.append(textDiv("Idle projects — hibernate?", "group-title medium"));
    for (const p of candidates) {
      const row = document.createElement("div");
      row.className = "proj";
      const info = document.createElement("div");
      info.className = "f-info";
      info.append(
        textDiv(p.name, "f-name"),
        textDiv(
          p.hibernated
            ? `hibernated · restore: ${p.rebuild_cmd ?? "manual"}`
            : `idle ${p.days_idle}d · ${fmt(p.regen_low_bytes)} reclaimable`,
          "f-path"
        )
      );
      const btn = document.createElement("button");
      btn.className = "btn mini";
      btn.textContent = p.hibernated ? "Restore" : "Hibernate";
      btn.addEventListener("click", () => (p.hibernated ? restoreProject(p) : hibernateProject(p)));
      row.append(info, btn);
      box.append(row);
    }
  }

  // Recently cleaned (undo window).
  const recent = await invoke("recently_cleaned").catch(() => []);
  if (recent.length) {
    box.append(textDiv("Recently cleaned (in Trash, restorable)", "group-title low"));
    recent.slice(0, 15).forEach((r) => {
      const row = document.createElement("div");
      row.className = "finding";
      const info = document.createElement("div");
      info.className = "f-info";
      info.append(textDiv(r.path, "f-path"));
      row.append(info, textDiv(fmt(r.size_bytes), "f-size"));
      box.append(row);
    });
  }
  updateCleanBtn();
  resizeForTab();
}

$("clean-btn").addEventListener("click", async () => {
  const paths = [...selected];
  const nodes = [
    textDiv(`These ${paths.length} items will move to the Trash (recoverable for 30 days):`),
    ...paths.map(pathLine),
  ];
  if (!(await confirmModal("Move to Trash?", nodes, "Move to Trash"))) return;
  const res = await invoke("clean_paths", { paths });
  toast(`Recovered ${fmt(res.total_bytes)}${res.errors.length ? ` · ${res.errors.length} errors` : ""}`);
  if (res.errors.length) console.warn(res.errors);
  lastScan.findings = lastScan.findings.filter((f) => !res.moved.includes(f.path));
  selected.clear();
  renderScan();
});

// ---------- archive flow (v4) ----------
/// Modal with one button per option; resolves the chosen value or null.
function chooseModal(title, bodyNodes, options) {
  return new Promise((resolve) => {
    $("modal-title").textContent = title;
    const body = $("modal-body");
    const buttons = options.map(([label, value]) => {
      const b = document.createElement("button");
      b.className = "btn";
      b.style.margin = "3px 0";
      b.style.width = "100%";
      b.textContent = label;
      b.addEventListener("click", () => done(value));
      return b;
    });
    body.replaceChildren(...bodyNodes, ...buttons);
    $("modal-confirm").hidden = true;
    $("modal").hidden = false;
    const done = (v) => {
      $("modal").hidden = true;
      $("modal-confirm").hidden = false;
      $("modal-cancel").onclick = null;
      resolve(v);
    };
    $("modal-cancel").onclick = () => done(null);
  });
}

async function archiveFlow(f) {
  const volumes = await invoke("list_volumes").catch(() => []);
  const options = [];
  for (const vol of volumes) {
    options.push([`Move to ${vol.split("/").pop()} + symlink back (lossless)`, { kind: "move", vol }]);
  }
  options.push(["Convert to FP16 in place (~50% smaller, lossy)", { kind: "fp16" }]);
  if (!f.path.endsWith(".safetensors")) {
    options.push(["Quantize to INT8 in place (~75% smaller, lossy)", { kind: "int8" }]);
  }
  const nodes = [
    pathLine(f.path),
    textDiv(`Size: ${fmt(f.size_bytes)}`),
    textDiv("Conversions are NOT bit-identical to the original. Nothing happens without your click below.", "hint"),
  ];
  if (!volumes.length) nodes.push(textDiv("No external volume mounted — move option unavailable.", "hint"));
  const choice = await chooseModal("Archive checkpoint", nodes, options);
  if (!choice) return;
  try {
    if (choice.kind === "move") {
      const res = await invoke("archive_move", { path: f.path, volume: choice.vol });
      toast(res.message, 6000);
    } else {
      toast(`Converting to ${choice.kind}… this can take a while`, 8000);
      const res = await invoke("archive_convert", { path: f.path, mode: choice.kind });
      toast(`${res.message}: ${fmt(res.before_bytes)} → ${fmt(res.after_bytes)}`, 6000);
      f.size_bytes = res.after_bytes;
      renderScan();
      renderModels();
    }
  } catch (e) {
    toast(String(e), 6000);
  }
}

// ---------- hibernation (v5) ----------
async function hibernateProject(p) {
  const nodes = [
    textDiv(`${p.name} — idle ${p.days_idle} days.`),
    textDiv(`All regenerable content (~${fmt(p.regen_low_bytes)}) moves to Trash; a marker file records how to rebuild (${p.rebuild_cmd ?? "no rebuild command detected"}).`),
  ];
  if (!(await confirmModal("Hibernate project?", nodes, "Hibernate"))) return;
  try {
    const res = await invoke("hibernate_project", { root: p.root });
    toast(`Hibernated · reclaimed ${fmt(res.cleaned_bytes)}`);
    p.hibernated = true;
    lastScan.findings = lastScan.findings.filter((f) => !res.cleaned_paths.includes(f.path));
    renderScan();
  } catch (e) {
    toast(String(e), 6000);
  }
}

async function restoreProject(p) {
  if (!(await confirmModal("Restore project?", [textDiv(`Runs "${p.rebuild_cmd}" in ${p.root} — can take a few minutes.`)], "Restore"))) return;
  toast("Restoring… running rebuild command", 10000);
  try {
    const res = await invoke("restore_project", { root: p.root });
    toast(res.success ? "Restored ✓" : "Rebuild failed — see console", 6000);
    if (!res.success) console.warn(res.output);
    p.hibernated = !res.success;
    renderScan();
  } catch (e) {
    toast(String(e), 6000);
  }
}

// ---------- docker tab (v4) ----------
let dockerLoaded = false;
async function refreshDocker() {
  $("docker-status").textContent = "loading…";
  const info = await invoke("docker_info").catch(() => null);
  dockerLoaded = true;
  const box = $("docker-results");
  box.replaceChildren();
  if (!info || !info.available) {
    $("docker-status").textContent = "";
    box.append(textDiv(info?.error ? `Docker unavailable: ${info.error}` : "Docker is not running.", "hint"));
    return;
  }
  $("docker-status").textContent = "";

  const section = (title, cls) => box.append(textDiv(title, `group-title ${cls}`));

  const byStatus = { "dangling": [], "unused": [], "in-use": [] };
  info.images.forEach((i) => byStatus[i.status]?.push(i));

  const imgRow = (img, removable) => {
    const row = document.createElement("div");
    row.className = "finding";
    const infoEl = document.createElement("div");
    infoEl.className = "f-info";
    infoEl.append(textDiv(img.repo_tag, "f-name"), textDiv(img.status, "f-path"));
    row.append(infoEl, textDiv(fmt(img.size_bytes), "f-size"));
    if (removable) {
      const btn = document.createElement("button");
      btn.className = "btn mini";
      btn.textContent = "Remove";
      btn.addEventListener("click", async () => {
        if (!(await confirmModal("Remove image?", [pathLine(img.repo_tag)], "Remove"))) return;
        try {
          await invoke("docker_remove", { kind: "image", id: img.id });
          toast("Image removed");
          refreshDocker();
        } catch (e) { toast(String(e), 6000); }
      });
      row.append(btn);
    }
    return row;
  };

  if (byStatus["dangling"].length) {
    section("Dangling images — safe to remove", "low");
    byStatus["dangling"].forEach((i) => box.append(imgRow(i, true)));
  }
  if (byStatus["unused"].length) {
    section("Unused images (no container references them)", "medium");
    byStatus["unused"].forEach((i) => box.append(imgRow(i, true)));
  }
  if (byStatus["in-use"].length) {
    section("In use — protected", "precious");
    byStatus["in-use"].forEach((i) => box.append(imgRow(i, false)));
  }

  if (info.containers.length) {
    section("Containers", "medium");
    info.containers.forEach((c) => {
      const row = document.createElement("div");
      row.className = "finding";
      const infoEl = document.createElement("div");
      infoEl.className = "f-info";
      infoEl.append(textDiv(c.name, "f-name"), textDiv(`${c.image} · ${c.state}`, "f-path"));
      row.append(infoEl);
      if (c.state !== "running") {
        const btn = document.createElement("button");
        btn.className = "btn mini";
        btn.textContent = "Remove";
        btn.addEventListener("click", async () => {
          if (!(await confirmModal("Remove container?", [pathLine(c.name)], "Remove"))) return;
          try {
            await invoke("docker_remove", { kind: "container", id: c.id });
            toast("Container removed");
            refreshDocker();
          } catch (e) { toast(String(e), 6000); }
        });
        row.append(btn);
      }
      box.append(row);
    });
  }

  if (info.build_cache_bytes > 0) {
    section("Build cache", "low");
    const row = document.createElement("div");
    row.className = "finding";
    const infoEl = document.createElement("div");
    infoEl.className = "f-info";
    infoEl.append(textDiv("docker build cache", "f-name"));
    row.append(infoEl, textDiv(fmt(info.build_cache_bytes), "f-size"));
    const btn = document.createElement("button");
    btn.className = "btn mini";
    btn.textContent = "Prune";
    btn.addEventListener("click", async () => {
      if (!(await confirmModal("Prune build cache?", [textDiv("Runs docker builder prune -f.")], "Prune"))) return;
      try {
        await invoke("docker_remove", { kind: "build-cache", id: "" });
        toast("Build cache pruned");
        refreshDocker();
      } catch (e) { toast(String(e), 6000); }
    });
    row.append(btn);
    box.append(row);
  }
  resizeForTab();
}
$("docker-refresh").addEventListener("click", refreshDocker);

// ---------- settings tab ----------
async function loadSettings() {
  const cfg = await invoke("get_config");
  $("cfg-roots").value = cfg.scan_roots.join("\n");
  $("cfg-ignore").value = cfg.ignore_patterns.join("\n");
  $("cfg-never").value = cfg.never_touch.join("\n");
  $("cfg-poll").value = cfg.poll_interval_secs;
  $("cfg-mem").value = cfg.mem_warn_pct;
  $("cfg-disk").value = cfg.disk_warn_gb;
  $("cfg-hib").value = cfg.hibernation_age_days;
  $("cfg-traytext").checked = cfg.tray_live_text;
}

$("cfg-save").addEventListener("click", async () => {
  const lines = (v) => v.split("\n").map((s) => s.trim()).filter(Boolean);
  const config = {
    scan_roots: lines($("cfg-roots").value),
    ignore_patterns: lines($("cfg-ignore").value),
    never_touch: lines($("cfg-never").value),
    poll_interval_secs: Number($("cfg-poll").value) || 3,
    mem_warn_pct: Number($("cfg-mem").value) || 85,
    disk_warn_gb: Number($("cfg-disk").value) || 10,
    hibernation_age_days: Number($("cfg-hib").value) || 60,
    tray_live_text: $("cfg-traytext").checked,
  };
  try {
    await invoke("save_config", { config });
    refreshThresholds();
    toast("Settings saved");
  } catch (e) {
    toast(String(e), 6000);
  }
});

// ---------- misc ----------
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    if (!$("modal").hidden) $("modal").hidden = true;
    else window.__TAURI__.window.getCurrentWindow().hide();
  }
});
