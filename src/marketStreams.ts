export type MarketStreamId = "upbit_spot" | "binance_spot" | "binance_usdm" | "binance_coinm";
export type MarketStreamPhase = "idle" | "connecting" | "live" | "stale" | "reconnecting" | "error" | "stopped";

export type MarketStreamSample = {
  streamId: MarketStreamId;
  provider: "Upbit" | "Binance";
  market: "spot" | "usd_m" | "coin_m";
  symbol: string;
  price: number;
  priceText: string;
  quantityText?: string;
  currency: "KRW" | "USD";
  assetClass: "crypto_spot" | "crypto_futures";
  aggregationEligible: boolean;
  eventAtMs: number;
  receivedAtMs: number;
  sequence?: number;
  markPrice?: number;
  indexPrice?: number;
  fundingRate?: number;
  nextFundingAtMs?: number;
};

export type MarketStreamSnapshot = {
  id: MarketStreamId;
  label: string;
  phase: MarketStreamPhase;
  attempt: number;
  connectedAtMs?: number;
  lastReceivedAtMs?: number;
  sample?: MarketStreamSample;
  issue?: string;
};

export type MarketAggregationInput = {
  streamId: MarketStreamId;
  provider: "Upbit" | "Binance";
  assetClass: "crypto_spot" | "crypto_futures";
  symbol: string;
  currency: "KRW" | "USD";
  eventAtMs: number;
  receivedAtMs: number;
  sequence?: number;
  price: string;
  quantity: string;
};

type StreamDefinition = {
  id: MarketStreamId;
  label: string;
  url: string;
  subscribe?: string;
};

type SocketEvent = { data: string | ArrayBuffer | Blob };
type SocketLike = {
  binaryType: BinaryType;
  onopen: (() => void) | null;
  onmessage: ((event: SocketEvent) => void) | null;
  onerror: (() => void) | null;
  onclose: (() => void) | null;
  send(data: string): void;
  close(code?: number, reason?: string): void;
};

type SocketFactory = (url: string) => SocketLike;

export const MARKET_STREAM_DEFINITIONS: readonly StreamDefinition[] = [
  {
    id: "upbit_spot",
    label: "Upbit KRW-BTC 현물",
    url: "wss://api.upbit.com/websocket/v1",
    subscribe: JSON.stringify([
      { ticket: "investa-public-market" },
      { type: "trade", codes: ["KRW-BTC"], is_only_realtime: true },
      { format: "DEFAULT" },
    ]),
  },
  { id: "binance_spot", label: "Binance BTCUSDT 현물", url: "wss://stream.binance.com:9443/ws/btcusdt@aggTrade" },
  { id: "binance_usdm", label: "Binance BTCUSDT USDⓈ-M", url: "wss://fstream.binance.com/stream?streams=btcusdt@aggTrade/btcusdt@markPrice@1s" },
  { id: "binance_coinm", label: "Binance BTCUSD COIN-M", url: "wss://dstream.binance.com/stream?streams=btcusd_perp@aggTrade/btcusd_perp@markPrice@1s" },
] as const;

const finitePositive = (value: unknown) => {
  const parsed = typeof value === "number" ? value : typeof value === "string" ? Number(value) : Number.NaN;
  return Number.isFinite(parsed) && parsed > 0 ? parsed : undefined;
};

const finiteNumber = (value: unknown) => {
  const parsed = typeof value === "number" ? value : typeof value === "string" ? Number(value) : Number.NaN;
  return Number.isFinite(parsed) ? parsed : undefined;
};

const positiveDecimalText = (value: unknown) => {
  const parsed = finitePositive(value);
  if (!parsed) return undefined;
  return typeof value === "string"
    ? value
    : parsed.toLocaleString("en-US", { useGrouping: false, maximumFractionDigits: 20 });
};

const finiteTimestamp = (value: unknown, fallback: number) => {
  const parsed = finitePositive(value);
  return parsed && parsed <= fallback + 60_000 ? parsed : fallback;
};

const optionalSequence = (value: unknown) => {
  const parsed = finitePositive(value);
  return parsed && Number.isSafeInteger(parsed) ? parsed : undefined;
};

