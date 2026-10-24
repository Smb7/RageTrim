import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  createReadStream,
  createWriteStream,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, sep } from "node:path";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const pinPath = join(root, "src-tauri", "ffmpeg-bundle.json");
const outDir = join(root, "src-tauri", "resources", "ffmpeg");
const stampPath = join(outDir, ".stamp");
const pin = JSON.parse(readFileSync(pinPath, "utf8"));

if (!pin.url || !pin.sha256 || !pin.version) {
  throw new Error(`ffmpeg-bundle.json is missing url, sha256, or version: ${pinPath}`);
}

const expected = pin.sha256.toLowerCase();
const cacheDir = join(process.env.LOCALAPPDATA || tmpdir(), "RageTrim", "ffmpeg-cache");
const zipPath = join(cacheDir, `ffmpeg-${pin.version}-essentials_build.zip`);

function licenseNames(dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir).filter((name) => /^(license|copying)(\.|$)/i.test(name));
}

function alreadyFetched() {
  return (
    existsSync(join(outDir, "ffmpeg.exe")) &&
    existsSync(join(outDir, "ffprobe.exe")) &&
    existsSync(join(outDir, "SOURCE.txt")) &&
    licenseNames(outDir).length > 0 &&
    existsSync(stampPath) &&
    readFileSync(stampPath, "utf8").trim().toLowerCase() === expected
  );
}

function sha256File(path) {
  return new Promise((resolve, reject) => {
    const hash = createHash("sha256");
    const stream = createReadStream(path);
    stream.on("error", reject);
    stream.on("data", (chunk) => hash.update(chunk));
    stream.on("end", () => resolve(hash.digest("hex")));
  });
}

function walk(dir, found = []) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) walk(full, found);
    else if (entry.isFile()) found.push(full);
  }
  return found;
}

function preferBin(paths, fileName) {
  const matches = paths.filter((path) => basename(path).toLowerCase() === fileName);
  if (matches.length === 0) {
    throw new Error(`Pinned FFmpeg archive has no ${fileName}.`);
  }
  return (
    matches.find((path) => path.toLowerCase().includes(`${sep}bin${sep}`)) || matches[0]
  );
}

function licenseSources(paths) {
  const matches = paths.filter((path) => /^(license|copying)(\.|$)/i.test(basename(path)));
  matches.sort((left, right) => left.split(sep).length - right.split(sep).length);
  const picked = [];
  const seen = new Set();
  for (const path of matches) {
    const name = basename(path).toLowerCase();
    if (seen.has(name)) continue;
    seen.add(name);
    picked.push(path);
  }
  if (picked.length === 0) {
    throw new Error("Pinned FFmpeg archive has no LICENSE or COPYING file.");
  }
  return picked;
}

function runChecked(tool, args) {
  const result = spawnSync(tool, args, { encoding: "utf8", windowsHide: true });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(
      `${basename(tool)} ${args.join(" ")} failed:\n${result.stderr || result.stdout}`,
    );
  }
  return `${result.stdout || ""}\n${result.stderr || ""}`;
}

async function download() {
  mkdirSync(cacheDir, { recursive: true });
  if (existsSync(zipPath)) {
    const cached = await sha256File(zipPath);
    if (cached === expected) {
      console.log(`Using cached FFmpeg archive ${zipPath}`);
      return;
    }
    console.log("Cached FFmpeg archive checksum did not match. Downloading again.");
    rmSync(zipPath, { force: true });
  }
  console.log(`Downloading ${pin.url}`);
  const response = await fetch(pin.url, {
    redirect: "follow",
    signal: AbortSignal.timeout(15 * 60 * 1000),
    headers: { "user-agent": "RageTrim-ffmpeg-fetch" },
  });
  if (!response.ok || !response.body) {
    throw new Error(`FFmpeg download failed: ${response.status} ${response.statusText}`);
  }
  const partial = `${zipPath}.partial`;
  await pipeline(Readable.fromWeb(response.body), createWriteStream(partial));
  const actual = await sha256File(partial);
  if (actual !== expected) {
    rmSync(partial, { force: true });
    throw new Error(`FFmpeg checksum mismatch.\nexpected ${expected}\nactual   ${actual}`);
  }
  rmSync(zipPath, { force: true });
  renameSync(partial, zipPath);
}

async function extract() {
  const extractDir = mkdtempSync(join(tmpdir(), "ragetrim-ffmpeg-"));
  try {
    const extracted = spawnSync("tar", ["-xf", zipPath, "-C", extractDir], {
      encoding: "utf8",
      windowsHide: true,
    });
    if (extracted.error) throw extracted.error;
    if (extracted.status !== 0) {
      throw new Error(`Could not extract FFmpeg archive:\n${extracted.stderr || extracted.stdout}`);
    }
    const files = walk(extractDir);
    const ffmpeg = preferBin(files, "ffmpeg.exe");
    const ffprobe = preferBin(files, "ffprobe.exe");
    const licenses = licenseSources(files);
    rmSync(outDir, { recursive: true, force: true });
    mkdirSync(outDir, { recursive: true });
    copyFileSync(ffmpeg, join(outDir, "ffmpeg.exe"));
    copyFileSync(ffprobe, join(outDir, "ffprobe.exe"));
    for (const license of licenses) {
      copyFileSync(license, join(outDir, basename(license)));
    }
    writeFileSync(
      join(outDir, "SOURCE.txt"),
      [
        `FFmpeg ${pin.version} essentials static build`,
        `URL: ${pin.url}`,
        `SHA-256: ${expected}`,
        `Upstream: ${pin.source}`,
        "License: GNU General Public License version 3. See the LICENSE file in this folder.",
        "This program is not covered by RageTrim's MIT license.",
        "RageTrim starts ffmpeg.exe and ffprobe.exe as separate processes and does not link them.",
        "",
      ].join("\n"),
      "utf8",
    );
  } finally {
    rmSync(extractDir, { recursive: true, force: true });
  }
}

function verifyTools() {
  const ffmpeg = join(outDir, "ffmpeg.exe");
  const ffprobe = join(outDir, "ffprobe.exe");
  const version = runChecked(ffmpeg, ["-hide_banner", "-version"]);
  if (!/ffmpeg version/i.test(version)) {
    throw new Error("ffmpeg.exe did not report a version.");
  }
  const encoders = runChecked(ffmpeg, ["-hide_banner", "-encoders"]);
  if (!/\blibx264\b/.test(encoders)) {
    throw new Error("Pinned FFmpeg is missing libx264, which Discord compress needs.");
  }
  const probe = runChecked(ffprobe, ["-hide_banner", "-version"]);
  if (!/ffprobe version/i.test(probe)) {
    throw new Error("ffprobe.exe did not report a version.");
  }
  if (licenseNames(outDir).length === 0) {
    throw new Error("FFmpeg license file was not copied next to the binaries.");
  }
}

if (alreadyFetched()) {
  console.log(`FFmpeg ${pin.version} already present in ${outDir}`);
} else {
  await download();
  await extract();
  verifyTools();
  writeFileSync(stampPath, `${expected}\n`, "utf8");
  console.log(`Fetched FFmpeg ${pin.version} into ${outDir}`);
}
