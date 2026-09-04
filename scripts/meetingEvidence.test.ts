import assert from "node:assert/strict";
import test from "node:test";
import { analysisEvidenceId, analysisRecordTitle, buyingPowerEvidence, invalidReportEvidenceIds, isHoldingsAnalysisRequest, portfolioPositionEvidence, positionEvidenceForSymbol, resolveHoldingsAnalysisRequest, SHADOW_RUNTIME_EVIDENCE, telegramEvidenceId } from "../src/meetingEvidence.ts";

const accountSnapshot = (items: Array<{ symbol: string; name: string }>) => ({
  provider: "TOSS_OPEN_API",
  fetchedAtMs: 200,
  readOnly: true,
  liveOrderEnabled: false,
  accounts: [{ holdings: { items: items.map((item) => ({
    ...item,
    marketCountry: "KR",
    currency: "KRW",
    quantity: "3",
    lastPrice: "42000",
    averagePurchasePrice: "39000",
  })) } }],
});

test("보유종목 자연어 요청은 단일 보유종목 코드로 안전하게 해석한다", () => {
  const request = "내가 보유하고 있는 종목을 분석해줘";
  assert.equal(isHoldingsAnalysisRequest(request), true);
  const resolution = resolveHoldingsAnalysisRequest(request, accountSnapshot([{ symbol: "000880", name: "한화" }]));
  assert.equal(resolution.status, "resolved");
  assert.match(resolution.query ?? "", /한화 \(000880\)/);
});

test("여러 보유종목은 임의 선택 없이 전체 포트폴리오 분석 질의로 만든다", () => {
  const resolution = resolveHoldingsAnalysisRequest("내 보유 종목 분석", accountSnapshot([
    { symbol: "000880", name: "한화" },
    { symbol: "005930", name: "삼성전자" },
  ]));
  assert.equal(resolution.status, "portfolio");
  assert.deepEqual(resolution.queries, ["한화 (000880)", "삼성전자 (005930)"]);
  assert.equal(resolution.query, undefined);
});

test("긴 회의 요청은 전문을 바꾸지 않고 저장 제목만 240자로 축약한다", () => {
  const request = `전체 포트폴리오를 분석해 ${"가".repeat(260)}`;
  const title = analysisRecordTitle(request);
  assert.equal(Array.from(title).length, 240);
  assert.equal(title.endsWith("…"), true);
  assert.equal(request.length > title.length, true);
});

test("계좌 매수 가능 금액은 계좌 식별자 없는 근거로 변환한다", () => {
  const evidence = buyingPowerEvidence({
    ...accountSnapshot([]),
    accounts: [{
      holdings: { items: [] },
      buyingPower: [{ currency: "KRW", cashBuyingPower: "1250000" }],
      buyingPowerErrors: [],
    }],
  });
  assert.equal(evidence.length, 1);
  assert.equal(evidence[0].currency, "KRW");
  assert.equal(evidence[0].cashBuyingPower, "1250000");
  assert.ok(!("accountAlias" in evidence[0]) && !("maskedAccountNo" in evidence[0]));
});

test("안전 상한을 넘는 보유종목은 일부만 몰래 분석하지 않는다", () => {
  const resolution = resolveHoldingsAnalysisRequest("내 보유 종목 분석", accountSnapshot(Array.from({ length: 21 }, (_, index) => ({
    symbol: String(index).padStart(6, "0"), name: `종목${index}`,
  }))));
  assert.equal(resolution.status, "too_many");
  assert.equal(resolution.queries, undefined);
  assert.match(resolution.message ?? "", /안전 상한 20개/);
});

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

test("원장용 전체 포지션 근거도 계좌 식별정보를 제거한다", () => {
  const evidence = portfolioPositionEvidence({
    ...accountSnapshot([]),
    accounts: [{
      accountAlias: "노출 금지",
      maskedAccountNo: "1234****",
      holdings: { items: [{ symbol: "000880", name: "한화", marketCountry: "KR", currency: "KRW", quantity: "3", lastPrice: "42000", averagePurchasePrice: "39000" }] },
    }],
  });
  assert.equal(evidence.length, 1);
  assert.equal(evidence[0].symbol, "000880");
  assert.ok(!("accountAlias" in evidence[0]) && !("maskedAccountNo" in evidence[0]));
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
