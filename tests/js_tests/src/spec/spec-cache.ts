import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import type { SpecBinding } from "../config/types";
import type { SpecFileLock, SpecLockEntry } from "./spec-sync-manifest";
import { SPEC_LOCK_MANIFEST } from "./spec-sync-manifest";

const DEFAULT_SPEC_CACHE_DIR = path.resolve(
  process.cwd(),
  ".cache",
  "specs",
  "starknet",
);

function computeSha256(content: Buffer): string {
  return crypto.createHash("sha256").update(content).digest("hex");
}

function getManifestEntry(specTag: string): SpecLockEntry {
  const entry = SPEC_LOCK_MANIFEST.find((item) => item.tag === specTag);
  if (!entry) {
    throw new Error(`Missing spec lock manifest entry for ${specTag}`);
  }
  return entry;
}

function getFileLock(entry: SpecLockEntry, fileName: string): SpecFileLock {
  const file = entry.files.find((candidate) => candidate.file === fileName);
  if (!file) {
    throw new Error(
      `Spec lock manifest for ${entry.tag} does not include ${fileName}`,
    );
  }
  return file;
}

function normalizeCacheDir(value: string): string {
  if (!value) {
    return DEFAULT_SPEC_CACHE_DIR;
  }
  return path.isAbsolute(value) ? value : path.resolve(process.cwd(), value);
}

export function resolveSpecCacheDir(): string {
  return normalizeCacheDir(process.env.STARKNET_SPEC_CACHE_DIR || "");
}

export function resolveCachedSpecFilePath(specTag: string, fileName: string): string {
  return path.resolve(resolveSpecCacheDir(), specTag, fileName);
}

function isValidCachedFile(filePath: string, expectedSha256: string): boolean {
  if (!fs.existsSync(filePath)) {
    return false;
  }
  const content = fs.readFileSync(filePath);
  const actual = computeSha256(content);
  return actual === expectedSha256;
}

async function fetchSpecBytes(sourceUrl: string): Promise<Buffer> {
  const response = await fetch(sourceUrl);
  if (!response.ok) {
    throw new Error(
      `Failed to fetch Starknet spec from ${sourceUrl}: HTTP ${response.status}`,
    );
  }
  const bytes = await response.arrayBuffer();
  return Buffer.from(bytes);
}

async function cacheSingleFile(specTag: string, fileLock: SpecFileLock): Promise<void> {
  const targetPath = resolveCachedSpecFilePath(specTag, fileLock.file);

  if (isValidCachedFile(targetPath, fileLock.sha256)) {
    return;
  }

  const bytes = await fetchSpecBytes(fileLock.sourceUrl);
  const downloadedSha256 = computeSha256(bytes);

  if (downloadedSha256 !== fileLock.sha256) {
    throw new Error(
      `Checksum mismatch for ${specTag}/${fileLock.file}: expected ${fileLock.sha256}, got ${downloadedSha256}`,
    );
  }

  fs.mkdirSync(path.dirname(targetPath), { recursive: true });
  fs.writeFileSync(targetPath, bytes);
}

export async function ensureSpecCached(binding: SpecBinding): Promise<void> {
  const entry = getManifestEntry(binding.specTag);
  const requiredFiles = new Set(Object.values(binding.files));

  for (const fileName of requiredFiles) {
    await cacheSingleFile(binding.specTag, getFileLock(entry, fileName));
  }
}