export function parseMarketStreamMessage(id: MarketStreamId, raw: string, receivedAtMs: number): MarketStreamSample {
  let payload: Record<string, unknown>;
  try {
    payload = JSON.parse(raw) as Record<string, unknown>;
  } catch {
    throw new Error("실시간 시세 응답이 JSON 형식이 아닙니다.");
  }

  if (payload.data && typeof payload.data === "object" && !Array.isArray(payload.data)) {
    payload = payload.data as Record<string, unknown>;
  }

  if (id === "upbit_spot") {
    const priceText = positiveDecimalText(payload.trade_price);
    const quantityText = positiveDecimalText(payload.trade_volume);
    const sequence = optionalSequence(payload.sequential_id);
    if (payload.type !== "trade" || payload.code !== "KRW-BTC" || !priceText || !quantityText || !sequence) {
      throw new Error("Upbit trade의 종목·현재가·체결수량·순번이 올바르지 않습니다.");
    }
    return {
      streamId: id,
      provider: "Upbit",
      market: "spot",
      symbol: "KRW-BTC",
      price: Number(priceText),
      priceText,
      quantityText,
      currency: "KRW",
      assetClass: "crypto_spot",
      aggregationEligible: true,
      eventAtMs: finiteTimestamp(payload.trade_timestamp ?? payload.timestamp, receivedAtMs),
      receivedAtMs,
      sequence,
    };
  }

  const expectedSymbol = id === "binance_coinm" ? "BTCUSD_PERP" : "BTCUSDT";
  if (payload.s !== expectedSymbol) throw new Error("Binance stream의 종목이 요청과 일치하지 않습니다.");
  const market = id === "binance_spot" ? "spot" : id === "binance_usdm" ? "usd_m" : "coin_m";
  const isAggregateTrade = payload.e === "aggTrade";
  if (id === "binance_spot" && !isAggregateTrade) {
    throw new Error("Binance Spot은 aggTrade 체결 메시지만 허용합니다.");
  }
  const priceText = positiveDecimalText(payload.p);
  const quantityText = positiveDecimalText(payload.q);
  if (!priceText) throw new Error("Binance stream의 현재가가 올바르지 않습니다.");
  if ((id === "binance_spot" || isAggregateTrade) && !quantityText) {
    throw new Error("Binance 체결수량이 올바르지 않습니다.");
  }
  const aggregationEligible = id === "binance_spot" || isAggregateTrade;
  const sequence = isAggregateTrade ? optionalSequence(payload.a) : undefined;
  if (aggregationEligible && !sequence) {
    throw new Error("Binance 체결 순번이 올바르지 않습니다.");
  }
  return {
    streamId: id,
    provider: "Binance",
    market,
    symbol: expectedSymbol,
    price: Number(priceText),
    priceText,
    quantityText,
    currency: "USD",
    assetClass: id === "binance_spot" ? "crypto_spot" : "crypto_futures",
    aggregationEligible,
    eventAtMs: finiteTimestamp(isAggregateTrade ? payload.T ?? payload.E : payload.E, receivedAtMs),
    receivedAtMs,
    sequence,
    markPrice: id === "binance_spot" || isAggregateTrade ? undefined : Number(priceText),
    indexPrice: id === "binance_spot" || isAggregateTrade ? undefined : finitePositive(payload.i),
    fundingRate: id === "binance_spot" || isAggregateTrade ? undefined : finiteNumber(payload.r),
    nextFundingAtMs: id === "binance_spot" || isAggregateTrade ? undefined : optionalSequence(payload.T),
  };
}

export function toMarketAggregationInput(sample: MarketStreamSample): MarketAggregationInput | undefined {
  if (!sample.aggregationEligible || !sample.quantityText) return undefined;
  return {
    streamId: sample.streamId,
    provider: sample.provider,
    assetClass: sample.assetClass,
    symbol: sample.symbol,
    currency: sample.currency,
    eventAtMs: sample.eventAtMs,
    receivedAtMs: sample.receivedAtMs,
    sequence: sample.sequence,
    price: sample.priceText,
    quantity: sample.quantityText,
  };
}

export async function decodeSocketData(data: SocketEvent["data"]): Promise<string> {
  if (typeof data === "string") return data;
  if (data instanceof ArrayBuffer) return new TextDecoder().decode(data);
  return new TextDecoder().decode(await data.arrayBuffer());
}

export function reconnectDelayMs(attempt: number, jitterUnit = 0): number {
  const base = Math.min(30_000, 1_000 * 2 ** Math.max(0, Math.min(attempt, 5)));
  return Math.round(base + base * 0.2 * Math.max(0, Math.min(jitterUnit, 1)));
}

