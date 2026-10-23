// NeoShard landing site: downloads from downloads/latest.json (written by
// deploy/windows/publish-downloads.sh), donation links from site.json, small effects.
"use strict";

const $ = (s, root = document) => root.querySelector(s);
const $$ = (s, root = document) => [...root.querySelectorAll(s)];

// Header gets a backdrop once the page scrolls.
const nav = $(".nav");
const onScroll = () => nav.classList.toggle("scrolled", window.scrollY > 8);
addEventListener("scroll", onScroll, { passive: true });
onScroll();

// Sections fade in as they come into view.
const io = "IntersectionObserver" in window
  ? new IntersectionObserver((entries) => {
      for (const e of entries) if (e.isIntersecting) { e.target.classList.add("shown"); io.unobserve(e.target); }
    }, { rootMargin: "0px 0px -10% 0px" })
  : null;
for (const el of $$(".feature-art, .feature-copy, .card, .dl, .privacy h2, .download h2, .support-copy, .final h2")) {
  if (!io) break;
  el.classList.add("reveal");
  io.observe(el);
}
// Cards in a row appear one after another.
$$(".cards .card, .dl-grid .dl").forEach((el, i) => (el.style.transitionDelay = `${(i % 4) * 90}ms`));

const platform = /android/i.test(navigator.userAgent) ? "android" : /windows/i.test(navigator.userAgent) ? "windows" : "";
const LABEL = { windows: "Скачать для Windows", android: "Скачать для Android" };

const mb = (n) => `${(n / 1048576).toFixed(1).replace(".", ",")} МБ`;

/** site.json: donation links and the releases page (both optional). */
const siteConfig = fetch("site.json", { cache: "no-store" })
  .then((r) => (r.ok ? r.json() : {}))
  .catch(() => ({}));

async function loadReleases() {
  let rel;
  try {
    const r = await fetch("downloads/latest.json", { cache: "no-store" });
    if (!r.ok) throw new Error(String(r.status));
    rel = await r.json();
  } catch {
    // No builds on this server: send people to the project's releases page.
    const releases = (await siteConfig).releases;
    if (/^https:\/\//.test(releases ?? "")) {
      $("#release-line").textContent = "Готовые сборки для всех устройств — на странице релизов проекта.";
      for (const a of $$(".dl [data-file]")) {
        a.href = releases;
        a.rel = "noopener";
      }
      const btn = $("#primary-download");
      btn.href = releases;
      $("span", btn).textContent = "Скачать NeoShard";
    } else {
      $("#release-line").textContent = "Сборки скоро появятся здесь. Пока можно пользоваться веб-версией.";
      for (const a of $$(".dl [data-file]")) a.setAttribute("aria-disabled", "true");
    }
    return;
  }
  const date = rel.date ? new Date(rel.date).toLocaleDateString("ru-RU", { day: "numeric", month: "long", year: "numeric" }) : "";
  $("#release-line").textContent = `Версия ${rel.version}${date ? ` от ${date}` : ""} — для всех устройств.`;
  const hashes = $("#dl-hashes");
  for (const card of $$(".dl[data-platform]")) {
    const p = card.dataset.platform;
    const f = rel.files?.[p];
    const a = $("[data-file]", card);
    if (!f) {
      a.setAttribute("aria-disabled", "true");
      continue;
    }
    a.href = `downloads/${encodeURIComponent(f.file)}`;
    a.setAttribute("download", f.file);
    $(".dl-meta", card).textContent += ` · ${mb(f.size)}`;
    if (p === platform) card.classList.add("mine");
    const li = document.createElement("li");
    li.innerHTML = `<b></b><br><code></code>`;
    li.querySelector("b").textContent = f.file;
    li.querySelector("code").textContent = f.sha256;
    hashes.append(li);
  }
  // The hero button downloads the build for this device right away.
  const mine = rel.files?.[platform];
  if (mine) {
    const btn = $("#primary-download");
    btn.href = `downloads/${encodeURIComponent(mine.file)}`;
    btn.setAttribute("download", mine.file);
    $("span", btn).textContent = LABEL[platform];
    $("#primary-note").textContent = `Версия ${rel.version} · ${mb(mine.size)} · есть и для других устройств ниже`;
  }
}

async function loadDonations() {
  const cfg = await siteConfig;
  const list = (cfg.donate ?? []).filter((d) => d && d.title && /^https:\/\//.test(d.url ?? ""));
  if (!list.length) return;
  const box = $("#donate");
  box.textContent = "";
  for (const d of list) {
    const a = document.createElement("a");
    a.className = "donate-item";
    a.href = d.url;
    a.target = "_blank";
    a.rel = "noopener noreferrer";
    const b = document.createElement("b");
    b.textContent = d.title;
    a.append(b);
    if (d.note) {
      const s = document.createElement("span");
      s.textContent = d.note;
      a.append(s);
    }
    box.append(a);
  }
}

void loadReleases();
void loadDonations();

// The penguin: a click (or Enter) makes it hop and say something; it greets once on load.
const LINES = [
  "Ну, заходи.",
  "Свой сервер. Своя рыба.",
  "Рекламы нет. Я проверял.",
  "Шифруемся.",
  "Я просто постою.",
  "Голосовой? Я на подхвате.",
  "Печатаю. Ластами. Медленно.",
  "Рюкзак не трогать.",
];
const penguin = $(".mascot-hit");
const bubble = $(".mascot-say");
if (penguin && bubble) {
  let last = -1;
  let hideTimer = 0;
  const say = (text) => {
    bubble.textContent = text;
    bubble.classList.add("on");
    clearTimeout(hideTimer);
    hideTimer = setTimeout(() => bubble.classList.remove("on"), 2600);
  };
  penguin.addEventListener("click", () => {
    let i;
    do i = Math.floor(Math.random() * LINES.length);
    while (i === last && LINES.length > 1);
    last = i;
    say(LINES[i]);
    penguin.classList.remove("hop");
    void penguin.offsetWidth; // restart the hop
    penguin.classList.add("hop");
  });
  penguin.addEventListener("animationend", (e) => e.animationName === "hop" && penguin.classList.remove("hop"));
  setTimeout(() => say("Привет. Я тут за порядком слежу."), 1800);
}
