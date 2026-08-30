import { mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";

const argumentsMap = new Map();
for (let index = 2; index < process.argv.length; index += 2) {
  argumentsMap.set(process.argv[index], process.argv[index + 1]);
}
const durationSeconds = Number(argumentsMap.get("--duration-seconds") ?? 60);
const outputPath = resolve(argumentsMap.get("--output") ?? "market-stream-soak-report.json");
if (!Number.isFinite(durationSeconds) || durationSeconds < 10 || durationSeconds > 172_800) {
  throw new Error("--duration-seconds는 10~172800 사이여야 합니다.");
}

const definitions = [
  {
    id: "upbit_spot",
    url: "wss://api.upbit.com/websocket/v1",
    subscribe: JSON.stringify([
      { ticket: "investa-soak-audit" },
      { type: "trade", codes: ["KRW-BTC"], is_only_realtime: true },
      { format: "DEFAULT" },
    ]),
  },
  { id: "binance_spot", url: "wss://stream.binance.com:9443/ws/btcusdt@aggTrade" },
  { id: "binance_usdm", url: "wss://fstream.binance.com/stream?streams=btcusdt@aggTrade/btcusdt@markPrice@1s" },
  { id: "binance_coinm", url: "wss://dstream.binance.com/stream?streams=btcusd_perp@aggTrade/btcusd_perp@markPrice@1s" },
];

const startedAtMs = Date.now();
const endsAtMs = startedAtMs + durationSeconds * 1_000;
const state = new Map(definitions.map((definition) => [definition.id, {
  messages: 0,
  reconnects: 0,
  errors: 0,
  staleEvents: 0,
  firstMessageAtMs: null,
  lastMessageAtMs: null,
  maxGapMs: 0,
}]));
const sockets = new Map();
const reconnectTimers = new Map();
const retryAttempts = new Map(definitions.map((definition) => [definition.id, 0]));
let stopping = false;

function scheduleReconnect(definition) {
  if (stopping || Date.now() >= endsAtMs || reconnectTimers.has(definition.id)) return;
  const current = state.get(definition.id);
  current.reconnects += 1;
  const attempt = retryAttempts.get(definition.id) + 1;
  retryAttempts.set(definition.id, attempt);
  const delay = Math.min(30_000, 1_000 * 2 ** Math.min(attempt - 1, 5));
  const timer = setTimeout(() => {
    reconnectTimers.delete(definition.id);
    connect(definition);
  }, delay + Math.floor(delay * 0.2 * Math.random()));
  reconnectTimers.set(definition.id, timer);
}

function connect(definition) {
  if (stopping || Date.now() >= endsAtMs) return;
  const socket = new WebSocket(definition.url);
  sockets.set(definition.id, socket);
  socket.onopen = () => {
    if (definition.subscribe) socket.send(definition.subscribe);
  };
  socket.onmessage = () => {
    const current = state.get(definition.id);
    const now = Date.now();
    if (current.lastMessageAtMs) current.maxGapMs = Math.max(current.maxGapMs, now - current.lastMessageAtMs);
    current.messages += 1;
    retryAttempts.set(definition.id, 0);
    current.firstMessageAtMs ??= now;
    current.lastMessageAtMs = now;
  };
  socket.onerror = () => {
    state.get(definition.id).errors += 1;
  };
  socket.onclose = () => {
    if (sockets.get(definition.id) !== socket) return;
    sockets.delete(definition.id);
    scheduleReconnect(definition);
  };
}

for (const definition of definitions) connect(definition);

const staleTimer = setInterval(() => {
  const now = Date.now();
  for (const definition of definitions) {
    const current = state.get(definition.id);
    const reference = current.lastMessageAtMs ?? startedAtMs;
    if (now - reference <= 20_000) continue;
    if (!sockets.has(definition.id) && reconnectTimers.has(definition.id)) continue;
    current.staleEvents += 1;
    const socket = sockets.get(definition.id);
    if (socket) {
      sockets.delete(definition.id);
      socket.close(1000, "Investa soak stale reconnect");
    }
    scheduleReconnect(definition);
  }
}, 5_000);

async function finish(signal = null) {
  if (stopping) return;
  stopping = true;
  clearInterval(staleTimer);
  for (const timer of reconnectTimers.values()) clearTimeout(timer);
  for (const socket of sockets.values()) socket.close(1000, "Investa soak complete");
  const finishedAtMs = Date.now();
  const streams = Object.fromEntries([...state.entries()]);
  const issues = Object.entries(streams).flatMap(([id, value]) => [
    ...(value.messages === 0 ? [`${id}: 시세 메시지 미수신`] : []),
    ...(value.maxGapMs > 20_000 ? [`${id}: 최대 수신 간격 ${value.maxGapMs}ms`] : []),
  ]);
  const report = {
    schemaVersion: 1,
    simulatedTimeline: false,
    startedAtMs,
    finishedAtMs,
    durationMs: finishedAtMs - startedAtMs,
    actualElapsed24hQualified: finishedAtMs - startedAtMs >= 86_400_000,
    signal,
    streams,
    passed: issues.length === 0,
    issues,
  };
  await mkdir(dirname(outputPath), { recursive: true });
  await writeFile(outputPath, `${JSON.stringify(report, null, 2)}\n`, { encoding: "utf8", flag: "w" });
  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
  process.exitCode = report.passed ? 0 : 2;
}

process.on("SIGINT", () => void finish("SIGINT"));
process.on("SIGTERM", () => void finish("SIGTERM"));
setTimeout(() => void finish(), durationSeconds * 1_000);
