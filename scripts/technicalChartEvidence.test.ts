import assert from "node:assert/strict";
import test from "node:test";
import { buildTechnicalChartAnnotations, buildTechnicalChartEvidence, buildTechnicalChartEvidenceCollection, validatePointInTimeChartBars, type TechnicalChartBar } from "../src/technicalChartEvidence.ts";

const bars = Array.from({ length: 80 }, (_, index): TechnicalChartBar => ({
  periodStartMs: 1_700_000_000_000 + index * 86_400_000,
  periodEndMs: 1_700_000_000_000 + (index + 1) * 86_400_000,
  openMinor: 10_000 + index * 10,
  highMinor: 10_300 + index * 10,
  lowMinor: 9_800 + index * 10,
  closeMinor: 10_100 + index * 10,
  volume: 1_000 + index,
  completed: true,
}));

test("기술 차트 주석은 관측 고저점·저점 연결선·최근 범위를 만든다", () => {
  const annotations = buildTechnicalChartAnnotations(bars);
  assert.deepEqual(annotations.map((item) => item.kind), ["horizontal_line", "horizontal_line", "trend_line", "rectangle"]);
  assert.equal(annotations[0].startPriceMinor, bars[20].lowMinor);
  assert.equal(annotations[1].startPriceMinor, bars[79].highMinor);
  assert.equal(annotations[3].startTime, bars[60].periodStartMs);
});

test("미완료 봉을 제거하고 불변 차트 근거를 만든다", () => {
  const evidence = buildTechnicalChartEvidence({
    snapshotId: "snapshot-1", provider: "test", symbol: "005930", name: "삼성전자", market: "korea", currency: "KRW",
    interval: "1d", adjusted: true, asOfMs: bars[79].periodEndMs, bars: [...bars, { ...bars[79], periodStartMs: bars[79].periodStartMs + 86_400_000, completed: false }],
  });
  assert.ok(evidence);
  assert.equal(evidence.bars.length, 80);
  assert.equal(evidence.annotations.length, 4);
  assert.match(evidence.method, /결정론/);
});

test("완료 봉이 부족하면 리포트 차트를 만들지 않는다", () => {
  const evidence = buildTechnicalChartEvidence({
    snapshotId: "snapshot-short", provider: "test", symbol: "AAPL", name: "Apple", market: "united_states", currency: "USD",
    interval: "1d", adjusted: true, asOfMs: 1, bars: bars.slice(0, 19),
  });
  assert.equal(evidence, null);
});

test("코인 현물은 24시간 연속시장 30봉 범위를 사용한다", () => {
  const cryptoBars = bars.map((bar) => ({ ...bar, availableAtMs: bar.periodEndMs, ingestedAtMs: bar.periodEndMs + 1 }));
  const evidence = buildTechnicalChartEvidence({
    snapshotId: "upbit-btc", provider: "upbit", symbol: "KRW-BTC", name: "비트코인", market: "coin", currency: "KRW",
    interval: "1d", adjusted: false, asOfMs: cryptoBars[79].ingestedAtMs!, bars: cryptoBars,
  });
  assert.ok(evidence);
  assert.equal(evidence.assetClass, "crypto_spot");
  assert.match(evidence.method, /24시간 연속시장/);
  assert.match(evidence.annotations.at(-1)!.label, /30봉/);
});

test("증권선물 추세선은 현재 계약 안에서만 계산하고 롤·정산가를 표시한다", () => {
  const futureBars = bars.map((bar, index) => ({
    ...bar, availableAtMs: bar.periodEndMs, ingestedAtMs: bar.periodEndMs + 1,
    contractCode: index < 50 ? "KOSPI200-202609" : "KOSPI200-202612", settlementPriceMinor: bar.closeMinor - 5,
  }));
  const evidence = buildTechnicalChartEvidence({
    snapshotId: "future-1", provider: "official-master", symbol: "KOSPI200", name: "코스피200 선물", market: "securities_futures", currency: "KRW",
    interval: "1d", adjusted: false, asOfMs: futureBars[79].ingestedAtMs!, bars: futureBars,
  });
  assert.ok(evidence);
  assert.equal(evidence.assetClass, "securities_future");
  assert.ok(evidence.annotations.some((item) => item.basis === "contract" && item.kind === "vertical_line"));
  assert.ok(evidence.annotations.some((item) => item.basis === "settlement"));
  const currentStart = futureBars[50].periodStartMs;
  assert.ok(evidence.annotations.filter((item) => item.basis === "trade").every((item) => item.startTime >= currentStart));
});

