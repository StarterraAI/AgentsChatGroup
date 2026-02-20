#!/usr/bin/env node

const { execSync, spawn, spawnSync } = require("child_process");
const fs = require("fs");
const os = require("os");
const path = require("path");
const AdmZip = require("adm-zip");

const {
  ensureBinary,
  getLatestVersion,
  BINARY_TAG,
  CACHE_DIR,
  LOCAL_DEV_MODE,
  LOCAL_DIST_DIR,
  resolveRemoteSource,
} = require("./download");

const CLI_VERSION = require("../package.json").version;

const APP_NAME = "agents-chatgroup";
const APP_BINARY_BASE = "agents-chatgroup";

const INSTALL_DIR = path.join(os.homedir(), ".agents-chatgroup");
const BIN_DIR = path.join(INSTALL_DIR, "bin");
const METADATA_PATH = path.join(INSTALL_DIR, "install.json");

function printBanner() {
  console.log("");
  console.log("  ===============================================");
  console.log("            Agents ChatGroup Binary Installer");
  console.log("  ===============================================");
  console.log("");
}

function printInfo(message) {
  console.log(`  ${message}`);
}

function printStep(step, message) {
  console.log(`  [${step}] ${message}`);
}

function printSuccess(message) {
  console.log(`  OK  ${message}`);
}

function printWarning(message) {
  console.log(`  WARN  ${message}`);
}

function printError(message) {
  console.error(`  ERROR  ${message}`);
}

function getEffectiveArch() {
  const platform = process.platform;
  const nodeArch = process.arch;

  if (platform === "darwin") {
    if (nodeArch === "arm64") return "arm64";

    try {
      const translated = execSync("sysctl -in sysctl.proc_translated", {
        encoding: "utf8",
      }).trim();
      if (translated === "1") return "arm64";
    } catch (_err) {
      // Ignore and fallback to x64.
    }

    return "x64";
  }

  if (/arm/i.test(nodeArch)) {
    return "arm64";
  }

  if (platform === "win32") {
    const architecture = process.env.PROCESSOR_ARCHITECTURE || "";
    const wow64 = process.env.PROCESSOR_ARCHITEW6432 || "";
    if (/arm/i.test(architecture) || /arm/i.test(wow64)) {
      return "arm64";
    }
  }

  return "x64";
}

function getPlatformTarget() {
  const platform = process.platform;
  const arch = getEffectiveArch();

  if (platform === "linux" && arch === "x64") {
    return { platformDir: "linux-x64", note: null };
  }

  if (platform === "linux" && arch === "arm64") {
    return { platformDir: "linux-arm64", note: null };
  }

  if (platform === "darwin" && arch === "x64") {
    return { platformDir: "macos-x64", note: null };
  }

  if (platform === "darwin" && arch === "arm64") {
    return { platformDir: "macos-arm64", note: null };
  }

  if (platform === "win32" && arch === "x64") {
    return { platformDir: "windows-x64", note: null };
  }

  if (platform === "win32" && arch === "arm64") {
    return {
      platformDir: "windows-x64",
      note:
        "Windows ARM64 binary is not published yet. Falling back to windows-x64 binary.",
    };
  }

  printError(`Unsupported platform: ${platform}-${arch}`);
  printError("Supported targets: linux-x64, linux-arm64, macos-x64, macos-arm64, windows-x64");
  process.exit(1);
}

function getBinaryName(baseName) {
  return process.platform === "win32" ? `${baseName}.exe` : baseName;
}

function getInstalledBinaryPath() {
  return path.join(BIN_DIR, getBinaryName(APP_BINARY_BASE));
}

function readInstallMetadata() {
  if (!fs.existsSync(METADATA_PATH)) {
    return null;
  }

  try {
    return JSON.parse(fs.readFileSync(METADATA_PATH, "utf8"));
  } catch (_err) {
    return null;
  }
}

