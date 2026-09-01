import React from "react";
import {
  Database,
  FileArchive,
  FileCode2,
  FileImage,
  FileJson2,
  FileTerminal,
  FileText,
  Minus,
  Package,
  Plus,
  RotateCcw,
  Settings2,
  ShieldAlert,
  type LucideIcon,
} from "lucide-react";
import {
  siAngular,
  siAstro,
  siBun,
  siC,
  siCplusplus,
  siCss,
  siDart,
  siDeno,
  siDocker,
  siDotenv,
  siDotnet,
  siEslint,
  siGit,
  siGnubash,
  siGo,
  siGraphql,
  siHtml5,
  siJavascript,
  siJson,
  siKotlin,
  siLess,
  siLua,
  siMarkdown,
  siNextdotjs,
  siNpm,
  siOpenjdk,
  siPhp,
  siPnpm,
  siPostgresql,
  siPrettier,
  siPython,
  siR,
  siReact,
  siRuby,
  siRust,
  siSass,
  siSqlite,
  siSvelte,
  siSvg,
  siSwift,
  siTailwindcss,
  siTerraform,
  siToml,
  siTypescript,
  siVite,
  siVuedotjs,
  siXml,
  siYaml,
  siYarn,
  siZig,
} from "simple-icons";
import type { SimpleIcon } from "simple-icons";
import type { SourceControlDiffArea, SourceControlFile } from "@/types";
import { resolveLocalPathToAbsolutePath } from "@/utils/readOnlyLinks";
import {
  getFileActionDisabledReason,
  sourceControlStatusLabel,
  translateSourceControl,
  type SourceControlFileAction,
  type SourceControlPanelViewModel,
  type SourceControlTranslator,
} from "./sourceControlViewModel";

interface SourceControlFileRowProps {
  file: SourceControlFile;
  area: SourceControlDiffArea;
  viewModel: SourceControlPanelViewModel;
  pending: boolean;
  selected: boolean;
  rowRef: (node: HTMLDivElement | null) => void;
  t: SourceControlTranslator;
  onOpenDiff: (file: SourceControlFile, area: SourceControlDiffArea) => void;
  onStage: (file: SourceControlFile) => void;
  onUnstage: (file: SourceControlFile) => void;
  onDiscard: (file: SourceControlFile) => void;
  onSelect: (path: string) => void;
}

const statusTone: Record<SourceControlFile["status"], string> = {
  modified: "text-[color-mix(in_srgb,#f59e0b_62%,var(--ink-subtle))]",
  added: "text-[color-mix(in_srgb,var(--success)_62%,var(--ink-subtle))]",
  deleted: "text-[color-mix(in_srgb,#f43f5e_68%,var(--ink-subtle))]",
  untracked: "text-[color-mix(in_srgb,#38bdf8_56%,var(--ink-subtle))]",
  renamed: "text-[color-mix(in_srgb,var(--primary)_62%,var(--ink-subtle))]",
  copied: "text-[color-mix(in_srgb,#22d3ee_54%,var(--ink-subtle))]",
  type_changed: "text-[color-mix(in_srgb,#fb923c_58%,var(--ink-subtle))]",
};

const exactFileTypeIcons: Record<string, SimpleIcon> = {
  ".dockerignore": siDocker,
  ".env": siDotenv,
  ".eslintignore": siEslint,
  ".eslintrc": siEslint,
  ".gitignore": siGit,
  ".npmrc": siNpm,
  ".prettierrc": siPrettier,
  "angular.json": siAngular,
  "bun.lock": siBun,
  "bun.lockb": siBun,
  "cargo.lock": siRust,
  "cargo.toml": siRust,
  dockerfile: siDocker,
  "deno.json": siDeno,
  "deno.jsonc": siDeno,
  "eslint.config.cjs": siEslint,
  "eslint.config.js": siEslint,
  "eslint.config.mjs": siEslint,
  "eslint.config.ts": siEslint,
  "go.mod": siGo,
  "go.sum": siGo,
  "jsconfig.json": siJavascript,
  "next.config.js": siNextdotjs,
  "next.config.mjs": siNextdotjs,
  "next.config.ts": siNextdotjs,
  "package-lock.json": siNpm,
  "package.json": siNpm,
  "pnpm-lock.yaml": siPnpm,
  "prettier.config.cjs": siPrettier,
  "prettier.config.js": siPrettier,
  "prettier.config.mjs": siPrettier,
  "prettier.config.ts": siPrettier,
  "tailwind.config.cjs": siTailwindcss,
  "tailwind.config.js": siTailwindcss,
  "tailwind.config.mjs": siTailwindcss,
  "tailwind.config.ts": siTailwindcss,
  "tsconfig.json": siTypescript,
  "vite.config.js": siVite,
  "vite.config.mjs": siVite,
  "vite.config.ts": siVite,
  "yarn.lock": siYarn,
};

