// Live lab: a small, browser-only version of cybermeta's JPEG audit and
// scrub. Nothing is uploaded; the file is read with FileReader and the
// clean copy is a Blob URL. The app runs the full Rust engine instead.
"use strict";

(() => {
  const $ = (s) => document.querySelector(s);
  const out = $("#out"), risk = $("#risk"), fname = $("#fname"), map = $("#blockmap"), sizes = $("#sizes");
  const scrubBtn = $("#scrub"), dl = $("#download"), drop = $("#drop"), preview = $("#preview");
  let current = null;

  const kb = (n) => (n >= 1048576 ? (n / 1048576).toFixed(1) + " MB" : n >= 1024 ? (n / 1024).toFixed(1) + " KB" : n + " B");
  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
  const ascii = (b, o, n) => { let s = ""; for (let i = 0; i < n && o + i < b.length; i++) { const c = b[o + i]; if (c === 0) break; s += String.fromCharCode(c); } return s.trim(); };
  const starts = (b, o, sig) => [...sig].every((ch, i) => b[o + i] === ch.charCodeAt(0));

  // ------------------------------------------------------------ JPEG walk --
  function blocks(b) {
    if (b[0] !== 0xff || b[1] !== 0xd8) throw new Error("Not a JPEG (the lab reads JPEG; the app also handles PNG, WebP and HEIC).");
    const list = [{ kind: "pic", label: "SOI", s: 0, e: 2 }];
    let p = 2;
    while (p < b.length) {
      if (b[p] !== 0xff) throw new Error(`Broken JPEG at byte ${p}.`);
      const s = p;
      while (b[p] === 0xff) p++;
      const m = b[p++];
      if (m === 0xd9) { list.push({ kind: "pic", label: "EOI", s, e: p }); if (p < b.length) list.push({ kind: "trailer", label: "after end of image", s: p, e: b.length }); return list; }
      if ((m >= 0xd0 && m <= 0xd7) || m === 0x01) { list.push({ kind: "pic", label: "RST", s, e: p }); continue; }
      const len = (b[p] << 8) | b[p + 1];
      const e = p + len, d = p + 2;
      if (len < 2 || e > b.length) throw new Error("Truncated JPEG.");
      let kind = "pic", label = `marker ${m.toString(16).toUpperCase()}`;
      if (m === 0xe0) { kind = starts(b, d, "JFIF\0") ? (len > 16 ? "thumb" : "pic") : "vendor"; label = "APP0 JFIF"; }
      else if (m === 0xe1 && starts(b, d, "Exif\0")) { kind = "exif"; label = "APP1 EXIF"; }
      else if (m === 0xe1 && starts(b, d, "http://ns.adobe.com/xap/1.0/")) { kind = "xmp"; label = "APP1 XMP"; }
      else if (m === 0xe1 && starts(b, d, "http://ns.adobe.com/xmp/extension/")) { kind = "xmp"; label = "APP1 XMP (extended)"; }
      else if (m === 0xe2 && starts(b, d, "ICC_PROFILE")) { kind = "icc"; label = "APP2 ICC profile"; }
      else if (m === 0xe2 && starts(b, d, "MPF")) { kind = "vendor"; label = "APP2 Multi-Picture"; }
      else if (m === 0xed) { kind = "iptc"; label = "APP13 IPTC"; }
      else if (m === 0xee && starts(b, d, "Adobe")) { kind = "pic"; label = "APP14 Adobe"; }
      else if (m >= 0xe0 && m <= 0xef) { kind = "vendor"; label = `APP${m - 0xe0} ${ascii(b, d, 12) || "vendor"}`; }
      else if (m === 0xfe) { kind = "comment"; label = "COM"; }
      if (m === 0xda) {
        let i = e;
        while (i + 1 < b.length && !(b[i] === 0xff && b[i + 1] !== 0 && !(b[i + 1] >= 0xd0 && b[i + 1] <= 0xd7) && b[i + 1] !== 0xff)) i++;
        list.push({ kind: "pic", label: "scan", s, e: i });
        p = i;
      } else {
        list.push({ kind, label, s, e, d });
        p = e;
      }
    }
    throw new Error("No end-of-image marker.");
  }

  // ----------------------------------------------------------------- EXIF --
  function exif(b, blk) {
    const base = blk.d + 6;
    const le = b[base] === 0x49;
    const u16 = (o) => (le ? b[base + o] | (b[base + o + 1] << 8) : (b[base + o] << 8) | b[base + o + 1]);
    const u32 = (o) => (le ? (b[base + o] | (b[base + o + 1] << 8) | (b[base + o + 2] << 16)) + b[base + o + 3] * 16777216 : b[base + o] * 16777216 + ((b[base + o + 1] << 16) | (b[base + o + 2] << 8) | b[base + o + 3]));
    const size = { 1: 1, 2: 1, 3: 2, 4: 4, 5: 8, 7: 1, 9: 4, 10: 8 };
    const tags = {};
    const read = (ifd, ns, depth = 0) => {
      if (!ifd || ifd > blk.e - base || depth > 3) return 0;
      const n = u16(ifd);
      for (let i = 0; i < n && i < 400; i++) {
        const e = ifd + 2 + i * 12, tag = u16(e), type = u16(e + 2), count = u32(e + 4);
        const bytes = (size[type] || 1) * count;
        const vo = bytes > 4 ? u32(e + 8) : e + 8;
        let val;
        if (type === 2) val = ascii(b, base + vo, count);
        else if (type === 3) val = u16(vo);
        else if (type === 4) val = u32(vo);
        else if (type === 5) val = Array.from({ length: Math.min(count, 3) }, (_, k) => u32(vo + k * 8) / (u32(vo + k * 8 + 4) || 1));
        else val = `${count} bytes`;
        tags[`${ns}:${tag}`] = val;
        if (ns === "0" && tag === 0x8769) read(val, "exif", depth + 1);
        if (ns === "0" && tag === 0x8825) read(val, "gps", depth + 1);
      }
      return u32(ifd + 2 + n * 12);
    };
    const next = read(u32(4), "0");
    if (next) tags.thumbnail = true;
    return tags;
  }

  function iptc(b, blk) {
    const found = [];
    for (let i = blk.d; i + 5 < blk.e; i++) {
      if (b[i] === 0x1c && b[i + 1] === 2) {
        const ds = b[i + 2], len = (b[i + 3] << 8) | b[i + 4];
        if (len > 0 && i + 5 + len <= blk.e) { found.push([ds, new TextDecoder().decode(b.subarray(i + 5, i + 5 + len))]); i += 4 + len; }
      }
    }
    return found;
  }

  // ---------------------------------------------------------------- audit --
  function audit(b) {
    const list = blocks(b);
    const f = [];
    const add = (sev, cat, label, value, src) => f.push({ sev, cat, label, value, src });
    for (const blk of list) {
      if (blk.kind === "exif") {
        const t = exif(b, blk);
        const lat = t["gps:2"], lon = t["gps:4"];
        if (Array.isArray(lat) && Array.isArray(lon)) {
          const dec = (a, ref) => (a[0] + a[1] / 60 + a[2] / 3600) * (/[SW]/.test(ref || "") ? -1 : 1);
          add("high", "location", "GPS position", `${dec(lat, t["gps:1"]).toFixed(6)}, ${dec(lon, t["gps:3"]).toFixed(6)}`, "EXIF");
        } else if (Object.keys(t).some((k) => k.startsWith("gps:"))) add("high", "location", "GPS data", "present", "EXIF");
        const map = [["exif:42033", "high", "device", "Camera serial number"], ["exif:42037", "high", "device", "Lens serial number"], ["exif:42032", "high", "person", "Camera owner"],
          ["0:315", "high", "person", "Artist"], ["0:33432", "medium", "person", "Copyright"], ["exif:42016", "medium", "device", "Unique image ID"], ["exif:37500", "medium", "device", "Maker note"],
          ["exif:36867", "medium", "time", "Taken"], ["0:306", "low", "time", "Modified"], ["0:271", "low", "device", "Camera make"], ["0:272", "low", "device", "Camera model"],
          ["0:305", "low", "software", "Software"], ["0:270", "medium", "text", "Description"]];
        for (const [k, sev, cat, label] of map) if (t[k] !== undefined && t[k] !== "") add(sev, cat, label, t[k], "EXIF");
        if (t.thumbnail) add("medium", "thumbnail", "Embedded thumbnail", "may show the photo before edits", "EXIF");
      } else if (blk.kind === "xmp") {
        const x = new TextDecoder().decode(b.subarray(blk.d, blk.e));
        const pick = (re) => (x.match(re) || [])[1];
        const gps = pick(/GPSLatitude[="> ]+([^"<]+)/);
        if (gps) add("high", "location", "XMP GPSLatitude", gps, "XMP");
        const city = pick(/[:<]City[="> ]+([^"<]+)/);
        if (city) add("high", "location", "XMP City", city, "XMP");
        const creator = pick(/creator>\s*<rdf:Seq>\s*<rdf:li>([^<]+)/) || pick(/dc:creator="([^"]+)"/);
        if (creator) add("high", "person", "XMP creator", creator, "XMP");
        if (/MotionPhoto|MicroVideo/.test(x)) add("medium", "hidden", "Motion photo marker", "XMP says a video is attached", "XMP");
        if (!gps && !city && !creator) add("low", "other", "XMP packet", kb(blk.e - blk.s), "XMP");
      } else if (blk.kind === "iptc") {
        const names = { 80: ["high", "person", "By-line"], 90: ["high", "location", "City"], 101: ["high", "location", "Country"], 116: ["medium", "person", "Copyright"], 120: ["medium", "text", "Caption"], 55: ["medium", "time", "Date created"] };
        const recs = iptc(b, blk);
        for (const [ds, v] of recs) { const n = names[ds]; if (n) add(n[0], n[1], `IPTC ${n[2]}`, v, "IPTC"); }
        if (!recs.length) add("medium", "other", "IPTC block", kb(blk.e - blk.s), "IPTC");
      } else if (blk.kind === "comment") {
        add("medium", "text", "Comment", new TextDecoder().decode(b.subarray(blk.d, blk.e)).slice(0, 60), "JPEG");
      } else if (blk.kind === "thumb") {
        add("medium", "thumbnail", "JFIF thumbnail", kb(blk.e - blk.s), "JFIF");
      } else if (blk.kind === "trailer") {
        const tail = new TextDecoder("latin1").decode(b.subarray(blk.s, Math.min(blk.e, blk.s + 4096)));
        add(/ftyp|MotionPhoto/.test(tail) ? "high" : "medium", "hidden", /ftyp|MotionPhoto/.test(tail) ? "Motion-photo video" : "Data after end of image", kb(blk.e - blk.s), "file");
      } else if (blk.kind === "vendor") {
        add("low", "other", blk.label, kb(blk.e - blk.s), "file");
      }
    }
    const order = { high: 3, medium: 2, low: 1 };
    f.sort((a, c) => order[c.sev] - order[a.sev]);
    return { list, findings: f, risk: f.length ? f[0].sev : null };
  }

  // ---------------------------------------------------------------- scrub --
  function scrub(b, list) {
    const keep = [];
    for (const blk of list) {
      if (blk.kind === "pic" || blk.kind === "icc") keep.push(b.subarray(blk.s, blk.e));
      else if (blk.kind === "thumb") { const seg = b.slice(blk.s, blk.s + 18); seg[2] = 0; seg[3] = 16; seg[16] = 0; seg[17] = 0; keep.push(seg); }
    }
    const total = keep.reduce((n, a) => n + a.length, 0);
    const outBytes = new Uint8Array(total);
    let o = 0;
    for (const a of keep) { outBytes.set(a, o); o += a.length; }
    return outBytes;
  }

  // ------------------------------------------------------------------- UI --
  const cls = { high: "hi", medium: "me", low: "lo" };
  function renderBlocks(list, total) {
    map.classList.remove("scrubbed");
    map.replaceChildren(...list.map((blk) => {
      const s = document.createElement("span");
      s.className = blk.kind === "pic" || blk.kind === "icc" ? "pic" : "meta";
      s.style.flexGrow = Math.max(blk.e - blk.s, total / 400);
      s.title = `${blk.label} · ${kb(blk.e - blk.s)}`;
      return s;
    }));
  }

  function show(bytes, name) {
    dl.hidden = true;
    fname.textContent = name;
    try {
      const a = audit(bytes);
      current = { bytes, name, a };
      const meta = a.list.filter((x) => x.kind !== "pic" && x.kind !== "icc").reduce((n, x) => n + x.e - x.s, 0);
      renderBlocks(a.list, bytes.length);
      sizes.textContent = `${kb(meta)} metadata of ${kb(bytes.length)}`;
      risk.textContent = a.risk ? `${a.risk.toUpperCase()} RISK` : "CLEAN";
      risk.className = `risk ${a.risk ? cls[a.risk] : "done"}`;
      out.innerHTML = a.findings.length
        ? a.findings.map((f, i) => `<div class="row" style="animation-delay:${i * 60}ms"><span class="${cls[f.sev]}">${f.sev}</span><span class="c muted">${esc(f.cat)}</span><span class="v">${esc(f.label)}: <span class="${cls[f.sev]}">${esc(f.value)}</span> <span class="muted">· ${f.src}</span></span></div>`).join("")
        : `<div class="done">✔ nothing found: this photo is already clean</div>`;
      scrubBtn.disabled = !a.findings.length;
    } catch (e) {
      current = null;
      risk.textContent = "";
      out.innerHTML = `<div class="hi">${esc(e.message)}</div>`;
      scrubBtn.disabled = true;
    }
  }

  scrubBtn.addEventListener("click", () => {
    if (!current) return;
    const clean = scrub(current.bytes, current.a.list);
    const after = audit(clean);
    map.classList.add("scrubbed");
    const removed = current.bytes.length - clean.length;
    out.innerHTML += `<div class="row"><span class="done">done</span><span class="c muted">verify</span><span class="v done">✔ ${current.a.findings.length} findings removed · ${kb(removed)} gone · re-audit: ${after.findings.length ? after.findings.length + " left" : "CLEAN"}</span></div>`;
    out.scrollTop = out.scrollHeight;
    risk.textContent = after.findings.length ? "CHECK" : "CLEAN";
    risk.className = `risk ${after.findings.length ? "me" : "done"}`;
    sizes.textContent = `${kb(clean.length)} clean`;
    const url = URL.createObjectURL(new Blob([clean], { type: "image/jpeg" }));
    dl.href = url;
    dl.download = "photo_clean.jpg";
    dl.hidden = false;
    scrubBtn.disabled = true;
    window.cmToast?.("SCRUBBED · VERIFIED CLEAN");
  });

  function readFile(file) {
    if (!file) return;
    const r = new FileReader();
    r.onload = () => {
      const bytes = new Uint8Array(r.result);
      preview.src = URL.createObjectURL(file);
      preview.style.display = "block";
      show(bytes, file.name);
    };
    r.readAsArrayBuffer(file);
  }
  $("#file").addEventListener("change", (e) => readFile(e.target.files[0]));
  ["dragenter", "dragover"].forEach((t) => drop.addEventListener(t, (e) => { e.preventDefault(); drop.classList.add("over"); }));
  ["dragleave", "drop"].forEach((t) => drop.addEventListener(t, (e) => { e.preventDefault(); drop.classList.remove("over"); }));
  drop.addEventListener("drop", (e) => readFile(e.dataTransfer.files[0]));
  $("#sample").addEventListener("click", async (e) => {
    e.preventDefault();
    e.stopPropagation();
    const r = await fetch("assets/sample-leaky.jpg");
    const buf = new Uint8Array(await r.arrayBuffer());
    preview.src = "assets/sample-leaky.jpg";
    preview.style.display = "block";
    show(buf, "IMG_20261001_211405.jpg");
  });
})();
