// The hero terminal: types a real cybermeta session (audit → scrub →
// audit again), then loops. Output matches cybermeta 0.3.1.
"use strict";
(() => {
  const pre = document.querySelector("#termOut");
  if (!pre) return;
  const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const S = [
    ["cmd", "cybermeta audit IMG_2041.jpg"],
    ["out", `IMG_2041.jpg  <span class="r">[HIGH]</span> JPEG
  <span class="r">high  </span> location  EXIF         GPS position: 45.515200, -122.678400
  <span class="r">high  </span> device    EXIF         Camera serial number: R5CX21B7QKM
  <span class="r">high  </span> person    EXIF         Camera owner: Jamie R.
  <span class="r">high  </span> location  XMP          City: Portland
  <span class="r">high  </span> person    IPTC         By-line (creator): Jamie R.
  <span class="r">high  </span> hidden    container    Motion-photo video: 2.1 MB
  <span class="o">medium</span> time      EXIF         Taken: 2026-10-18 19:38:44
  <span class="o">medium</span> thumbnail EXIF         Embedded thumbnail
  <span class="c">low   </span> device    EXIF         Camera model: SM-S938B
  <span class="m">map: https://www.openstreetmap.org/?mlat=45.515200&amp;mlon=-122.678400</span>`],
    ["cmd", "cybermeta scrub IMG_2041.jpg"],
    ["out", `wrote IMG_2041.cybermeta.jpg — cleared 9 finding(s), removed 5 block(s) (2.2 MB)
  - EXIF [APP1 Exif] 18.4 KB
  - XMP [APP1 XMP] 6.1 KB
  - IPTC / Photoshop [APP13 Photoshop / IPTC] 312 bytes
  - Multi-Picture index [APP2 MPF] 94 bytes
  - data after end of image [after EOI] 2.1 MB
  <span class="o">note: Data hidden after the image (e.g. a motion-photo video) was removed; the photo stays.</span>
  <span class="o">note: Kept: colour profile, rotation.</span>`],
    ["cmd", "cybermeta audit IMG_2041.cybermeta.jpg"],
    ["out", `IMG_2041.cybermeta.jpg  <span class="g">[CLEAN]</span> JPEG`],
  ];
  let html = "";
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  async function run() {
    for (;;) {
      html = "";
      for (const [kind, text] of S) {
        if (kind === "cmd") {
          html += `<span class="p">❯</span> `;
          for (const ch of text) {
            html += ch;
            pre.innerHTML = html + '<span class="caret"></span>';
            await sleep(reduce ? 0 : 28 + Math.random() * 40);
          }
          html += "\n";
          await sleep(reduce ? 0 : 380);
        } else {
          for (const line of text.split("\n")) {
            html += line + "\n";
            pre.innerHTML = html + '<span class="caret"></span>';
            await sleep(reduce ? 0 : 70);
          }
          await sleep(reduce ? 0 : 1100);
        }
      }
      if (reduce) return;
      await sleep(5000);
    }
  }
  run();
})();
