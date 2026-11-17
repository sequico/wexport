---
name: offline html archive
description: Build an archive that a browser opens from the file system with no server, no install and no network. Use when producing self-contained HTML archives, chunked data for file:// pages, virtualised timelines or long offline documents.
---

# Offline HTML archives (`file://`)

An archive is a folder the recipient opens by double-clicking `index.html`. Nothing is installed, no
server runs, no request leaves the machine. That environment decides the design; this skill is the
technique, `docs/implementation-plan.md` §6 is the spec for wexport.

## What `file://` permits

| Mechanism | Available | Consequence |
|---|---|---|
| classic `<script src>` | yes | the only way to load data |
| `fetch` / XHR | no (origin is `null`) | no JSON files, no `.db`, no manifest fetches |
| ES modules | no (CORS) | one classic script, IIFE, explicit globals |
| Web Workers | no (or unreliable) | parse on the main thread; keep chunks small |
| `localStorage` / `sessionStorage` | unreliable | treat as absent; state lives in the URL hash |
| `<img>`, `<video>`, `<audio>`, `<source>` | yes | relative media paths work, video seeks |
| `<track>` (subtitles) | unreliable (CORS) | avoid, or inline cues |
| fonts, icons, stylesheets | yes, if local | no CDN, ever |

Verify the matrix in Chrome, Firefox and Edge by double-clicking the real folder — not by running a
dev server, which proves a different environment.

## Data as chunks

```js
// data/2026-11.js
WEX.chunk("2026-11", [
  { id: 1, ts: 1763382725000, tsl: "2026-11-17 14:32:05 +01:00", kind: "text", text: "ciao" },
]);
```

- One chunk per time bucket (a month is the usual size), newest first.
- Emit the data as a **literal script**, not as JSON: no parse step, no `JSON.parse` of a large
  string, no escaping surprises beyond the one below.
- Escape `<` as `\u003c` in every string. Message text containing `</script>` must not be able to
  close the tag. Escape `\u2028` and `\u2029` too: they break JavaScript string literals.
- A `manifest.js` chunk describes the archive (header, chunk list, counts, build time, versions) so
  the first paint does not need any other chunk.

## Loading chunks lazily

```js
function loadChunk(name) {
  return new Promise((resolve, reject) => {
    const s = document.createElement("script");
    s.src = "data/" + name + ".js";
    s.onload = resolve;
    s.onerror = () => reject(new Error("chunk " + name + " missing"));
    document.head.append(s);
  });
}
```

Load the newest chunk on open, the previous one when the user scrolls near the top, and release
chunks that leave the window. Without release, a 300k-message conversation ends as an out-of-memory
tab on an old laptop, no matter how well the DOM is virtualised.

## Rendering a very long timeline

- Virtualise: keep only the rows around the viewport in the DOM (a few hundred nodes), position them
  with a spacer element whose height comes from the accumulated row heights.
- Precompute row heights at build time when content allows it (text rows); estimate for media and
  correct once measured.
- Day separators and absolute timestamps are part of the row, not decoration added later.
- Every visible time is a **string**, not a `Date` computation. Store epoch milliseconds for sorting
  and comparison only.

## Media

- Relative paths, no leading slash, no `file://` prefix: `media/ab/ab12….mp4`.
- Content-addressed filenames (`<sha256><ext>`): dedupe inside the archive, short names, no Windows
  path problems, no renaming when a chat is renamed.
- Generate derivatives at build time (thumbnail, poster frame, waveform peaks) and reference them;
  never decode media in the browser to draw a UI.
- Voice notes: draw the waveform from a precomputed peak array, then attach one `<audio
  preload="none">`. Decoding to draw is how a gallery of 400 voice notes freezes.
- Video: `preload="metadata"` plus a poster. Loading many videos eagerly is the other freezing bug.
- `loading="lazy"` on images, `decoding="async"`, and explicit `width`/`height` so the timeline does
  not jump while scrolling.

## Durability

The archive has to be readable in ten years, by a browser that no longer remembers today:

- No external references of any kind. If it is not in the folder, it does not exist.
- Absolute timestamps with explicit UTC offset, produced at build; never "today", never a locale
  format, never a value recomputed by the viewer.
- Keep the plain structures: HTML, CSS, one classic script, one JSON-free data format. No build-time
  framework in the output.
- Ship a `README.txt` inside the folder saying what it is, when it was built, and how to open it.
- Record the source app version, the builder version and the timezone in the manifest.

## Checklist before an archive is shipped

- [ ] Double-clicked in Chrome, Firefox and Edge, from a path containing a space.
- [ ] Opened after being zipped, unzipped elsewhere, and opened again.
- [ ] No request in the network panel; no `file://` fetch errors in the console.
- [ ] Scrolled to the oldest message of the longest chat without a memory blow-up.
- [ ] A video seeks; a voice note plays; a missing media file shows a placeholder, not a broken icon.
- [ ] Every timestamp readable with its offset, identical on a machine in another timezone.
- [ ] Search returns and jumps; a search across unloaded chunks shows progress instead of hanging.
