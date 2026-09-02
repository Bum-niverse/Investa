import { DatabaseSync } from "node:sqlite";
import { randomUUID } from "node:crypto";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  classifyUpbitPayload,
  createMarketStreamState,
  evaluateMarketStreams,
  isTransportTimedOut,
  observeMarketGap,
  recordMarketMessage,
  recordTransportHeartbeat,
} from "./market-health.mjs";

const argumentsMap = new Map();
for (let index = 2; index < process.argv.length; index += 2) {
  argumentsMap.set(process.argv[index], process.argv[index + 1]);
}

const mode = argumentsMap.get("--mode") ?? "market";
const durationSeconds = Number(argumentsMap.get("--duration-seconds") ?? 86_400);
const heartbeatSeconds = Number(argumentsMap.get("--heartbeat-seconds") ?? 60);
if (!['market', 'shadow-contract'].includes(mode)) {
  throw new Error("--mode는 market 또는 shadow-contract여야 합니다.");
}
if (!Number.isFinite(durationSeconds) || durationSeconds < 10 || durationSeconds > 172_800) {
  throw new Error("--duration-seconds는 10~172800 사이여야 합니다.");
}
if (!Number.isFinite(heartbeatSeconds) || heartbeatSeconds < 5 || heartbeatSeconds > 300) {
  throw new Error("--heartbeat-seconds는 5~300 사이여야 합니다.");
}

const startedAtMs = Date.now();
const endsAtMs = startedAtMs + durationSeconds * 1_000;
let stopping = false;

function emit(event, detail = {}) {
  process.stdout.write(`${JSON.stringify({
    schema: "investa.cloud-soak.v2",
    mode,
    event,
    observedAtMs: Date.now(),
    elapsedMs: Date.now() - startedAtMs,
    ...detail,
  })}\n`);
}

function installSignals(finish) {
  process.on("SIGINT", () => void finish("SIGINT"));
  process.on("SIGTERM", () => void finish("SIGTERM"));
}

async function runMarket() {
  const definitions = [
    {
      id: "upbit_spot",
      url: "wss://api.upbit.com/websocket/v1",
      subscription: () => JSON.stringify([
        { ticket: `investa-cloud-soak-${randomUUID()}` },
        { type: "trade", codes: ["KRW-BTC"], is_only_realtime: true },
        { format: "DEFAULT" },
      ]),
      eventDriven: true,
      keepAliveText: "PING",
      transportTimeoutMs: 45_000,
      marketGapThresholdMs: 20_000,
    },
    {
      id: "binance_spot",
      url: "wss://stream.binance.com:9443/ws/btcusdt@aggTrade",
      eventDriven: false,
      transportTimeoutMs: 20_000,
      marketGapThresholdMs: 20_000,
    },
    {
      id: "binance_usdm",
      url: "wss://fstream.binance.com/market/ws/btcusdt@markPrice@1s",
      eventDriven: false,
      transportTimeoutMs: 20_000,
      marketGapThresholdMs: 20_000,
    },
    {
      id: "binance_coinm",
      url: "wss://dstream.binance.com/ws/btcusd_perp@markPrice@1s",
      eventDriven: false,
      transportTimeoutMs: 20_000,
      marketGapThresholdMs: 20_000,
    },
  ];
  const state = new Map(definitions.map(({ id }) => [id, createMarketStreamState()]));
  const sockets = new Map();
  const openedAtMs = new Map();
  const reconnectTimers = new Map();
  const retryAttempts = new Map(definitions.map(({ id }) => [id, 0]));

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
      openedAtMs.set(definition.id, Date.now());
      emit("stream_opened", { streamId: definition.id });
      if (definition.subscription) socket.send(definition.subscription());
    };
    socket.onmessage = async (event) => {
      const current = state.get(definition.id);
      const now = Date.now();
      const payload = typeof event.data === "string"
        ? event.data
        : event.data instanceof Blob
          ? await event.data.text()
          : event.data;
      if (definition.id === "upbit_spot" && classifyUpbitPayload(payload) === "heartbeat") {
        recordTransportHeartbeat(current, now);
        return;
      }
      recordMarketMessage(current, now);
      retryAttempts.set(definition.id, 0);
    };
    socket.onerror = (event) => {
      state.get(definition.id).errors += 1;
      emit("stream_error", {
        streamId: definition.id,
        message: typeof event?.message === "string" ? event.message.slice(0, 200) : "websocket error",
      });
    };
    socket.onclose = (event) => {
      if (sockets.get(definition.id) !== socket) return;
      sockets.delete(definition.id);
      emit("stream_closed", { streamId: definition.id, code: event.code });
      scheduleReconnect(definition);
    };
  }

  for (const definition of definitions) connect(definition);
  const keepAliveTimer = setInterval(() => {
    for (const definition of definitions) {
      if (!definition.keepAliveText) continue;
      const socket = sockets.get(definition.id);
      if (socket?.readyState === WebSocket.OPEN) socket.send(definition.keepAliveText);
    }
  }, 30_000);
  const livenessTimer = setInterval(() => {
    const now = Date.now();
    for (const definition of definitions) {
      const current = state.get(definition.id);
      if (observeMarketGap(current, now, startedAtMs, definition.marketGapThresholdMs)) {
        emit("market_gap", {
          streamId: definition.id,
          eventDriven: definition.eventDriven,
          lastMessageAtMs: current.lastMessageAtMs,
        });
      }
      if (!isTransportTimedOut(
        current,
        now,
        openedAtMs.get(definition.id) ?? startedAtMs,
        definition.transportTimeoutMs,
      )) continue;
      if (!sockets.has(definition.id) && reconnectTimers.has(definition.id)) continue;
      current.transportTimeouts += 1;
      const socket = sockets.get(definition.id);
      if (socket) {
        sockets.delete(definition.id);
        socket.close(1000, "Investa cloud soak transport timeout");
      }
      scheduleReconnect(definition);
    }
  }, 5_000);
  const heartbeatTimer = setInterval(() => emit("heartbeat", {
    streams: Object.fromEntries([...state.entries()].map(([id, value]) => [id, {
      messages: value.messages,
      reconnects: value.reconnects,
      errors: value.errors,
      transportTimeouts: value.transportTimeouts,
      transportHeartbeats: value.transportHeartbeats,
      marketGapEvents: value.marketGapEvents,
      lastMessageAtMs: value.lastMessageAtMs,
      lastTransportAtMs: value.lastTransportAtMs,
    }])),
  }), heartbeatSeconds * 1_000);

  async function finish(signal = null) {
    if (stopping) return;
    stopping = true;
    clearInterval(keepAliveTimer);
    clearInterval(livenessTimer);
    clearInterval(heartbeatTimer);
    for (const timer of reconnectTimers.values()) clearTimeout(timer);
    for (const socket of sockets.values()) socket.close(1000, "Investa cloud soak complete");
    const streams = Object.fromEntries([...state.entries()]);
    const { issues, warnings } = evaluateMarketStreams(definitions, state);
    emit("completed", {
      signal,
      actualElapsed24hQualified: Date.now() - startedAtMs >= 86_400_000,
      passed: issues.length === 0,
      issues,
      warnings,
      streams,
    });
    process.exitCode = issues.length === 0 ? 0 : 2;
  }

  installSignals(finish);
  setTimeout(() => void finish(), durationSeconds * 1_000);
}

