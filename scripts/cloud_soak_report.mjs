import { execFile } from "node:child_process";
import { access, mkdir, rename, rm, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, isAbsolute, join } from "node:path";
import { promisify } from "node:util";
import { fileURLToPath, pathToFileURL } from "node:url";

const execFileAsync = promisify(execFile);
const MAX_COMMAND_OUTPUT_BYTES = 4 * 1024 * 1024;
const COMMAND_TIMEOUT_MS = 30_000;
const WINDOWS_GCLOUD_WRAPPER = fileURLToPath(new URL("./invoke_gcloud.ps1", import.meta.url));
const PROJECT_ID = "investa-remote-bumniverse";
const REGION = "asia-northeast3";
const SUPPORTED_LOG_SCHEMAS = new Set(["investa.cloud-soak.v1", "investa.cloud-soak.v2"]);
const JOBS = [
  { mode: "market", jobName: "investa-market-soak-24h-v2" },
  { mode: "shadow-contract", jobName: "investa-shadow-contract-soak-24h" },
];

function asObject(value) {
  return value && typeof value === "object" && !Array.isArray(value) ? value : {};
}

function asFiniteNumber(value) {
  const number = Number(value);
  return Number.isFinite(number) ? number : undefined;
}

function timestampMs(value) {
  if (typeof value !== "string") return undefined;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : undefined;
}

function conditionState(execution) {
  const conditions = Array.isArray(execution?.status?.conditions) ? execution.status.conditions : [];
  const completed = conditions.find((item) => item?.type === "Completed");
  if (completed?.status === "True") return "completed";
  if (execution?.status?.cancelledCount > 0 || completed?.reason === "Cancelled") return "cancelled";
  if (completed?.status === "False" && completed?.reason) return "failed";
  return execution?.metadata?.name ? "running" : "unavailable";
}

function codedError(code, message, cause) {
  const error = new Error(message, cause ? { cause } : undefined);
  error.code = code;
  return error;
}

function windowsGcloudCandidates(env = process.env) {
  return [
    env.LOCALAPPDATA && join(env.LOCALAPPDATA, "Google", "Cloud SDK", "google-cloud-sdk", "bin", "gcloud.cmd"),
    env.ProgramFiles && join(env.ProgramFiles, "Google", "Cloud SDK", "google-cloud-sdk", "bin", "gcloud.cmd"),
    env["ProgramFiles(x86)"] && join(env["ProgramFiles(x86)"], "Google", "Cloud SDK", "google-cloud-sdk", "bin", "gcloud.cmd"),
  ].filter(Boolean);
}

async function firstAccessible(paths) {
  for (const path of paths) {
    try {
      await access(path);
      return path;
    } catch (error) {
      if (error?.code !== "ENOENT") throw error;
    }
  }
  return undefined;
}

async function resolveGcloudExecutable(platform = process.platform, env = process.env) {
  if (env.GCLOUD_BIN) {
    if (!isAbsolute(env.GCLOUD_BIN)) {
      throw codedError("GCLOUD_BIN_INVALID", "GCLOUD_BIN은 절대 경로여야 합니다.");
    }
    const configured = await firstAccessible([env.GCLOUD_BIN]);
    if (!configured) throw codedError("GCLOUD_NOT_FOUND", "GCLOUD_BIN 경로에서 Google Cloud CLI를 찾지 못했습니다.");
    return configured;
  }
  if (platform !== "win32") return "gcloud";

  const wherePath = join(env.SystemRoot || "C:\\Windows", "System32", "where.exe");
  try {
    const { stdout } = await execFileAsync(wherePath, ["gcloud.cmd"], {
      encoding: "utf8",
      timeout: 5_000,
      maxBuffer: 64 * 1024,
      windowsHide: true,
    });
    const resolved = stdout.split(/\r?\n/).map((line) => line.trim()).find(Boolean);
    if (resolved && isAbsolute(resolved)) return resolved;
  } catch (error) {
    if (error?.code !== 1 && error?.code !== "ENOENT") throw error;
  }
  const candidate = await firstAccessible(windowsGcloudCandidates(env));
  if (candidate) return candidate;
  throw codedError("GCLOUD_NOT_FOUND", "Google Cloud CLI를 찾지 못했습니다.");
}

