/* dok site — theme, navigation, copy buttons, search, docs helpers. */
(function () {
  "use strict";

  var docEl = document.documentElement;
  var root = docEl.getAttribute("data-root") || "";
  var reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  /* ---------- theme ---------- */

  var themeBtn = document.querySelector("[data-theme-toggle]");
  function currentTheme() {
    return docEl.getAttribute("data-theme") === "dark" ? "dark" : "light";
  }
  function syncThemeButton() {
    if (!themeBtn) return;
    var dark = currentTheme() === "dark";
    themeBtn.setAttribute("aria-label", dark ? "Switch to light theme" : "Switch to dark theme");
    themeBtn.setAttribute("title", dark ? "Light theme" : "Dark theme");
  }
  if (themeBtn) {
    syncThemeButton();
    themeBtn.addEventListener("click", function () {
      var next = currentTheme() === "dark" ? "light" : "dark";
      if (next === "dark") {
        docEl.setAttribute("data-theme", "dark");
      } else {
        docEl.removeAttribute("data-theme");
      }
      try { localStorage.setItem("dok-theme", next); } catch (e) {}
      syncThemeButton();
    });
  }

  /* ---------- mobile menu ---------- */

  var menuBtn = document.querySelector("[data-menu-toggle]");
  var mobileMenu = document.getElementById("mobile-menu");
  if (menuBtn && mobileMenu) {
    menuBtn.addEventListener("click", function () {
      var open = mobileMenu.classList.toggle("open");
      menuBtn.setAttribute("aria-expanded", open ? "true" : "false");
    });
    mobileMenu.addEventListener("click", function (e) {
      if (e.target.closest("a")) {
        mobileMenu.classList.remove("open");
        menuBtn.setAttribute("aria-expanded", "false");
      }
    });
    document.addEventListener("keydown", function (e) {
      if (e.key === "Escape" && mobileMenu.classList.contains("open")) {
        mobileMenu.classList.remove("open");
        menuBtn.setAttribute("aria-expanded", "false");
        menuBtn.focus();
      }
    });
  }

  /* ---------- docs sidebar (mobile) ---------- */

  var docsBtn = document.querySelector("[data-docs-toggle]");
  var docsNav = document.getElementById("docs-nav");
  if (docsBtn && docsNav) {
    docsBtn.addEventListener("click", function () {
      var open = docsNav.classList.toggle("open");
      docsBtn.setAttribute("aria-expanded", open ? "true" : "false");
      var label = docsBtn.querySelector("span");
      if (label) label.textContent = open ? "Hide docs menu" : "Browse docs";
      var sign = docsBtn.querySelector("span:last-child");
      if (sign) sign.textContent = open ? "−" : "+";
    });
  }

  /* ---------- copy buttons ---------- */

  function copyText(text) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      return navigator.clipboard.writeText(text);
    }
    return new Promise(function (resolve, reject) {
      var ta = document.createElement("textarea");
      ta.value = text;
      ta.setAttribute("readonly", "");
      ta.style.position = "fixed";
      ta.style.top = "-1000px";
      document.body.appendChild(ta);
      ta.select();
      try {
        document.execCommand("copy");
        resolve();
      } catch (err) {
        reject(err);
      } finally {
        document.body.removeChild(ta);
      }
    });
  }

  function makeCopyButton(getText, compact) {
    var btn = document.createElement("button");
    btn.type = "button";
    btn.className = "copy-btn";
    btn.innerHTML = '<svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.4"><rect x="5.5" y="5.5" width="8" height="8" rx="1"/><path d="M10.5 3.5V2.5h-8v8h1"/></svg>' +
      (compact ? "" : "<span>Copy</span>");
    btn.setAttribute("aria-label", "Copy code to clipboard");
    btn.addEventListener("click", function () {
      copyText(getText()).then(function () {
        btn.classList.add("copied");
        var label = btn.querySelector("span");
        if (label) label.textContent = "Copied";
        btn.setAttribute("aria-label", "Copied");
        window.setTimeout(function () {
          btn.classList.remove("copied");
          if (label) label.textContent = "Copy";
          btn.setAttribute("aria-label", "Copy code to clipboard");
        }, 1700);
      }).catch(function () {
        var label = btn.querySelector("span");
        if (label) label.textContent = "Select";
      });
    });
    return btn;
  }

  document.querySelectorAll(".code").forEach(function (block) {
    if (block.classList.contains("no-copy")) return;
    var pre = block.querySelector("pre");
    if (!pre) return;
    var head = block.querySelector(".code-head");
    if (!head) {
      head = document.createElement("div");
      head.className = "code-head";
      block.insertBefore(head, pre);
      var lang = block.getAttribute("data-lang") || "text";
      var lab = document.createElement("span");
      lab.className = "code-lang";
      lab.textContent = lang;
      head.appendChild(lab);
    }
    if (head.querySelector(".copy-btn")) return;
    head.appendChild(makeCopyButton(function () { return pre.innerText; }, false));
  });

  document.querySelectorAll("[data-copy]").forEach(function (btn) {
    btn.addEventListener("click", function () {
      copyText(btn.getAttribute("data-copy")).then(function () {
        var original = btn.innerHTML;
        btn.classList.add("copied");
        btn.innerHTML = "Copied";
        window.setTimeout(function () {
          btn.classList.remove("copied");
          btn.innerHTML = original;
        }, 1600);
      });
    });
  });

  /* ---------- search ---------- */

  var modal = document.getElementById("search-modal");
  var input = document.getElementById("search-input");
  var results = document.getElementById("search-results");
  var docBase = root === "../" ? "" : "docs/";
  var index = Array.isArray(window.DOK_SEARCH) ? window.DOK_SEARCH : [];
  var active = -1;
  var lastFocus = null;

  function resultLink(item) {
    var a = document.createElement("a");
    a.className = "search-result";
    a.href = docBase + item.u;
    a.innerHTML = '<span class="r-title"><span></span><span class="r-page"></span></span><span class="r-desc"></span>';
    a.querySelector(".r-title span:first-child").textContent = item.t;
    a.querySelector(".r-page").textContent = item.t === item.p ? "" : item.p;
    a.querySelector(".r-desc").textContent = item.d || "";
    return a;
  }

  function score(item, q) {
    var t = (item.t || "").toLowerCase();
    var p = (item.p || "").toLowerCase();
    var d = (item.d || "").toLowerCase();
    var k = (item.k || "").toLowerCase();
    var s = 0;
    if (t === q) s += 100;
    else if (t.indexOf(q) === 0) s += 60;
    else if (t.indexOf(q) > -1) s += 40;
    if (t === p) s += 50; // prefer the guide itself over a section with the same name
    if (p.indexOf(q) > -1) s += 12;
    if (k.indexOf(q) > -1) s += 18;
    if (d.indexOf(q) > -1) s += 8;
    return s;
  }

  function render() {
    if (!results) return;
    var q = input ? input.value.trim().toLowerCase() : "";
    var items;
    if (!q) {
      items = index.filter(function (item) { return item.t === item.p; }).slice(0, 8);
    } else {
      items = index
        .map(function (item) { return { item: item, s: score(item, q) }; })
        .filter(function (r) { return r.s > 0; })
        .sort(function (a, b) { return b.s - a.s; })
        .slice(0, 9)
        .map(function (r) { return r.item; });
    }
    results.innerHTML = "";
    active = -1;
    if (!items.length) {
      var empty = document.createElement("p");
      empty.className = "search-empty";
      empty.textContent = "No match for “" + (input ? input.value.trim() : "") + "”. Try “deploy”, “domain”, “token” or “backup”.";
      results.appendChild(empty);
      return;
    }
    items.forEach(function (item, i) {
      var a = resultLink(item);
      a.dataset.i = String(i);
      results.appendChild(a);
    });
    setActive(0, false);
  }

  function setActive(i, scroll) {
    var links = results ? results.querySelectorAll(".search-result") : [];
    if (!links.length) return;
    active = (i + links.length) % links.length;
    links.forEach(function (l, n) { l.classList.toggle("is-active", n === active); });
    if (scroll) links[active].scrollIntoView({ block: "nearest" });
  }

  function openSearch() {
    if (!modal) return;
    lastFocus = document.activeElement;
    modal.hidden = false;
    document.body.style.overflow = "hidden";
    if (input) {
      input.value = "";
      render();
      window.setTimeout(function () { input.focus(); }, 20);
    }
    document.querySelectorAll("[data-search-open]").forEach(function (b) {
      b.setAttribute("aria-expanded", "true");
    });
  }

  function closeSearch() {
    if (!modal || modal.hidden) return;
    modal.hidden = true;
    document.body.style.overflow = "";
    document.querySelectorAll("[data-search-open]").forEach(function (b) {
      b.setAttribute("aria-expanded", "false");
    });
    if (lastFocus && lastFocus.focus) lastFocus.focus();
  }

  document.querySelectorAll("[data-search-open]").forEach(function (btn) {
    btn.addEventListener("click", openSearch);
  });

  if (modal) {
    modal.addEventListener("click", function (e) {
      if (e.target === modal) closeSearch();
    });
    document.addEventListener("keydown", function (e) {
      if (e.key === "Escape" && !modal.hidden) {
        closeSearch();
        return;
      }
      var typing = /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement.tagName) || document.activeElement.isContentEditable;
      if (!typing && (e.key === "/" || ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k"))) {
        e.preventDefault();
        openSearch();
      }
      if (modal.hidden) return;
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setActive(active + 1, true);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setActive(active - 1, true);
      } else if (e.key === "Enter" && active > -1) {
        var link = results.querySelectorAll(".search-result")[active];
        if (link) window.location.href = link.href;
      }
    });
    if (input) input.addEventListener("input", render);
    // Keep Tab inside the dialog while it is open.
    modal.addEventListener("keydown", function (e) {
      if (e.key !== "Tab") return;
      var focusables = modal.querySelectorAll('input, a[href], button:not([disabled]), [tabindex]:not([tabindex="-1"])');
      if (!focusables.length) return;
      var first = focusables[0];
      var last = focusables[focusables.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    });
    if (results) {
      results.addEventListener("mousemove", function (e) {
        var link = e.target.closest(".search-result");
        if (link) setActive(Number(link.dataset.i), false);
      });
    }
    render();
  }

  /* ---------- docs hub filter ---------- */

  var filter = document.getElementById("docs-filter");
  if (filter) {
    var cards = Array.prototype.slice.call(document.querySelectorAll("[data-doc]"));
    var empty = document.getElementById("docs-empty");
    var count = document.getElementById("docs-count");
    var apply = function () {
      var q = filter.value.trim().toLowerCase();
      var shown = 0;
      cards.forEach(function (card) {
        var hay = (card.getAttribute("data-keywords") || "") + " " + card.textContent;
        var hit = !q || hay.toLowerCase().indexOf(q) > -1;
        card.hidden = !hit;
        if (hit) shown++;
      });
      if (empty) empty.hidden = shown !== 0;
      if (count) count.textContent = q ? shown + " of " + cards.length + " guides" : cards.length + " guides";
    };
    filter.addEventListener("input", apply);
    apply();
  }

  /* ---------- docs TOC scrollspy ---------- */

  var tocLinks = Array.prototype.slice.call(document.querySelectorAll(".toc-link"));
  if (tocLinks.length) {
    var targets = tocLinks.map(function (link) {
      var id = link.getAttribute("href").slice(1);
      return document.getElementById(id);
    }).filter(Boolean);
    var ticking = false;
    var spy = function () {
      ticking = false;
      var current = targets[0];
      for (var i = 0; i < targets.length; i++) {
        if (targets[i].getBoundingClientRect().top <= 150) current = targets[i];
      }
      tocLinks.forEach(function (link) {
        link.classList.toggle("is-active", current && link.getAttribute("href") === "#" + current.id);
      });
    };
    window.addEventListener("scroll", function () {
      if (!ticking) {
        ticking = true;
        window.requestAnimationFrame(spy);
      }
    }, { passive: true });
    spy();
  }

  /* ---------- active docs nav from URL ---------- */

  var here = window.location.pathname.split("/").pop() || "index.html";
  document.querySelectorAll(".side-link").forEach(function (link) {
    var href = link.getAttribute("href");
    if (href === here && !link.hasAttribute("aria-current")) {
      link.setAttribute("aria-current", "page");
    }
  });

  /* ---------- misc ---------- */

  var year = document.getElementById("year");
  if (year) year.textContent = String(new Date().getFullYear());

  // Re-render the search index once on bfcache restore (harmless, keeps focus sane).
  window.addEventListener("pageshow", function (e) {
    if (e.persisted && modal) closeSearch();
  });
})();
