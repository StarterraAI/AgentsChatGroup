const https = require("https");
const fs = require("fs");
const path = require("path");
const os = require("os");
const crypto = require("crypto");

// Replaced during npm pack by workflow.
const DEFAULT_OSS_BASE_URL = "__OSS_PUBLIC_URL__";
const DEFAULT_R2_BASE_URL = "__R2_PUBLIC_URL__";
const DEFAULT_BINARY_TAG = "__BINARY_TAG__";

const OSS_BASE_URL = normalizeBaseUrl(
  process.env.AGENTS_CHATGROUP_OSS_BASE_URL || DEFAULT_OSS_BASE_URL,
);
const R2_BASE_URL = normalizeBaseUrl(
  process.env.AGENTS_CHATGROUP_R2_BASE_URL || DEFAULT_R2_BASE_URL,
);
const BINARY_TAG =
  process.env.AGENTS_CHATGROUP_BINARY_TAG || DEFAULT_BINARY_TAG;

const INSTALL_DIR = path.join(os.homedir(), ".agents-chatgroup");
const CACHE_DIR = path.join(INSTALL_DIR, "cache");

// Local development mode: use binaries from agents-chatgroup-npx/dist/
const LOCAL_DIST_DIR = path.join(__dirname, "..", "dist");
const LOCAL_DEV_MODE =
  fs.existsSync(LOCAL_DIST_DIR) || process.env.AGENTS_CHATGROUP_LOCAL === "1";

function normalizeBaseUrl(url) {
  if (!url) return "";
  return url.replace(/\/+$/, "");
}

function isConfiguredBaseUrl(url, placeholder) {
  return Boolean(url) && !url.includes(placeholder);
}

function resolveRemoteSource() {
  if (isConfiguredBaseUrl(OSS_BASE_URL, DEFAULT_OSS_BASE_URL)) {
    return {
      provider: "oss",
      baseUrl: OSS_BASE_URL,
    };
  }

  if (isConfiguredBaseUrl(R2_BASE_URL, DEFAULT_R2_BASE_URL)) {
    return {
      provider: "r2",
      baseUrl: R2_BASE_URL,
    };
  }

  return null;
}

function ensureRemoteConfig() {
  if (LOCAL_DEV_MODE) return;

  const source = resolveRemoteSource();
  if (!source) {
    throw new Error(
      "Binary source URL is not configured. Set AGENTS_CHATGROUP_OSS_BASE_URL or AGENTS_CHATGROUP_R2_BASE_URL, or publish npm package with URL injection.",
    );
  }

  if (BINARY_TAG.includes("__BINARY_TAG__")) {
    throw new Error(
      "Binary tag is not configured. The npm package was published without binary tag injection.",
    );
  }

  return source;
}

function fetchJson(url) {
  return new Promise((resolve, reject) => {
    https
      .get(url, (res) => {
        if (res.statusCode === 301 || res.statusCode === 302) {
          return fetchJson(res.headers.location).then(resolve).catch(reject);
        }

        if (res.statusCode !== 200) {
          return reject(new Error(`HTTP ${res.statusCode} while fetching ${url}`));
        }

        let body = "";
        res.on("data", (chunk) => {
          body += chunk;
        });
        res.on("end", () => {
          try {
            resolve(JSON.parse(body));
          } catch (_err) {
            reject(new Error(`Invalid JSON response from ${url}`));
          }
        });
      })
      .on("error", reject);
  });
}

function downloadFile(url, destinationPath, expectedSha256, onProgress) {
  const tempPath = `${destinationPath}.tmp`;

  return new Promise((resolve, reject) => {
    const file = fs.createWriteStream(tempPath);
    const hash = crypto.createHash("sha256");

    const cleanup = () => {
      try {
        fs.unlinkSync(tempPath);
      } catch (_err) {
        // Ignore cleanup errors.
      }
    };

    https
      .get(url, (res) => {
        if (res.statusCode === 301 || res.statusCode === 302) {
          file.close();
          cleanup();
          return downloadFile(
            res.headers.location,
            destinationPath,
            expectedSha256,
            onProgress,
          )
            .then(resolve)
            .catch(reject);
        }

        if (res.statusCode !== 200) {
          file.close();
          cleanup();
          return reject(new Error(`HTTP ${res.statusCode} while downloading ${url}`));
        }

        const totalSize = Number.parseInt(res.headers["content-length"], 10);
        let downloadedSize = 0;

        res.on("data", (chunk) => {
          downloadedSize += chunk.length;
          hash.update(chunk);
          if (onProgress) {
            onProgress(downloadedSize, Number.isFinite(totalSize) ? totalSize : 0);
          }
        });

        res.pipe(file);

        file.on("finish", () => {
          file.close();

          const actualSha256 = hash.digest("hex");
          if (expectedSha256 && actualSha256 !== expectedSha256) {
            cleanup();
            return reject(
              new Error(
                `Checksum mismatch, expected ${expectedSha256}, got ${actualSha256}`,
              ),
            );
          }

          try {
            fs.renameSync(tempPath, destinationPath);
            resolve(destinationPath);
          } catch (err) {
            cleanup();
            reject(err);
          }
        });
      })
      .on("error", (err) => {
        file.close();
        cleanup();
        reject(err);
      });
  });
}

async function ensureBinary(platform, binaryName, onProgress) {
  if (LOCAL_DEV_MODE) {
    const localZipPath = path.join(LOCAL_DIST_DIR, platform, `${binaryName}.zip`);
    if (fs.existsSync(localZipPath)) {
      return localZipPath;
    }

    throw new Error(
      `Local binary not found: ${localZipPath}\nRun your local binary packaging first.`,
    );
  }

  const source = ensureRemoteConfig();

  const platformCacheDir = path.join(CACHE_DIR, BINARY_TAG, platform);
  const zipPath = path.join(platformCacheDir, `${binaryName}.zip`);

  if (fs.existsSync(zipPath)) {
    return zipPath;
  }

  fs.mkdirSync(platformCacheDir, { recursive: true });

  const manifest = await fetchJson(
    `${source.baseUrl}/binaries/${BINARY_TAG}/manifest.json`,
  );
  const binaryInfo = manifest.platforms?.[platform]?.[binaryName];

  if (!binaryInfo) {
    throw new Error(
      `Binary ${binaryName} is not available for platform ${platform} in tag ${BINARY_TAG}.`,
    );
  }

  const binaryUrl = `${source.baseUrl}/binaries/${BINARY_TAG}/${platform}/${binaryName}.zip`;
  await downloadFile(binaryUrl, zipPath, binaryInfo.sha256, onProgress);

  return zipPath;
}

async function getLatestVersion() {
  if (LOCAL_DEV_MODE) return null;

  const source = ensureRemoteConfig();

  const manifest = await fetchJson(`${source.baseUrl}/binaries/manifest.json`);
  return manifest.latest || null;
}

module.exports = {
  OSS_BASE_URL,
  R2_BASE_URL,
  BINARY_TAG,
  CACHE_DIR,
  LOCAL_DEV_MODE,
  LOCAL_DIST_DIR,
  resolveRemoteSource,
  ensureBinary,
  getLatestVersion,
};