function writeInstallMetadata(binaryPath, platformDir) {
  const remoteSource = resolveRemoteSource();
  const metadata = {
    app: APP_NAME,
    cliVersion: CLI_VERSION,
    binaryTag: BINARY_TAG,
    platform: platformDir,
    binaryPath,
    installedAt: new Date().toISOString(),
    source: LOCAL_DEV_MODE
      ? "local-dist"
      : remoteSource
        ? `${remoteSource.provider}:${remoteSource.baseUrl}`
        : "unconfigured",
  };

  fs.mkdirSync(INSTALL_DIR, { recursive: true });
  fs.writeFileSync(METADATA_PATH, JSON.stringify(metadata, null, 2), "utf8");
}

function prependPathForCurrentProcess() {
  const currentPath = process.env.PATH || "";
  const entries = currentPath.split(path.delimiter).filter(Boolean);
  if (!entries.includes(BIN_DIR)) {
    process.env.PATH = [BIN_DIR, ...entries].join(path.delimiter);
  }
}

function appendLineIfMissing(filePath, line) {
  let content = "";
  if (fs.existsSync(filePath)) {
    content = fs.readFileSync(filePath, "utf8");
    if (content.includes(line)) {
      return false;
    }
  }

  if (content.length > 0 && !content.endsWith("\n")) {
    content += "\n";
  }
  content += `${line}\n`;
  fs.writeFileSync(filePath, content, "utf8");
  return true;
}

function persistPathOnUnix() {
  const exportLine = 'export PATH="$HOME/.agents-chatgroup/bin:$PATH"';
  const home = os.homedir();
  const files = [
    path.join(home, ".zshrc"),
    path.join(home, ".bashrc"),
    path.join(home, ".profile"),
  ];

  let changedCount = 0;
  for (const rcFile of files) {
    const isProfile = rcFile.endsWith(`${path.sep}.profile`);
    if (!isProfile && !fs.existsSync(rcFile)) {
      continue;
    }

    try {
      if (appendLineIfMissing(rcFile, exportLine)) {
        changedCount += 1;
      }
    } catch (_err) {
      // Ignore write failures for non-primary shell files.
    }
  }

  return changedCount;
}