const extensionFileTypeIcons: Record<string, SimpleIcon> = {
  astro: siAstro,
  bash: siGnubash,
  c: siC,
  cc: siCplusplus,
  cpp: siCplusplus,
  cs: siDotnet,
  css: siCss,
  cts: siTypescript,
  cxx: siCplusplus,
  dart: siDart,
  env: siDotenv,
  go: siGo,
  gql: siGraphql,
  graphql: siGraphql,
  h: siC,
  hpp: siCplusplus,
  htm: siHtml5,
  html: siHtml5,
  hxx: siCplusplus,
  java: siOpenjdk,
  js: siJavascript,
  json: siJson,
  jsonc: siJson,
  jsx: siReact,
  kt: siKotlin,
  kts: siKotlin,
  less: siLess,
  lua: siLua,
  md: siMarkdown,
  mdx: siReact,
  mjs: siJavascript,
  mts: siTypescript,
  php: siPhp,
  pgsql: siPostgresql,
  psql: siPostgresql,
  py: siPython,
  r: siR,
  rb: siRuby,
  rs: siRust,
  sass: siSass,
  scss: siSass,
  sh: siGnubash,
  sql: siSqlite,
  svg: siSvg,
  svelte: siSvelte,
  swift: siSwift,
  tf: siTerraform,
  toml: siToml,
  ts: siTypescript,
  tsx: siReact,
  vue: siVuedotjs,
  xml: siXml,
  yaml: siYaml,
  yml: siYaml,
  zig: siZig,
  zsh: siGnubash,
};

const getFileName = (path: string) =>
  path.split(/[\\/]/).pop()?.toLowerCase() ?? path.toLowerCase();

const getFileExtension = (fileName: string) => {
  if (fileName.startsWith(".env")) return "env";
  const dotIndex = fileName.lastIndexOf(".");
  if (dotIndex === 0 && fileName.indexOf(".", 1) === -1) {
    return fileName.slice(1);
  }
  return dotIndex > 0 && dotIndex < fileName.length - 1
    ? fileName.slice(dotIndex + 1)
    : "";
};

const imageExtensions = new Set([
  "avif",
  "bmp",
  "gif",
  "ico",
  "jpeg",
  "jpg",
  "png",
  "webp",
]);

const archiveExtensions = new Set([
  "7z",
  "br",
  "gz",
  "rar",
  "tar",
  "tgz",
  "xz",
  "zip",
]);

const terminalExtensions = new Set(["bat", "cmd", "fish", "ps1"]);
const configExtensions = new Set(["cfg", "conf", "ini"]);
const textExtensions = new Set(["csv", "log", "txt"]);

const getFallbackFileIcon = (
  fileName: string,
  extension: string,
): LucideIcon => {
  if (fileName === "makefile") return FileTerminal;
  if (imageExtensions.has(extension)) return FileImage;
  if (archiveExtensions.has(extension)) return FileArchive;
  if (terminalExtensions.has(extension)) return FileTerminal;
  if (configExtensions.has(extension)) return Settings2;
  if (textExtensions.has(extension)) return FileText;
  if (extension === "lock") return Package;
  if (extension === "json") return FileJson2;
  if (["db", "sqlite", "sqlite3"].includes(extension)) return Database;
  if (extension) return FileCode2;
  return FileText;
};