test("코인 무기한선물은 마크·지수·펀딩을 체결가 선과 분리한다", () => {
  const perpetualBars = bars.map((bar, index) => ({
    ...bar, availableAtMs: bar.periodEndMs, ingestedAtMs: bar.periodEndMs + 1,
    markPriceMinor: bar.closeMinor + 3, indexPriceMinor: bar.closeMinor - 2,
    fundingTimeMs: index % 8 === 0 ? bar.periodEndMs : undefined, fundingRateBps: index % 8 === 0 ? 1 : undefined,
  }));
  const evidence = buildTechnicalChartEvidence({
    snapshotId: "perp-1", provider: "binance", symbol: "BTCUSDT", name: "BTC 무기한", market: "crypto_futures", currency: "USD",
    interval: "4h", adjusted: false, asOfMs: perpetualBars[79].ingestedAtMs!, bars: perpetualBars,
  });
  assert.ok(evidence);
  assert.deepEqual(new Set(evidence.annotations.map((item) => item.basis)), new Set(["trade", "mark", "index", "funding"]));
});

test("분석 시각 뒤에 공개된 봉은 PIT 계약에서 거부한다", () => {
  const future = { ...bars[0], availableAtMs: bars[0].periodEndMs + 10, ingestedAtMs: bars[0].periodEndMs + 11 };
  const validation = validatePointInTimeChartBars([future], bars[0].periodEndMs, "crypto_spot");
  assert.equal(validation.bars.length, 0);
  assert.match(validation.errors.join(" "), /분석 기준 시각 뒤/);
});

test("역순 봉을 자동 정렬해 숨기지 않고 PIT 계약 오류로 반환한다", () => {
  const reversed = [{ ...bars[1], availableAtMs: bars[1].periodEndMs, ingestedAtMs: bars[1].periodEndMs }, { ...bars[0], availableAtMs: bars[0].periodEndMs, ingestedAtMs: bars[0].periodEndMs }];
  const validation = validatePointInTimeChartBars(reversed, bars[1].periodEndMs, "crypto_spot");
  assert.equal(validation.bars.length, 0);
  assert.match(validation.errors.join(" "), /시각 오름차순/);
});

test("자산별 차트 근거는 분석 보관함 JSON 왕복 뒤에도 좌표와 근거 기준이 불변이다", () => {
  const cryptoBars = bars.map((bar) => ({ ...bar, availableAtMs: bar.periodEndMs, ingestedAtMs: bar.periodEndMs + 1 }));
  const evidence = buildTechnicalChartEvidence({ snapshotId: "vault-1", provider: "upbit", symbol: "KRW-ETH", name: "이더리움", market: "coin", currency: "KRW", interval: "1d", adjusted: false, asOfMs: cryptoBars[79].ingestedAtMs!, bars: cryptoBars });
  assert.ok(evidence);
  const replay = JSON.parse(JSON.stringify(evidence));
  assert.deepEqual(replay.annotations, evidence.annotations);
  assert.equal(replay.sourceSnapshotId, "vault-1");
  assert.equal(replay.schemaVersion, "2.0");
});

test("포트폴리오 차트 묶음은 종목별 선을 보존하고 동일 스냅샷을 중복 저장하지 않는다", () => {
  const snapshot = { snapshotId: "portfolio-1", provider: "toss", symbol: "000880", name: "한화", market: "korea", currency: "KRW", interval: "1d", adjusted: true, asOfMs: bars[79].periodEndMs, bars, indicators: { rsi14: 61.2, macdLine: 12, macdSignal: 10, bollingerMiddle: 10_500, twentyDayReturnVolatilityPercent: 1.8 } };
  const evidence = buildTechnicalChartEvidenceCollection([snapshot, snapshot]);
  assert.equal(evidence.length, 1);
  assert.equal(evidence[0].symbol, "000880");
  assert.equal(evidence[0].indicators?.macdLine, 12);
  assert.deepEqual(evidence[0].annotations.map((item) => item.kind), ["horizontal_line", "horizontal_line", "trend_line", "rectangle"]);
});
