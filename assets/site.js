// Cybermeta for Android — site behaviour: themes, hero animation, leak
// cards, reveals, phone demo, install commands. No trackers, no analytics.
"use strict";

const root = document.documentElement;
const css = (name) => getComputedStyle(root).getPropertyValue(name).trim();
const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;

// ---------------------------------------------------------------- themes --
// The CYBERGRID catalog lives on the org site (same origin), so every
// Cybercore page and tool shares one list. cybermeta's own palette leads.
const TAG_REDACT = {
  bg: "070a09", panel: "0e1614", line: "1e2d29", white: "eafaf5", muted: "7d9992",
  cyan: "2dd4bf", hot_pink: "ff6b5c", purple: "8b7cf6", acid_green: "2dd4bf", orange: "ffb347", red: "ff6b5c",
};
const picker = document.querySelector("#themeSelect");
const themeButton = document.querySelector("#themeButton");
const menu = document.querySelector("#themeMenu");
const labels = {
  cybermeta: "CYBERMETA", "cybercore-tech": "CYBERCORE TECH / MASTER MIX", cyberdyne: "CYBERDYNE", cyberpunk: "CYBERPUNK",
  default: "DEFAULT / CLASSICS", dystopian: "DYSTOPIAN", neosynth: "NEOSYNTH", "omarchy-live": "OMARCHY LIVE", synthwave: "SYNTHWAVE",
};
const display = (name) => name.replaceAll("-", " ").toUpperCase();
const cache = new Map([["cybermeta/tag-redact", Promise.resolve(TAG_REDACT)]]);
const STORE = "cybermeta-android-site-theme";

function applyPalette(name, p) {
  const vars = { bg: "--bg", white: "--white", acid_green: "--acid", hot_pink: "--pink", purple: "--purple", cyan: "--cyan", orange: "--orange", red: "--red", panel: "--panel", line: "--line", muted: "--muted" };
  for (const [k, v] of Object.entries(vars)) if (p[k]) root.style.setProperty(v, `#${String(p[k]).replace(/^#/, "")}`);
  document.querySelector('meta[name="theme-color"]')?.setAttribute("content", `#${String(p.bg).replace(/^#/, "")}`);
  try { localStorage.setItem(STORE, name); } catch { /* private mode */ }
  syncMenu();
}

function loadPalette(name, family) {
  const key = `${family}/${name}`;
  if (!cache.has(key)) {
    const url = `https://cybercoretech.github.io/data/themes/${encodeURIComponent(family)}/${encodeURIComponent(name)}.json`;
    cache.set(key, fetch(url).then((r) => { if (!r.ok) throw new Error(`theme ${r.status}`); return r.json(); }));
  }
  return cache.get(key);
}

function syncMenu() {
  themeButton.textContent = picker.selectedOptions[0]?.textContent || "TAG REDACT";
  menu.querySelectorAll(".theme-option").forEach((o) => o.setAttribute("aria-selected", String(o.dataset.value === picker.value)));
}

function addGroup(family, names) {
  const group = document.createElement("optgroup");
  group.label = labels[family] || family.toUpperCase();
  for (const name of names) {
    const option = document.createElement("option");
    option.value = name;
    option.textContent = display(name);
    option.dataset.family = family;
    group.append(option);
  }
  picker.append(group);
}

function buildMenu() {
  menu.replaceChildren();
  for (const group of picker.querySelectorAll("optgroup")) {
    const h = document.createElement("div");
    h.className = "theme-group-label";
    h.textContent = group.label;
    menu.append(h);
    for (const src of group.querySelectorAll("option")) {
      const b = document.createElement("button");
      b.type = "button";
      b.className = "theme-option";
      b.setAttribute("role", "option");
      b.dataset.value = src.value;
      b.textContent = src.textContent;
      b.addEventListener("click", () => { picker.value = src.value; picker.dispatchEvent(new Event("change")); closeMenu(); themeButton.focus(); });
      menu.append(b);
    }
  }
  syncMenu();
}

function closeMenu() { menu.hidden = true; themeButton.setAttribute("aria-expanded", "false"); }