function persistPathOnWindows() {
  const escapedBinDir = BIN_DIR.replace(/'/g, "''");
  const script = [
    `$bin='${escapedBinDir}'`,
    "$current=[Environment]::GetEnvironmentVariable('Path','User')",
    "if(-not $current){$current=''}",
    "$parts=($current -split ';') | Where-Object { $_ -and $_.Trim() -ne '' }",
    "if($parts -contains $bin){'UNCHANGED'} else {",
    "  $new=if([string]::IsNullOrWhiteSpace($current)){$bin}else{\"$bin;$current\"}",
    "  [Environment]::SetEnvironmentVariable('Path',$new,'User')",
    "  'UPDATED'",
    "}",
  ].join("; ");

  const result = spawnSync(
    "powershell",
    [
      "-NoProfile",
      "-NonInteractive",
      "-ExecutionPolicy",
      "Bypass",
      "-Command",
      script,
    ],
    {
      encoding: "utf8",
    },
  );

  if (result.status !== 0) {
    throw new Error((result.stderr || result.stdout || "Failed to update PATH").trim());
  }

  return (result.stdout || "").includes("UPDATED");
}

function ensurePathConfigured() {
  prependPathForCurrentProcess();

  try {
    if (process.platform === "win32") {
      const changed = persistPathOnWindows();
      if (changed) {
        printInfo("Added install directory to user PATH.");
      }
      return;
    }

    const changedCount = persistPathOnUnix();
    if (changedCount > 0) {
      printInfo("Added install directory to shell PATH profile.");
    }
  } catch (err) {
    printWarning(`Could not persist PATH automatically: ${err.message}`);
  }
}

function showProgress(downloaded, total) {
  const percent = total > 0 ? Math.round((downloaded / total) * 100) : 0;
  const downloadedMb = (downloaded / (1024 * 1024)).toFixed(1);
  const totalMb = total > 0 ? (total / (1024 * 1024)).toFixed(1) : "?";
  process.stderr.write(
    `\r   Downloading: ${downloadedMb}MB / ${totalMb}MB (${percent}%)`,
  );
}

function cleanupOldCaches() {
  if (!fs.existsSync(CACHE_DIR)) {
    return;
  }

  for (const entry of fs.readdirSync(CACHE_DIR, { withFileTypes: true })) {
    if (!entry.isDirectory() || entry.name === BINARY_TAG) {
      continue;
    }

    try {
      fs.rmSync(path.join(CACHE_DIR, entry.name), {
        recursive: true,
        force: true,
      });
    } catch (_err) {
      // Ignore cleanup failures.
    }
  }
}

function extractBinary(zipPath) {
  fs.mkdirSync(BIN_DIR, { recursive: true });

  const expectedPath = getInstalledBinaryPath();
  try {
    if (fs.existsSync(expectedPath)) {
      fs.unlinkSync(expectedPath);
    }
  } catch (_err) {
    // Ignore cleanup failure before extraction.
  }

  const zip = new AdmZip(zipPath);
  zip.extractAllTo(BIN_DIR, true);

  let binaryPath = expectedPath;
  if (!fs.existsSync(binaryPath)) {
    const candidates = fs
      .readdirSync(BIN_DIR)
      .filter((name) =>
        [APP_BINARY_BASE, `${APP_BINARY_BASE}.exe`].includes(name),
      );

    if (candidates.length > 0) {
      binaryPath = path.join(BIN_DIR, candidates[0]);
    }
  }

  if (!fs.existsSync(binaryPath)) {
    throw new Error(
      `Extracted binary not found in ${BIN_DIR}. The archive may be invalid.`,
    );
  }

  if (process.platform !== "win32") {
    try {
      fs.chmodSync(binaryPath, 0o755);
    } catch (_err) {
      // Ignore chmod failures and try to run anyway.
    }
  }

  return binaryPath;
}

async function installBinary(options = {}) {
  const { force = false } = options;
  const target = getPlatformTarget();
  const zipCachePath = path.join(
    CACHE_DIR,
    BINARY_TAG,
    target.platformDir,
    `${APP_BINARY_BASE}.zip`,
  );

  if (target.note) {
    printWarning(target.note);
  }

  printStep("1/3", "Preparing binary package...");
  fs.mkdirSync(INSTALL_DIR, { recursive: true });

  if (force) {
    try {
      fs.rmSync(zipCachePath, { force: true });
      fs.rmSync(getInstalledBinaryPath(), { force: true });
    } catch (_err) {
      // Ignore force-clean failures.
    }
  }

  printStep("2/3", "Downloading prebuilt binary...");
  let zipPath;
  try {
    zipPath = await ensureBinary(target.platformDir, APP_BINARY_BASE, showProgress);
    if (!LOCAL_DEV_MODE) {
      process.stderr.write("\n");
    }
  } catch (err) {
    process.stderr.write("\n");
    throw new Error(`Download failed: ${err.message}`);
  }

  printStep("3/3", "Extracting and installing binary...");
  const binaryPath = extractBinary(zipPath);

  ensurePathConfigured();
  writeInstallMetadata(binaryPath, target.platformDir);

  if (!LOCAL_DEV_MODE) {
    cleanupOldCaches();
  }

  printSuccess(`Installed ${APP_NAME} to ${binaryPath}`);
  return binaryPath;
}

function isInstalled() {
  return fs.existsSync(getInstalledBinaryPath());
}

async function ensureInstalled() {
  if (isInstalled()) {
    prependPathForCurrentProcess();
    return getInstalledBinaryPath();
  }

  printInfo("No local installation found. Installing now...");
  return installBinary();
}

function launchBinary(binaryPath, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(binaryPath, args, {
      stdio: "inherit",
      env: process.env,
    });

    child.on("error", (err) => reject(err));
    child.on("exit", (code) => resolve(code || 0));

    process.on("SIGINT", () => {
      child.kill("SIGINT");
    });
    process.on("SIGTERM", () => {
      child.kill("SIGTERM");
    });
  });
}

function showHelp() {
  console.log(`
Usage: npx agents-chatgroup [command] [args]

Commands:
  install       Download and install prebuilt binary only
  start         Install if needed, then run binary
  update        Force re-download and reinstall current binary tag
  status        Show installation and binary source status
  uninstall     Remove local installation under ~/.agents-chatgroup
  --help, -h    Show help
  --version     Show CLI version

Default behavior:
  npx agents-chatgroup
  -> install (if needed) + run binary

Pass-through args:
  npx agents-chatgroup -- --port 54321
  npx agents-chatgroup start --port 54321
`);
}

