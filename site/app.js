"use strict";

// Gallery and disclosure interactions. Language preference is handled by locale.js.
const tabs = [...document.querySelectorAll('[role="tab"]')];
function selectTab(tab, focus = false) {
  for (const item of tabs) {
    const selected = item === tab;
    item.setAttribute("aria-selected", String(selected));
    item.tabIndex = selected ? 0 : -1;
    document.getElementById(item.getAttribute("aria-controls")).hidden = !selected;
  }
  if (focus) tab.focus();
}
for (const tab of tabs) {
  tab.addEventListener("click", () => selectTab(tab));
  tab.addEventListener("keydown", (event) => {
    let index = tabs.indexOf(tab);
    if (event.key === "ArrowRight") index = (index + 1) % tabs.length;
    else if (event.key === "ArrowLeft") index = (index - 1 + tabs.length) % tabs.length;
    else if (event.key === "Home") index = 0;
    else if (event.key === "End") index = tabs.length - 1;
    else return;
    event.preventDefault();
    selectTab(tabs[index], true);
  });
}
const languageMenu = document.querySelector(".language-menu");
document.addEventListener("click", (event) => {
  if (!languageMenu.contains(event.target)) languageMenu.open = false;
});
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && languageMenu.open) {
    languageMenu.open = false;
    languageMenu.querySelector("summary").focus();
  }
});

// Reveal each section once. No scroll handlers, and no hidden content when
// JavaScript, IntersectionObserver, or motion preferences disable enhancement.
const motionPreference = window.matchMedia("(prefers-reduced-motion: reduce)");
if (!motionPreference.matches && "IntersectionObserver" in window) {
  const observer = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      entry.target.classList.add("is-visible");
      observer.unobserve(entry.target);
    }
  }, { threshold: 0.08 });

  const sections = document.querySelectorAll(
    ".hero h1, .hero-copy, .cover-figure, .ways article, .section-heading, " +
    ".gallery, .steps li, .audio-route, .engine-intro, .engine-list, " +
    ".faq h2, .faq-items, .closing .wrap, .site-footer"
  );
  for (const section of sections) {
    section.classList.add("will-reveal");
    observer.observe(section);
  }
  for (const selector of [".ways article", ".steps li"]) {
    document.querySelectorAll(selector).forEach((item, index) => {
      item.style.setProperty("--reveal-delay", `${index * 60}ms`);
    });
  }

  // A preference change must never leave a section waiting to become visible.
  motionPreference.addEventListener("change", (event) => {
    if (!event.matches) return;
    observer.disconnect();
    sections.forEach((section) => section.classList.add("is-visible"));
  });
}
