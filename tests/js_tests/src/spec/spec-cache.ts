import * as fs from "fs";
import * as path from "path";
import * as crypto from "crypto";
import { SpecFileLock, SpecLockEntry, getSpecLock } from "./spec-manifest";

const CACHE_DIR = path.resolve(__dirname, "../../.cache/specs");

function cachedFilePath(tag: string, file: string): string {
  return path.join(CACHE_DIR, tag, file);
}

function computeSha256(data: Buffer): string {
  return crypto.createHash("sha256").update(data).digest("hex");
}

function isValidCached(filePath: string, expectedSha: string): boolean {
  if (!fs.existsSync(filePath)) return false;
  const data = fs.readFileSync(filePath);
  return computeSha256(data) === expectedSha;
}

async function downloadFile(url: string): Promise<Buffer> {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`Failed to download ${url}: ${response.status}`);
  }
  const arrayBuffer = await response.arrayBuffer();
  return Buffer.from(arrayBuffer);
}

async function cacheSingleFile(
  tag: string,
  fileLock: SpecFileLock,
): Promise<void> {
  const targetPath = cachedFilePath(tag, fileLock.file);

  if (isValidCached(targetPath, fileLock.sha256)) {
    return;
  }

  console.log(`[spec-cache] Downloading ${tag}/${fileLock.file}...`);
  const data = await downloadFile(fileLock.sourceUrl);
  const sha = computeSha256(data);

  if (sha !== fileLock.sha256) {
    throw new Error(
      `SHA-256 mismatch for ${tag}/${fileLock.file}: expected ${fileLock.sha256}, got ${sha}`,
    );
  }

  fs.mkdirSync(path.dirname(targetPath), { recursive: true });
  fs.writeFileSync(targetPath, data);
}

/**
 * Ensure all spec files for a version tag are downloaded and verified.
 */
export async function ensureSpecCached(tag: string): Promise<void> {
  const lock = getSpecLock(tag);
  if (!lock) {
    throw new Error(`No spec lock entry for tag "${tag}"`);
  }

  for (const fileLock of lock.files) {
    await cacheSingleFile(tag, fileLock);
  }
}

/**
 * Load a cached spec file as parsed JSON.
 */
export function loadCachedSpec(tag: string, file: string): any {
  const filePath = cachedFilePath(tag, file);
  if (!fs.existsSync(filePath)) {
    throw new Error(
      `Spec file not cached: ${tag}/${file}. Call ensureSpecCached first.`,
    );
  }
  return JSON.parse(fs.readFileSync(filePath, "utf-8"));
}
