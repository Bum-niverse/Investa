import assert from "node:assert/strict";
import test from "node:test";
import { forecastAssetMarket, inferAnalysisMarket } from "../src/analysisMarket.ts";

test("안건 문구에서 증권 선물과 코인 선물을 분리한다", () => {
  assert.equal(inferAnalysisMarket("KOSPI200 지수선물 진입 분석"), "securities_futures");
  assert.equal(inferAnalysisMarket("BTCUSDT 무기한 선물 숏 전략"), "crypto_futures");
});

test("현물 시장과 복합 안건을 보수적으로 분류한다", () => {
  assert.equal(inferAnalysisMarket("하이닉스 실적 분석"), "kr");
  assert.equal(inferAnalysisMarket("애플과 삼성전자 비교"), "mixed");
  assert.equal(inferAnalysisMarket("근거를 검토해줘"), "mixed");
});

test("예측 자산 계약을 분석 필터에 매핑한다", () => {
  assert.equal(forecastAssetMarket("equity_future"), "securities_futures");
  assert.equal(forecastAssetMarket("crypto_perpetual"), "crypto_futures");
});
