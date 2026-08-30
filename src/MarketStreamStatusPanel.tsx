import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef, useState } from "react";
import { MARKET_STREAM_DEFINITIONS, OfficialMarketStreamSupervisor, shouldAttemptAutomaticGapBackfill, toMarketAggregationInput, type MarketAggregationInput, type MarketStreamId, type MarketStreamSnapshot } from "./marketStreams";

const supervisor = new OfficialMarketStreamSupervisor();

type AggregationStatus = {
  streamId: MarketStreamId;
  retainedMinuteBarCount: number;
  latestCompletedAtMs?: number;
  gapCount: number;
  updatedAtMs: number;
};

type AggregationUpdate = {
  streamId: MarketStreamId;
  restoredFromCheckpoint: boolean;
  completedMinuteBars: Array<{ periodEndMs: number }>;
  retainedMinuteBarCount: number;
  gaps: unknown[];
  updatedAtMs: number;
};

type GapBackfillResult = {
  streamId: MarketStreamId;
  insertedBarCount: number;
  remainingGapCount: number;
};

type TossMarketStreamStatus = {
  phase: "idle" | "connecting" | "awaiting_ack" | "live" | "reconnecting" | "error" | "stopped";
  attempt: number;
  configuredTopics: number;
  subscribedTopics: number;
  rejectedTopics: number;
  tradeMessageCount: number;
  orderbookMessageCount: number;
  completedMinuteBarCount: number;
  connectedAtMs?: number;
  lastReceivedAtMs?: number;
  lastPongAtMs?: number;
  nextReconnectAtMs?: number;
  issue?: string;
  liveOrderAllowed: boolean;
};

const EMPTY_TOSS_STREAM: TossMarketStreamStatus = {
  phase: "idle",
  attempt: 0,
  configuredTopics: 0,
  subscribedTopics: 0,
  rejectedTopics: 0,
  tradeMessageCount: 0,
  orderbookMessageCount: 0,
  completedMinuteBarCount: 0,
  liveOrderAllowed: false,
};

const phaseLabel = (snapshot: MarketStreamSnapshot) => ({
  idle: "대기",
  connecting: "연결 중",
  live: snapshot.sample ? "실시간" : "첫 시세 대기",
  stale: "지연",
  reconnecting: `재연결 ${snapshot.attempt}회`,
  error: "오류",
  stopped: "중지",
}[snapshot.phase]);

const price = (snapshot: MarketStreamSnapshot) => snapshot.sample
  ? new Intl.NumberFormat("ko-KR", { maximumFractionDigits: snapshot.id === "upbit_spot" ? 0 : 2 }).format(snapshot.sample.price)
  : "—";

