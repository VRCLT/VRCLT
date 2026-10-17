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
