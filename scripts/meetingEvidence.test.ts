import assert from "node:assert/strict";
import test from "node:test";
import { analysisEvidenceId, invalidReportEvidenceIds, positionEvidenceForSymbol, SHADOW_RUNTIME_EVIDENCE, telegramEvidenceId } from "../src/meetingEvidence.ts";

test("회의 포지션 근거는 해당 종목만 포함하고 계좌 식별정보를 만들지 않는다", () => {
  const evidence = positionEvidenceForSymbol("toss-000880-100", "000880", {
    provider: "TOSS_OPEN_API",
    fetchedAtMs: 200,
    readOnly: true,
    liveOrderEnabled: false,
    accounts: [
      { holdings: { items: [
        { symbol: "000880", name: "한화", marketCountry: "KR", currency: "KRW", quantity: "3", lastPrice: "42000", averagePurchasePrice: "39000" },
        { symbol: "005930", name: "삼성전자", marketCountry: "KR", currency: "KRW", quantity: "1", lastPrice: "70000", averagePurchasePrice: "65000" },
      ] } },
      { holdings: { items: [
        { symbol: "000880", name: "한화", marketCountry: "KR", currency: "KRW", quantity: "2", lastPrice: "42000", averagePurchasePrice: "41000" },
      ] } },
    ],
  });

  assert.equal(evidence.length, 2);
  assert.deepEqual(evidence.map((item) => item.evidenceId), ["toss-000880-100-position-1", "toss-000880-100-position-2"]);
  assert.ok(evidence.every((item) => item.symbol === "000880" && item.readOnly));
  assert.ok(evidence.every((item) => !("maskedAccountNo" in item) && !("accountAlias" in item)));
});

test("회의 근거 ID와 SHADOW ONLY 경계는 결정론적으로 고정된다", () => {
  assert.equal(analysisEvidenceId("toss-BRK.B-100", "technical"), "toss-brk-b-100-technical");
  assert.equal(telegramEvidenceId(55, 1_000, 0), "telegram-55-1000-1");
  assert.deepEqual(SHADOW_RUNTIME_EVIDENCE, {
    evidenceId: "runtime-shadow-only-v1",
    mode: "SHADOW_ONLY",
    liveOrderAllowed: false,
    internalPaperCandidateAllowed: true,
  });
});

test("부서 보고가 전달받지 않은 근거 ID를 만들면 검출한다", () => {
  assert.deepEqual(invalidReportEvidenceIds([
    { evidenceIds: ["snapshot-price", "snapshot-technical"] },
    { evidenceIds: ["snapshot-price", "invented-news-id"] },
  ], ["snapshot-price", "snapshot-technical"]), ["invented-news-id"]);
  assert.deepEqual(invalidReportEvidenceIds([{ evidenceIds: [] }], []), []);
});