export function SourceControlFileTypeIcon({ path }: { path: string }) {
  const fileName = getFileName(path);
  const extension = getFileExtension(fileName);
  const simpleIcon =
    exactFileTypeIcons[fileName] ?? extensionFileTypeIcons[extension];

  if (simpleIcon) {
    return (
      <svg
        aria-hidden="true"
        viewBox="0 0 24 24"
        className="h-3.5 w-3.5 shrink-0 text-[var(--ink-tertiary)] opacity-80"
        fill="currentColor"
      >
        <path d={simpleIcon.path} />
      </svg>
    );
  }

  const FallbackIcon = getFallbackFileIcon(fileName, extension);
  return (
    <FallbackIcon
      aria-hidden="true"
      className="h-3.5 w-3.5 shrink-0 text-[var(--ink-tertiary)] opacity-80"
      strokeWidth={1.5}
    />
  );
}

function SourceControlIconButton({
  title,
  disabled,
  onClick,
  children,
}: {
  title: string;
  disabled: boolean;
  onClick: (event: React.MouseEvent<HTMLButtonElement>) => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="flex h-6 w-6 items-center justify-center rounded-[6px] text-[var(--ink-tertiary)] opacity-0 transition hover:bg-[var(--surface-3)] hover:text-[var(--ink)] focus:opacity-100 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-[var(--primary)] group-hover/source-file:opacity-100 disabled:cursor-not-allowed disabled:opacity-35"
      title={title}
      aria-label={title}
    >
      {children}
    </button>
  );
}

function FileWarningIndicator({
  file,
  t,
}: {
  file: SourceControlFile;
  t: SourceControlTranslator;
}) {
  if (!file.shared && !file.blocked_reason) return null;

  const title =
    file.blocked_reason ??
    translateSourceControl(
      t,
      "sourceControl.sharedWithAnotherSession",
      "Shared with another active session",
    );

  return (
    <span
      className={`flex h-5 w-5 shrink-0 items-center justify-center rounded-[6px] ${
        file.shared ? "text-[var(--ink-subtle)]" : "text-rose-500"
      }`}
      title={title}
      aria-label={title}
    >
      <ShieldAlert className="h-3.5 w-3.5" strokeWidth={1.5} />
    </span>
  );
}

const disabledTitle = (
  baseTitle: string,
  reason: string | null,
  pending: boolean,
  t: SourceControlTranslator,
) => {
  if (pending) {
    return translateSourceControl(
      t,
      "sourceControl.operationRunning",
      "Source-control operation is running",
    );
  }
  if (reason) {
    return translateSourceControl(
      t,
      "sourceControl.disabledTitle",
      "{action}: {reason}",
      { action: baseTitle, reason },
    );
  }
  return baseTitle;
};

