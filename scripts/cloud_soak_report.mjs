import { execFile } from "node:child_process";
import { mkdir, rename, rm, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { promisify } from "node:util";
import { pathToFileURL } from "node:url";

const execFileAsync = promisify(execFile);
const MAX_COMMAND_OUTPUT_BYTES = 4 * 1024 * 1024;
const COMMAND_TIMEOUT_MS = 30_000;
const PROJECT_ID = "investa-remote-bumniverse";
const REGION = "asia-northeast3";
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
  if (completed?.status === "False" && completed?.reason) return "failed";
  return execution?.metadata?.name ? "running" : "unavailable";
}

export function summarizeExecution(definition, execution, logEntries, collectedAtMs = Date.now()) {
  const executionName = typeof execution?.metadata?.name === "string" ? execution.metadata.name : undefined;
  const startedAtMs = timestampMs(execution?.status?.startTime) ?? timestampMs(execution?.metadata?.creationTimestamp);
  const completedAtMs = timestampMs(execution?.status?.completionTime);
  const payloads = (Array.isArray(logEntries) ? logEntries : [])
    .map((entry) => ({ payload: asObject(entry?.jsonPayload), observedAtMs: timestampMs(entry?.timestamp) }))
    .filter(({ payload }) => payload.schema === "investa.cloud-soak.v2" && payload.mode === definition.mode)
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
  if (jobs.some((job) => job.warnings.length > 0 || job.collectionIssue)) return "warning";
  return "running";
}

async function runGcloud(args) {
  const executable = process.env.GCLOUD_BIN || (process.platform === "win32" ? "gcloud.cmd" : "gcloud");
  const { stdout } = await execFileAsync(executable, args, {
    encoding: "utf8",
    timeout: COMMAND_TIMEOUT_MS,
    maxBuffer: MAX_COMMAND_OUTPUT_BYTES,
    windowsHide: true,
  });
  return JSON.parse(stdout || "[]");
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
      `resource.type=\"cloud_run_job\" AND resource.labels.job_name=\"${definition.jobName}\" AND labels.\"run.googleapis.com/execution_name\"=\"${executionName}\" AND jsonPayload.schema=\"investa.cloud-soak.v2\" AND (jsonPayload.event=\"heartbeat\" OR jsonPayload.event=\"completed\")`,
      `--project=${PROJECT_ID}`, "--freshness=2d", "--limit=1600", "--order=asc", "--format=json",
    ]);
    const summary = summarizeExecution(definition, execution, logs, collectedAtMs);
    return summary.latestHeartbeatAtMs
      ? summary
      : { ...summary, collectionIssue: "v2 heartbeat 또는 완료 로그를 찾지 못했습니다." };
  } catch (error) {
    const unavailable = summarizeExecution(definition, {}, [], collectedAtMs);
    const code = typeof error?.code === "string" ? error.code : "GCLOUD_READ_FAILED";
    return { ...unavailable, collectionIssue: `Cloud Run 조회 실패 (${code})` };
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
