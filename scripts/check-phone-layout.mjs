#!/usr/bin/env node
/**
 * Checks the frontend at iPhone geometries, in a desktop browser.
 *
 * The layout questions a phone asks — does anything sit under the Dynamic Island, is
 * every control big enough for a finger, does a bar silently clip the button on its
 * right-hand end — have answers that are invisible at desktop width and expensive to
 * discover on a device. This drives the built app at six screen sizes and asserts them,
 * then drives real multi-touch at the piano roll to confirm two-finger navigation works
 * and has not eaten the one-finger editing it sits beside.
 *
 * Safe-area insets are simulated by overriding the `--safe-*` variables, which works
 * only because every inset in the app is read through them (see styles/tokens.css).
 * That makes this a check of the real values, not a mock of them.
 *
 * It is NOT a substitute for a device — see the "Not verified" list in DECISIONS.md
 * Phase 11, particularly software-keyboard avoidance, which nothing here can reach.
 *
 * Playwright is deliberately not a dependency of this project: it is a browser download
 * that no build or shipped target needs, and adding one is a decision per the technical
 * README. Install it for the run and let it go:
 *
 *     npm run build
 *     npm run preview -- --port 4173 &
 *     npm install --no-save playwright-core
 *     node scripts/check-phone-layout.mjs
 *
 * Set BASE to point at a different server. Set SHOTS to a directory to also write a
 * screenshot per geometry there; without it, nothing is written to disk.
 */
import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";

import { chromium, devices } from "playwright-core";

const BASE = process.env.BASE ?? "http://localhost:4173";

/**
 * playwright-core ships no browser of its own, so one has to be pointed at. Set
 * CHROMIUM to skip the search; otherwise take whatever a `playwright install chromium`
 * left in the usual cache, newest first.
 */
function findChromium() {
  if (process.env.CHROMIUM) return process.env.CHROMIUM;
  const roots = [
    process.env.PLAYWRIGHT_BROWSERS_PATH,
    join(process.env.HOME ?? "", ".cache/ms-playwright"),
    join(process.env.HOME ?? "", "Library/Caches/ms-playwright"),
  ].filter(Boolean);

  for (const root of roots) {
    if (!existsSync(root)) continue;
    const builds = readdirSync(root)
      .filter((name) => name.startsWith("chromium-"))
      .sort()
      .reverse();
    for (const build of builds) {
      for (const relative of [
        "chrome-linux/chrome",
        "chrome-mac/Chromium.app/Contents/MacOS/Chromium",
      ]) {
        const candidate = join(root, build, relative);
        if (existsSync(candidate)) return candidate;
      }
    }
  }
  throw new Error(
    "No Chromium found. Set CHROMIUM=/path/to/chrome, or run `npx playwright install chromium`.",
  );
}

const EXECUTABLE = findChromium();

// width, height, and the insets iOS reports for that device in that orientation.
const GEOMETRIES = [
  { name: "iPhone SE portrait", w: 375, h: 667, safe: [20, 0, 0, 0] },
  { name: "iPhone 16 Pro portrait", w: 393, h: 852, safe: [59, 0, 34, 0] },
  { name: "iPhone 16 Pro Max portrait", w: 430, h: 932, safe: [59, 0, 34, 0] },
  { name: "iPhone 16 Pro landscape", w: 852, h: 393, safe: [0, 59, 21, 59] },
  { name: "iPad portrait", w: 820, h: 1180, safe: [24, 0, 20, 0] },
  { name: "desktop", w: 1440, h: 900, safe: [0, 0, 0, 0] },
];

const safeCss = ([t, r, b, l]) => `:root{
  --safe-top:${t}px; --safe-right:${r}px; --safe-bottom:${b}px; --safe-left:${l}px;
}`;