export function assessMarketStream(snapshot: MarketStreamSnapshot, nowMs: number, staleAfterMs: number): MarketStreamSnapshot {
  if (snapshot.phase !== "live" || !snapshot.lastReceivedAtMs) return snapshot;
  if (nowMs - snapshot.lastReceivedAtMs <= staleAfterMs) return snapshot;
  return { ...snapshot, phase: "stale", issue: `마지막 관측 후 ${Math.floor((nowMs - snapshot.lastReceivedAtMs) / 1_000)}초 경과` };
}

export function shouldAttemptAutomaticGapBackfill(previous: MarketStreamPhase | undefined, current: MarketStreamPhase) {
  return current === "live" && previous != null && ["stale", "reconnecting", "error"].includes(previous);
}

export class OfficialMarketStreamSupervisor {
  private readonly socketFactory: SocketFactory;
  private readonly now: () => number;
  private readonly staleAfterMs: number;
  private readonly random: () => number;
  private readonly sockets = new Map<MarketStreamId, SocketLike>();
  private readonly reconnectTimers = new Map<MarketStreamId, ReturnType<typeof setTimeout>>();
  private readonly rotationTimers = new Map<MarketStreamId, ReturnType<typeof setTimeout>>();
  private readonly listeners = new Set<(snapshots: MarketStreamSnapshot[]) => void>();
  private readonly sampleListeners = new Set<(sample: MarketStreamSample) => void>();
  private readonly messageQueues = new Map<MarketStreamId, Promise<void>>();
  private snapshots = new Map<MarketStreamId, MarketStreamSnapshot>();
  private staleTimer?: ReturnType<typeof setInterval>;
  private running = false;

  constructor(
    socketFactory: SocketFactory = (url) => new WebSocket(url) as SocketLike,
    now: () => number = Date.now,
    staleAfterMs = 15_000,
    random: () => number = Math.random,
  ) {
    this.socketFactory = socketFactory;
    this.now = now;
    this.staleAfterMs = staleAfterMs;
    this.random = random;
    for (const definition of MARKET_STREAM_DEFINITIONS) {
      this.snapshots.set(definition.id, { id: definition.id, label: definition.label, phase: "idle", attempt: 0 });
    }
  }

  subscribe(listener: (snapshots: MarketStreamSnapshot[]) => void) {
    this.listeners.add(listener);
    listener(this.getSnapshots());
    return () => { this.listeners.delete(listener); };
  }

  subscribeSamples(listener: (sample: MarketStreamSample) => void) {
    this.sampleListeners.add(listener);
    return () => { this.sampleListeners.delete(listener); };
  }

  getSnapshots() {
    return MARKET_STREAM_DEFINITIONS.map(({ id }) => ({ ...this.snapshots.get(id)! }));
  }

  start() {
    if (this.running) return;
    this.running = true;
    for (const definition of MARKET_STREAM_DEFINITIONS) this.connect(definition);
    this.staleTimer = setInterval(() => {
      let changed = false;
      for (const [id, snapshot] of this.snapshots) {
        const assessed = assessMarketStream(snapshot, this.now(), this.staleAfterMs);
        if (assessed !== snapshot) { this.snapshots.set(id, assessed); changed = true; this.reconnect(id, "stale stream"); }
      }
      if (changed) this.emit();
    }, Math.min(5_000, this.staleAfterMs));
  }

  stop() {
    this.running = false;
    if (this.staleTimer) clearInterval(this.staleTimer);
    this.staleTimer = undefined;
    for (const timer of this.reconnectTimers.values()) clearTimeout(timer);
    for (const timer of this.rotationTimers.values()) clearTimeout(timer);
    this.reconnectTimers.clear(); this.rotationTimers.clear();
    this.messageQueues.clear();
    for (const socket of this.sockets.values()) socket.close(1000, "Investa stream stopped");
    this.sockets.clear();
    for (const definition of MARKET_STREAM_DEFINITIONS) {
      this.snapshots.set(definition.id, { ...this.snapshots.get(definition.id)!, phase: "stopped", issue: undefined });
    }
    this.emit();
  }

  restart() {
    this.stop();
    for (const definition of MARKET_STREAM_DEFINITIONS) {
      this.snapshots.set(definition.id, { id: definition.id, label: definition.label, phase: "idle", attempt: 0 });
    }
    this.start();
  }