function validateGcloudArgument(value) {
  if (typeof value !== "string" || value.length > 8_192 || /[\0\r\n]/u.test(value)) {
    throw codedError("GCLOUD_ARGUMENT_INVALID", "Cloud CLI 인자에 허용되지 않은 문자가 있습니다.");
  }
  return value;
}

export function buildWindowsGcloudInvocation(executable, args, env = process.env) {
  if (!isAbsolute(executable) || !/\.(?:cmd|bat|exe)$/iu.test(executable)) {
    throw codedError("GCLOUD_BIN_INVALID", "Windows Cloud CLI는 확인된 절대 경로만 실행할 수 있습니다.");
  }
  const powershell = join(
    env.SystemRoot || "C:\\Windows",
    "System32", "WindowsPowerShell", "v1.0", "powershell.exe",
  );
  return {
    file: powershell,
    args: [
      "-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
      "-File", WINDOWS_GCLOUD_WRAPPER, executable, ...args.map(validateGcloudArgument),
    ],
  };
}

export function classifyGcloudError(error) {
  const code = typeof error?.code === "string" ? error.code : undefined;
  if (code === "GCLOUD_NOT_FOUND" || code === "ENOENT") return "Google Cloud CLI를 찾지 못했습니다.";
  if (code === "GCLOUD_BIN_INVALID" || code === "GCLOUD_ARGUMENT_INVALID") return "Google Cloud CLI 실행 설정이 안전하지 않습니다.";
  if (code === "ETIMEDOUT") return "Google Cloud CLI 조회 시간이 초과됐습니다.";
  const stderr = typeof error?.stderr === "string" ? error.stderr.toLowerCase() : "";
  if (/login|logged in|credential|reauth|authentication|account/u.test(stderr)) {
    return "Google Cloud CLI 로그인이 필요합니다.";
  }
  if (/permission|denied|forbidden|does not have/u.test(stderr)) {
    return "Cloud Run 또는 Logging 읽기 권한이 없습니다.";
  }
  if (code === "GCLOUD_OUTPUT_INVALID") return "Google Cloud CLI 응답 형식이 올바르지 않습니다.";
  return `Cloud Run 조회 실패 (${code || "GCLOUD_READ_FAILED"})`;
}

export function summarizeExecution(definition, execution, logEntries, collectedAtMs = Date.now()) {
  const executionName = typeof execution?.metadata?.name === "string" ? execution.metadata.name : undefined;
  const startedAtMs = timestampMs(execution?.status?.startTime) ?? timestampMs(execution?.metadata?.creationTimestamp);
  const completedAtMs = timestampMs(execution?.status?.completionTime);
  const payloads = (Array.isArray(logEntries) ? logEntries : [])
    .map((entry) => ({ payload: asObject(entry?.jsonPayload), observedAtMs: timestampMs(entry?.timestamp) }))
    .filter(({ payload }) => SUPPORTED_LOG_SCHEMAS.has(payload.schema) && payload.mode === definition.mode)
    .sort((left, right) => (left.observedAtMs ?? 0) - (right.observedAtMs ?? 0));
  const completed = [...payloads].reverse().find(({ payload }) => payload.event === "completed");
  const heartbeat = [...payloads].reverse().find(({ payload }) => payload.event === "heartbeat") ?? completed;
  const issues = Array.isArray(completed?.payload?.issues)
    ? completed.payload.issues.filter((item) => typeof item === "string").slice(0, 50)
    : [];
  const warnings = Array.isArray(completed?.payload?.warnings)
    ? completed.payload.warnings.filter((item) => typeof item === "string").slice(0, 50)
    : [];
  const elapsedMs = asFiniteNumber(heartbeat?.payload?.elapsedMs)
    ?? (startedAtMs ? Math.max(0, (completedAtMs ?? collectedAtMs) - startedAtMs) : undefined);
  return {
    mode: definition.mode,
    jobName: definition.jobName,
    executionName,
    state: conditionState(execution),
    startedAtMs,
    completedAtMs,
    elapsedMs,
    latestHeartbeatAtMs: heartbeat?.observedAtMs,
    heartbeat: heartbeat ? sanitizeHeartbeat(definition.mode, heartbeat.payload) : undefined,
    passed: typeof completed?.payload?.passed === "boolean" ? completed.payload.passed : undefined,
    actualElapsed24hQualified: typeof completed?.payload?.actualElapsed24hQualified === "boolean"
      ? completed.payload.actualElapsed24hQualified
      : false,
    issues,
    warnings,
  };
}