/** Runs in the page. Returns every finding for the current screen. */
function audit([coarse, floor]) {
  const findings = [];
  const doc = document.documentElement;

  if (doc.scrollWidth > doc.clientWidth + 1) {
    findings.push(`page scrolls horizontally: ${doc.scrollWidth} > ${doc.clientWidth}`);
  }

  // Anything you press must be reachable and big enough to press.
  if (coarse) {
    const seen = new Set();
    for (const el of document.querySelectorAll("button, input, select, textarea, [role=button]")) {
      const r = el.getBoundingClientRect();
      if (r.width === 0 && r.height === 0) continue; // display:none is a decision, not a bug
      const label = (el.getAttribute("aria-label") || el.textContent || el.className || el.tagName)
        .trim()
        .slice(0, 34);
      // Piano keys are deliberately narrow-and-tall; they are audited separately.
      if (el.className && String(el.className).includes("keys__")) continue;
      // Visually-hidden submit buttons exist for the Enter key, not the finger.
      if (el.className && String(el.className).includes("sr-only")) continue;
      const key = `${label}|${Math.round(r.width)}x${Math.round(r.height)}`;
      if (seen.has(key)) continue;
      seen.add(key);
      // A checkbox is aimed at through the label wrapping it; measure that instead.
      const box = el.type === "checkbox" || el.type === "radio" ? el.closest("label") ?? el : el;
      const br = box.getBoundingClientRect();
      if (br.height < floor) findings.push(`target too short (${Math.round(br.height)}px): ${label}`);
      if (br.width < 28) findings.push(`target too narrow (${Math.round(br.width)}px): ${label}`);
      const style = getComputedStyle(el);
      if (parseFloat(style.fontSize) < 16 && ["INPUT", "SELECT", "TEXTAREA"].includes(el.tagName)) {
        findings.push(`field font ${style.fontSize} will trigger iOS focus-zoom: ${label}`);
      }
    }
  }

  // Nothing that matters may sit under a system inset.
  const inset = (name) =>
    parseFloat(getComputedStyle(doc).getPropertyValue(name)) || 0;
  const top = inset("--safe-top");
  const bottom = inset("--safe-bottom");
  // Only the fixed chrome. Content inside a scrolling pane passes under the insets by
  // design — that is what scrolling is — so the question is whether the *bars* clear them.
  const chrome =
    ".editor__titlebar, .editor__bottom, .picker__bar, .modal, .tedit__head, .tedit__controls";
  for (const el of document.querySelectorAll(
    [...chrome.split(", ")].map((c) => `${c} button`).join(", ") +
      ", .transport__position, .editor__title",
  )) {
    const r = el.getBoundingClientRect();
    if (r.height === 0) continue;
    // Only what is actually on screen. Something scrolled past the bottom of a pane is
    // not "under the home indicator", it is just further down the pane.
    if (r.bottom < 0 || r.top > window.innerHeight) continue;
    const label = (el.getAttribute("aria-label") || el.textContent || "").trim().slice(0, 30);
    if (r.top < top - 0.5) findings.push(`under the status bar: ${label} (top ${Math.round(r.top)})`);
    if (bottom > 0 && r.bottom > window.innerHeight - bottom + 0.5) {
      findings.push(`under the home indicator: ${label} (bottom ${Math.round(r.bottom)})`);
    }
  }

  // A bar with `overflow: hidden` silently swallows whatever does not fit.
  for (const sel of [".roll__toolbar", ".editor__transport", ".editor__keyboard"]) {
    const el = document.querySelector(sel);
    if (!el) continue;
    const style = getComputedStyle(el);
    if (style.overflow === "hidden" || style.overflowX === "hidden") {
      if (el.scrollWidth > el.clientWidth + 1) {
        findings.push(`${sel} clips content: ${el.scrollWidth} > ${el.clientWidth}`);
      }
      // Flex children shrink into each other rather than overflowing, so a bar can look
      // the right width while its labels sit on top of its controls.
      const kids = [...el.children].map((k) => k.getBoundingClientRect()).filter((r) => r.width);
      for (let i = 1; i < kids.length; i += 1) {
        if (kids[i].left < kids[i - 1].right - 0.5) {
          findings.push(`${sel} children overlap at index ${i}`);
          break;
        }
      }
    }
  }

  // The roll is the point of the screen; report what it actually got.
  const roll = document.querySelector(".roll__canvas-wrap");
  const info = roll
    ? `roll ${Math.round(roll.getBoundingClientRect().width)}×${Math.round(
        roll.getBoundingClientRect().height,
      )}`
    : "roll absent";

  const board = document.querySelector(".editor__keyboard");
  const whites = document.querySelectorAll(".keys__white");
  const keyWidth = whites.length ? Math.round(whites[0].getBoundingClientRect().width) : 0;
  const keys =
    board && getComputedStyle(board).display === "none"
      ? "keyboard hidden"
      : `${whites.length} white keys @ ${keyWidth}px`;

  return { findings, info: `${info}, ${keys}` };
}

const browser = await chromium.launch({ executablePath: EXECUTABLE });
let failures = 0;