async function runShadowContract() {
  const database = new DatabaseSync(join(tmpdir(), `investa-cloud-shadow-${startedAtMs}.sqlite3`));
  database.exec(`
    PRAGMA journal_mode=WAL;
    CREATE TABLE ledger_events (
      event_id TEXT PRIMARY KEY,
      currency TEXT NOT NULL CHECK(currency IN ('KRW','USD')),
      amount_minor INTEGER NOT NULL,
      occurred_at_ms INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE checkpoints (
      checkpoint_id INTEGER PRIMARY KEY CHECK(checkpoint_id=1),
      event_count INTEGER NOT NULL,
      krw_minor INTEGER NOT NULL,
      usd_minor INTEGER NOT NULL,
      updated_at_ms INTEGER NOT NULL
    ) STRICT;
    INSERT INTO checkpoints VALUES(1,0,10000000000,10000000,${startedAtMs});
  `);
  const insertEvent = database.prepare(
    "INSERT INTO ledger_events(event_id,currency,amount_minor,occurred_at_ms) VALUES(?,?,?,?)",
  );
  const updateCheckpoint = database.prepare(
    "UPDATE checkpoints SET event_count=event_count+1,krw_minor=krw_minor+?,usd_minor=usd_minor+?,updated_at_ms=? WHERE checkpoint_id=1",
  );
  const readCounts = database.prepare(`
    SELECT
      (SELECT COUNT(*) FROM ledger_events) AS ledger_count,
      event_count,
      krw_minor,
      usd_minor
    FROM checkpoints WHERE checkpoint_id=1
  `);
  let sequence = 0;
  let failures = 0;

  function sample() {
    sequence += 1;
    const now = Date.now();
    const currency = sequence % 2 === 0 ? "USD" : "KRW";
    const amountMinor = sequence % 4 < 2 ? 1 : -1;
    database.exec("BEGIN IMMEDIATE");
    try {
      insertEvent.run(`cloud-shadow-${startedAtMs}-${sequence}`, currency, amountMinor, now);
      updateCheckpoint.run(currency === "KRW" ? amountMinor : 0, currency === "USD" ? amountMinor : 0, now);
      database.exec("COMMIT");
    } catch (error) {
      database.exec("ROLLBACK");
      failures += 1;
      emit("sample_error", { message: error instanceof Error ? error.message : "unknown" });
    }
  }

  function inspect() {
    const quickCheck = database.prepare("PRAGMA quick_check(1)").get().quick_check;
    const counts = readCounts.get();
    const reconciliationPassed = counts.ledger_count === counts.event_count;
    if (quickCheck !== "ok" || !reconciliationPassed) failures += 1;
    return { quickCheck, reconciliationPassed, ...counts, failures };
  }

  sample();
  const sampleTimer = setInterval(sample, 60_000);
  const heartbeatTimer = setInterval(() => emit("heartbeat", inspect()), heartbeatSeconds * 1_000);

  async function finish(signal = null) {
    if (stopping) return;
    stopping = true;
    clearInterval(sampleTimer);
    clearInterval(heartbeatTimer);
    const result = inspect();
    database.close();
    emit("completed", {
      signal,
      actualElapsed24hQualified: Date.now() - startedAtMs >= 86_400_000,
      passed: result.failures === 0,
      ...result,
    });
    process.exitCode = result.failures === 0 ? 0 : 2;
  }

  installSignals(finish);
  setTimeout(() => void finish(), durationSeconds * 1_000);
}

emit("started", { durationSeconds, heartbeatSeconds });
if (mode === "market") await runMarket();
else await runShadowContract();