export const SourceControlFileRow: React.FC<SourceControlFileRowProps> = ({
  file,
  area,
  viewModel,
  pending,
  selected,
  rowRef,
  t,
  onOpenDiff,
  onStage,
  onUnstage,
  onDiscard,
  onSelect,
}) => {
  const stageDisabledReason = getFileActionDisabledReason(
    viewModel,
    file,
    "stage",
    t,
  );
  const unstageDisabledReason = getFileActionDisabledReason(
    viewModel,
    file,
    "unstage",
    t,
  );
  const discardDisabledReason = getFileActionDisabledReason(
    viewModel,
    file,
    "discard",
    t,
  );
  const isActionDisabled = (
    _action: SourceControlFileAction,
    reason: string | null,
  ) => pending || Boolean(reason);
  const stageLabel = translateSourceControl(
    t,
    "sourceControl.action.stage",
    "Stage",
  );
  const discardLabel = translateSourceControl(
    t,
    "sourceControl.action.discard",
    "Discard",
  );
  const unstageLabel = translateSourceControl(
    t,
    "sourceControl.action.unstage",
    "Unstage",
  );
  const fullPath =
    resolveLocalPathToAbsolutePath(file.path, viewModel.workspacePath) ??
    file.path;
  const fileName = file.path.split(/[\\/]/).pop() || file.path;

  return (
    <div
      ref={rowRef}
      data-source-control-path={file.path}
      tabIndex={selected ? 0 : -1}
      onFocus={() => onSelect(file.path)}
      onClick={() => onSelect(file.path)}
      className={`group/source-file relative flex min-h-8 w-full min-w-0 items-center gap-2 rounded-lg border bg-[color-mix(in_srgb,var(--surface-1)_76%,var(--canvas))] px-2 py-1 text-left text-[12px] transition-colors hover:border-[color-mix(in_srgb,var(--hairline)_72%,transparent)] hover:bg-[var(--surface-2)] focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-[var(--primary)] ${
        selected
          ? "border-[color-mix(in_srgb,var(--primary)_45%,var(--hairline))]"
          : "border-transparent"
      }`}
    >
      <div className="flex min-w-0 flex-1 items-center">
        <button
          type="button"
          onClick={() => onOpenDiff(file, area)}
          title={fullPath}
          data-tooltip-break-all
          data-tooltip-hover-only
          className="flex min-w-0 flex-1 items-center gap-2 text-left focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-[var(--primary)]"
          aria-label={translateSourceControl(
            t,
            "sourceControl.openDiffFor",
            "Open diff for {path}",
            { path: file.path },
          )}
        >
          <SourceControlFileTypeIcon path={file.path} />
          <span className="flex min-w-0 flex-1 items-baseline gap-1.5">
            <span className="min-w-0 shrink truncate font-mono text-[12px] text-[var(--ink)]">
              {fileName}
            </span>
            {fileName !== file.path && (
              <span className="min-w-0 flex-1 truncate font-mono text-[10px] text-[var(--ink-subtle)]">
                {file.path}
              </span>
            )}
          </span>
        </button>
      </div>

      <div className="flex w-10 shrink-0 items-center justify-end gap-1 transition-opacity group-hover/source-file:opacity-0 group-focus-within/source-file:opacity-0">
        <FileWarningIndicator file={file} t={t} />
        <span
          className={`w-4 text-right font-mono text-[11px] font-medium ${
            statusTone[file.status]
          }`}
        >
          {sourceControlStatusLabel(file.status)}
        </span>
      </div>

      <div className="pointer-events-none absolute right-2 top-1/2 flex -translate-y-1/2 items-center justify-end gap-0.5 rounded-[6px] bg-[var(--surface-2)] opacity-0 transition group-hover/source-file:pointer-events-auto group-hover/source-file:opacity-100 focus-within:pointer-events-auto focus-within:opacity-100">
        {area === "changes" ? (
          <>
            <SourceControlIconButton
              title={disabledTitle(stageLabel, stageDisabledReason, pending, t)}
              disabled={isActionDisabled("stage", stageDisabledReason)}
              onClick={(event) => {
                event.stopPropagation();
                onStage(file);
              }}
            >
              <Plus className="h-3.5 w-3.5" strokeWidth={1.5} />
            </SourceControlIconButton>
            <SourceControlIconButton
              title={disabledTitle(
                discardLabel,
                discardDisabledReason,
                pending,
                t,
              )}
              disabled={isActionDisabled("discard", discardDisabledReason)}
              onClick={(event) => {
                event.stopPropagation();
                onDiscard(file);
              }}
            >
              <RotateCcw className="h-3.5 w-3.5" strokeWidth={1.5} />
            </SourceControlIconButton>
          </>
        ) : (
          <SourceControlIconButton
            title={disabledTitle(
              unstageLabel,
              unstageDisabledReason,
              pending,
              t,
            )}
            disabled={isActionDisabled("unstage", unstageDisabledReason)}
            onClick={(event) => {
              event.stopPropagation();
              onUnstage(file);
            }}
          >
            <Minus className="h-3.5 w-3.5" strokeWidth={1.5} />
          </SourceControlIconButton>
        )}
        <FileWarningIndicator file={file} t={t} />
      </div>
    </div>
  );
};