async function showStatus() {
  printBanner();

  const target = getPlatformTarget();
  const metadata = readInstallMetadata();
  const installed = isInstalled();

  printInfo(`CLI version: ${CLI_VERSION}`);
  printInfo(`Binary tag: ${BINARY_TAG}`);
  printInfo(`Platform target: ${target.platformDir}`);
  printInfo(`Install dir: ${INSTALL_DIR}`);
  printInfo(`Binary path: ${getInstalledBinaryPath()}`);
  printInfo(`Installed: ${installed ? "yes" : "no"}`);

  if (target.note) {
    printWarning(target.note);
  }

  if (metadata) {
    printInfo(`Installed at: ${metadata.installedAt}`);
    printInfo(`Binary source: ${metadata.source}`);
  }

  if (LOCAL_DEV_MODE) {
    printInfo(`Source mode: local dist (${LOCAL_DIST_DIR})`);
  } else {
    const remoteSource = resolveRemoteSource();
    if (remoteSource) {
      printInfo(
        `Source mode: ${remoteSource.provider.toUpperCase()} (${remoteSource.baseUrl})`,
      );
    } else {
      printWarning(
        "Source mode: unconfigured (set AGENTS_CHATGROUP_OSS_BASE_URL or AGENTS_CHATGROUP_R2_BASE_URL).",
      );
    }
  }

  try {
    if (!LOCAL_DEV_MODE) {
      const latest = await getLatestVersion();
      if (latest) {
        printInfo(`Latest published version: ${latest}`);
      }
    }
  } catch (_err) {
    // Ignore remote check failures in status.
  }

  console.log("");
}

function uninstall() {
  printBanner();
  printStep("1/1", "Removing local installation...");

  if (fs.existsSync(INSTALL_DIR)) {
    fs.rmSync(INSTALL_DIR, { recursive: true, force: true });
    printSuccess("Uninstalled successfully.");
    printWarning("If PATH was persisted, remove ~/.agents-chatgroup/bin manually from shell profile if needed.");
  } else {
    printWarning("Nothing to uninstall. Installation directory does not exist.");
  }

  console.log("");
}

function checkNodeVersion() {
  const major = Number.parseInt(process.versions.node.split(".")[0], 10);
  if (Number.isNaN(major) || major < 18) {
    printError(`Node.js 18+ is required, found v${process.versions.node}`);
    process.exit(1);
  }
}

async function main() {
  checkNodeVersion();

  const rawArgs = process.argv.slice(2);
  const args = rawArgs[0] === "--" ? rawArgs.slice(1) : rawArgs;
  const command = args[0];

  if (command === "--help" || command === "-h") {
    printBanner();
    showHelp();
    return;
  }

  if (command === "--version" || command === "-v") {
    console.log(`${APP_NAME} CLI v${CLI_VERSION}`);
    return;
  }

  if (command === "status") {
    await showStatus();
    return;
  }

  if (command === "uninstall") {
    uninstall();
    return;
  }

  if (command === "install") {
    printBanner();
    await installBinary();
    console.log("");
    printInfo("Run `npx agents-chatgroup start` to launch.");
    console.log("");
    return;
  }

  if (command === "update") {
    printBanner();
    await installBinary({ force: true });
    console.log("");
    printInfo("Update completed.");
    console.log("");
    return;
  }

  let runArgs = args;
  if (command === "start") {
    runArgs = args.slice(1);
  }

  printBanner();
  const binaryPath = await ensureInstalled();
  printInfo(`Launching ${APP_NAME}...`);
  console.log("");

  const exitCode = await launchBinary(binaryPath, runArgs);
  process.exit(exitCode);
}

main().catch((err) => {
  printError(err.message || String(err));
  if (process.env.AGENTS_CHATGROUP_DEBUG === "1") {
    console.error(err.stack || err);
  }
  process.exit(1);
});
