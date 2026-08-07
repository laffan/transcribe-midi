#!/usr/bin/env node
/**
 * Drives the Listen review stage — the take, the notes, the dials — in a desktop browser.
 *
 * This is the largest view in the app and the one that had never been opened outside a
 * Mac: the backend it needs is Rust, so the browser preview refused every call and the
 * whole stage was unreachable. It is reachable now because the mock hands back a *canned
 * take* (src/lib/mockTake.ts) — a fixture, not an analysis. What that buys is this file:
 * the gestures the stage lives by, asserted, on any machine, in about ten seconds.
 *
 * What it cannot do is judge a transcription. Nothing here measures a pitch; every real
 * answer still comes from `unplugged-transcribe`, whose tests are in Rust. This asserts
 * the *editor* — that a pinch zooms, that a loop loops, that a semitone step is a
 * semitone — and those are exactly the things that were being found on a device.
 *
 * Playwright is deliberately not a dependency (see check-phone-layout.mjs):
 *
 *     npm run build
 *     npm run preview -- --port 4173 &
 *     npm install --no-save playwright-core
 *     node scripts/check-listen-editor.mjs
 *
 * Set BASE for a different server, SHOTS for a directory to write screenshots into.
 */
import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";

import { chromium } from "playwright-core";

const BASE = process.env.BASE ?? "http://localhost:4173";
const SHOTS = process.env.SHOTS ?? "";

function findChromium() {
  if (process.env.CHROMIUM) return process.env.CHROMIUM;
  const roots = [
    process.env.PLAYWRIGHT_BROWSERS_PATH,
    join(process.env.HOME ?? "", ".cache/ms-playwright"),
    join(process.env.HOME ?? "", "Library/Caches/ms-playwright"),
  ].filter(Boolean);

  for (const root of roots) {
    if (!existsSync(root)) continue;
    for (const build of readdirSync(root)
      .filter((name) => name.startsWith("chromium-"))
      .sort()
      .reverse()) {
      for (const relative of [
        "chrome-linux/chrome",
        "chrome-mac/Chromium.app/Contents/MacOS/Chromium",
        "chrome-win/chrome.exe",
      ]) {
        const path = join(root, build, relative);
        if (existsSync(path)) return path;
      }
    }
  }
  return undefined;
}

let failures = 0;
const check = (name, ok, detail = "") => {
  if (!ok) failures += 1;
  console.log(`   ${ok ? "✓" : "✗"} ${name}${detail ? `  (${detail})` : ""}`);
};

const browser = await chromium.launch({ executablePath: findChromium() });
// A touchscreen at desk width: an iPad, which is where this stage is used and where
// every finding that prompted this file came from.
const context = await browser.newContext({
  viewport: { width: 1180, height: 820 },
  hasTouch: true,
});
const page = await context.newPage();
const errors = [];
page.on("pageerror", (error) => errors.push(String(error)));

await page.goto(BASE, { waitUntil: "networkidle" });
await page.getByRole("button", { name: /^New Project$/ }).click();
await page.waitForTimeout(200);
await page.getByRole("button", { name: /^Create$/ }).click();
await page.waitForTimeout(700);

// --- into the review stage ------------------------------------------------

await page.locator("button", { hasText: /^▶?\s*Listen$/i }).first().click();
await page.waitForTimeout(400);
await page.locator(".listen button", { hasText: /stop/i }).first().click();
await page.waitForTimeout(900);

const overlay = page.locator(".listen");
check("the take opens into the review stage", await overlay.isVisible());

// --- what the user asked to start open, open ------------------------------

check("the analysis panel starts open", await page.locator(".tuning__body").isVisible());
check("its dials are sliders, not range inputs", (await page.locator(".slider").count()) === 5);
check(
  "Snap starts off",
  (await page.locator(".tedit__control select").inputValue()) === "0",
  `value ${await page.locator(".tedit__control select").inputValue()}`,
);

// The fixture is recorded quietly, like a real take on a tablet. Drawn at absolute
// amplitude it is a flat line in a 96px lane, which reads as a broken waveform rather
// than as a quiet one — so the lane normalises, and says so.
const readoutText = (await page.locator(".tedit__readout").textContent()) ?? "";
check(
  "a quiet take is amplified to be visible",
  /waveform ×\d+/.test(readoutText),
  readoutText.trim(),
);