for (const geo of GEOMETRIES) {
  const coarse = !geo.name.startsWith("desktop");
  const context = await browser.newContext({
    viewport: { width: geo.w, height: geo.h },
    deviceScaleFactor: coarse ? 3 : 1,
    isMobile: coarse,
    hasTouch: coarse,
    userAgent: coarse ? devices["iPhone 13"].userAgent : undefined,
  });
  const page = await context.newPage();
  const errors = [];
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(String(e)));

  await page.goto(BASE, { waitUntil: "networkidle" });
  await page.addStyleTag({ content: safeCss(geo.safe) });

  // Into the editor: the picker is one screen, the editor is the app. The mock backend
  // starts empty, so a project has to be made before there is one to open.
  if (process.env.SCREEN !== "picker") {
    await page.getByRole("button", { name: /^New Project$/ }).click();
    await page.waitForTimeout(200);
    await page.getByRole("button", { name: /^Create$/ }).click();
    await page.waitForTimeout(700);
  }

  const screen = (await page.locator(".editor").count()) ? "editor" : "picker";
  const { findings, info } = await page.evaluate(audit,
    // 34px in phone landscape: on a 393pt-tall screen the compact controls are a
    // documented deviation from the 44pt floor (see DECISIONS.md). Anything below the
    // compromise is still a finding.
    [coarse, geo.h <= 460 && coarse ? 34 : 40],
  );

  console.log(`\n── ${geo.name}  (${geo.w}×${geo.h}, ${screen})`);
  console.log(`   ${info}`);
  for (const f of findings) console.log(`   ✗ ${f}`);
  for (const e of errors) console.log(`   ! console: ${e}`);
  failures += findings.length + errors.length;
  if (!findings.length && !errors.length) console.log("   ✓ clean");

  // Off by default: the repository is not where screenshots live.
  if (process.env.SHOTS) {
    await page.screenshot({ path: `${process.env.SHOTS}/${geo.name.replace(/\s+/g, "-")}.png` });
  }
  await context.close();
}


// ---------------------------------------------------------------- gestures --

console.log("\n── two-finger navigation  (393×852)");
const ctx = await browser.newContext({
  viewport: { width: 393, height: 852 }, deviceScaleFactor: 3,
  isMobile: true, hasTouch: true, userAgent: devices["iPhone 13"].userAgent,
});
const page = await ctx.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(BASE, { waitUntil: "networkidle" });
await page.getByRole("button", { name: /^New Project$/ }).click();
await page.waitForTimeout(200);
await page.getByRole("button", { name: /^Create$/ }).click();
await page.waitForTimeout(700);

const cdp = await ctx.newCDPSession(page);
const box = await page.locator(".roll__canvas").boundingBox();
const at = (dx, dy) => ({ x: box.x + dx, y: box.y + dy, radiusX: 8, radiusY: 8, force: 1 });

async function touch(type, points) {
  await cdp.send("Input.dispatchTouchEvent", { type, touchPoints: points });
  await page.waitForTimeout(40);
}

// The roll draws its pitch labels in the gutter; read one to detect vertical movement,
// and the bar numbers in the ruler for horizontal. Simpler and more robust: read the
// canvas pixels' checksum before and after.
const snapshot = () =>
  page.evaluate(() => {
    const c = document.querySelector(".roll__canvas");
    const g = c.getContext("2d");
    const d = g.getImageData(0, 0, c.width, c.height).data;
    let h = 0;
    for (let i = 0; i < d.length; i += 97) h = (h * 31 + d[i]) >>> 0;
    return h;
  });

// A note first: an empty grid is a repeating pattern, so panning it by a whole number
// of grid columns produces pixels identical to where it started. One note is an anchor.
await page.locator(".roll__canvas").tap({ position: { x: 200, y: 160 } });
await page.waitForTimeout(400);
const notesBefore = await page.locator(".tracklist__count").first().textContent();

const before = await snapshot();
const zoomOf = () => page.locator('input[aria-label="Horizontal zoom"]').inputValue();
const zoomBefore = await zoomOf();

// --- two-finger pan: both fingers left and down, spread held constant ---
await touch("touchStart", [at(150, 120), at(250, 140)]);
await touch("touchMove", [at(120, 150), at(220, 170)]);
await touch("touchMove", [at(90, 180), at(190, 200)]);
await touch("touchMove", [at(60, 200), at(160, 220)]);
await touch("touchEnd", []);
const afterPan = await snapshot();
const zoomAfterPan = await zoomOf();

// --- pinch out about a fixed centre ---
await touch("touchStart", [at(140, 150), at(240, 150)]);
await touch("touchMove", [at(100, 150), at(280, 150)]);
await touch("touchMove", [at(40, 150), at(340, 150)]);
await touch("touchEnd", []);
const zoomAfterPinch = await zoomOf();

// --- one finger still edits, and two fingers still do not ---
const notesAfterGestures = await page.locator(".tracklist__count").first().textContent();

const check = (name, ok, detail = "") => {
  if (!ok) failures += 1;
  console.log(`   ${ok ? "✓" : "✗"} ${name}${detail ? `  (${detail})` : ""}`);
};

check("one finger draws a note", notesBefore === "1", `→ ${notesBefore}`);
check("two-finger pan moves the roll", afterPan !== before, `${before} → ${afterPan}`);
check("pan leaves the zoom alone", zoomAfterPan === zoomBefore, `${zoomBefore} → ${zoomAfterPan}`);
check("pinch changes the zoom", zoomAfterPinch !== zoomAfterPan, `${zoomAfterPan} → ${zoomAfterPinch}`);
check("no gesture drew a stray note", notesAfterGestures === "1", `→ ${notesAfterGestures}`);
for (const e of errors) {
  failures += 1;
  console.log("   ! " + e);
}

await browser.close();
console.log(`\n${failures} finding(s)`);
process.exit(failures === 0 ? 0 : 1);
