import assert from "node:assert/strict";
import test from "node:test";
import { selectAnalysisSnapshotCommand } from "../src/analysisSnapshotRouting.ts";

test("분석 요청은 자산군별 공개 스냅샷 명령으로 결정론적으로 분리한다", () => {
  assert.equal(selectAnalysisSnapshotCommand("SK하이닉스 기술 분석"), "toss_analysis_snapshot");
  assert.equal(selectAnalysisSnapshotCommand("KRW-BTC 일봉 분석"), "upbit_analysis_snapshot");
  assert.equal(selectAnalysisSnapshotCommand("비트코인 현물 추세"), "upbit_analysis_snapshot");
  assert.equal(selectAnalysisSnapshotCommand("BTCUSDT 4시간봉 무기한 선물"), "binance_perpetual_analysis_snapshot");
  assert.equal(selectAnalysisSnapshotCommand("코인 선물 청산 위험"), "binance_perpetual_analysis_snapshot");
  assert.equal(selectAnalysisSnapshotCommand("KIS 지수선물 101W09 일봉 분석"), "kis_futures_analysis_snapshot");
  assert.equal(selectAnalysisSnapshotCommand("증권 선물 계약 분석"), "kis_futures_analysis_snapshot");
});