export function MarketStreamStatusPanel() {
  const [snapshots, setSnapshots] = useState<MarketStreamSnapshot[]>(() => supervisor.getSnapshots());
  const [aggregation, setAggregation] = useState<Partial<Record<MarketStreamId, AggregationStatus>>>({});
  const [aggregationIssue, setAggregationIssue] = useState<string>();
  const [aggregationNotice, setAggregationNotice] = useState<string>();
  const [backfillingStreamId, setBackfillingStreamId] = useState<MarketStreamId>();
  const [tossKrSymbols, setTossKrSymbols] = useState("005930");
  const [tossUsSymbols, setTossUsSymbols] = useState("AAPL");
  const [includeTossOrderbook, setIncludeTossOrderbook] = useState(true);
  const [tossStream, setTossStream] = useState<TossMarketStreamStatus>(EMPTY_TOSS_STREAM);
  const [tossStreamIssue, setTossStreamIssue] = useState<string>();
  const queues = useRef(new Map<MarketStreamId, Promise<void>>());
  const previousPhases = useRef(new Map(snapshots.map((snapshot) => [snapshot.id, snapshot.phase])));
  const automaticBackfills = useRef(new Set<MarketStreamId>());
  useEffect(() => {
    const automaticBackfill = async (streamId: MarketStreamId) => {
      if (automaticBackfills.current.has(streamId)) return;
      automaticBackfills.current.add(streamId);
      try {
        const statuses = await invoke<AggregationStatus[]>("market_stream_aggregation_status");
        const target = statuses.find((status) => status.streamId === streamId);
        if (target?.gapCount) {
          await invoke<GapBackfillResult>("market_stream_gap_backfill", { request: { streamId } });
          const refreshed = await invoke<AggregationStatus[]>("market_stream_aggregation_status");
          setAggregation(Object.fromEntries(refreshed.map((status) => [status.streamId, status])));
          setAggregationNotice(`${streamId} 재연결 뒤 공식 REST gap 복구를 실행했습니다.`);
        }
      } catch (reason) {
        setAggregationIssue(`재연결 gap 자동 복구 실패 · ${String(reason)}`);
      } finally {
        automaticBackfills.current.delete(streamId);
      }
    };
    const unsubscribeSnapshots = supervisor.subscribe((nextSnapshots) => {
      if (isTauri()) {
        for (const snapshot of nextSnapshots) {
          const previous = previousPhases.current.get(snapshot.id);
          if (shouldAttemptAutomaticGapBackfill(previous, snapshot.phase)) {
            void automaticBackfill(snapshot.id);
          }
          previousPhases.current.set(snapshot.id, snapshot.phase);
        }
      }
      setSnapshots(nextSnapshots);
    });
    if (!isTauri()) return unsubscribeSnapshots;

    const applyUpdates = (updates: AggregationUpdate[]) => {
      setAggregation((current) => {
        const next = { ...current };
        for (const update of updates) {
          const previous = next[update.streamId];
          const latestCompleted = update.completedMinuteBars[update.completedMinuteBars.length - 1];
          next[update.streamId] = {
            streamId: update.streamId,
            retainedMinuteBarCount: update.retainedMinuteBarCount,
            latestCompletedAtMs: latestCompleted?.periodEndMs ?? previous?.latestCompletedAtMs,
            gapCount: update.gaps.length,
            updatedAtMs: update.updatedAtMs,
          };
        }
        return next;
      });
    };
    void invoke<AggregationStatus[]>("market_stream_aggregation_status")
      .then((statuses) => setAggregation(Object.fromEntries(statuses.map((status) => [status.streamId, status]))))
      .catch((reason) => setAggregationIssue(String(reason)));
    const unsubscribeSamples = supervisor.subscribeSamples((sample) => {
      const input: MarketAggregationInput | undefined = toMarketAggregationInput(sample);
      if (!input) return;
      const previous = queues.current.get(sample.streamId) ?? Promise.resolve();
      const queued = previous
        .then(async () => {
          const update = await invoke<AggregationUpdate>("market_stream_tick_ingest", { input });
          applyUpdates([update]);
          setAggregationIssue(undefined);
        })
        .catch((reason) => setAggregationIssue(String(reason)));
      queues.current.set(sample.streamId, queued);
    });
    const flushTimer = setInterval(() => {
      const pending = [...queues.current.values()];
      void Promise.all(pending)
        .then(() => invoke<AggregationUpdate[]>("market_stream_aggregation_flush", { watermarkMs: Date.now() }))
        .then(applyUpdates)
        .catch((reason) => setAggregationIssue(String(reason)));
    }, 5_000);
    const refreshTossStream = () => invoke<TossMarketStreamStatus>("toss_market_stream_status")
      .then((status) => {
        setTossStream(status);
        setTossStreamIssue(undefined);
      })
      .catch((reason) => setTossStreamIssue(String(reason)));
    void refreshTossStream();
    const tossStatusTimer = setInterval(() => void refreshTossStream(), 2_000);
    return () => {
      unsubscribeSnapshots();
      unsubscribeSamples();
      clearInterval(flushTimer);
      clearInterval(tossStatusTimer);
      queues.current.clear();
      automaticBackfills.current.clear();
    };
  }, []);
  const running = useMemo(
    () => snapshots.some((item) => item.phase !== "idle" && item.phase !== "stopped"),
    [snapshots],
  );
  const liveCount = useMemo(() => snapshots.filter((item) => item.phase === "live" && item.sample).length, [snapshots]);
  const toggle = () => {
    if (running) supervisor.stop(); else supervisor.start();
  };
  const backfillGap = async (streamId: MarketStreamId) => {
    if (!isTauri() || backfillingStreamId) return;
    setBackfillingStreamId(streamId);
    setAggregationIssue(undefined);
    setAggregationNotice(undefined);
    try {
      const result = await invoke<GapBackfillResult>("market_stream_gap_backfill", { request: { streamId } });
      const statuses = await invoke<AggregationStatus[]>("market_stream_aggregation_status");
      setAggregation(Object.fromEntries(statuses.map((status) => [status.streamId, status])));
      if (result.insertedBarCount === 0) {
        setAggregationNotice(`${streamId} 공식 REST에도 거래 봉이 없어 gap을 유지했습니다.`);
      } else {
        setAggregationNotice(`${streamId} 완료 1분봉 ${result.insertedBarCount}개를 복구했습니다. 남은 gap ${result.remainingGapCount}개입니다.`);
      }
    } catch (reason) {
      setAggregationIssue(String(reason));
    } finally {
      setBackfillingStreamId(undefined);
    }
  };
  const parseSymbols = (value: string) => value.split(/[\s,]+/).map((symbol) => symbol.trim()).filter(Boolean);
  const tossRunning = ["connecting", "awaiting_ack", "live", "reconnecting"].includes(tossStream.phase);
  const startTossStream = async () => {
    if (!isTauri()) return;
    setTossStreamIssue(undefined);
    try {
      const status = await invoke<TossMarketStreamStatus>("toss_market_stream_start", {
        request: {
          krSymbols: parseSymbols(tossKrSymbols),
          usSymbols: parseSymbols(tossUsSymbols),
          includeOrderbook: includeTossOrderbook,
        },
      });
      setTossStream(status);
    } catch (reason) {
      setTossStreamIssue(String(reason));
    }
  };
  const stopTossStream = async () => {
    if (!isTauri()) return;
    try {
      setTossStream(await invoke<TossMarketStreamStatus>("toss_market_stream_stop"));
      setTossStreamIssue(undefined);
    } catch (reason) {
      setTossStreamIssue(String(reason));
    }
  };
  return <article className="market-stream-status">
    <h4>공식 공개 실시간 스트림</h4>
    <p>계좌 키 없이 BTC 대표 시세만 연결합니다. 실전 주문과 개인계좌 스트림은 열지 않습니다.</p>
    <div className="readiness-actions">
      <button type="button" onClick={toggle}>{running ? "실시간 스트림 중지" : "실시간 스트림 시작"}</button>
      <button type="button" onClick={() => supervisor.restart()} disabled={!running}>전체 재연결</button>
    </div>
    <small role="status">정상 수신 {liveCount}/{MARKET_STREAM_DEFINITIONS.length}</small>
    {aggregationNotice ? <small role="status">{aggregationNotice}</small> : null}
    {aggregationIssue ? <small className="settings-error" role="alert">완료 봉 집계 중단 · {aggregationIssue}</small> : null}
    {snapshots.map((snapshot) => <div className={`readiness-row is-stream-${snapshot.phase}`} key={snapshot.id}>
      <b>{snapshot.label} · {phaseLabel(snapshot)}</b>
      <span>{price(snapshot)}{snapshot.sample?.fundingRate != null ? ` · 펀딩 ${(snapshot.sample.fundingRate * 100).toFixed(4)}%` : ""}</span>
      <small>{snapshot.lastReceivedAtMs ? `수신 ${new Date(snapshot.lastReceivedAtMs).toLocaleTimeString("ko-KR")}` : snapshot.issue ?? "연결 전"}</small>
      <small>{aggregation[snapshot.id]
        ? `완료 1분봉 ${aggregation[snapshot.id]!.retainedMinuteBarCount}개 · gap ${aggregation[snapshot.id]!.gapCount}개`
        : "완료 봉 집계 전"}</small>
      {aggregation[snapshot.id]?.gapCount ? <button
        className="market-gap-backfill"
        type="button"
        onClick={() => void backfillGap(snapshot.id)}
        disabled={Boolean(backfillingStreamId)}
      >{backfillingStreamId === snapshot.id ? "공식 REST 복구 중…" : "첫 gap 공식 REST 복구"}</button> : null}
    </div>)}
    <section className="toss-market-stream-controls" aria-labelledby="toss-market-stream-title">
      <h4 id="toss-market-stream-title">토스 국장·미장 인증 실시간</h4>
      <p>Rust 내부 인증 헤더로 체결·호가만 수신합니다. 개인 주문 채널과 실전 주문은 열지 않습니다.</p>
      <div className="settings-form-grid">
        <label>국장 종목코드
          <input value={tossKrSymbols} onChange={(event) => setTossKrSymbols(event.target.value)} placeholder="005930, 000660" disabled={tossRunning} />
        </label>
        <label>미장 티커
          <input value={tossUsSymbols} onChange={(event) => setTossUsSymbols(event.target.value)} placeholder="AAPL, MSFT" disabled={tossRunning} />
        </label>
      </div>
      <label className="settings-check-row">
        <input type="checkbox" checked={includeTossOrderbook} onChange={(event) => setIncludeTossOrderbook(event.target.checked)} disabled={tossRunning} />
        체결과 함께 호가 수신
      </label>
      <div className="readiness-actions">
        <button type="button" onClick={() => void startTossStream()} disabled={tossRunning}>토스 실시간 시작</button>
        <button type="button" onClick={() => void stopTossStream()} disabled={!tossRunning}>토스 실시간 중지</button>
      </div>
      <small role="status">상태 {tossStream.phase} · 구독 {tossStream.subscribedTopics}/{tossStream.configuredTopics} · 체결 {tossStream.tradeMessageCount} · 호가 {tossStream.orderbookMessageCount} · 완료봉 {tossStream.completedMinuteBarCount}</small>
      {tossStream.lastReceivedAtMs ? <small>마지막 수신 {new Date(tossStream.lastReceivedAtMs).toLocaleTimeString("ko-KR")}</small> : null}
      {tossStream.issue ? <small className="settings-error" role="alert">{tossStream.issue}</small> : null}
      {tossStreamIssue ? <small className="settings-error" role="alert">{tossStreamIssue}</small> : null}
      <small>SHADOW ONLY · 실전 주문 {tossStream.liveOrderAllowed ? "허용" : "잠금"}</small>
    </section>
  </article>;
}
