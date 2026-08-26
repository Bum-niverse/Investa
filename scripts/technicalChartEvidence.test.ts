import assert from "node:assert/strict";
import test from "node:test";
import { buildTechnicalChartAnnotations, buildTechnicalChartEvidence, type TechnicalChartBar } from "../src/technicalChartEvidence.ts";

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
