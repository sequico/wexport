/* wexport archive viewer.
 *
 * Classic script, no modules, no fetch, no workers, no storage: everything here works when the
 * folder is opened by double-clicking index.html. See docs/implementation-plan.md §6.
 */
(function () {
  "use strict";

  var timeline = document.getElementById("timeline");
  var rows = document.getElementById("rows");
  var statusLine = document.getElementById("status");
  var hits = document.getElementById("hits");
  var titleEl = document.getElementById("title");
  var subtitleEl = document.getElementById("subtitle");

  var manifest = WEX.manifestData;
  if (!manifest || !manifest.chunks || !manifest.chunks.length) {
    titleEl.textContent = "Empty archive";
    subtitleEl.textContent = "data/manifest.js is missing or lists no messages.";
    return;
  }

  var chunks = manifest.chunks;
  var starts = [];
  var total = 0;
  chunks.forEach(function (chunk) {
    starts.push(total);
    total += chunk.c;
  });

  var LIMIT = 300;      // rows in the DOM at most (plan §6: <= 250 nodes target, a little slack)
  var STEP = 80;
  var start = Math.max(0, total - 120);
  var end = total;
  var searching = false;

  titleEl.textContent = manifest.title;
  document.title = manifest.title + " — wexport archive";
  var parts = [total.toLocaleString("en") + " messages"];
  if (manifest.is_group && manifest.people) parts.push(manifest.people + " participants");
  if (manifest.first && manifest.last) parts.push(manifest.first + " → " + manifest.last);
  parts.push(manifest.tz);
  if (manifest.missing) parts.push(manifest.missing + " media not copied");
  subtitleEl.textContent = parts.join(" · ");

  function status(text) {
    statusLine.textContent = text || "";
  }

  /* ---------------------------------------------------------------- chunk loading */

  function chunkIndexFor(index) {
    var low = 0;
    var high = starts.length - 1;
    while (low < high) {
      var mid = (low + high + 1) >> 1;
      if (starts[mid] <= index) low = mid;
      else high = mid - 1;
    }
    return low;
  }

  function loadChunk(name) {
    return new Promise(function (resolve, reject) {
      if (WEX.chunks[name]) return resolve();
      var script = document.createElement("script");
      script.src = "data/" + name + ".js";
      // The element goes as soon as it has done its job: a scroll up and down the same chunk would
      // otherwise leave one script tag per visit in the head.
      script.onload = function () {
        script.remove();
        if (WEX.chunks[name]) resolve();
        else reject(new Error(name));
      };
      script.onerror = function () {
        script.remove();
        reject(new Error(name));
      };
      document.head.appendChild(script);
    });
  }

  function loadRange(from, to) {
    var first = chunkIndexFor(Math.max(0, from));
    var last = chunkIndexFor(Math.max(0, Math.min(total - 1, to)));
    var jobs = [];
    for (var i = first; i <= last; i++) jobs.push(loadChunk(chunks[i].n));
    return Promise.all(jobs);
  }

  function messageAt(index) {
    var ci = chunkIndexFor(index);
    var list = WEX.chunks[chunks[ci].n];
    if (!list) return null;
    return list[index - starts[ci]] || null;
  }

  /* ---------------------------------------------------------------- row building */

  function dayLabel(isoDate) {
    // Pure date arithmetic on a frozen string: no timezone involved, no locale surprises.
    var bits = isoDate.split("-");
    var date = new Date(Date.UTC(+bits[0], +bits[1] - 1, +bits[2]));
    var days = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    var months = ["January", "February", "March", "April", "May", "June", "July", "August",
      "September", "October", "November", "December"];
    return days[date.getUTCDay()] + ", " + (+bits[2]) + " " + months[+bits[1] - 1] + " " + bits[0];
  }

  function utcOf(epoch) {
    return new Date(epoch).toISOString();
  }

  function el(tag, cls, text) {
    var node = document.createElement(tag);
    if (cls) node.className = cls;
    if (text != null) node.textContent = text;
    return node;
  }

  function mediaNode(message) {
    var media = message.md;
    if (!media) return null;
    if (media.mi || !media.p) {
      var missing = el("div", "body", "[" + message.k + " — file not in the copied folder]");
      missing.classList.add("missing");
      return missing;
    }
    var wrap = document.createElement("div");
    var kind = message.k;
    if (kind === "image" || kind === "sticker") {
      var img = document.createElement("img");
      img.className = "thumb";
      img.loading = "lazy";
      img.decoding = "async";
      img.alt = media.nm || "";
      img.src = media.th || media.p;
      // Keep the aspect ratio when it is known; leave small or unknown images to CSS.
      if (media.w > 32 && media.h) {
        img.width = Math.min(media.w, kind === "sticker" ? 140 : 480);
        img.height = Math.round((img.width * media.h) / media.w);
      }
      var link = document.createElement("a");
      link.href = media.p;
      link.target = "_blank";
      link.rel = "noopener";
      link.appendChild(img);
      wrap.appendChild(link);
      if (kind === "sticker") wrap.className = "sticker";
      return wrap;
    }
    if (kind === "video" || kind === "gif") {
      var video = document.createElement("video");
      video.controls = true;
      video.preload = "metadata";
      if (media.th) video.poster = media.th;
      if (kind === "gif") {
        video.autoplay = true;
        video.loop = true;
        video.muted = true;
      }
      video.src = media.alt || media.p;
      wrap.appendChild(video);
      return wrap;
    }
    if (kind === "audio") {
      var wave = document.createElement("div");
      wave.className = "wave";
      var canvas = document.createElement("canvas");
      canvas.width = 220;
      canvas.height = 34;
      drawWave(canvas, media.pk || []);
      var audio = document.createElement("audio");
      audio.controls = true;
      audio.preload = "none";
      audio.src = media.alt || media.p;
      wave.appendChild(canvas);
      wave.appendChild(audio);
      wrap.appendChild(wave);
      return wrap;
    }
    var doc = document.createElement("div");
    doc.className = "doc";
    var name = media.nm || (media.p || "").split("/").pop();
    var a = document.createElement("a");
    a.href = media.p;
    a.textContent = name;
    a.target = "_blank";
    a.rel = "noopener";
    doc.appendChild(a);
    if (media.sz) doc.appendChild(el("span", null, " (" + humanSize(media.sz) + ")"));
    wrap.appendChild(doc);
    return wrap;
  }

  function humanSize(bytes) {
    if (bytes < 1024) return bytes + " B";
    if (bytes < 1024 * 1024) return Math.round(bytes / 1024) + " kB";
    if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + " MB";
    return (bytes / (1024 * 1024 * 1024)).toFixed(1) + " GB";
  }

  function drawWave(canvas, peaks) {
    var context = canvas.getContext("2d");
    var width = canvas.width;
    var height = canvas.height;
    var style = getComputedStyle(document.body).color;
    context.strokeStyle = style;
    context.globalAlpha = 0.55;
    context.beginPath();
    var count = peaks.length || 0;
    for (var i = 0; i < count; i++) {
      var x = (i / count) * width;
      var amplitude = (peaks[i] / 100) * (height / 2);
      context.moveTo(x, height / 2 - amplitude);
      context.lineTo(x, height / 2 + amplitude);
    }
    context.stroke();
    context.globalAlpha = 1;
  }

  function bodyNode(message) {
    if (message.k === "deleted") return el("div", "body", "message deleted");
    if (message.k === "system") return el("div", "body system", message.x || "system message");
    if (message.k === "call") {
      var call = message.pl || {};
      var bits = [];
      if (call.missed) bits.push("missed");
      bits.push(call.video ? "video call" : "voice call");
      if (call.duration_s) bits.push(call.duration_s + "s");
      return el("div", "body call", "☎ " + bits.join(", "));
    }
    if (message.k === "location" && message.pl) {
      return el("div", "body", "📍 " + message.pl.lat + ", " + message.pl.lon);
    }
    if (message.k === "contact") {
      return el("div", "body", "👤 " + ((message.pl && message.pl.name) || "contact"));
    }
    var node = mediaNode(message);
    if (message.x) {
      var text = el("div", "body", message.x);
      if (node) {
        var holder = document.createElement("div");
        holder.appendChild(text);
        holder.appendChild(node);
        return holder;
      }
      return text;
    }
    return node || el("div", "body", "");
  }

  function rowFor(message, index) {
    var row = el("article", "msg" + (message.f ? " mine" : "") + (message.k === "deleted" ? " deleted" : ""));
    row.dataset.i = String(index);
    row.title = utcOf(message.t) + "  (" + message.t + ")";

    var meta = el("div", "meta");
    if (message.a && !message.f) meta.appendChild(el("span", "who", message.a));
    meta.appendChild(el("span", "when", message.s));
    if (message.e) meta.appendChild(el("span", null, "edited"));
    if (message.st && message.f) meta.appendChild(el("span", null, statusWord(message.st)));
    row.appendChild(meta);

    row.appendChild(bodyNode(message));

    if (message.q) row.appendChild(el("div", "meta", "↳ replying to #" + message.q));
    if (message.rx && message.rx.length) {
      var reactions = el("div", "reactions", message.rx.join(" "));
      row.appendChild(reactions);
    }
    return row;
  }

  function statusWord(code) {
    // shortcut: WhatsApp's status codes are not documented; these bands match what real backups
    // show (1/5/13). Revisit when a real dump proves a different mapping.
    if (code >= 13) return "read";
    if (code >= 5) return "delivered";
    if (code >= 1) return "sent";
    return "";
  }

  /* ---------------------------------------------------------------- rendering */

  var renderToken = 0;
  var searchToken = 0;

  function fold(text) {
    // Search ignores case and accents: "caffe" finds "caffè".
    return text.normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLowerCase();
  }

  function render(anchor) {
    var token = ++renderToken;
    status("loading…");
    return loadRange(start, end - 1)
      .then(function () {
        if (token !== renderToken) return;
        paint(anchor);
        releaseChunks();
        status("");
      })
      .catch(function (error) {
        status("could not load " + error.message + " — the archive is incomplete");
      });
  }

  function releaseChunks() {
    // Plan §6: chunks far from the viewport are released, or a 300k-message chat fills the tab.
    if (searching) return;
    var first = chunkIndexFor(start);
    var last = chunkIndexFor(Math.max(start, end - 1));
    for (var i = 0; i < chunks.length; i++) {
      if (i < first - 1 || i > last + 1) delete WEX.chunks[chunks[i].n];
    }
  }

  function paint(anchor) {
    var previous = rows.offsetHeight;
    var previousDate = null;
    var fragment = document.createDocumentFragment();
    for (var i = start; i < end; i++) {
      var message = messageAt(i);
      if (!message) {
        var pending = el("article", "msg pending", "loading…");
        pending.dataset.i = String(i);
        fragment.appendChild(pending);
        continue;
      }
      var date = (message.s || "").slice(0, 10);
      if (date !== previousDate) {
        fragment.appendChild(el("div", "day", dayLabel(date)));
        previousDate = date;
      }
      fragment.appendChild(rowFor(message, i));
    }
    rows.replaceChildren(fragment);
    var delta = rows.offsetHeight - previous;
    if (anchor === "top") timeline.scrollTop += delta;
  }

  function clampWindow(direction) {
    while (end - start > LIMIT) {
      if (direction === "down") {
        var before = rows.offsetHeight;
        start += 40;
        paint("none");
        timeline.scrollTop += rows.offsetHeight - before;
      } else {
        end -= 40;
        paint("none");
      }
    }
  }

  var scheduled = false;

  function onScroll() {
    if (scheduled) return;
    scheduled = true;
    requestAnimationFrame(function () {
      scheduled = false;
      var nearTop = timeline.scrollTop < 300;
      var nearBottom = timeline.scrollTop + timeline.clientHeight > timeline.scrollHeight - 300;
      if (nearTop && start > 0) {
        start = Math.max(0, start - STEP);
        render("top").then(function () { clampWindow("up"); });
      } else if (nearBottom && end < total) {
        end = Math.min(total, end + STEP);
        render("bottom").then(function () { clampWindow("down"); });
      }
    });
  }

  /* ---------------------------------------------------------------- search and jump */

  function search(term) {
    term = term.trim();
    var token = ++searchToken;
    // A new search (or an empty one) stops whatever was running, and lets chunks be released again.
    searching = false;
    hits.hidden = true;
    hits.replaceChildren();
    if (!term) return;
    var needle = fold(term);
    var found = [];
    var index = 0;
    searching = true;

    function step() {
      if (token !== searchToken) return; // a newer search owns the list now
      if (index >= chunks.length) {
        searching = false;
        showResults(term, found);
        return;
      }
      var chunk = chunks[index];
      status("searching " + (index + 1) + "/" + chunks.length + " …");
      loadChunk(chunk.n)
        .then(function () {
          if (token !== searchToken) return;
          var list = WEX.chunks[chunk.n] || [];
          for (var i = 0; i < list.length; i++) {
            var text = list[i].x;
            if (text && fold(text).indexOf(needle) !== -1) {
              found.push({ index: starts[index] + i, message: list[i] });
              if (found.length >= 200) {
                index = chunks.length;
                break;
              }
            }
          }
          index += 1;
          step();
        })
        .catch(function () {
          if (token !== searchToken) return;
          status("chunk " + chunk.n + " missing");
          index += 1;
          step();
        });
    }
    step();
  }

  function showResults(term, found) {
    status(found.length ? found.length + (found.length === 200 ? "+ matches" : " matches") : "no match");
    if (!found.length) return;
    var fragment = document.createDocumentFragment();
    found.forEach(function (hit) {
      var button = document.createElement("button");
      var text = (hit.message.x || "").slice(0, 120);
      button.appendChild(document.createTextNode(text));
      button.appendChild(el("small", null, hit.message.s));
      button.addEventListener("click", function () {
        jumpTo(hit.index);
        hits.hidden = true;
      });
      fragment.appendChild(button);
    });
    hits.replaceChildren(fragment);
    hits.hidden = false;
  }

  function jumpTo(index) {
    start = Math.max(0, index - 15);
    end = Math.min(total, start + LIMIT);
    render("none").then(function () {
      var row = rows.querySelector('[data-i="' + index + '"]');
      if (row) row.scrollIntoView({ block: "center" });
    });
  }

  function jumpToDate(value) {
    var target = new Date(value + "T00:00:00Z").getTime();
    for (var index = 0; index < chunks.length; index++) {
      if (chunks[index].l >= target) {
        var chosen = index;  // captured explicitly: the callback runs after this function returns
        loadChunk(chunks[chosen].n).then(function () {
          var list = WEX.chunks[chunks[chosen].n] || [];
          for (var i = 0; i < list.length; i++) {
            if (list[i].t >= target) {
              jumpTo(starts[chosen] + i);
              return;
            }
          }
          jumpTo(starts[chosen]);
        });
        return;
      }
    }
    status("no messages on or after that date");
  }

  /* ---------------------------------------------------------------- wiring */

  document.getElementById("q").addEventListener("keydown", function (event) {
    if (event.key === "Enter") search(event.target.value);
  });

  document.getElementById("date").addEventListener("change", function (event) {
    if (event.target.value) jumpToDate(event.target.value);
  });

  document.addEventListener("keydown", function (event) {
    if (event.target.tagName === "INPUT") return;
    if (event.key === "/") {
      event.preventDefault();
      document.getElementById("q").focus();
    } else if (event.key === "j") {
      timeline.scrollTop += 90;
    } else if (event.key === "k") {
      timeline.scrollTop -= 90;
    } else if (event.key === "G") {
      timeline.scrollTop = timeline.scrollHeight;
    } else if (event.key === "g") {
      timeline.scrollTop = 0;
    }
  });

  timeline.addEventListener("scroll", onScroll, { passive: true });

  render("none").then(function () {
    timeline.scrollTop = timeline.scrollHeight;
  });
})();