async function bootThemes() {
  addGroup("cybermeta", ["tag-redact"]);
  try {
    const r = await fetch("https://cybercoretech.github.io/data/cybergrid.json");
    if (!r.ok) throw new Error(`catalog ${r.status}`);
    const catalog = await r.json();
    for (const [family, names] of Object.entries(catalog.families)) addGroup(family, names);
  } catch (e) {
    console.warn("CYBERGRID catalog offline; only Tag Redact available", e);
  }
  buildMenu();
  let saved = null;
  try { saved = localStorage.getItem(STORE); } catch { /* ignore */ }
  const wanted = new URLSearchParams(location.search).get("theme") || saved || "tag-redact";
  const option = [...picker.options].find((o) => o.value === wanted) || picker.options[0];
  picker.value = option.value;
  try { applyPalette(option.value, await loadPalette(option.value, option.dataset.family)); } catch { applyPalette("tag-redact", TAG_REDACT); }
  picker.addEventListener("change", async () => {
    const o = picker.selectedOptions[0];
    try {
      applyPalette(o.value, await loadPalette(o.value, o.dataset.family));
      history.replaceState(null, "", `?theme=${encodeURIComponent(o.value)}${location.hash}`);
      toast(`THEME · ${o.textContent}`);
    } catch { toast("THEME OFFLINE"); }
  });
}
themeButton.addEventListener("click", () => {
  const opening = menu.hidden;
  menu.hidden = !opening;
  themeButton.setAttribute("aria-expanded", String(opening));
  if (opening) menu.querySelector('[aria-selected="true"]')?.focus();
});
document.addEventListener("pointerdown", (e) => { if (!e.target.closest(".theme-picker")) closeMenu(); });
document.addEventListener("keydown", (e) => { if (e.key === "Escape") { closeMenu(); closeModal(); } });
bootThemes();

// ----------------------------------------------------------------- toast --
const toastEl = document.querySelector("#toast");
let toastTimer;
function toast(text) {
  toastEl.textContent = text;
  toastEl.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => toastEl.classList.remove("show"), 1800);
}
window.cmToast = toast;

// --------------------------------------------------------- cursor + tilt --
addEventListener("pointermove", (e) => {
  root.style.setProperty("--x", `${e.clientX}px`);
  root.style.setProperty("--y", `${e.clientY}px`);
}, { passive: true });

// --------------------------------------------------------------- reveals --
const io = new IntersectionObserver((entries) => {
  for (const e of entries) {
    if (!e.isIntersecting) continue;
    e.target.classList.add("in");
    e.target.querySelectorAll("[data-count]").forEach(countUp);
    io.unobserve(e.target);
  }
}, { threshold: .15 });
document.querySelectorAll(".reveal").forEach((el) => io.observe(el));

