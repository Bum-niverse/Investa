import assert from "node:assert/strict";
import test from "node:test";
import { buildMeetingBacktestReport, parseMeetingStrategy } from "../src/meetingBacktest.ts";

test("회의 전략 문장은 지원 플러그인 계약으로만 변환한다", () => {
  assert.deepEqual(parseMeetingStrategy("5/20 이동평균 교차").entry, {
    type: "moving_average_cross", fastWindow: 5, slowWindow: 20, direction: "above",
  });
  assert.deepEqual(parseMeetingStrategy("20봉 가격 채널 돌파").exit, {
    type: "price_channel_breakout", lookback: 20, direction: "below",
  });
  assert.throws(() => parseMeetingStrategy("좋아 보이면 적당히 매수"), /지원 전략 형식/);
});

test("회의 백테스트 보고서는 로컬 분석 계보와 SHADOW 한계를 보존한다", () => {
  const report = buildMeetingBacktestReport({
    workflowJobId: "meeting-100",
    topic: "한화 포지션 분석",
    analysisRecordId: "analysis-meeting-100",
    symbol: "000880",
    strategy: "5/20 이동평균 교차",
    market: "korea",
    currency: "KRW",
  });
  assert.equal(report.evidence[0].kind, "local_analysis");
  assert.equal(report.evidence[0].sourceUrl, "investa://analysis/analysis-meeting-100");
  assert.equal(report.strategyCandidate.symbol, "000880");
  assert.match(report.strategyCandidate.limitations.join(" "), /신호가 없으면 섀도우 감시/);
});

test("지원하지 않는 선물 시장과 불명확 종목은 자동 승격하지 않는다", () => {
  const base = {
    workflowJobId: "meeting-101", topic: "검증", analysisRecordId: "analysis-meeting-101",
    symbol: "000880", strategy: "5/20 이동평균 교차", market: "securities_futures", currency: "KRW",
  };
  assert.throws(() => buildMeetingBacktestReport(base), /국장·미장·코인 현물/);
  assert.throws(() => buildMeetingBacktestReport({ ...base, market: "korea", symbol: "한화" }), /종목 코드/);
});