function sanitizeHeartbeat(mode, payload) {
  if (mode === "market") {
    const streams = asObject(payload.streams);
    return {
      streams: Object.fromEntries(Object.entries(streams).slice(0, 12).map(([streamId, raw]) => {
        const value = asObject(raw);
        return [streamId.slice(0, 64), {
          messages: asFiniteNumber(value.messages) ?? 0,
          reconnects: asFiniteNumber(value.reconnects) ?? 0,
          errors: asFiniteNumber(value.errors) ?? 0,
          transportTimeouts: asFiniteNumber(value.transportTimeouts) ?? 0,
          marketGapEvents: asFiniteNumber(value.marketGapEvents) ?? 0,
          lastMessageAtMs: asFiniteNumber(value.lastMessageAtMs),
        }];
      })),
    };
  }
  return {
    eventCount: asFiniteNumber(payload.event_count) ?? 0,
    ledgerCount: asFiniteNumber(payload.ledger_count) ?? 0,
    failureCount: asFiniteNumber(payload.failures) ?? 0,
    reconciliationPassed: payload.reconciliationPassed === true,
  };
}

export function evaluateReport(jobs) {
  if (!jobs.length || jobs.every((job) => job.state === "unavailable")) return "unavailable";
  if (jobs.some((job) => job.state === "failed" || job.passed === false || job.issues.length > 0)) return "failed";
  if (jobs.every((job) => job.state === "completed" && job.actualElapsed24hQualified && job.passed === true)) return "completed";
  if (jobs.some((job) => job.state === "cancelled" || job.warnings.length > 0 || job.collectionIssue)) return "warning";
  return "running";
}

async function runGcloud(args) {
  const executable = await resolveGcloudExecutable();
  const options = {
    encoding: "utf8",
    timeout: COMMAND_TIMEOUT_MS,
    maxBuffer: MAX_COMMAND_OUTPUT_BYTES,
    windowsHide: true,
  };
  const invocation = process.platform === "win32"
    ? buildWindowsGcloudInvocation(executable, args)
    : { file: executable, args };
  const { stdout } = await execFileAsync(invocation.file, invocation.args, options);
  try {
    return JSON.parse(stdout || "[]");
  } catch (error) {
    throw codedError("GCLOUD_OUTPUT_INVALID", "Google Cloud CLI가 JSON이 아닌 응답을 반환했습니다.", error);
  }
}