const closeBox = await page.locator(".listen__close").boundingBox();
check(
  "the close button is at least twice a control",
  closeBox.width >= 60 && closeBox.height >= 60,
  `${Math.round(closeBox.width)}×${Math.round(closeBox.height)}`,
);

for (const [label, selector] of [
  ["slider", ".slider__hit"],
  ["zoom button", ".tedit__zoom .btn"],
]) {
  const box = await page.locator(selector).first().boundingBox();
  check(`the ${label} is a finger-sized target`, box.height >= 44, `${Math.round(box.height)}px`);
}

// --- gestures --------------------------------------------------------------

const cdp = await context.newCDPSession(page);
const canvas = await page.locator(".tedit__canvas canvas").boundingBox();
/** A point inside the canvas, as a fraction of it: the lane is whatever height is left
    once the controls have taken theirs, so pixel offsets from its top-left miss. */
const at = (fx, fy) => ({
  x: canvas.x + canvas.width * fx,
  y: canvas.y + canvas.height * fy,
  radiusX: 8,
  radiusY: 8,
  force: 1,
});
async function touch(type, points) {
  await cdp.send("Input.dispatchTouchEvent", { type, touchPoints: points });
  await page.waitForTimeout(40);
}

const readout = () => page.locator(".tedit__readout").textContent();
const spanOf = async () => {
  const text = await readout();
  const match = /([\d.]+)s across/.exec(text ?? "");
  return match ? Number(match[1]) : Number(/([\d.]+)s take/.exec(text ?? "")?.[1] ?? 0);
};

const spanBefore = await spanOf();

// Fingers apart, sideways: time zooms in.
await touch("touchStart", [at(0.35, 0.55), at(0.55, 0.56)]);
await touch("touchMove", [at(0.28, 0.55), at(0.62, 0.56)]);
await touch("touchMove", [at(0.18, 0.55), at(0.72, 0.56)]);
await touch("touchEnd", []);
const spanAfterPinch = await spanOf();
check(
  "a sideways pinch zooms time in",
  spanAfterPinch < spanBefore,
  `${spanBefore}s → ${spanAfterPinch}s`,
);

// Fingers apart, up and down: pitch zooms, time is left alone.
const semitonesOf = async () => Number(/· (\d+)px a semitone/.exec((await readout()) ?? "")?.[1] ?? 0);
const pxBefore = await semitonesOf();
await touch("touchStart", [at(0.45, 0.45), at(0.46, 0.75)]);
await touch("touchMove", [at(0.45, 0.38), at(0.46, 0.85)]);
await touch("touchMove", [at(0.45, 0.30), at(0.46, 0.95)]);
await touch("touchEnd", []);
const pxAfter = await semitonesOf();
const spanAfterVertical = await spanOf();
check("an up-and-down pinch zooms pitch", pxAfter > pxBefore, `${pxBefore}px → ${pxAfter}px`);
check(
  "and leaves time where it was",
  Math.abs(spanAfterVertical - spanAfterPinch) < 0.01,
  `${spanAfterPinch}s → ${spanAfterVertical}s`,
);

await page.locator(".tedit__zoom button", { hasText: "Fit" }).click();
await page.waitForTimeout(200);
check("Fit shows the whole take again", (await spanOf()) >= spanBefore - 0.01);

// --- one finger still edits ------------------------------------------------

// Where the notes are on screen depends on the pitch range of the take, so the tap
// walks down the first note's column until it lands on one rather than assuming a row.
let grabbed = 0;
let landed = 0;
for (const fy of [0.55, 0.65, 0.72, 0.8, 0.88, 0.95]) {
  await page.locator(".tedit__canvas canvas").tap({
    position: { x: canvas.width * 0.1, y: canvas.height * fy },
  });
  await page.waitForTimeout(200);
  grabbed = await page.locator(".tedit__note-name").count();
  if (grabbed === 1) {
    landed = fy;
    break;
  }
}
check("one finger selects a note", grabbed === 1, `at ${landed || "no"} height`);