  private connect(definition: StreamDefinition) {
    if (!this.running) return;
    const previous = this.snapshots.get(definition.id)!;
    this.snapshots.set(definition.id, { ...previous, phase: previous.attempt ? "reconnecting" : "connecting", issue: undefined });
    this.emit();
    let socket: SocketLike;
    try {
      socket = this.socketFactory(definition.url);
    } catch {
      this.scheduleReconnect(definition, "WebSocket 생성 실패");
      return;
    }
    socket.binaryType = "arraybuffer";
    this.sockets.set(definition.id, socket);
    socket.onopen = () => {
      if (!this.running || this.sockets.get(definition.id) !== socket) return;
      if (definition.subscribe) socket.send(definition.subscribe);
      const connectedAtMs = this.now();
      this.snapshots.set(definition.id, { ...this.snapshots.get(definition.id)!, phase: "live", attempt: 0, connectedAtMs, issue: "첫 시세 수신 대기" });
      this.emit();
      const rotation = setTimeout(() => this.reconnect(definition.id, "24시간 연결 만료 전 교체"), 23 * 60 * 60 * 1_000 + 45 * 60 * 1_000);
      this.rotationTimers.set(definition.id, rotation);
    };
    socket.onmessage = (event) => {
      const previous = this.messageQueues.get(definition.id) ?? Promise.resolve();
      const queued = previous
        .then(() => this.handleMessage(definition, socket, event.data))
        .catch(() => undefined);
      this.messageQueues.set(definition.id, queued);
    };
    socket.onerror = () => {
      if (this.sockets.get(definition.id) !== socket) return;
      this.snapshots.set(definition.id, { ...this.snapshots.get(definition.id)!, phase: "error", issue: "WebSocket 오류" });
      this.emit();
      this.reconnect(definition.id, "WebSocket 오류");
    };
    socket.onclose = () => {
      if (this.sockets.get(definition.id) !== socket) return;
      this.sockets.delete(definition.id);
      this.clearRotationTimer(definition.id);
      this.scheduleReconnect(definition, "연결 종료");
    };
  }

  private async handleMessage(definition: StreamDefinition, socket: SocketLike, data: SocketEvent["data"]) {
    if (!this.running || this.sockets.get(definition.id) !== socket) return;
    try {
      const raw = await decodeSocketData(data);
      if (!this.running || this.sockets.get(definition.id) !== socket) return;
      const sample = parseMarketStreamMessage(definition.id, raw, this.now());
      const previous = this.snapshots.get(definition.id)!;
      if (sample.sequence && previous.sample?.sequence && sample.sequence < previous.sample.sequence) {
        this.snapshots.set(definition.id, { ...previous, phase: "error", issue: "이벤트 순번이 이전 관측보다 작습니다." });
        this.emit(); this.reconnect(definition.id, "sequence regression"); return;
      }
      this.snapshots.set(definition.id, { ...previous, phase: "live", lastReceivedAtMs: sample.receivedAtMs, sample, issue: undefined });
      this.emit();
      for (const listener of this.sampleListeners) listener(sample);
    } catch (reason) {
      this.snapshots.set(definition.id, { ...this.snapshots.get(definition.id)!, phase: "error", issue: String(reason) });
      this.emit();
    }
  }

  private reconnect(id: MarketStreamId, reason: string) {
    this.clearRotationTimer(id);
    const socket = this.sockets.get(id);
    if (socket) { this.sockets.delete(id); socket.close(1000, reason); }
    const definition = MARKET_STREAM_DEFINITIONS.find((item) => item.id === id)!;
    this.scheduleReconnect(definition, reason);
  }

  private scheduleReconnect(definition: StreamDefinition, issue: string) {
    if (!this.running || this.reconnectTimers.has(definition.id)) return;
    const previous = this.snapshots.get(definition.id)!;
    const attempt = previous.attempt + 1;
    this.snapshots.set(definition.id, { ...previous, phase: "reconnecting", attempt, issue });
    this.emit();
    const timer = setTimeout(() => {
      this.reconnectTimers.delete(definition.id);
      this.connect(definition);
    }, reconnectDelayMs(attempt - 1, this.random()));
    this.reconnectTimers.set(definition.id, timer);
  }

  private clearRotationTimer(id: MarketStreamId) {
    const timer = this.rotationTimers.get(id);
    if (timer) clearTimeout(timer);
    this.rotationTimers.delete(id);
  }

  private emit() {
    const value = this.getSnapshots();
    for (const listener of this.listeners) listener(value);
  }
}