async function collectJob(definition, collectedAtMs) {
  try {
    const executions = await runGcloud([
      "run", "jobs", "executions", "list",
      `--project=${PROJECT_ID}`, `--region=${REGION}`, `--job=${definition.jobName}`,
      "--limit=1", "--sort-by=~metadata.creationTimestamp", "--format=json",
    ]);
    const execution = Array.isArray(executions) ? executions[0] : undefined;
    if (!execution?.metadata?.name) {
      return { ...summarizeExecution(definition, {}, [], collectedAtMs), collectionIssue: "실행 이력이 없습니다." };
    }
    const executionName = execution.metadata.name;
    const logs = await runGcloud([
      "logging", "read",
      `resource.type="cloud_run_job" AND resource.labels.job_name="${definition.jobName}" AND (jsonPayload.schema="investa.cloud-soak.v1" OR jsonPayload.schema="investa.cloud-soak.v2") AND (jsonPayload.event="heartbeat" OR jsonPayload.event="completed")`,
      `--project=${PROJECT_ID}`, "--freshness=2d", "--limit=1600", "--order=desc", "--format=json",
    ]);
    const executionLogs = Array.isArray(logs)
      ? logs.filter((entry) => asObject(entry?.labels)["run.googleapis.com/execution_name"] === executionName)
      : [];
    const summary = summarizeExecution(definition, execution, executionLogs, collectedAtMs);
    return summary.latestHeartbeatAtMs
      ? summary
      : { ...summary, collectionIssue: "지원하는 heartbeat 또는 완료 로그를 찾지 못했습니다." };
  } catch (error) {
    const unavailable = summarizeExecution(definition, {}, [], collectedAtMs);
    return { ...unavailable, collectionIssue: classifyGcloudError(error) };
  }
}

function defaultReportPath() {
  const root = process.platform === "win32"
    ? process.env.APPDATA
    : process.env.XDG_DATA_HOME || join(homedir(), ".local", "share");
  if (!root) throw new Error("앱 데이터 경로를 확인할 수 없습니다.");
  return join(root, "com.bumniverse.investa", "audits", "cloud-soak-status.json");
}

function markdown(report) {
  const lines = [
    "# Investa Cloud Run 24시간 검사 상태",
    "",
    `- 수집 시각: ${new Date(report.collectedAtMs).toISOString()}`,
    `- 종합 판정: ${report.status}`,
    `- 실전 주문: 잠금`,
    "",
  ];
  for (const job of report.jobs) {
    lines.push(`## ${job.jobName}`, "", `- 상태: ${job.state}`, `- 실행: ${job.executionName ?? "확인 불가"}`, `- 경과: ${job.elapsedMs ?? 0}ms`, `- 24시간 실측 적격: ${job.actualElapsed24hQualified ? "예" : "아니오"}`, `- 이슈: ${job.issues.join(" · ") || job.collectionIssue || "없음"}`, "");
  }
  return `${lines.join("\n")}\n`;
}

async function writeAtomically(path, content) {
  await mkdir(dirname(path), { recursive: true });
  const temporary = `${path}.${process.pid}.tmp`;
  await writeFile(temporary, content, { encoding: "utf8", mode: 0o600 });
  if (process.platform !== "win32") {
    await rename(temporary, path);
    return;
  }
  const previous = `${path}.${process.pid}.previous`;
  try {
    await rename(path, previous);
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
  }
  try {
    await rename(temporary, path);
    await rm(previous, { force: true });
  } catch (error) {
    try {
      await rename(previous, path);
    } catch (restoreError) {
      if (restoreError?.code !== "ENOENT") {
        throw new Error("검사 캐시 교체와 이전 캐시 복구가 모두 실패했습니다.", { cause: error });
      }
    }
    throw error;
  }
}

export async function collectCloudSoakReport() {
  const collectedAtMs = Date.now();
  const jobs = await Promise.all(JOBS.map((definition) => collectJob(definition, collectedAtMs)));
  const report = {
    schema: "investa.cloud-soak-report.v1",
    collectedAtMs,
    projectId: PROJECT_ID,
    region: REGION,
    source: "gcloud-read-only",
    status: evaluateReport(jobs),
    liveOrderEnabled: false,
    jobs,
  };
  const reportPath = defaultReportPath();
  await writeAtomically(reportPath, `${JSON.stringify(report, null, 2)}\n`);
  await writeAtomically(reportPath.replace(/\.json$/, ".md"), markdown(report));
  return { report, reportPath };
}

const isMain = process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href;
if (isMain) {
  const { report, reportPath } = await collectCloudSoakReport();
  process.stdout.write(`${JSON.stringify({ status: report.status, reportPath, collectedAtMs: report.collectedAtMs })}\n`);
}
