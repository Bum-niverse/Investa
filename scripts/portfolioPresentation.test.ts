import assert from "node:assert/strict";
import test from "node:test";
import { buildPortfolioCurrencyGroups } from "../src/portfolioPresentation.ts";
import type { MeetingPositionEvidence } from "../src/meetingEvidence.ts";

const position = (overrides: Partial<MeetingPositionEvidence>): MeetingPositionEvidence => ({
  evidenceId: "position-1",
  provider: "TOSS_OPEN_API",
  observedAtMs: 100,
  readOnly: true,
  symbol: "000880",
  name: "한화",
  marketCountry: "KR",
  currency: "KRW",
  quantity: "2",
  lastPrice: "50000",
  averagePurchasePrice: "40000",
  ...overrides,
});

test("실계좌 보유자산은 통화별로 분리하고 평가손익과 비중을 계산한다", () => {
  const groups = buildPortfolioCurrencyGroups([
    position({}),
    position({ evidenceId: "position-2", symbol: "NVDA", name: "NVIDIA", marketCountry: "US", currency: "USD", quantity: "1", lastPrice: "200", averagePurchasePrice: "150" }),
  ], [
    { evidenceId: "cash-1", currency: "KRW", cashBuyingPower: "300000", observedAtMs: 100, provider: "TOSS_OPEN_API", readOnly: true },
  ]);
  assert.deepEqual(groups.map((group) => group.currency), ["KRW", "USD"]);
  assert.equal(groups[0].marketValue, 100000);
  assert.equal(groups[0].profitLoss, 20000);
  assert.equal(groups[0].buyingPower, 300000);
  assert.equal(groups[0].slices[0].weightBps, 10000);
  assert.equal(groups[1].buyingPower, null);
});

test("같은 통화 종목이 많으면 상위 5개와 기타로 시각화하되 합계를 보존한다", () => {
  const positions = Array.from({ length: 7 }, (_, index) => position({
    evidenceId: `position-${index}`,
    symbol: `S${index}`,
    name: `종목 ${index}`,
    quantity: "1",
    lastPrice: String(700 - index * 100),
    averagePurchasePrice: String(600 - index * 80),
  }));
  const [group] = buildPortfolioCurrencyGroups(positions);
  assert.equal(group.slices.length, 6);
  assert.equal(group.slices[5].isOther, true);
  assert.equal(group.slices.reduce((sum, slice) => sum + slice.marketValue, 0), group.marketValue);
});

test("잘못된 숫자와 음수 수량은 자산 비중에서 제외한다", () => {
  const [group] = buildPortfolioCurrencyGroups([
    position({ evidenceId: "bad", quantity: "not-a-number" }),
    position({ evidenceId: "negative", symbol: "NEG", quantity: "-1" }),
  ], [{ evidenceId: "cash", currency: "KRW", cashBuyingPower: "1000", observedAtMs: 100, provider: "TOSS_OPEN_API", readOnly: true }]);
  assert.equal(group.marketValue, 0);
  assert.equal(group.slices.length, 0);
  assert.equal(group.buyingPower, 1000);
});

test("보유분과 매수 가능 금액이 모두 0인 통화는 빈 카드로 만들지 않는다", () => {
  const groups = buildPortfolioCurrencyGroups([], [
    { evidenceId: "cash", currency: "USD", cashBuyingPower: "0", observedAtMs: 100, provider: "TOSS_OPEN_API", readOnly: true },
  ]);
  assert.deepEqual(groups, []);
});