if (grabbed === 1) {
  const nameBefore = await page.locator(".tedit__note-name").textContent();
  await page.locator(".tedit__note button").last().click(); // ▲ up a semitone
  await page.waitForTimeout(300);
  const nameAfter = await page.locator(".tedit__note-name").textContent();
  check("the arrow steps it one semitone", nameBefore !== nameAfter, `${nameBefore} → ${nameAfter}`);
  check(
    "and says how far the take actually was",
    (await page.locator(".tedit__note-cents").count()) === 1,
    (await page.locator(".tedit__note-cents").textContent()) ?? "absent",
  );
}

// --- the loop --------------------------------------------------------------

await page.locator(".listen button", { hasText: /Loop/ }).first().click();
await page.locator(".listen button", { hasText: /Play/ }).first().click();
await page.waitForTimeout(600);
check("playing with the loop on keeps playing", await page.locator(".listen button", { hasText: /Stop/ }).isVisible());
await page.waitForTimeout(1200);
check("the loop is still running a second later", await page.locator(".listen button", { hasText: /Stop/ }).isVisible());
await page.locator(".listen button", { hasText: /Stop/ }).first().click();
await page.waitForTimeout(200);
check("stop stops it", await page.locator(".listen button", { hasText: /Play/ }).isVisible());

// --- marking a stretch to loop ---------------------------------------------

// A drag across the waveform lane marks it; a press without travel plays from there.
await touch("touchStart", [at(0.3, 0.06)]);
await touch("touchMove", [at(0.4, 0.06)]);
await touch("touchMove", [at(0.55, 0.06)]);
await touch("touchEnd", []);
const marked = page.locator(".tedit__loop button").nth(1);
check("dragging the waveform marks a stretch", (await marked.count()) === 1);
if ((await marked.count()) === 1) {
  const label = await marked.textContent();
  const seconds = Number(/([\d.]+)s/.exec(label ?? "")?.[1] ?? 0);
  check("of about the length that was dragged", seconds > 0.3 && seconds < 3, `${seconds}s`);
  check(
    "and turns looping on",
    (await page.locator(".listen button[aria-pressed=true]", { hasText: /Loop/ }).count()) === 1,
  );
  await marked.click();
  await page.waitForTimeout(200);
  check("clearing it goes back to looping the window", (await marked.count()) === 0);
}

// --- the dials ------------------------------------------------------------

const firstDial = page.locator(".slider").first();
const valueOf = () => firstDial.locator(".slider__value").textContent();
const dialBefore = await valueOf();
const dialBox = await firstDial.locator(".slider__hit").boundingBox();
await page.mouse.move(dialBox.x + dialBox.width * 0.5, dialBox.y + dialBox.height / 2);
await page.mouse.down();
await page.mouse.move(dialBox.x + dialBox.width * 0.85, dialBox.y + dialBox.height / 2, { steps: 6 });
await page.mouse.up();
await page.waitForTimeout(200);
const dialAfter = await valueOf();
check("a dial follows the pointer", dialAfter !== dialBefore, `${dialBefore} → ${dialAfter}`);
check(
  "and says the result on screen is now stale",
  (await page.locator(".tuning--pending").count()) === 1,
);
check(
  "with the re-read offered over the take",
  await page.locator(".tedit__reprocess").isVisible(),
);

await page.locator(".tuning__head button", { hasText: /Reset/ }).click();
await page.waitForTimeout(200);
check("Reset puts them back", (await valueOf()) === dialBefore, `${await valueOf()}`);

// --- one decision, not two -------------------------------------------------

if (SHOTS) await page.screenshot({ path: join(SHOTS, "listen-review.png") });

await page.locator(".listen button", { hasText: /Add to track/ }).click();
await page.waitForTimeout(700);
check("Add to track closes the stage", (await page.locator(".listen").count()) === 0);
check(
  "and asks nothing further",
  (await page.locator("button", { hasText: /Add to track/ }).count()) === 0,
);

for (const error of errors) {
  failures += 1;
  console.log(`   ✗ page error: ${error}`);
}

await browser.close();
console.log(failures === 0 ? "\nListen editor: clean\n" : `\nListen editor: ${failures} failure(s)\n`);
process.exit(failures === 0 ? 0 : 1);