function countUp(el) {
  const target = Number(el.dataset.count);
  const suffix = el.dataset.suffix || "";
  if (reduceMotion || target === 0) { el.textContent = target + suffix; return; }
  const start = performance.now();
  const step = (t) => {
    const k = Math.min(1, (t - start) / 1400);
    el.textContent = Math.round(target * (1 - Math.pow(1 - k, 3))) + suffix;
    if (k < 1) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}

// ---------------------------------------------------------------- ticker --
const tickerItems = [
  ["GPS", "45.515200, -122.678400"], ["SERIAL", "R5CX21B7QKM"], ["OWNER", "Jamie R."], ["TAKEN", "2026-10-18 19:38:44"],
  ["LENS SERIAL", "0042017"], ["CITY", "Portland"], ["MOTION VIDEO", "2.1 MB"], ["THUMBNAIL", "160×120"], ["SOFTWARE", "Gallery 15.1"],
];
const track = document.querySelector("#ticker");
if (track) {
const tickerHtml = tickerItems.map(([k, v]) => `<span><b>${k}</b> ${v} <i>→ REMOVED</i></span>`).join("");
track.innerHTML = tickerHtml + tickerHtml;
}

// ------------------------------------------------------------ leak cards --
const LEAKS = [
  { icon: "📍", sev: "high", title: "Where you were", body: "GPS coordinates to a few metres, plus city and country written by apps.",
    detail: "Phones write latitude, longitude, altitude and even which way the camera faced. Photo apps and editors often copy the position into XMP, and newsroom tools add the city and country as IPTC fields. Clearing only the EXIF GPS leaves the copies behind.",
    example: "GPSLatitude   <em>45.515200 N</em>\nGPSLongitude  <em>122.678400 W</em>\nXMP exif:GPSLatitude  <em>45,30.9N</em>\nIPTC City     <em>Portland</em>",
    fix: "Every copy goes: EXIF GPS, XMP location properties and IPTC places. \"Location only\" mode removes just these." },
  { icon: "👤", sev: "high", title: "Who you are", body: "Your name as owner, artist or creator, plus contact details from editing apps.",
    detail: "Cameras store an owner name; Lightroom and other editors stamp creator, copyright and sometimes email and phone into XMP and IPTC. That travels with every copy you post.",
    example: "CameraOwnerName  <em>Jamie R.</em>\nXMP dc:creator   <em>Jamie R.</em>\nIPTC By-line     <em>Jamie R.</em>\nCiEmailWork      <em>jamie@…</em>",
    fix: "Removed from EXIF, XMP and IPTC. Your clean copy carries no names." },
  { icon: "📷", sev: "high", title: "Your exact camera", body: "Body and lens serial numbers link every photo you ever posted to one device.",
    detail: "Serial numbers and unique image IDs are fingerprints: two photos with the same body serial came from the same camera, even years apart and from different accounts. Maker notes, a private blob from the manufacturer, often hide them too.",
    example: "BodySerialNumber  <em>R5CX21B7QKM</em>\nLensSerialNumber  <em>0042017</em>\nImageUniqueID     <em>a3f1…9c</em>\nMakerNote         <em>8.2 KB (vendor data)</em>",
    fix: "Serials, unique IDs and the whole maker note are dropped. \"Anonymous camera\" also relabels the photo as Generic Camera." },
  { icon: "🕒", sev: "med", title: "When, to the second", body: "Capture time, time zone and editing times reveal your routine.",
    detail: "Times pin down when you're home, at work or away. Even the camera's file name (IMG_20261018_193844) repeats the timestamp.",
    example: "DateTimeOriginal  <em>2026:10:18 19:38:44</em>\nOffsetTimeOriginal <em>-07:00</em>\nFile name  <em>IMG_20261018_193844.jpg</em>",
    fix: "Removed by default (keep it if you want), or shifted by an offset you choose. Clean copies get random file names." },
  { icon: "🖼️", sev: "med", title: "A hidden preview", body: "An embedded thumbnail can still show what you cropped out.",
    detail: "Many cameras and some editors store a small preview inside the file. Crop a photo badly and the thumbnail may still show the uncropped original: a famous way leaks happen.",
    example: "IFD1 thumbnail  <em>160×120 JPEG, 6.4 KB</em>\nJFIF thumbnail  <em>present</em>",
    fix: "Thumbnails always go, whatever you choose to keep." },
  { icon: "🎞️", sev: "high", title: "Hidden video", body: "Motion photos hide a video clip, with sound, after the end of the image.",
    detail: "Samsung and Pixel motion photos are normal JPEGs with an MP4 glued on after the image ends. Most viewers show only the still, but anyone can extract the clip, and its audio and its own metadata.",
    example: "End of image at  <em>3.1 MB</em>\nAfter it:  <em>2.1 MB MP4 (MotionPhoto_Data)</em>",
    fix: "Everything after the end of the image is removed. The still photo stays." },
  { icon: "✍️", sev: "med", title: "Captions and comments", body: "Descriptions, keywords and comments you forgot you wrote.",
    detail: "Captions, Windows title and comment tags, JPEG comments and PNG text chunks can hold anything: addresses, names, notes to yourself.",
    example: "JPEG comment  <em>\"home address\"</em>\nPNG tEXt Author  <em>Jamie R.</em>\nImageDescription  <em>Backyard, new fence</em>",
    fix: "Every comment and text block is removed." },
  { icon: "🧰", sev: "low", title: "Software and settings", body: "Editing history, apps used, and the camera's exposure settings.",
    detail: "Low risk on its own, but it narrows down who you are: which phone model, which editor, which firmware.",
    example: "Software  <em>Gallery 15.1</em>\nModel     <em>SM-S938B</em>\nXMP history  <em>3 edits in Lightroom</em>",
    fix: "Gone with everything else. Colour profile and rotation can stay; they say nothing about you." },
];
const sevName = { high: "HIGH", med: "MEDIUM", low: "LOW" };
const grid = document.querySelector("#leakGrid");
if (grid) LEAKS.forEach((l, i) => {
  const b = document.createElement("button");
  b.type = "button";
  b.className = `leak reveal sev-${l.sev} d${i % 4}`;
  b.innerHTML = `<span class="sev">${sevName[l.sev]}</span><div class="ico">${l.icon}</div><h3>${l.title}</h3><p>${l.body}</p><span class="more">DETAILS →</span>`;
  b.addEventListener("click", () => openModal(l));
  b.addEventListener("pointermove", (e) => {
    const r = b.getBoundingClientRect();
    const px = (e.clientX - r.left) / r.width, py = (e.clientY - r.top) / r.height;
    b.style.setProperty("--mx", `${px * 100}%`);
    b.style.setProperty("--my", `${py * 100}%`);
    if (!reduceMotion) b.style.transform = `perspective(700px) rotateX(${(.5 - py) * 8}deg) rotateY(${(px - .5) * 10}deg) translateY(-3px)`;
  });
  b.addEventListener("pointerleave", () => { b.style.transform = ""; });
  grid.append(b);
  io.observe(b);
});

// ----------------------------------------------------------------- modal --
const modal = document.querySelector("#modal");
const modalCard = document.querySelector("#modalCard");
let lastFocus = null;
function openModal(l) {
  lastFocus = document.activeElement;
  modalCard.className = `modal-card sev-${l.sev}`;
  document.querySelector("#modalBody").innerHTML =
    `<span class="tag">${sevName[l.sev]} RISK</span><h3 id="modalTitle">${l.icon} ${l.title}</h3><p>${l.detail}</p>` +
    `<div class="example">${l.example}</div><p><strong style="color:var(--cyan)">What Cybermeta does:</strong> ${l.fix}</p>`;
  modal.classList.add("open");
  document.querySelector("#modalClose").focus();
}
function closeModal() {
  if (!modal.classList.contains("open")) return;
  modal.classList.remove("open");
  lastFocus?.focus();
}
document.querySelector("#modalClose").addEventListener("click", closeModal);
modal.addEventListener("click", (e) => { if (e.target === modal) closeModal(); });

// ------------------------------------------------------------ phone demo --
const views = [...document.querySelectorAll("#screen .view")];
const steps = [...document.querySelectorAll("#steps .step")];
let viewIdx = 0;
let phoneTimer;
function showView(i) {
  viewIdx = i;
  views.forEach((v, k) => v.classList.toggle("on", k === i));
  steps.forEach((s, k) => s.classList.toggle("active", k === i));
}
function cyclePhone() { if (!views.length) return; clearInterval(phoneTimer); phoneTimer = setInterval(() => showView((viewIdx + 1) % views.length), 4200); }
steps.forEach((s, i) => s.addEventListener("click", () => { showView(i); cyclePhone(); }));
cyclePhone();

// --------------------------------------------------------------- install --
const REPO = "https://github.com/cybercoretech/cybermeta-android-app";
// A page can set window.CM_CMDS before this script to use its own commands.
const CMDS = window.CM_CMDS || {
  curl: ["curl -fsSL https://raw.githubusercontent.com/cybercoretech/cybermeta-android-app/main/install.sh | sh"],
  manual: [
    `curl -fLO ${REPO}/releases/latest/download/cybermeta.apk`,
    `curl -fLO ${REPO}/releases/latest/download/cybermeta.apk.sha256 && sha256sum -c cybermeta.apk.sha256`,
    "apksigner verify --print-certs cybermeta.apk | grep SHA-256",
  ],
  adb: [`curl -fLO ${REPO}/releases/latest/download/cybermeta.apk && adb install -r cybermeta.apk`],
};
const cmds = document.querySelector("#cmds");
function showCmds(key) {
  document.querySelectorAll(".tab").forEach((t) => t.classList.toggle("on", t.dataset.cmd === key));
  cmds.replaceChildren(...CMDS[key].map((c) => {
    const row = document.createElement("div");
    row.className = "command";
    const code = document.createElement("code");
    code.textContent = c;
    const copy = document.createElement("button");
    copy.className = "copy";
    copy.type = "button";
    copy.textContent = "COPY";
    copy.addEventListener("click", () => navigator.clipboard?.writeText(c).then(() => toast("COPIED TO CLIPBOARD"), () => toast("COPY FAILED")));
    row.append(code, copy);
    return row;
  }));
}
document.querySelectorAll(".tab").forEach((t) => t.addEventListener("click", () => showCmds(t.dataset.cmd)));
if (cmds) showCmds(Object.keys(CMDS)[0]);

// ------------------------------------------------------ background bits --
(() => {
  const c = document.querySelector("#particles");
  const g = c.getContext("2d");
  let w, h, bits;
  const resize = () => {
    w = c.width = innerWidth; h = c.height = innerHeight;
    bits = Array.from({ length: Math.round((w * h) / 26000) }, () => ({ x: Math.random() * w, y: Math.random() * h, s: 1 + Math.random() * 2.5, v: .15 + Math.random() * .5, hot: Math.random() < .35 }));
  };
  resize();
  addEventListener("resize", resize);
  const frame = () => {
    g.clearRect(0, 0, w, h);
    const cyan = css("--cyan"), pink = css("--pink");
    for (const b of bits) {
      b.y -= b.v; b.x += Math.sin(b.y / 60) * .2;
      if (b.y < -5) { b.y = h + 5; b.x = Math.random() * w; }
      g.globalAlpha = .18;
      g.fillStyle = b.hot ? pink : cyan;
      g.fillRect(b.x, b.y, b.s, b.s);
    }
    if (!reduceMotion) requestAnimationFrame(frame);
  };
  frame();
})();

// ------------------------------------------------------------ hero scene --
(() => {
  const c = document.querySelector("#hero");
  if (!c) return;
  const g = c.getContext("2d");
  const W = 900, H = 900;
  const photo = { x: 200, y: 270, w: 500, h: 360, r: 26 };
  const hudState = document.querySelector("#hudState");
  const hudCount = document.querySelector("#hudCount");
  const TAGS = [
    { k: "GPS", v: "45.5152, -122.6784", sev: "red", ax: 620, ay: 320, tx: 470, ty: 70 },
    { k: "XMP CITY", v: "Portland", sev: "red", ax: 480, ay: 350, tx: 60, ty: 110 },
    { k: "OWNER", v: "Jamie R.", sev: "red", ax: 300, ay: 380, tx: 40, ty: 190 },
    { k: "SERIAL", v: "R5CX21B7QKM", sev: "red", ax: 660, ay: 420, tx: 560, ty: 160 },
    { k: "TAKEN", v: "2026-10-18 19:38", sev: "orange", ax: 270, ay: 500, tx: 30, ty: 670 },
    { k: "MODEL", v: "SM-S938B", sev: "orange", ax: 640, ay: 520, tx: 620, ty: 670 },
    { k: "THUMBNAIL", v: "160×120", sev: "orange", ax: 420, ay: 560, tx: 60, ty: 750 },
    { k: "MOTION VIDEO", v: "2.1 MB", sev: "red", ax: 560, ay: 600, tx: 520, ty: 750 },
    { k: "SOFTWARE", v: "Gallery 15", sev: "cyan", ax: 350, ay: 610, tx: 300, ty: 830 },
  ];
  let sparks = [];
  let t0 = performance.now();
  const CYCLE = 9000;

  function roundRect(x, y, w, h, r) {
    g.beginPath();
    g.moveTo(x + r, y); g.arcTo(x + w, y, x + w, y + h, r); g.arcTo(x + w, y + h, x, y + h, r);
    g.arcTo(x, y + h, x, y, r); g.arcTo(x, y, x + w, y, r); g.closePath();
  }

  function scene() {
    const { x, y, w, h } = photo;
    const sky = g.createLinearGradient(0, y, 0, y + h * .62);
    sky.addColorStop(0, "#1a0b3a"); sky.addColorStop(.55, "#7a1f6b"); sky.addColorStop(1, "#ff7a59");
    g.fillStyle = sky; g.fillRect(x, y, w, h);
    // Striped sun.
    const sx = x + w * .5, sy = y + h * .56, sr = 82;
    g.save(); g.beginPath(); g.arc(sx, sy, sr, 0, Math.PI * 2); g.clip();
    const sun = g.createLinearGradient(0, sy - sr, 0, sy + sr);
    sun.addColorStop(0, "#ffe27a"); sun.addColorStop(1, "#ff4f8b");
    g.fillStyle = sun; g.fillRect(sx - sr, sy - sr, sr * 2, sr * 2);
    g.fillStyle = "#7a1f6b";
    for (let i = 0; i < 6; i++) g.fillRect(sx - sr, sy + 6 + i * 14, sr * 2, 3 + i * 1.4);
    g.restore();
    // Mountains.
    g.fillStyle = "#24103f";
    g.beginPath(); g.moveTo(x, y + h * .66);
    [[.12, .5], [.24, .6], [.36, .44], [.5, .62], [.63, .47], [.78, .58], [.9, .5], [1, .6]].forEach(([px, py]) => g.lineTo(x + w * px, y + h * py));
    g.lineTo(x + w, y + h * .66); g.closePath(); g.fill();
    // Neon floor grid.
    const fy = y + h * .66;
    g.fillStyle = "#0d0620"; g.fillRect(x, fy, w, h - (fy - y));
    g.strokeStyle = "rgba(255,79,139,.55)"; g.lineWidth = 1.4;
    for (let i = 0; i < 9; i++) { const yy = fy + Math.pow(i / 8, 1.8) * (h - (fy - y)); g.beginPath(); g.moveTo(x, yy); g.lineTo(x + w, yy); g.stroke(); }
    for (let i = -10; i <= 10; i++) { g.beginPath(); g.moveTo(sx + i * 12, fy); g.lineTo(sx + i * 70, y + h); g.stroke(); }
  }

  function draw(now) {
    const pink = css("--pink"), cyan = css("--cyan"), white = css("--white"), panel = css("--panel"), line = css("--line"), red = css("--red"), orange = css("--orange"), bg = css("--bg");
    const sev = { red, orange, cyan };
    const t = ((now - t0) % CYCLE) / CYCLE;
    const scanning = t > .12 && t < .62;
    const beamY = photo.y + Math.min(1, Math.max(0, (t - .12) / .5)) * photo.h;
    g.clearRect(0, 0, W, H);
    g.fillStyle = bg; g.fillRect(0, 0, W, H);
    // Faint grid.
    g.strokeStyle = line; g.globalAlpha = .45; g.lineWidth = 1;
    for (let i = 0; i <= W; i += 45) { g.beginPath(); g.moveTo(i, 0); g.lineTo(i, H); g.stroke(); g.beginPath(); g.moveTo(0, i); g.lineTo(W, i); g.stroke(); }
    g.globalAlpha = 1;
    // Photo.
    g.save(); roundRect(photo.x, photo.y, photo.w, photo.h, photo.r); g.clip(); scene();
    if (t > .12) {
      // Scrubbed region gets a clean cyan wash.
      g.fillStyle = cyan; g.globalAlpha = .07; g.fillRect(photo.x, photo.y, photo.w, Math.max(0, beamY - photo.y)); g.globalAlpha = 1;
    }
    g.restore();
    g.lineWidth = 6; g.strokeStyle = white; roundRect(photo.x, photo.y, photo.w, photo.h, photo.r); g.stroke();
    g.shadowColor = cyan; g.shadowBlur = 40; g.lineWidth = 2; g.strokeStyle = cyan; g.globalAlpha = .4; roundRect(photo.x - 10, photo.y - 10, photo.w + 20, photo.h + 20, photo.r + 8); g.stroke(); g.globalAlpha = 1; g.shadowBlur = 0;

    // Tags.
    let alive = 0;
    g.font = "600 17px JetBrains Mono, monospace";
    for (const tag of TAGS) {
      const hit = t > .12 && beamY > tag.ay;
      const reset = t > .9 ? (t - .9) / .1 : 0;
      if (hit && !tag.dead && t < .9) {
        tag.dead = true;
        for (let i = 0; i < 26; i++) sparks.push({ x: tag.tx + Math.random() * 230, y: tag.ty + Math.random() * 46, vx: 1.5 + Math.random() * 4, vy: -2 + Math.random() * 3, life: 1, c: Math.random() < .7 ? pink : cyan, s: 3 + Math.random() * 5 });
      }
      if (t < .05) tag.dead = false;
      const visible = !tag.dead || reset > 0;
      if (!visible) continue;
      alive++;
      const a = tag.dead ? reset : 1;
      const label = `${tag.k}  `;
      const tw = g.measureText(label + tag.v).width + 28;
      g.globalAlpha = a;
      // Leader line to the anchor on the photo.
      g.strokeStyle = sev[tag.sev]; g.lineWidth = 1.5; g.setLineDash([5, 5]);
      g.beginPath(); g.moveTo(tag.tx + tw / 2, tag.ty + 23); g.lineTo(tag.ax, tag.ay); g.stroke(); g.setLineDash([]);
      g.fillStyle = sev[tag.sev]; g.beginPath(); g.arc(tag.ax, tag.ay, 6, 0, Math.PI * 2); g.fill();
      g.globalAlpha = a * .35; g.beginPath(); g.arc(tag.ax, tag.ay, 12 + Math.sin(now / 200) * 3, 0, Math.PI * 2); g.fill();
      g.globalAlpha = a;
      g.fillStyle = panel; roundRect(tag.tx, tag.ty, tw, 46, 12); g.fill();
      g.strokeStyle = sev[tag.sev]; g.lineWidth = 1.5; g.stroke();
      // Glitch jitter just before the beam reaches it.
      const near = scanning && !tag.dead && tag.ay - beamY < 60;
      const jx = near ? (Math.random() - .5) * 6 : 0;
      g.fillStyle = sev[tag.sev]; g.fillText(tag.k, tag.tx + 14 + jx, tag.ty + 29);
      g.fillStyle = white; g.fillText(tag.v, tag.tx + 14 + g.measureText(label).width + jx, tag.ty + 29);
      g.globalAlpha = 1;
    }
    // Beam.
    if (scanning) {
      const grd = g.createLinearGradient(0, beamY - 70, 0, beamY + 10);
      grd.addColorStop(0, "transparent"); grd.addColorStop(1, pink);
      g.globalAlpha = .35; g.fillStyle = grd; g.fillRect(photo.x - 40, beamY - 70, photo.w + 80, 80); g.globalAlpha = 1;
      g.shadowColor = pink; g.shadowBlur = 30; g.fillStyle = pink; g.fillRect(photo.x - 50, beamY - 4, photo.w + 100, 8); g.shadowBlur = 0;
    }
    // Sparks.
    sparks = sparks.filter((s) => s.life > 0);
    for (const s of sparks) {
      s.x += s.vx; s.y += s.vy; s.vy -= .04; s.life -= .018;
      g.globalAlpha = Math.max(0, s.life); g.fillStyle = s.c; g.fillRect(s.x, s.y, s.s, s.s);
    }
    g.globalAlpha = 1;
    // Clean stamp.
    if (t > .64 && t < .92) {
      const k = Math.min(1, (t - .64) / .06);
      g.save(); g.translate(W / 2, photo.y + photo.h / 2); g.rotate(-.12); g.scale(.6 + k * .4, .6 + k * .4);
      g.globalAlpha = k; g.strokeStyle = cyan; g.lineWidth = 8; roundRect(-190, -62, 380, 124, 18); g.stroke();
      g.fillStyle = cyan; g.font = "700 64px JetBrains Mono, monospace"; g.textAlign = "center"; g.fillText("✔ CLEAN", 0, 22);
      g.restore(); g.textAlign = "left"; g.globalAlpha = 1;
    }
    hudState.textContent = t < .12 ? "ARMED" : scanning ? "SCRUBBING" : t < .92 ? "VERIFIED" : "RESET";
    hudState.className = !scanning && t > .62 && t < .92 ? "ok" : "";
    hudCount.textContent = `${alive} LEAK${alive === 1 ? "" : "S"}`;
    if (!reduceMotion) requestAnimationFrame(draw);
  }
  requestAnimationFrame(draw);
})();
